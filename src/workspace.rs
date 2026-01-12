use std::fs;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum WorkspaceError {
    #[error("Failed to create workspace directory: {0}")]
    CreateFailed(#[from] std::io::Error),
}

pub struct Workspace {
    path: PathBuf,
}

impl Workspace {
    /// Create a new workspace directory for the given instance
    /// Creates: {run_dir}/c{instance_id}/
    pub fn create(run_dir: &Path, instance_id: usize) -> Result<Self, WorkspaceError> {
        let path = run_dir.join(format!("c{}", instance_id));
        fs::create_dir_all(&path)?;

        Ok(Self { path })
    }

    /// Get the workspace path
    pub fn path(&self) -> &Path {
        &self.path
    }
}
