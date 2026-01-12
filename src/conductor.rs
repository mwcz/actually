use crate::session::{ClaudeSession, SessionResult};
use crate::strategy::{build_implementation_prompt, build_strategy_prompt, parse_strategy};
use crate::workspace::Workspace;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use futures::future::join_all;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, List, ListItem, ListState, Paragraph, Wrap},
};
use std::io::{stdout, Write};
use std::path::Path;
use std::process::Command;
use tempfile::NamedTempFile;

#[derive(Debug, Clone)]
pub struct InstanceResult {
    pub instance_id: usize,
    pub strategy: String,
    pub workspace_path: String,
    pub success: bool,
    pub error: Option<String>,
    pub transcript: String,
}

#[derive(Debug, Clone)]
struct StrategyInfo {
    strategy: String,
    transcript: String,
    failed: bool,
    error: Option<String>,
    manually_edited: bool,
}

pub async fn run(
    prompt: &str,
    n: usize,
    run_dir: &Path,
    dry_run: bool,
    interactive: bool,
) -> anyhow::Result<Vec<InstanceResult>> {
    let mut strategy_infos: Vec<StrategyInfo> = Vec::with_capacity(n);

    // Phase 1: Sequential strategy collection
    if interactive {
        println!("Phase 1: Collecting strategies from {} instances", n);
    } else {
        tracing::info!("Phase 1: Collecting strategies from {} instances", n);
    }

    for i in 0..n {
        if interactive {
            println!("  Extracting strategy for C{}...", i);
        } else {
            tracing::info!(instance = i, "Extracting strategy for C{}", i);
        }

        let existing_strategies: Vec<String> = strategy_infos
            .iter()
            .filter(|s| !s.failed)
            .map(|s| s.strategy.clone())
            .collect();

        let strategy_prompt = build_strategy_prompt(prompt, &existing_strategies);

        if dry_run {
            println!("\n=== DRY RUN: Strategy prompt for C{} ===", i);
            println!("{}", strategy_prompt);
            println!("=== END PROMPT ===\n");

            strategy_infos.push(StrategyInfo {
                strategy: format!("[DRY RUN] Strategy {} would be generated here", i),
                transcript: strategy_prompt,
                failed: false,
                error: None,
                manually_edited: false,
            });
            continue;
        }

        let session = ClaudeSession::new();

        match session.query_strategy(&strategy_prompt).await {
            Ok(response) => {
                let strategy = parse_strategy(&response);
                if interactive {
                    println!("  C{}: {}", i, truncate_for_log(&strategy, 60));
                } else {
                    tracing::info!(instance = i, strategy = %strategy, "Strategy extracted");
                }

                strategy_infos.push(StrategyInfo {
                    strategy,
                    transcript: response,
                    failed: false,
                    error: None,
                    manually_edited: false,
                });
            }
            Err(e) => {
                let error_msg = format!("Failed to extract strategy: {}", e);
                eprintln!("ERROR [C{}]: {}", i, error_msg);
                if !interactive {
                    tracing::error!(instance = i, error = %e, "Failed to extract strategy");
                }

                strategy_infos.push(StrategyInfo {
                    strategy: format!("[FAILED] {}", error_msg),
                    transcript: format!("Error: {}", e),
                    failed: true,
                    error: Some(error_msg),
                    manually_edited: false,
                });
            }
        }
    }

    // Interactive strategy review
    if interactive && !dry_run {
        println!();
        strategy_infos = interactive_strategy_review(prompt, strategy_infos).await?;
    }

    if dry_run {
        println!(
            "\n=== DRY RUN: Implementation phase would launch {} parallel instances ===",
            n
        );
        for (i, info) in strategy_infos.iter().enumerate() {
            let excluded: Vec<String> = strategy_infos
                .iter()
                .enumerate()
                .filter(|(idx, s)| *idx != i && !s.failed)
                .map(|(_, s)| s.strategy.clone())
                .collect();

            let impl_prompt = build_implementation_prompt(prompt, &info.strategy, &excluded);
            println!("\n=== DRY RUN: Implementation prompt for C{} ===", i);
            println!("{}", impl_prompt);
            println!("=== END PROMPT ===");
        }

        return Ok(strategy_infos
            .into_iter()
            .enumerate()
            .map(|(i, info)| InstanceResult {
                instance_id: i,
                strategy: info.strategy,
                workspace_path: String::new(),
                success: true,
                error: None,
                transcript: info.transcript,
            })
            .collect());
    }

    if interactive {
        println!("Phase 2: Launching {} parallel implementations", n);
    } else {
        tracing::info!("Phase 2: Launching {} parallel implementations", n);
    }

    // Phase 2: Parallel execution
    let handles: Vec<_> = strategy_infos
        .iter()
        .enumerate()
        .map(|(i, info)| {
            let prompt = prompt.to_string();
            let strategy = info.strategy.clone();
            let strategy_transcript = info.transcript.clone();
            let failed = info.failed;
            let strategy_error = info.error.clone();

            let excluded: Vec<String> = strategy_infos
                .iter()
                .enumerate()
                .filter(|(idx, s)| *idx != i && !s.failed)
                .map(|(_, s)| s.strategy.clone())
                .collect();
            let run_dir = run_dir.to_path_buf();

            tokio::spawn(async move {
                if failed {
                    return InstanceResult {
                        instance_id: i,
                        strategy,
                        workspace_path: String::new(),
                        success: false,
                        error: strategy_error,
                        transcript: strategy_transcript,
                    };
                }
                run_instance(i, &prompt, &strategy, &strategy_transcript, &excluded, &run_dir)
                    .await
            })
        })
        .collect();

    let results: Vec<InstanceResult> = join_all(handles)
        .await
        .into_iter()
        .enumerate()
        .map(|(i, r)| match r {
            Ok(result) => result,
            Err(e) => InstanceResult {
                instance_id: i,
                strategy: strategy_infos
                    .get(i)
                    .map(|s| s.strategy.clone())
                    .unwrap_or_default(),
                workspace_path: String::new(),
                success: false,
                error: Some(format!("Task join error: {}", e)),
                transcript: String::new(),
            },
        })
        .collect();

    let succeeded = results.iter().filter(|r| r.success).count();
    let failed_count = results.iter().filter(|r| !r.success).count();

    if interactive {
        println!("Complete: {} succeeded, {} failed", succeeded, failed_count);
    } else {
        tracing::info!(succeeded, failed = failed_count, "Claudissent complete");
    }

    for result in &results {
        if result.success {
            if interactive {
                println!(
                    "  C{}: {} ({})",
                    result.instance_id,
                    truncate_for_log(&result.strategy, 40),
                    result.workspace_path
                );
            } else {
                tracing::info!(
                    instance = result.instance_id,
                    workspace = %result.workspace_path,
                    strategy = %result.strategy,
                    "Instance succeeded"
                );
            }
        } else if !interactive {
            tracing::error!(
                instance = result.instance_id,
                error = ?result.error,
                "Instance failed"
            );
        }
    }

    Ok(results)
}

fn truncate_for_log(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    } else {
        s.to_string()
    }
}

/// Convert markdown text to ratatui styled Text with syntax highlighting
fn markdown_to_styled_text(md: &str) -> Text<'static> {
    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut in_code_block = false;

    for line in md.lines() {
        let trimmed = line.trim();

        // Code block toggle
        if trimmed.starts_with("```") {
            in_code_block = !in_code_block;
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::DarkGray),
            )));
            continue;
        }

        // Inside code block
        if in_code_block {
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default().fg(Color::LightYellow),
            )));
            continue;
        }

        // Headers
        if trimmed.starts_with("### ") {
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default()
                    .fg(Color::Yellow)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if trimmed.starts_with("## ") {
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default()
                    .fg(Color::Magenta)
                    .add_modifier(Modifier::BOLD),
            )));
        } else if trimmed.starts_with("# ") {
            lines.push(Line::from(Span::styled(
                line.to_string(),
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )));
        }
        // Bullet points
        else if trimmed.starts_with("- ") || trimmed.starts_with("* ") {
            let bullet = &line[..line.find(|c| c == '-' || c == '*').unwrap() + 2];
            let rest = &line[line.find(|c| c == '-' || c == '*').unwrap() + 2..];
            lines.push(Line::from(vec![
                Span::styled(bullet.to_string(), Style::default().fg(Color::Blue)),
                Span::styled(rest.to_string(), Style::default().fg(Color::White)),
            ]));
        }
        // Numbered lists
        else if trimmed.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false)
            && trimmed.contains(". ")
        {
            if let Some(dot_pos) = trimmed.find(". ") {
                let prefix_len = line.len() - trimmed.len();
                let num_part = &line[..prefix_len + dot_pos + 2];
                let rest = &line[prefix_len + dot_pos + 2..];
                lines.push(Line::from(vec![
                    Span::styled(num_part.to_string(), Style::default().fg(Color::Blue)),
                    Span::styled(rest.to_string(), Style::default().fg(Color::White)),
                ]));
            } else {
                lines.push(Line::from(line.to_string()));
            }
        }
        // Regular text with inline formatting (code, bold)
        else {
            lines.push(parse_inline_formatting(line));
        }
    }

    Text::from(lines)
}

/// Parse inline formatting: `code` and **bold**
fn parse_inline_formatting(line: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut chars = line.chars().peekable();
    let mut current_text = String::new();
    let mut in_code = false;
    let mut in_bold = false;

    while let Some(c) = chars.next() {
        // Check for ** (bold)
        if c == '*' && chars.peek() == Some(&'*') && !in_code {
            chars.next(); // consume second *

            // Flush current text
            if !current_text.is_empty() {
                let style = if in_bold {
                    Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
                } else {
                    Style::default().fg(Color::Gray)
                };
                spans.push(Span::styled(std::mem::take(&mut current_text), style));
            }
            in_bold = !in_bold;
        }
        // Check for ` (inline code)
        else if c == '`' && !in_bold {
            // Flush current text
            if !current_text.is_empty() {
                let style = if in_code {
                    Style::default().fg(Color::LightYellow)
                } else {
                    Style::default().fg(Color::Gray)
                };
                spans.push(Span::styled(std::mem::take(&mut current_text), style));
            }
            in_code = !in_code;
        }
        else {
            current_text.push(c);
        }
    }

    // Flush remaining text
    if !current_text.is_empty() {
        let style = if in_code {
            Style::default().fg(Color::LightYellow)
        } else if in_bold {
            Style::default().fg(Color::White).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(Color::Gray)
        };
        spans.push(Span::styled(current_text, style));
    }

    if spans.is_empty() {
        Line::from("")
    } else {
        Line::from(spans)
    }
}

/// Interactive strategy review using ratatui TUI
async fn interactive_strategy_review(
    prompt: &str,
    mut strategy_infos: Vec<StrategyInfo>,
) -> anyhow::Result<Vec<StrategyInfo>> {
    let n = strategy_infos.len();

    // Setup terminal
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut list_state = ListState::default();
    list_state.select(Some(n)); // Default to Accept option

    let mut status_message: Option<String> = None;

    loop {
        let selected_idx = list_state.selected().unwrap_or(n);

        // Draw UI
        terminal.draw(|frame| {
            let area = frame.area();

            // Determine if we have enough width for preview panel (min 80 cols for preview)
            let show_preview = area.width >= 100;

            let main_chunks = if show_preview {
                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
                    .split(area)
            } else {
                Layout::default()
                    .direction(Direction::Horizontal)
                    .constraints([Constraint::Percentage(100)])
                    .split(area)
            };

            let left_chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Length(2), // Title
                    Constraint::Min(5),    // List
                    Constraint::Length(2), // Help
                    Constraint::Length(1), // Status
                ])
                .split(main_chunks[0]);

            // Title
            let title = Paragraph::new("STRATEGY REVIEW")
                .style(Style::default().fg(Color::LightYellow).add_modifier(Modifier::BOLD));
            frame.render_widget(title, left_chunks[0]);

            // Build list items (truncated for list view)
            let list_width = left_chunks[1].width.saturating_sub(15) as usize; // Account for prefix
            let mut items: Vec<ListItem> = strategy_infos
                .iter()
                .enumerate()
                .map(|(i, info)| {
                    let (status, status_style) = if info.failed {
                        ("[FAIL]", Style::default().fg(Color::Red))
                    } else if info.manually_edited {
                        ("[EDIT]", Style::default().fg(Color::Yellow))
                    } else {
                        ("[OK]", Style::default().fg(Color::Green))
                    };

                    // Truncate strategy for list display
                    let strategy_display = if info.strategy.len() > list_width {
                        format!("{}…", &info.strategy[..list_width.saturating_sub(1)])
                    } else {
                        info.strategy.clone()
                    };

                    let line = Line::from(vec![
                        Span::styled(format!("C{} ", i), Style::default().add_modifier(Modifier::BOLD)),
                        Span::styled(status, status_style),
                        Span::raw(" "),
                        Span::raw(strategy_display),
                    ]);

                    ListItem::new(line)
                })
                .collect();

            // Add Accept option
            items.push(ListItem::new(Line::from(vec![
                Span::styled(
                    ">>> Accept all and proceed <<<",
                    Style::default()
                        .fg(Color::Green)
                        .add_modifier(Modifier::BOLD),
                ),
            ])));

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title("Strategies"))
                .highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("▶ ");

            frame.render_stateful_widget(list, left_chunks[1], &mut list_state);

            // Help text
            let help = Paragraph::new("↑/k ↓/j: Navigate | Enter: Edit/Accept | q: Quit")
                .style(Style::default().fg(Color::DarkGray));
            frame.render_widget(help, left_chunks[2]);

            // Status message
            if let Some(ref msg) = status_message {
                let status = Paragraph::new(msg.as_str())
                    .style(Style::default().fg(Color::Yellow));
                frame.render_widget(status, left_chunks[3]);
            }

            // Preview panel (if showing)
            if show_preview {
                let preview_title = if selected_idx < n {
                    format!(" C{} Preview ", selected_idx)
                } else {
                    " Preview ".to_string()
                };

                let preview_text = if selected_idx < n {
                    let info = &strategy_infos[selected_idx];

                    // Build status line with explicit color
                    let status_line = if info.failed {
                        Line::from(Span::styled(
                            "Status: FAILED",
                            Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                        ))
                    } else if info.manually_edited {
                        Line::from(Span::styled(
                            "Status: EDITED",
                            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
                        ))
                    } else {
                        Line::from(Span::styled(
                            "Status: OK",
                            Style::default().fg(Color::Green).add_modifier(Modifier::BOLD),
                        ))
                    };

                    // Build preview with colored status + markdown-rendered strategy
                    let mut text_lines = vec![status_line, Line::from("")];
                    let strategy_text = markdown_to_styled_text(&info.strategy);
                    text_lines.extend(strategy_text.lines.into_iter().map(|l| l.to_owned()));
                    Text::from(text_lines)
                } else {
                    Text::from("Select a strategy to preview, or press Enter to accept all.")
                };

                let preview = Paragraph::new(preview_text)
                    .block(Block::default().borders(Borders::ALL).title(preview_title))
                    .wrap(Wrap { trim: false });

                frame.render_widget(preview, main_chunks[1]);
            }
        })?;

        // Handle input
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    status_message = None; // Clear status on any keypress

                    // Handle Ctrl+C
                    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                        disable_raw_mode()?;
                        stdout().execute(LeaveAlternateScreen)?;
                        return Ok(vec![]);
                    }

                    match key.code {
                        KeyCode::Char('q') | KeyCode::Esc => {
                            // Cleanup and exit
                            disable_raw_mode()?;
                            stdout().execute(LeaveAlternateScreen)?;
                            return Ok(vec![]); // Return empty to signal quit
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            let selected = list_state.selected().unwrap_or(0);
                            let new_selected = if selected == 0 { n } else { selected - 1 };
                            list_state.select(Some(new_selected));
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            let selected = list_state.selected().unwrap_or(0);
                            let new_selected = if selected >= n { 0 } else { selected + 1 };
                            list_state.select(Some(new_selected));
                        }
                        KeyCode::Enter => {
                            let selected = list_state.selected().unwrap_or(n);

                            if selected == n {
                                // Accept selected - exit loop
                                break;
                            }

                            // Edit strategy - need to exit TUI temporarily
                            disable_raw_mode()?;
                            stdout().execute(LeaveAlternateScreen)?;

                            let idx = selected;
                            let original_strategy = strategy_infos[idx].strategy.clone();

                            match edit_strategy_in_editor(&original_strategy) {
                                Ok(Some(edited_strategy)) if edited_strategy != original_strategy => {
                                    println!("Strategy modified for C{}, creating new agent...", idx);

                                    match create_agent_with_edited_strategy(
                                        prompt,
                                        &strategy_infos,
                                        idx,
                                        &edited_strategy,
                                    )
                                    .await
                                    {
                                        Ok(new_info) => {
                                            strategy_infos[idx] = new_info;
                                            status_message = Some(format!("C{} strategy updated", idx));
                                        }
                                        Err(e) => {
                                            status_message = Some(format!("Error: {}", e));
                                        }
                                    }
                                }
                                Ok(_) => {
                                    status_message = Some("Strategy unchanged".to_string());
                                }
                                Err(e) => {
                                    status_message = Some(format!("Editor error: {}", e));
                                }
                            }

                            // Re-enter TUI
                            enable_raw_mode()?;
                            stdout().execute(EnterAlternateScreen)?;
                            terminal.clear()?;
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    // Cleanup
    disable_raw_mode()?;
    stdout().execute(LeaveAlternateScreen)?;

    Ok(strategy_infos)
}

/// Open a strategy in $EDITOR for editing
fn edit_strategy_in_editor(strategy: &str) -> anyhow::Result<Option<String>> {
    let editor = std::env::var("EDITOR").unwrap_or_else(|_| "vi".to_string());

    let mut temp_file = NamedTempFile::new()?;
    writeln!(temp_file, "# Edit the strategy below. Lines starting with # are ignored.")?;
    writeln!(temp_file, "# Save and exit to apply changes, or exit without saving to cancel.")?;
    writeln!(temp_file)?;
    writeln!(temp_file, "{}", strategy)?;
    temp_file.flush()?;

    let temp_path = temp_file.path().to_path_buf();
    let before_mtime = std::fs::metadata(&temp_path)?.modified()?;

    let status = Command::new(&editor).arg(&temp_path).status()?;

    if !status.success() {
        return Ok(None);
    }

    let after_mtime = std::fs::metadata(&temp_path)?.modified()?;
    if before_mtime == after_mtime {
        return Ok(None);
    }

    let content = std::fs::read_to_string(&temp_path)?;

    let edited: String = content
        .lines()
        .filter(|line| !line.trim_start().starts_with('#'))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string();

    if edited.is_empty() {
        return Ok(None);
    }

    Ok(Some(edited))
}

/// Create a fresh agent with an edited strategy
async fn create_agent_with_edited_strategy(
    prompt: &str,
    existing_infos: &[StrategyInfo],
    target_idx: usize,
    edited_strategy: &str,
) -> anyhow::Result<StrategyInfo> {
    let existing_strategies: Vec<String> = existing_infos
        .iter()
        .enumerate()
        .filter(|(i, s)| *i != target_idx && !s.failed)
        .map(|(_, s)| s.strategy.clone())
        .collect();

    let strategy_prompt = format!(
        r#"For the following task, you will use a specific implementation strategy that has been provided.

Task: {}

YOUR ASSIGNED STRATEGY (you must follow this exactly):
{}

{}

Confirm you understand by replying with:
STRATEGY: <restate the strategy in your own words>"#,
        prompt,
        edited_strategy,
        if existing_strategies.is_empty() {
            String::new()
        } else {
            format!(
                "Note: Other agents are using these approaches (for your awareness, not as constraints):\n{}",
                existing_strategies
                    .iter()
                    .enumerate()
                    .map(|(i, s)| format!("  {}. {}", i, s))
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        }
    );

    let session = ClaudeSession::new();

    match session.query_strategy(&strategy_prompt).await {
        Ok(response) => {
            let strategy = parse_strategy(&response);
            tracing::debug!(
                instance = target_idx,
                strategy = %strategy,
                "Agent created with edited strategy"
            );
            Ok(StrategyInfo {
                strategy: edited_strategy.to_string(),
                transcript: response,
                failed: false,
                error: None,
                manually_edited: true,
            })
        }
        Err(e) => {
            let error_msg = format!("Failed to create agent with edited strategy: {}", e);
            eprintln!("ERROR [C{}]: {}", target_idx, error_msg);
            Ok(StrategyInfo {
                strategy: format!("[FAILED] {}", error_msg),
                transcript: format!("Error: {}", e),
                failed: true,
                error: Some(error_msg),
                manually_edited: false,
            })
        }
    }
}

async fn run_instance(
    id: usize,
    prompt: &str,
    strategy: &str,
    strategy_transcript: &str,
    excluded_strategies: &[String],
    run_dir: &Path,
) -> InstanceResult {
    let workspace = match Workspace::create(run_dir, id) {
        Ok(ws) => ws,
        Err(e) => {
            return InstanceResult {
                instance_id: id,
                strategy: strategy.to_string(),
                workspace_path: String::new(),
                success: false,
                error: Some(format!("Failed to create workspace: {}", e)),
                transcript: String::new(),
            };
        }
    };

    let full_prompt = build_implementation_prompt(prompt, strategy, excluded_strategies);
    let session = ClaudeSession::with_cwd(workspace.path());

    match session.run_implementation(&full_prompt).await {
        Ok(SessionResult { transcript, success }) => {
            let full_transcript = format!(
                "=== STRATEGY SELECTION ===\n{}\n\n{}",
                strategy_transcript, transcript
            );
            InstanceResult {
                instance_id: id,
                strategy: strategy.to_string(),
                workspace_path: workspace.path().to_string_lossy().to_string(),
                success,
                error: if success {
                    None
                } else {
                    Some("Session reported failure".to_string())
                },
                transcript: full_transcript,
            }
        }
        Err(e) => InstanceResult {
            instance_id: id,
            strategy: strategy.to_string(),
            workspace_path: workspace.path().to_string_lossy().to_string(),
            success: false,
            error: Some(e.to_string()),
            transcript: format!(
                "=== STRATEGY SELECTION ===\n{}\n\n=== ERROR ===\n{}",
                strategy_transcript, e
            ),
        },
    }
}
