use anyhow::Result;
use indicatif::{MultiProgress, ProgressBar, ProgressStyle};
use std::path::Path;
use tracing::{info, debug};

use crate::core::{discovery, analyzer};
use crate::output::{self, Format};

pub struct AnalyzeArgs {
    pub path: String,
    pub output: String,
    pub module: Option<String>,
    pub provider: String,
    pub model: Option<String>,
    pub parallelism: usize,
    pub static_only: bool,
    pub format: Format,
}

pub async fn run(args: AnalyzeArgs) -> Result<()> {
    let path = Path::new(&args.path).canonicalize()?;
    info!("Analyzing codebase at: {}", path.display());

    // Set up progress bars
    let multi_progress = MultiProgress::new();
    let spinner_style = ProgressStyle::default_spinner()
        .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ")
        .template("{prefix:.bold.dim} {spinner} {wide_msg}")?;

    // Phase 1: Discovery
    let discovery_pb = multi_progress.add(ProgressBar::new_spinner());
    discovery_pb.set_style(spinner_style.clone());
    discovery_pb.set_prefix("[1/4]");
    discovery_pb.set_message("Discovering files...");
    discovery_pb.enable_steady_tick(std::time::Duration::from_millis(100));

    let inventory = discovery::discover(&path, args.module.as_deref()).await?;
    
    discovery_pb.finish_with_message(format!(
        "Found {} files ({} source, {} config, {} docs)",
        inventory.total_files(),
        inventory.source_files.len(),
        inventory.config_files.len(),
        inventory.doc_files.len()
    ));

    // Phase 2: Module Analysis
    let analysis_pb = multi_progress.add(ProgressBar::new_spinner());
    analysis_pb.set_style(spinner_style.clone());
    analysis_pb.set_prefix("[2/4]");
    analysis_pb.set_message("Analyzing modules...");
    analysis_pb.enable_steady_tick(std::time::Duration::from_millis(100));

    let analysis = if args.static_only {
        debug!("Running static analysis only (--static-only)");
        analyzer::analyze_static(&inventory).await?
    } else {
        let provider = crate::llm::get_provider(&args.provider, args.model.as_deref())?;
        analyzer::analyze(&inventory, provider.as_ref(), args.parallelism).await?
    };

    analysis_pb.finish_with_message(format!(
        "Analyzed {} modules, found {} exports",
        analysis.modules.len(),
        analysis.total_exports()
    ));

    // Phase 3: Cross-reference
    let crossref_pb = multi_progress.add(ProgressBar::new_spinner());
    crossref_pb.set_style(spinner_style.clone());
    crossref_pb.set_prefix("[3/4]");
    crossref_pb.set_message("Cross-referencing...");
    crossref_pb.enable_steady_tick(std::time::Duration::from_millis(100));

    let crossref = analyzer::cross_reference(&analysis).await?;

    crossref_pb.finish_with_message(format!(
        "Mapped {} dependencies, found {} potential gaps",
        crossref.dependencies.len(),
        crossref.gaps.len()
    ));

    // Phase 4: Output
    let output_pb = multi_progress.add(ProgressBar::new_spinner());
    output_pb.set_style(spinner_style);
    output_pb.set_prefix("[4/4]");
    output_pb.set_message("Generating documentation...");
    output_pb.enable_steady_tick(std::time::Duration::from_millis(100));

    let output_path = Path::new(&args.output);
    output::generate(&analysis, &crossref, output_path, args.format)?;

    output_pb.finish_with_message(format!("Output written to {}", output_path.display()));

    info!("✅ Analysis complete!");
    Ok(())
}
