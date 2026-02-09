use std::fmt;

/// Structured representation of a strategy
#[derive(Debug, Clone, PartialEq)]
pub struct Strategy {
    /// Full markdown-formatted strategy text
    pub markdown: String,
    /// Plain text with markdown syntax stripped
    pub raw: String,
    /// Key qualities/features extracted from **bold** markers
    pub highlights: Vec<String>,
}

impl Strategy {
    /// Parse a strategy string into structured form
    pub fn parse(text: &str) -> Self {
        let markdown = text.to_string();
        let (raw, highlights) = Self::extract_formatting(&markdown);
        Self {
            markdown,
            raw,
            highlights,
        }
    }

    /// Extract plain text and bold phrases from markdown
    fn extract_formatting(text: &str) -> (String, Vec<String>) {
        let mut raw = String::new();
        let mut highlights = Vec::new();
        let mut in_bold = false;
        let mut current_bold = String::new();
        let mut chars = text.chars().peekable();

        while let Some(c) = chars.next() {
            if c == '*' && chars.peek() == Some(&'*') {
                chars.next(); // consume second *
                if in_bold {
                    let phrase = current_bold.trim().to_string();
                    if !phrase.is_empty() {
                        highlights.push(phrase);
                    }
                    current_bold.clear();
                }
                in_bold = !in_bold;
            } else if c == '`' {
                // Skip backticks in raw output
                continue;
            } else {
                raw.push(c);
                if in_bold {
                    current_bold.push(c);
                }
            }
        }
        (raw, highlights)
    }

    /// Create a failed/placeholder strategy
    pub fn failed(error_msg: &str) -> Self {
        Self {
            markdown: format!("[FAILED] {}", error_msg),
            raw: format!("[FAILED] {}", error_msg),
            highlights: vec![],
        }
    }
}

impl fmt::Display for Strategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.markdown)
    }
}

const STRATEGY_PROMPT_TEMPLATE: &str = r#"For the following task, describe ONLY your implementation plan in 2-4 sentences. Do not implement anything yet.

Task: {task}

Formatting: Using Markdown, put bold markers on the main features of your approach, and wrap any code snippets in backticks.

Reply with exactly this format:
STRATEGY: <your approach in 2-4 sentences>

{exclusions}"#;

const EXCLUSION_HEADER: &str = "You MUST suggest a novel approach UTTERLY DIFFERENT from your competitors while still satisfying the task. The **bolded** text in each approach represents the key qualities you must avoid. Your competitors are using these approaches:";

const IMPLEMENTATION_PROMPT_TEMPLATE: &str = r#"Implement the following task using the specified strategy.

Task: {task}

YOUR STRATEGY (you must follow this):
{strategy}

{exclusions}

Proceed with implementation."#;

pub fn build_strategy_prompt(task: &str, existing_strategies: &[String]) -> String {
    let exclusions = if existing_strategies.is_empty() {
        String::new()
    } else {
        let mut lines = vec![EXCLUSION_HEADER.to_string()];
        for (i, strategy) in existing_strategies.iter().enumerate() {
            lines.push(format!("{}. {}", i + 1, strategy));
        }
        lines.join("\n")
    };

    STRATEGY_PROMPT_TEMPLATE
        .replace("{task}", task)
        .replace("{exclusions}", &exclusions)
}

pub fn build_implementation_prompt(
    task: &str,
    strategy: &str,
    excluded_strategies: &[String],
) -> String {
    let exclusions = if excluded_strategies.is_empty() {
        String::new()
    } else {
        let mut lines = vec!["FORBIDDEN APPROACHES (do not use these):".to_string()];
        for (i, s) in excluded_strategies.iter().enumerate() {
            lines.push(format!("{}. {}", i + 1, s));
        }
        lines.join("\n")
    };

    IMPLEMENTATION_PROMPT_TEMPLATE
        .replace("{task}", task)
        .replace("{strategy}", strategy)
        .replace("{exclusions}", &exclusions)
}

pub fn parse_strategy(response: &str) -> Strategy {
    // Look for "STRATEGY:" prefix and extract the rest
    let text = if let Some(idx) = response.find("STRATEGY:") {
        let after_prefix = &response[idx + "STRATEGY:".len()..];
        // Take until end of line or end of string, trimmed
        let strategy = after_prefix.lines().next().unwrap_or(after_prefix).trim();

        // If strategy is on subsequent lines (multiline response), grab more
        if strategy.is_empty() {
            // Strategy might be on the next lines
            after_prefix
                .lines()
                .skip(1)
                .take(4) // Max 4 lines
                .collect::<Vec<_>>()
                .join(" ")
                .trim()
                .to_string()
        } else {
            strategy.to_string()
        }
    } else {
        // Fallback: use first 500 chars as strategy
        tracing::warn!("No STRATEGY: prefix found, using raw response");
        response
            .chars()
            .take(500)
            .collect::<String>()
            .trim()
            .to_string()
    };
    Strategy::parse(&text)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_strategy_prompt_no_exclusions() {
        let prompt = build_strategy_prompt("Build a REST API", &[]);
        assert!(prompt.contains("Build a REST API"));
        assert!(!prompt.contains("MUST NOT"));
    }

    #[test]
    fn test_build_strategy_prompt_with_exclusions() {
        let existing = vec![
            "Use Express with SQLite".to_string(),
            "Use Fastify with PostgreSQL".to_string(),
        ];
        let prompt = build_strategy_prompt("Build a REST API", &existing);
        assert!(prompt.contains("UTTERLY DIFFERENT"));
        assert!(prompt.contains("bolded"));
        assert!(prompt.contains("Express with SQLite"));
        assert!(prompt.contains("Fastify with PostgreSQL"));
    }

    #[test]
    fn test_parse_strategy() {
        let response = "STRATEGY: I will use **Actix-web** with async **SQLx** for database access.";
        let strategy = parse_strategy(response);
        assert_eq!(
            strategy.markdown,
            "I will use **Actix-web** with async **SQLx** for database access."
        );
        assert_eq!(
            strategy.raw,
            "I will use Actix-web with async SQLx for database access."
        );
        assert_eq!(strategy.highlights, vec!["Actix-web", "SQLx"]);
    }

    #[test]
    fn test_parse_strategy_fallback() {
        let response = "Some response without the prefix";
        let strategy = parse_strategy(response);
        assert_eq!(strategy.markdown, "Some response without the prefix");
    }

    #[test]
    fn test_strategy_display() {
        let strategy = Strategy::parse("Use **bold** text");
        assert_eq!(format!("{}", strategy), "Use **bold** text");
    }
}
