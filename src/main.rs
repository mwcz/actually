mod conductor;
mod output;
mod session;
mod strategy;
mod workspace;

use clap::Parser;
use output::RunOutput;
use std::path::Path;
use tokio::signal;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

#[derive(Parser, Debug)]
#[command(name = "claudissent")]
#[command(about = "Orchestrate contrarian Claude Code instances")]
#[command(version)]
struct Args {
    /// The prompt/task to solve
    #[arg(required = true)]
    prompt: String,

    /// Number of contrarian instances to run
    #[arg(short = 'n', long = "num", default_value = "3")]
    num_instances: usize,

    /// Working directory for instance workspaces
    #[arg(short, long, default_value = ".")]
    workdir: String,

    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,

    /// Dry run: show prompts without executing
    #[arg(long)]
    dry_run: bool,

    /// Interactive strategy review and editing before implementation
    #[arg(short, long)]
    interactive: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // In interactive mode, suppress all tracing output
    // All user-facing output uses println
    let filter = if args.interactive {
        "off"
    } else if args.verbose {
        "claudissent=debug,claude_code_agent_sdk=debug"
    } else {
        "claudissent=info"
    };

    tracing_subscriber::registry()
        .with(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    if args.interactive {
        println!(
            "claudissent: {} instances, prompt: \"{}\"",
            args.num_instances,
            truncate(&args.prompt, 50)
        );
    } else {
        tracing::info!(
            num_instances = args.num_instances,
            dry_run = args.dry_run,
            "Starting claudissent"
        );
    }

    // Create output directory
    let run_output = RunOutput::create(Path::new(&args.workdir), &args.prompt, args.interactive)?;

    // Run with signal handling
    let results = tokio::select! {
        result = conductor::run(
            &args.prompt,
            args.num_instances,
            &args.workdir,
            args.dry_run,
            args.interactive,
        ) => result?,
        _ = signal::ctrl_c() => {
            if args.interactive {
                println!("\nInterrupted");
            } else {
                tracing::info!("Received SIGINT, shutting down");
            }
            return Ok(());
        }
    };

    // Write output files
    run_output.write_results(&results)?;

    if args.interactive {
        println!("Output: {}", run_output.path().display());
    } else {
        tracing::info!(
            output_dir = %run_output.path().display(),
            "Results written to output directory"
        );
    }

    Ok(())
}

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() > max_len {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    } else {
        s.to_string()
    }
}
