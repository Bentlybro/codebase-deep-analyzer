use anyhow::Result;
use clap::{Parser, Subcommand};
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt};

mod commands;
mod core;
mod llm;
mod output;

#[derive(Parser)]
#[command(name = "cda")]
#[command(
    author,
    version,
    about = "Codebase Deep Analyzer - Systematic codebase exploration and documentation"
)]
#[command(propagate_version = true)]
struct Cli {
    /// Enable verbose logging
    #[arg(short, long, global = true)]
    verbose: bool,

    /// Output format
    #[arg(short, long, global = true, default_value = "markdown")]
    format: output::Format,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Analyze a codebase and generate documentation
    Analyze {
        /// Path to the codebase to analyze
        #[arg(default_value = ".")]
        path: String,

        /// Output directory for generated documentation
        #[arg(short, long, default_value = "./cda-output")]
        output: String,

        /// Specific module or directory to analyze (for targeted analysis)
        #[arg(short, long)]
        module: Option<String>,

        /// LLM provider to use
        #[arg(long, env = "CDA_PROVIDER", default_value = "anthropic")]
        provider: String,

        /// Model to use for analysis
        #[arg(long, env = "CDA_MODEL")]
        model: Option<String>,

        /// Number of parallel analysis workers
        #[arg(short, long, default_value = "4")]
        parallelism: usize,

        /// Skip LLM analysis (static analysis only)
        #[arg(long)]
        static_only: bool,
    },

    /// Verify that documentation matches actual codebase behavior
    Verify {
        /// Path to the analysis output to verify
        #[arg(default_value = "./cda-output")]
        path: String,

        /// Run commands to verify behavior (may have side effects)
        #[arg(long)]
        run_commands: bool,
    },

    /// Show current configuration
    Config {
        /// Initialize a new config file
        #[arg(long)]
        init: bool,
    },
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    // Set up logging
    let filter = if cli.verbose { "debug" } else { "info" };

    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| filter.into()),
        )
        .with(tracing_subscriber::fmt::layer().with_target(false))
        .init();

    match cli.command {
        Commands::Analyze {
            path,
            output,
            module,
            provider,
            model,
            parallelism,
            static_only,
        } => {
            commands::analyze::run(commands::analyze::AnalyzeArgs {
                path,
                output,
                module,
                provider,
                model,
                parallelism,
                static_only,
                format: cli.format,
            })
            .await?;
        }
        Commands::Verify { path, run_commands } => {
            commands::verify::run(commands::verify::VerifyArgs { path, run_commands }).await?;
        }
        Commands::Config { init } => {
            commands::config::run(init)?;
        }
    }

    Ok(())
}
