use crate::session::{ClaudeSession, SessionResult};
use crate::strategy::{
    build_implementation_prompt, build_strategy_prompt, parse_strategy, Strategy,
};
use crate::workspace::Workspace;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
    ExecutableCommand,
};
use futures::future::join_all;
use ratatui::{
    prelude::*,
    widgets::{Block, Borders, Clear, List, ListItem, ListState, Paragraph},
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
    strategy: Strategy,
    transcript: String,
    failed: bool,
    error: Option<String>,
    manually_edited: bool,
}

/// Result of a chat session with Claude about a strategy
enum ChatResult {
    NoChanges,
    RevisedStrategy(String),
    Error(String),
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
            .map(|s| s.strategy.markdown.clone())
            .collect();

        let strategy_prompt = build_strategy_prompt(prompt, &existing_strategies);

        if dry_run {
            println!("\n=== DRY RUN: Strategy prompt for C{} ===", i);
            println!("{}", strategy_prompt);
            println!("=== END PROMPT ===\n");

            strategy_infos.push(StrategyInfo {
                strategy: Strategy::parse(&format!(
                    "[DRY RUN] Strategy {} would be generated here",
                    i
                )),
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
                    println!("  C{}: {}", i, truncate_for_log(&strategy.markdown, 60));
                } else {
                    tracing::info!(instance = i, strategy = %strategy.markdown, "Strategy extracted");
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
                    strategy: Strategy::failed(&error_msg),
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
                .map(|(_, s)| s.strategy.markdown.clone())
                .collect();

            let impl_prompt =
                build_implementation_prompt(prompt, &info.strategy.markdown, &excluded);
            println!("\n=== DRY RUN: Implementation prompt for C{} ===", i);
            println!("{}", impl_prompt);
            println!("=== END PROMPT ===");
        }

        return Ok(strategy_infos
            .into_iter()
            .enumerate()
            .map(|(i, info)| InstanceResult {
                instance_id: i,
                strategy: info.strategy.markdown,
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
            let strategy = info.strategy.markdown.clone();
            let strategy_transcript = info.transcript.clone();
            let failed = info.failed;
            let strategy_error = info.error.clone();

            let excluded: Vec<String> = strategy_infos
                .iter()
                .enumerate()
                .filter(|(idx, s)| *idx != i && !s.failed)
                .map(|(_, s)| s.strategy.markdown.clone())
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
                run_instance(
                    i,
                    &prompt,
                    &strategy,
                    &strategy_transcript,
                    &excluded,
                    &run_dir,
                )
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
                    .map(|s| s.strategy.markdown.clone())
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
        tracing::info!(succeeded, failed = failed_count, "Contra complete");
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

/// Wrap a Line to fit within max_width, preserving styles
fn wrap_styled_line(line: Line<'static>, max_width: usize) -> Vec<Line<'static>> {
    if max_width == 0 {
        return vec![line];
    }

    let mut result: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = Vec::new();
    let mut current_width: usize = 0;

    for span in line.spans {
        let style = span.style;
        let content = span.content.into_owned();
        let mut remaining = content.as_str();

        while !remaining.is_empty() {
            let available = max_width.saturating_sub(current_width);

            if available == 0 {
                // Current line is full, start new line
                result.push(Line::from(std::mem::take(&mut current_spans)));
                current_width = 0;
                continue;
            }

            // Find a good break point
            let take_chars: usize = if remaining.chars().count() <= available {
                // Everything fits
                remaining.chars().count()
            } else {
                // Need to break - prefer breaking at space
                let chars: Vec<char> = remaining.chars().collect();
                let mut break_at = available;

                // Look for last space within available width
                for i in (0..available).rev() {
                    if chars.get(i) == Some(&' ') {
                        break_at = i + 1; // Include the space
                        break;
                    }
                }

                // If no space found, hard break at available; ensure at least 1 char
                if break_at == 0 {
                    1
                } else {
                    break_at
                }
            };

            let byte_end: usize = remaining
                .char_indices()
                .nth(take_chars)
                .map(|(i, _)| i)
                .unwrap_or(remaining.len());

            let (taken, rest) = remaining.split_at(byte_end);
            current_spans.push(Span::styled(taken.to_string(), style));
            current_width += take_chars;
            remaining = rest;

            // If we took less than available, we're done with this span
            if remaining.is_empty() {
                break;
            }

            // Otherwise, we need to wrap - finish current line
            result.push(Line::from(std::mem::take(&mut current_spans)));
            current_width = 0;
        }
    }

    // Don't forget remaining spans
    if !current_spans.is_empty() {
        result.push(Line::from(current_spans));
    }

    if result.is_empty() {
        result.push(Line::from(""));
    }

    result
}

/// Wrap all lines in a Text to fit within max_width
fn wrap_styled_text(text: Text<'static>, max_width: usize) -> Text<'static> {
    let wrapped_lines: Vec<Line<'static>> = text
        .lines
        .into_iter()
        .flat_map(|line| wrap_styled_line(line, max_width))
        .collect();
    Text::from(wrapped_lines)
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
            let bullet = &line[..line.find(['-', '*']).unwrap() + 2];
            let rest = &line[line.find(['-', '*']).unwrap() + 2..];
            lines.push(Line::from(vec![
                Span::styled(bullet.to_string(), Style::default().fg(Color::Blue)),
                Span::styled(rest.to_string(), Style::default().fg(Color::White)),
            ]));
        }
        // Numbered lists
        else if trimmed
            .chars()
            .next()
            .map(|c| c.is_ascii_digit())
            .unwrap_or(false)
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
/// - Bold (**) is NOT processed inside code blocks (** may be code syntax)
/// - Code (`) IS processed inside bold (allows bold text with code snippets)
fn parse_inline_formatting(line: &str) -> Line<'static> {
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut chars = line.chars().peekable();
    let mut current_text = String::new();
    let mut in_code = false;
    let mut in_bold = false;

    // Helper to build style based on current state
    let make_style = |in_code: bool, in_bold: bool| -> Style {
        match (in_code, in_bold) {
            (true, true) => Style::default()
                .fg(Color::LightYellow)
                .add_modifier(Modifier::BOLD),
            (true, false) => Style::default().fg(Color::LightYellow),
            (false, true) => Style::default()
                .fg(Color::White)
                .add_modifier(Modifier::BOLD),
            (false, false) => Style::default().fg(Color::Gray),
        }
    };

    while let Some(c) = chars.next() {
        // Check for ** (bold) - only when NOT in code
        if c == '*' && chars.peek() == Some(&'*') && !in_code {
            chars.next(); // consume second *

            // Flush current text
            if !current_text.is_empty() {
                spans.push(Span::styled(
                    std::mem::take(&mut current_text),
                    make_style(in_code, in_bold),
                ));
            }
            in_bold = !in_bold;
        }
        // Check for ` (inline code) - always process
        else if c == '`' {
            // Flush current text
            if !current_text.is_empty() {
                spans.push(Span::styled(
                    std::mem::take(&mut current_text),
                    make_style(in_code, in_bold),
                ));
            }
            in_code = !in_code;
        } else {
            current_text.push(c);
        }
    }

    // Flush remaining text
    if !current_text.is_empty() {
        spans.push(Span::styled(current_text, make_style(in_code, in_bold)));
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
    // Setup terminal
    enable_raw_mode()?;
    stdout().execute(EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout()))?;

    let mut list_state = ListState::default();
    list_state.select(Some(strategy_infos.len())); // Default to Accept option

    let mut status_message: Option<String> = None;
    let mut clipboard = arboard::Clipboard::new().ok();
    let mut show_help_popup = false;

    loop {
        let n = strategy_infos.len();
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
                    Constraint::Min(5),    // List
                    Constraint::Length(1), // Help hint
                    Constraint::Length(1), // Status
                ])
                .split(main_chunks[0]);

            // Build list items (truncated for list view)
            let list_width = left_chunks[0].width.saturating_sub(15) as usize; // Account for prefix
            let mut items: Vec<ListItem> = strategy_infos
                .iter()
                .enumerate()
                .map(|(i, info)| {
                    // Only show status for failed/edited, not OK
                    let status_spans: Vec<Span> = if info.failed {
                        vec![
                            Span::styled("[FAIL]", Style::default().fg(Color::Red)),
                            Span::raw(" "),
                        ]
                    } else if info.manually_edited {
                        vec![
                            Span::styled("[EDIT]", Style::default().fg(Color::Yellow)),
                            Span::raw(" "),
                        ]
                    } else {
                        vec![]
                    };

                    // Show strategy highlights or truncated raw text
                    let strategy_display = if !info.strategy.highlights.is_empty() {
                        info.strategy.highlights.join(" · ")
                    } else if info.strategy.raw.len() > list_width {
                        format!("{}…", &info.strategy.raw[..list_width.saturating_sub(1)])
                    } else {
                        info.strategy.raw.clone()
                    };

                    let mut spans = vec![Span::styled(
                        format!("C{} ", i),
                        Style::default().fg(Color::Cyan),
                    )];
                    spans.extend(status_spans);
                    spans.push(Span::raw(strategy_display));

                    ListItem::new(Line::from(spans))
                })
                .collect();

            // Add Accept option
            items.push(ListItem::new(Line::from(vec![Span::styled(
                ">>> Accept all and proceed <<<",
                Style::default()
                    .fg(Color::Green)
                    .add_modifier(Modifier::BOLD),
            )])));

            let list = List::new(items)
                .block(Block::default().borders(Borders::ALL).title(" Strategies "))
                .highlight_style(
                    Style::default()
                        .bg(Color::DarkGray)
                        .add_modifier(Modifier::BOLD),
                )
                .highlight_symbol("▶ ");

            frame.render_stateful_widget(list, left_chunks[0], &mut list_state);

            // Help hint
            let help =
                Paragraph::new("?: Help & keymaps").style(Style::default().fg(Color::DarkGray));
            frame.render_widget(help, left_chunks[1]);

            // Status message
            if let Some(ref msg) = status_message {
                let status = Paragraph::new(msg.as_str()).style(Style::default().fg(Color::Yellow));
                frame.render_widget(status, left_chunks[2]);
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

                    // Render strategy with markdown styling
                    let strategy_text = markdown_to_styled_text(&info.strategy.markdown);

                    // Prepend status line for failed/edited
                    if info.failed {
                        let mut lines = vec![
                            Line::from(Span::styled(
                                "Status: FAILED",
                                Style::default().fg(Color::Red).add_modifier(Modifier::BOLD),
                            )),
                            Line::from(""),
                        ];
                        lines.extend(strategy_text.lines);
                        Text::from(lines)
                    } else if info.manually_edited {
                        let mut lines = vec![
                            Line::from(Span::styled(
                                "Status: EDITED",
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            )),
                            Line::from(""),
                        ];
                        lines.extend(strategy_text.lines);
                        Text::from(lines)
                    } else {
                        // OK case - just return the styled strategy directly
                        strategy_text
                    }
                } else {
                    Text::from("Select a strategy to preview, or press Enter to accept all.")
                };

                // Wrap text to fit panel width (account for borders)
                let wrap_width = main_chunks[1].width.saturating_sub(2) as usize;
                let wrapped_text = wrap_styled_text(preview_text, wrap_width);

                let preview = Paragraph::new(wrapped_text)
                    .block(Block::default().borders(Borders::ALL).title(preview_title));

                frame.render_widget(preview, main_chunks[1]);
            }

            // Help popup overlay
            if show_help_popup {
                let help_text = vec![
                    Line::from(vec![
                        Span::styled("↑/k", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw("  Move up"),
                    ]),
                    Line::from(vec![
                        Span::styled("↓/j", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw("  Move down"),
                    ]),
                    Line::from(vec![
                        Span::styled("Enter", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw("  Edit strategy / Accept all"),
                    ]),
                    Line::from(vec![
                        Span::styled("c", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw("  Copy to clipboard"),
                    ]),
                    Line::from(vec![
                        Span::styled("d", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw("  Delete strategy"),
                    ]),
                    Line::from(vec![
                        Span::styled("o", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw("  Add new strategy"),
                    ]),
                    Line::from(vec![
                        Span::styled("t", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw("  Talk/chat about strategy"),
                    ]),
                    Line::from(vec![
                        Span::styled("q/Esc", Style::default().add_modifier(Modifier::BOLD)),
                        Span::raw("  Quit"),
                    ]),
                    Line::from(""),
                    Line::from(Span::styled(
                        "Press any key to close",
                        Style::default().fg(Color::DarkGray),
                    )),
                ];

                let popup_width = 35;
                let popup_height = help_text.len() as u16 + 2; // +2 for borders
                let popup_area = Rect {
                    x: area.width.saturating_sub(popup_width) / 2,
                    y: area.height.saturating_sub(popup_height) / 2,
                    width: popup_width.min(area.width),
                    height: popup_height.min(area.height),
                };

                frame.render_widget(Clear, popup_area);
                let popup = Paragraph::new(help_text)
                    .block(Block::default().borders(Borders::ALL).title(" Keymaps "));
                frame.render_widget(popup, popup_area);
            }
        })?;

        // Handle input
        if event::poll(std::time::Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    status_message = None; // Clear status on any keypress

                    // Handle Ctrl+C
                    if key.code == KeyCode::Char('c')
                        && key.modifiers.contains(KeyModifiers::CONTROL)
                    {
                        disable_raw_mode()?;
                        stdout().execute(LeaveAlternateScreen)?;
                        return Ok(vec![]);
                    }

                    // Handle help popup
                    if show_help_popup {
                        show_help_popup = false;
                        continue;
                    }
                    if key.code == KeyCode::Char('?') {
                        show_help_popup = true;
                        continue;
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
                            let original_markdown = strategy_infos[idx].strategy.markdown.clone();

                            match edit_strategy_in_editor(&original_markdown) {
                                Ok(Some(edited_markdown))
                                    if edited_markdown != original_markdown =>
                                {
                                    println!(
                                        "Strategy modified for C{}, creating new agent...",
                                        idx
                                    );

                                    match create_agent_with_edited_strategy(
                                        prompt,
                                        &strategy_infos,
                                        idx,
                                        &edited_markdown,
                                    )
                                    .await
                                    {
                                        Ok(new_info) => {
                                            strategy_infos[idx] = new_info;
                                            status_message =
                                                Some(format!("C{} strategy updated", idx));
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
                        KeyCode::Char('d') | KeyCode::Delete => {
                            let selected = list_state.selected().unwrap_or(n);
                            if selected < n && n > 1 {
                                // Remove strategy from list (must keep at least 1)
                                strategy_infos.remove(selected);
                                status_message = Some(format!("Removed C{}", selected));

                                // Adjust selection if needed
                                let new_n = strategy_infos.len();
                                if selected >= new_n {
                                    list_state.select(Some(new_n)); // Select Accept
                                }
                            } else if selected < n && n == 1 {
                                status_message = Some("Cannot remove last strategy".to_string());
                            } else {
                                status_message = Some("Select a strategy to delete".to_string());
                            }
                        }
                        KeyCode::Char('c') => {
                            // Copy current strategy to clipboard
                            let selected = list_state.selected().unwrap_or(n);
                            if selected < n {
                                if let Some(ref mut cb) = clipboard {
                                    let strategy_text = &strategy_infos[selected].strategy.markdown;
                                    match cb.set_text(strategy_text.clone()) {
                                        Ok(()) => {
                                            status_message =
                                                Some(format!("C{} copied to clipboard", selected));
                                        }
                                        Err(e) => {
                                            status_message =
                                                Some(format!("Clipboard error: {}", e));
                                        }
                                    }
                                } else {
                                    status_message = Some("Clipboard unavailable".to_string());
                                }
                            } else {
                                status_message = Some("Select a strategy to copy".to_string());
                            }
                        }
                        KeyCode::Char('o') => {
                            // Add a new strategy
                            disable_raw_mode()?;
                            stdout().execute(LeaveAlternateScreen)?;

                            println!("Generating new strategy C{}...", n);

                            // Get existing non-failed strategies for exclusion
                            let existing_strategies: Vec<String> = strategy_infos
                                .iter()
                                .filter(|s| !s.failed)
                                .map(|s| s.strategy.markdown.clone())
                                .collect();

                            let strategy_prompt =
                                build_strategy_prompt(prompt, &existing_strategies);
                            let session = ClaudeSession::new();

                            match session.query_strategy(&strategy_prompt).await {
                                Ok(response) => {
                                    let strategy = parse_strategy(&response);
                                    println!(
                                        "  C{}: {}",
                                        n,
                                        truncate_for_log(&strategy.markdown, 60)
                                    );

                                    strategy_infos.push(StrategyInfo {
                                        strategy,
                                        transcript: response,
                                        failed: false,
                                        error: None,
                                        manually_edited: false,
                                    });
                                    status_message = Some(format!("Added C{}", n));
                                }
                                Err(e) => {
                                    let error_msg = format!("Failed to generate strategy: {}", e);
                                    eprintln!("ERROR: {}", error_msg);
                                    strategy_infos.push(StrategyInfo {
                                        strategy: Strategy::failed(&error_msg),
                                        transcript: format!("Error: {}", e),
                                        failed: true,
                                        error: Some(error_msg.clone()),
                                        manually_edited: false,
                                    });
                                    status_message = Some(format!("C{} failed: {}", n, error_msg));
                                }
                            }

                            // Re-enter TUI
                            enable_raw_mode()?;
                            stdout().execute(EnterAlternateScreen)?;
                            terminal.clear()?;
                        }
                        KeyCode::Char('t') => {
                            let selected = list_state.selected().unwrap_or(n);
                            if selected < n {
                                // Build list of other strategies to exclude
                                let excluded: Vec<String> = strategy_infos
                                    .iter()
                                    .enumerate()
                                    .filter(|(i, s)| *i != selected && !s.failed)
                                    .map(|(_, s)| s.strategy.markdown.clone())
                                    .collect();

                                // Exit TUI temporarily for chat
                                disable_raw_mode()?;
                                stdout().execute(LeaveAlternateScreen)?;

                                match chat_with_strategy(
                                    prompt,
                                    &strategy_infos[selected],
                                    selected,
                                    &excluded,
                                ) {
                                    ChatResult::NoChanges => {
                                        status_message =
                                            Some("Chat ended without changes".to_string());
                                    }
                                    ChatResult::RevisedStrategy(new_markdown) => {
                                        strategy_infos[selected] = StrategyInfo {
                                            strategy: Strategy::parse(&new_markdown),
                                            transcript: format!(
                                                "Revised via chat: {}",
                                                new_markdown
                                            ),
                                            failed: false,
                                            error: None,
                                            manually_edited: true,
                                        };
                                        status_message =
                                            Some(format!("C{} strategy revised", selected));
                                    }
                                    ChatResult::Error(msg) => {
                                        status_message = Some(format!("Chat error: {}", msg));
                                    }
                                }

                                // Re-enter TUI
                                enable_raw_mode()?;
                                stdout().execute(EnterAlternateScreen)?;
                                terminal.clear()?;
                            } else {
                                status_message = Some("Select a strategy to discuss".to_string());
                            }
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
    writeln!(
        temp_file,
        "# Edit the strategy below. Lines starting with # are ignored."
    )?;
    writeln!(
        temp_file,
        "# Save and exit to apply changes, or exit without saving to cancel."
    )?;
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

/// Open a chat session with Claude to discuss/revise a strategy
fn chat_with_strategy(
    task_prompt: &str,
    strategy_info: &StrategyInfo,
    strategy_idx: usize,
    excluded_strategies: &[String],
) -> ChatResult {
    // Create unique temp file path for revised strategy output
    let temp_path = std::env::temp_dir().join(format!(
        "contra-strategy-{}-{}.md",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));

    // Build forbidden approaches section
    let exclusions = if excluded_strategies.is_empty() {
        String::new()
    } else {
        let mut lines = vec![
            String::new(),
            "## FORBIDDEN APPROACHES (do not suggest these)".to_string(),
        ];
        for (i, s) in excluded_strategies.iter().enumerate() {
            lines.push(format!("{}. {}", i + 1, s));
        }
        lines.join("\n")
    };

    // Build system prompt with context
    let system_prompt = format!(
        r#"You are helping discuss a coding strategy for a task.

## Task
{}

## Current Strategy (C{})
{}
{}

---

START your first message with exactly:

Discussing strategy: {}

What would you like to know?

Tip: Say **"revise"** and I will update the strategy. Then you may exit claude to return to `contra`.

Then wait for the user's question. Answer their questions helpfully.
Do not suggest alternative strategies - focus on the current one.

If the user asks you to revise or update the strategy, write the complete revised
strategy (in markdown with **bold** key qualities) to this file:
{}

When writing to the file, include ONLY the strategy text, nothing else.
After writing the revised strategy, tell the user: "Strategy revised. Type `/exit` to return to contra.""#,
        task_prompt,
        strategy_idx,
        strategy_info.strategy.markdown,
        exclusions,
        strategy_info.strategy.markdown,
        temp_path.display()
    );

    // Spawn claude CLI as subprocess (interactive TUI mode with system prompt)
    // Pass a simple prompt to trigger Claude's greeting message
    let status = Command::new("claude")
        .arg("--system-prompt")
        .arg(&system_prompt)
        .arg("Talk strategy")
        .status();

    match status {
        Ok(exit_status) => {
            if !exit_status.success() {
                return ChatResult::Error(format!(
                    "Claude exited with status: {}",
                    exit_status.code().unwrap_or(-1)
                ));
            }
        }
        Err(e) => {
            return ChatResult::Error(format!("Failed to spawn claude: {}", e));
        }
    }

    // Check if temp file exists with revised strategy
    if temp_path.exists() {
        match std::fs::read_to_string(&temp_path) {
            Ok(content) => {
                let _ = std::fs::remove_file(&temp_path);
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    return ChatResult::RevisedStrategy(trimmed.to_string());
                }
            }
            Err(e) => {
                let _ = std::fs::remove_file(&temp_path);
                return ChatResult::Error(format!("Failed to read revised strategy: {}", e));
            }
        }
    }

    ChatResult::NoChanges
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
        .map(|(_, s)| s.strategy.markdown.clone())
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
            let _parsed = parse_strategy(&response);
            tracing::debug!(
                instance = target_idx,
                strategy = %edited_strategy,
                "Agent created with edited strategy"
            );
            Ok(StrategyInfo {
                strategy: Strategy::parse(edited_strategy),
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
                strategy: Strategy::failed(&error_msg),
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
        Ok(SessionResult {
            transcript,
            success,
        }) => {
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
