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
#[command(name = "actually")]
#[command(about = "Orchestrate multiple Claude Code instances with different strategies")]
#[command(version)]
struct Args {
    /// Natural language description of the coding task or problem to solve.
    /// This prompt is sent to multiple AI agents, each using a different strategy.
    #[arg(required = true)]
    prompt: String,

    /// Number of parallel agent instances to spawn, each developing an independent
    /// solution strategy. Higher values provide more diverse approaches but increase
    /// API costs and execution time.
    #[arg(short = 'n', long = "num", default_value = "3")]
    num_instances: usize,

    /// Directory where session artifacts are written, including strategy files,
    /// implementation logs, and per-agent workspace directories.
    #[arg(short, long, default_value = ".")]
    out_dir: String,

    /// Print detailed execution traces including API requests, token usage,
    /// and intermediate agent reasoning steps.
    #[arg(short, long)]
    verbose: bool,

    /// Generate and display the strategy prompts without invoking agents.
    /// Useful for inspecting what would be sent before committing to API calls.
    #[arg(long)]
    dry_run: bool,

    /// Skip interactive TUI and run in headless mode with tracing output.
    /// By default, actually runs interactively with strategy review.
    #[arg(long)]
    headless: bool,

    /// Optionally specify which model to use within the Claude Code instances.  If not specified,
    /// the model currently set within Claude Code as the default will be used.
    #[arg(short = 'm', long)]
    model: Option<String>,

    /// Optionally specify which model to use within the Claude Code instances for implementing
    /// strategies.  If not specified, the value given in `--model` will be used, and if `--model`
    /// is not given, the model currently set within Claude Code as the default will be used.
    #[arg(long = "impl-model")]
    impl_model: Option<String>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // In interactive mode (default), suppress all tracing output
    // All user-facing output uses println
    let interactive = !args.headless;
    let filter = if interactive {
        "off"
    } else if args.verbose {
        "actually=debug,claude_code_agent_sdk=debug"
    } else {
        "actually=info"
    };

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()),
        )
        .with(tracing_subscriber::fmt::layer())
        .init();

    if interactive {
        println!(
            "actually starting: {} instances, prompt: \"{}\"",
            args.num_instances,
            truncate(&args.prompt, 50)
        );
    } else {
        tracing::info!(
            num_instances = args.num_instances,
            dry_run = args.dry_run,
            "actually starting"
        );
    }

    // Create run output directory structure
    let run_output = RunOutput::create(Path::new(&args.out_dir), interactive)?;

    // Run with signal handling
    let results = tokio::select! {
        result = conductor::run(
            &args.prompt,
            args.num_instances,
            run_output.path(),
            args.dry_run,
            interactive,
            args.model.as_deref(),
            args.impl_model.as_deref(),
        ) => result?,
        _ = signal::ctrl_c() => {
            if interactive {
                println!("\nInterrupted");
            } else {
                tracing::info!("Received SIGINT, shutting down");
            }
            return Ok(());
        }
    };

    // Write output files
    run_output.write_results(&results)?;

    if interactive {
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
