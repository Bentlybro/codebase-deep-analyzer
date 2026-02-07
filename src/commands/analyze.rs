use anyhow::Result;
use indicatif::{ProgressBar, ProgressStyle};
use std::path::Path;
use tracing::{debug, info};

use crate::core::{analyzer, discovery};
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
    let output_path = Path::new(&args.output);
    
    info!("Analyzing codebase at: {}", path.display());
    info!("Output directory: {}", output_path.display());

    // Create output directory
    std::fs::create_dir_all(output_path)?;

    // Set up progress bar
    let spinner_style = ProgressStyle::default_spinner()
        .tick_chars("⠁⠂⠄⡀⢀⠠⠐⠈ ")
        .template("{prefix:.bold.dim} {spinner} {wide_msg}")?;

    // Phase 1: Discovery
    let discovery_pb = ProgressBar::new_spinner();
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
    let analysis_pb = ProgressBar::new_spinner();
    analysis_pb.set_style(spinner_style.clone());
    analysis_pb.set_prefix("[2/4]");

    let analysis = if args.static_only {
        analysis_pb.set_message("Analyzing modules (static only)...");
        analysis_pb.enable_steady_tick(std::time::Duration::from_millis(100));

        debug!("Running static analysis only (--static-only)");
        let result = analyzer::analyze_static(&inventory).await?;

        analysis_pb.finish_with_message(format!(
            "Analyzed {} modules, found {} exports (static)",
            result.modules.len(),
            result.total_exports()
        ));

        result
    } else {
        analysis_pb.set_message(format!(
            "Analyzing modules with {} LLM (streaming to disk)...",
            args.provider
        ));
        analysis_pb.enable_steady_tick(std::time::Duration::from_millis(100));

        let provider = crate::llm::get_provider(&args.provider, args.model.as_deref())?;
        
        // Use streaming analysis - writes each module to disk immediately
        let result = analyzer::analyze_streaming(
            &inventory,
            provider.as_ref(),
            output_path,
            args.parallelism,
        )
        .await?;

        let llm_count = result.modules.iter().filter(|m| m.has_deep_analysis).count();
        analysis_pb.finish_with_message(format!(
            "Analyzed {} modules ({} with LLM), found {} exports",
            result.modules.len(),
            llm_count,
            result.total_exports()
        ));

        result
    };

    // Phase 3: Cross-reference
    let crossref_pb = ProgressBar::new_spinner();
    crossref_pb.set_style(spinner_style.clone());
    crossref_pb.set_prefix("[3/4]");
    crossref_pb.set_message("Cross-referencing...");
    crossref_pb.enable_steady_tick(std::time::Duration::from_millis(100));

    let crossref = if args.static_only {
        analyzer::cross_reference(&analysis).await?
    } else {
        let provider = crate::llm::get_provider(&args.provider, args.model.as_deref())?;
        analyzer::cross_reference_with_llm(&analysis, provider.as_ref()).await?
    };

    let arch_status = if crossref.architecture_overview.is_some() {
        " + architecture overview"
    } else {
        ""
    };

    crossref_pb.finish_with_message(format!(
        "Mapped {} dependencies, found {} potential gaps{}",
        crossref.dependencies.len(),
        crossref.gaps.len(),
        arch_status
    ));

    // Phase 4: Output (README + gaps, modules already written)
    let output_pb = ProgressBar::new_spinner();
    output_pb.set_style(spinner_style);
    output_pb.set_prefix("[4/4]");
    output_pb.set_message("Generating index and gaps...");
    output_pb.enable_steady_tick(std::time::Duration::from_millis(100));

    output::generate(&analysis, &crossref, output_path, args.format)?;

    output_pb.finish_with_message(format!("Output written to {}", output_path.display()));

    info!("✅ Analysis complete!");
    Ok(())
}
