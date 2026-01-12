use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WorkspaceError {
    #[error("Failed to create workspace directory: {0}")]
    CreateFailed(#[from] std::io::Error),

    #[error("Invalid base directory: {0}")]
    InvalidBaseDir(String),
}

pub struct Workspace {
    path: PathBuf,
}

impl Workspace {
    /// Create a new workspace directory for the given instance
    pub fn create(base_dir: &Path, instance_id: usize) -> Result<Self, WorkspaceError> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        let workspace_name = format!("claudissent-{}-instance-{}", timestamp, instance_id);
        let path = base_dir.join(workspace_name);

        fs::create_dir_all(&path)?;

        tracing::debug!(
            path = %path.display(),
            instance = instance_id,
            "Created workspace"
        );

        Ok(Self { path })
    }

    /// Get the workspace path
    pub fn path(&self) -> &Path {
        &self.path
    }
}
