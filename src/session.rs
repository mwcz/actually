use claude_code_agent_sdk::{query, ClaudeAgentOptions, ClaudeClient, Message, PermissionMode};
use futures::StreamExt;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum SessionError {
    #[error("Claude Code SDK error: {0}")]
    SdkError(String),
}

impl From<claude_code_agent_sdk::ClaudeError> for SessionError {
    fn from(e: claude_code_agent_sdk::ClaudeError) -> Self {
        SessionError::SdkError(e.to_string())
    }
}

/// Result of a Claude session, containing the full transcript
#[derive(Debug, Clone)]
pub struct SessionResult {
    /// Full text log of the session (all messages concatenated)
    pub transcript: String,
    /// Whether the session completed successfully
    pub success: bool,
}

pub struct ClaudeSession {
    cwd: Option<PathBuf>,
    model: Option<String>,
}

impl ClaudeSession {
    pub fn with_model(model: Option<&str>) -> Self {
        Self {
            cwd: None,
            model: model.map(|s| s.to_string()),
        }
    }

    pub fn with_cwd_and_model(cwd: &Path, model: Option<&str>) -> Self {
        Self {
            cwd: Some(cwd.to_path_buf()),
            model: model.map(|s| s.to_string()),
        }
    }

    fn build_options(&self) -> ClaudeAgentOptions {
        ClaudeAgentOptions {
            permission_mode: Some(PermissionMode::BypassPermissions),
            cwd: self.cwd.clone(),
            model: self.model.clone(),
            ..Default::default()
        }
    }

    /// Query Claude for a strategy only (no implementation)
    /// Returns the full response text
    pub async fn query_strategy(&self, prompt: &str) -> Result<String, SessionError> {
        tracing::debug!(prompt = %prompt, "Querying for strategy");

        let options = self.build_options();
        let messages = query(prompt, Some(options)).await?;

        let mut response_text = String::new();
        for message in messages {
            if let Some(text) = extract_text_from_message(&message) {
                response_text.push_str(&text);
                response_text.push('\n');
            }
        }

        Ok(response_text)
    }

    /// Run full implementation in the given workspace with streaming
    /// Returns the complete session transcript
    pub async fn run_implementation(&self, prompt: &str) -> Result<SessionResult, SessionError> {
        tracing::debug!(prompt = %prompt, cwd = ?self.cwd, "Running implementation");

        let options = self.build_options();
        let mut client = ClaudeClient::new(options);

        client.connect().await?;
        client.query(prompt).await?;

        let mut transcript = String::new();
        transcript.push_str(&format!("=== PROMPT ===\n{}\n\n", prompt));
        transcript.push_str("=== SESSION ===\n");

        let mut stream = client.receive_response();
        while let Some(result) = stream.next().await {
            match result {
                Ok(message) => {
                    if let Some(text) = extract_text_from_message(&message) {
                        transcript.push_str(&text);
                        transcript.push('\n');
                    }
                    // Log message type for debugging
                    match &message {
                        Message::Result(_) => {
                            tracing::debug!("Received result message, session complete");
                            break;
                        }
                        Message::Assistant(_) => {
                            tracing::trace!("Received assistant message");
                        }
                        _ => {}
                    }
                }
                Err(e) => {
                    let error_msg = format!("Stream error: {}", e);
                    tracing::error!("{}", error_msg);
                    transcript.push_str(&format!("\n=== ERROR ===\n{}\n", error_msg));
                    drop(stream);
                    client.disconnect().await.ok();
                    return Ok(SessionResult {
                        transcript,
                        success: false,
                    });
                }
            }
        }

        drop(stream);
        client.disconnect().await.ok();

        Ok(SessionResult {
            transcript,
            success: true,
        })
    }
}

impl Default for ClaudeSession {
    fn default() -> Self {
        Self {
            cwd: None,
            model: None,
        }
    }
}

/// Extract text content from a Message
fn extract_text_from_message(message: &Message) -> Option<String> {
    match message {
        Message::Assistant(assistant_msg) => {
            let mut text = String::new();
            for block in &assistant_msg.message.content {
                match block {
                    claude_code_agent_sdk::ContentBlock::Text(t) => {
                        text.push_str(&t.text);
                    }
                    claude_code_agent_sdk::ContentBlock::ToolUse(tool) => {
                        text.push_str(&format!("[Tool: {}]\n", tool.name));
                    }
                    _ => {}
                }
            }
            if text.is_empty() {
                None
            } else {
                Some(text)
            }
        }
        Message::System(sys_msg) => Some(format!("[System: {}]", sys_msg.subtype)),
        Message::Result(result_msg) => Some(format!(
            "[Session complete - cost: ${:.4}]",
            result_msg.total_cost_usd.unwrap_or(0.0)
        )),
        _ => None,
    }
}
