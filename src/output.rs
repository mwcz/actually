use crate::conductor::InstanceResult;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use thiserror::Error;

#[derive(Error, Debug)]
pub enum OutputError {
    #[error("Failed to create output directory: {0}")]
    CreateDirFailed(#[from] std::io::Error),
}

/// Manages the output directory for a claudissent run
pub struct RunOutput {
    run_dir: PathBuf,
}

impl RunOutput {
    /// Create a new run output directory
    pub fn create(base_dir: &Path, prompt: &str, _interactive: bool) -> Result<Self, OutputError> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);

        // Create a slug from the prompt (first 30 chars, alphanumeric only)
        let prompt_slug: String = prompt
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == ' ')
            .take(30)
            .collect::<String>()
            .trim()
            .replace(' ', "-")
            .to_lowercase();

        let dir_name = format!("claudissent-{}-{}", timestamp, prompt_slug);
        let run_dir = base_dir.join(dir_name);

        fs::create_dir_all(&run_dir)?;

        Ok(Self { run_dir })
    }

    /// Get the run directory path
    pub fn path(&self) -> &Path {
        &self.run_dir
    }

    /// Write the strategies summary file
    pub fn write_strategies(&self, strategies: &[(usize, String, bool)]) -> Result<(), OutputError> {
        let strategies_path = self.run_dir.join("strategies");
        let mut file = fs::File::create(&strategies_path)?;

        writeln!(file, "CLAUDISSENT STRATEGIES")?;
        writeln!(file, "======================")?;
        writeln!(file)?;

        for (id, strategy, success) in strategies {
            let status = if *success { "OK" } else { "FAILED" };
            writeln!(file, "C{} [{}]:", id, status)?;
            writeln!(file, "  {}", strategy)?;
            writeln!(file)?;
        }

        Ok(())
    }

    /// Write a single agent's session log
    pub fn write_agent_log(
        &self,
        instance_id: usize,
        strategy: &str,
        transcript: &str,
        success: bool,
        error: Option<&str>,
    ) -> Result<(), OutputError> {
        let log_path = self.run_dir.join(format!("c{}", instance_id));
        let mut file = fs::File::create(&log_path)?;

        writeln!(file, "CLAUDISSENT AGENT C{}", instance_id)?;
        writeln!(file, "========================")?;
        writeln!(file)?;
        writeln!(file, "Status: {}", if success { "SUCCESS" } else { "FAILED" })?;
        if let Some(err) = error {
            writeln!(file, "Error: {}", err)?;
        }
        writeln!(file)?;
        writeln!(file, "Strategy:")?;
        writeln!(file, "  {}", strategy)?;
        writeln!(file)?;
        writeln!(file, "Session Transcript:")?;
        writeln!(file, "-------------------")?;
        writeln!(file, "{}", transcript)?;

        Ok(())
    }

    /// Write all outputs from a completed run
    pub fn write_results(&self, results: &[InstanceResult]) -> Result<(), OutputError> {
        // Write strategies summary
        let strategies: Vec<(usize, String, bool)> = results
            .iter()
            .map(|r| (r.instance_id, r.strategy.clone(), r.success))
            .collect();
        self.write_strategies(&strategies)?;

        // Write individual agent logs
        for result in results {
            self.write_agent_log(
                result.instance_id,
                &result.strategy,
                &result.transcript,
                result.success,
                result.error.as_deref(),
            )?;
        }

        Ok(())
    }
}
