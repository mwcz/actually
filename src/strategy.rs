const STRATEGY_PROMPT_TEMPLATE: &str = r#"For the following task, describe ONLY your implementation plan in 2-4 sentences. Do not implement anything yet.

Task: {task}

{exclusions}

Reply with exactly this format:
STRATEGY: <your approach in 2-4 sentences>"#;

const EXCLUSION_HEADER: &str = "You must suggest a novel approach UTTERLY DIFFERENT from your competitors while still satisfying the task.  Your competitors are using these approaches:";

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

pub fn parse_strategy(response: &str) -> String {
    // Look for "STRATEGY:" prefix and extract the rest
    if let Some(idx) = response.find("STRATEGY:") {
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
    }
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
        assert!(prompt.contains("MUST NOT"));
        assert!(prompt.contains("Express with SQLite"));
        assert!(prompt.contains("Fastify with PostgreSQL"));
    }

    #[test]
    fn test_parse_strategy() {
        let response = "STRATEGY: I will use Actix-web with async SQLx for database access.";
        let strategy = parse_strategy(response);
        assert_eq!(
            strategy,
            "I will use Actix-web with async SQLx for database access."
        );
    }

    #[test]
    fn test_parse_strategy_fallback() {
        let response = "Some response without the prefix";
        let strategy = parse_strategy(response);
        assert_eq!(strategy, "Some response without the prefix");
    }
}
