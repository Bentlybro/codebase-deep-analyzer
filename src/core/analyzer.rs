use anyhow::Result;
use std::collections::{HashMap, HashSet};
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tokio::time::sleep;
use tracing::{debug, info, warn};

use super::discovery::{FileInventory, Language, SourceFile};
use super::parser;
use crate::llm::{LlmConfig, LlmProvider, Message, Role};

/// Result of analyzing a codebase - lightweight version for cross-referencing
#[derive(Debug, Default)]
pub struct Analysis {
    pub modules: Vec<ModuleAnalysis>,
}

impl Analysis {
    pub fn total_exports(&self) -> usize {
        self.modules.iter().map(|m| m.exports.len()).sum()
    }
}

/// Analysis of a single module/file
#[derive(Debug)]
pub struct ModuleAnalysis {
    pub path: String,
    pub language: Language,
    pub exports: Vec<Export>,
    pub imports: Vec<Import>,
    pub summary: String,
    pub has_deep_analysis: bool,
}

/// An exported function, class, or type
#[derive(Debug, Clone)]
pub struct Export {
    pub name: String,
    pub kind: ExportKind,
    pub signature: Option<String>,
    pub description: String,
    pub line_number: usize,
}

#[derive(Debug, Clone, Copy)]
#[allow(dead_code)]
pub enum ExportKind {
    Function,
    Class,
    Type,
    Const,
    Enum,
    Trait,
    Struct,
    Module,
}

impl std::fmt::Display for ExportKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExportKind::Function => write!(f, "fn"),
            ExportKind::Class => write!(f, "class"),
            ExportKind::Type => write!(f, "type"),
            ExportKind::Const => write!(f, "const"),
            ExportKind::Enum => write!(f, "enum"),
            ExportKind::Trait => write!(f, "trait/interface"),
            ExportKind::Struct => write!(f, "struct"),
            ExportKind::Module => write!(f, "mod"),
        }
    }
}

/// An import/dependency
#[derive(Debug, Clone)]
pub struct Import {
    pub source: String,
    pub items: Vec<String>,
    pub is_external: bool,
}

/// Cross-reference analysis
#[derive(Debug, Default)]
pub struct CrossReference {
    pub dependencies: HashMap<String, Vec<String>>,
    pub gaps: Vec<Gap>,
    pub external_deps: Vec<String>,
    pub architecture_overview: Option<String>,
}

#[derive(Debug)]
pub struct Gap {
    pub kind: GapKind,
    pub description: String,
    pub location: Option<String>,
}

#[derive(Debug)]
#[allow(dead_code)]
pub enum GapKind {
    UnusedExport,
    MissingDocumentation,
    DeadCode,
    UntestedFunction,
    UndocumentedCommand,
}

/// Run static analysis (no LLM)
pub async fn analyze_static(inventory: &FileInventory) -> Result<Analysis> {
    info!(
        "Running static analysis on {} source files",
        inventory.source_files.len()
    );

    let mut analysis = Analysis::default();

    for file in &inventory.source_files {
        debug!("Parsing: {}", file.path);

        let content = match fs::read_to_string(&file.path) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read {}: {}", file.path, e);
                continue;
            }
        };

        let parse_result = match parser::parse_file(&content, file.language) {
            Ok(r) => r,
            Err(e) => {
                warn!("Failed to parse {}: {}", file.path, e);
                parser::ParseResult {
                    exports: vec![],
                    imports: vec![],
                }
            }
        };

        let summary = if parse_result.exports.is_empty() {
            format!("{:?} file with no public exports", file.language)
        } else {
            format!(
                "{:?} file with {} public exports",
                file.language,
                parse_result.exports.len()
            )
        };

        analysis.modules.push(ModuleAnalysis {
            path: file.path.clone(),
            language: file.language,
            exports: parse_result.exports,
            imports: parse_result.imports,
            summary,
            has_deep_analysis: false,
        });
    }

    Ok(analysis)
}

/// Load completed files from progress file
fn load_progress(output_path: &Path) -> HashSet<String> {
    let progress_file = output_path.join(".cda-progress");
    let mut completed = HashSet::new();

    if let Ok(file) = File::open(&progress_file) {
        let reader = BufReader::new(file);
        for line in reader.lines().map_while(Result::ok) {
            completed.insert(line);
        }
        info!("Resuming: {} files already completed", completed.len());
    }

    completed
}

/// Save completed file to progress
fn save_progress(output_path: &Path, file_path: &str) -> Result<()> {
    let progress_file = output_path.join(".cda-progress");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(progress_file)?;
    writeln!(file, "{}", file_path)?;
    Ok(())
}

/// Run full analysis with LLM assistance - streams output to disk with resume support
pub async fn analyze_streaming(
    inventory: &FileInventory,
    provider: &dyn LlmProvider,
    output_path: &Path,
    parallelism: usize,
) -> Result<Analysis> {
    info!(
        "Running streaming LLM analysis on {} source files (parallelism: {})",
        inventory.source_files.len(),
        parallelism
    );

    // Create modules directory upfront
    let modules_dir = output_path.join("modules");
    fs::create_dir_all(&modules_dir)?;

    // Load progress for resume capability
    let completed = load_progress(output_path);
    let remaining: Vec<&SourceFile> = inventory
        .source_files
        .iter()
        .filter(|f| !completed.contains(&f.path))
        .collect();

    info!(
        "Files to process: {} (skipping {} already done)",
        remaining.len(),
        completed.len()
    );

    let mut analysis = Analysis::default();
    let total_files = remaining.len();

    // Process files with concurrency control
    let semaphore = Arc::new(Semaphore::new(parallelism));
    let provider = Arc::new(provider);
    let modules_dir = Arc::new(modules_dir);
    let output_path = Arc::new(output_path.to_path_buf());

    // Process in batches for better progress reporting
    for (batch_idx, batch) in remaining.chunks(parallelism).enumerate() {
        let batch_start = batch_idx * parallelism;
        
        let mut handles = Vec::new();

        for (idx, file) in batch.iter().enumerate() {
            let file_idx = batch_start + idx + 1 + completed.len();
            let total = total_files + completed.len();
            
            info!("[{}/{}] Analyzing: {}", file_idx, total, file.path);

            let semaphore = Arc::clone(&semaphore);
            let modules_dir = Arc::clone(&modules_dir);
            let output_path = Arc::clone(&output_path);
            let file_path = file.path.clone();
            let file_language = file.language;

            // Read file content before spawning
            let content = match fs::read_to_string(&file.path) {
                Ok(c) => c,
                Err(e) => {
                    warn!("Failed to read {}: {}", file.path, e);
                    analysis.modules.push(ModuleAnalysis {
                        path: file.path.clone(),
                        language: file.language,
                        exports: vec![],
                        imports: vec![],
                        summary: format!("Failed to read: {}", e),
                        has_deep_analysis: false,
                    });
                    continue;
                }
            };

            let handle = tokio::spawn(async move {
                let _permit = semaphore.acquire().await.unwrap();

                // Parse with tree-sitter
                let parse_result = match parser::parse_file(&content, file_language) {
                    Ok(r) => r,
                    Err(e) => {
                        warn!("Failed to parse {}: {}", file_path, e);
                        parser::ParseResult {
                            exports: vec![],
                            imports: vec![],
                        }
                    }
                };

                // Build static context
                let static_context = build_static_context_from_parse(&file_path, &parse_result);

                // Get LLM analysis (skip very large files)
                let (summary, has_deep) = if content.len() > 100_000 {
                    warn!("Skipping LLM analysis for {} (file too large: {} bytes)", file_path, content.len());
                    (
                        format!("{:?} file with {} exports (too large for LLM)", file_language, parse_result.exports.len()),
                        false,
                    )
                } else {
                    match analyze_module_with_llm_retry(&file_path, &content, &static_context, 3).await {
                        Ok(deep) => {
                            let summary = deep.lines().next().unwrap_or("").to_string();
                            
                            // Write module markdown immediately
                            let safe_name = file_path.replace(['/', '.'], "_");
                            let module_path = modules_dir.join(format!("{}.md", safe_name));
                            
                            if let Err(e) = write_module_markdown(
                                &module_path,
                                &file_path,
                                file_language,
                                &parse_result,
                                Some(&deep),
                            ) {
                                warn!("Failed to write {}: {}", module_path.display(), e);
                            }

                            // Save progress
                            if let Err(e) = save_progress(&output_path, &file_path) {
                                warn!("Failed to save progress: {}", e);
                            }

                            (summary, true)
                        }
                        Err(e) => {
                            warn!("LLM analysis failed for {}: {}", file_path, e);
                            
                            // Still write static analysis
                            let safe_name = file_path.replace(['/', '.'], "_");
                            let module_path = modules_dir.join(format!("{}.md", safe_name));
                            let _ = write_module_markdown(
                                &module_path,
                                &file_path,
                                file_language,
                                &parse_result,
                                None,
                            );
                            let _ = save_progress(&output_path, &file_path);

                            (
                                format!("{:?} file with {} exports", file_language, parse_result.exports.len()),
                                false,
                            )
                        }
                    }
                };

                ModuleAnalysis {
                    path: file_path,
                    language: file_language,
                    exports: parse_result.exports,
                    imports: parse_result.imports,
                    summary,
                    has_deep_analysis: has_deep,
                }
            });

            handles.push(handle);
        }

        // Wait for batch to complete
        for handle in handles {
            match handle.await {
                Ok(module) => analysis.modules.push(module),
                Err(e) => warn!("Task failed: {}", e),
            }
        }
    }

    // Add already-completed modules (from resume)
    for path in &completed {
        analysis.modules.push(ModuleAnalysis {
            path: path.clone(),
            language: Language::Unknown,
            exports: vec![],
            imports: vec![],
            summary: "(previously analyzed)".to_string(),
            has_deep_analysis: true,
        });
    }

    Ok(analysis)
}

/// Analyze module with LLM with retry logic
async fn analyze_module_with_llm_retry(
    path: &str,
    content: &str,
    static_context: &str,
    max_retries: usize,
) -> Result<String> {
    let mut last_error = None;

    for attempt in 0..max_retries {
        if attempt > 0 {
            // Exponential backoff
            let delay = Duration::from_secs(2u64.pow(attempt as u32));
            info!("Retry {} for {} after {:?}", attempt + 1, path, delay);
            sleep(delay).await;
        }

        match analyze_module_with_llm(path, content, static_context).await {
            Ok(result) => return Ok(result),
            Err(e) => {
                let err_str = e.to_string();
                if err_str.contains("rate_limit") || err_str.contains("overloaded") {
                    warn!("Rate limited, will retry: {}", path);
                    last_error = Some(e);
                    continue;
                }
                return Err(e);
            }
        }
    }

    Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Max retries exceeded")))
}

/// Write module markdown to disk immediately
fn write_module_markdown(
    path: &Path,
    file_path: &str,
    language: Language,
    parse_result: &parser::ParseResult,
    deep_analysis: Option<&str>,
) -> Result<()> {
    let mut file = File::create(path)?;

    let module_name = Path::new(file_path)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("unknown");

    writeln!(file, "# {}\n", module_name)?;
    writeln!(file, "**Path:** `{}`\n", file_path)?;
    writeln!(file, "**Language:** {:?}\n", language)?;

    if let Some(deep) = deep_analysis {
        writeln!(file, "## Analysis\n")?;
        writeln!(file, "{}\n", deep)?;
    }

    if !parse_result.exports.is_empty() {
        writeln!(file, "## Exports\n")?;
        writeln!(file, "| Name | Kind | Line | Description |")?;
        writeln!(file, "|------|------|------|-------------|")?;

        for export in &parse_result.exports {
            let desc = if export.description.len() > 50 {
                format!("{}...", &export.description[..47])
            } else {
                export.description.clone()
            };
            writeln!(
                file,
                "| `{}` | {} | {} | {} |",
                export.name, export.kind, export.line_number, desc
            )?;
        }

        writeln!(file, "\n## Export Details\n")?;

        for export in &parse_result.exports {
            writeln!(file, "### `{}`\n", export.name)?;
            writeln!(
                file,
                "**Kind:** {} | **Line:** {}\n",
                export.kind, export.line_number
            )?;

            if let Some(sig) = &export.signature {
                writeln!(file, "```\n{}\n```\n", sig)?;
            }

            if !export.description.is_empty() {
                writeln!(file, "{}\n", export.description)?;
            }
        }
    }

    if !parse_result.imports.is_empty() {
        writeln!(file, "## Dependencies\n")?;

        let external: Vec<_> = parse_result
            .imports
            .iter()
            .filter(|i| i.is_external)
            .collect();
        let internal: Vec<_> = parse_result
            .imports
            .iter()
            .filter(|i| !i.is_external)
            .collect();

        if !external.is_empty() {
            writeln!(file, "### External\n")?;
            for import in external {
                writeln!(file, "- `{}`", import.source)?;
            }
            writeln!(file)?;
        }

        if !internal.is_empty() {
            writeln!(file, "### Internal\n")?;
            for import in internal {
                writeln!(file, "- `{}`", import.source)?;
            }
        }
    }

    Ok(())
}

/// Build context from parse results
fn build_static_context_from_parse(path: &str, parse_result: &parser::ParseResult) -> String {
    let mut ctx = String::new();

    ctx.push_str(&format!("## File: {}\n\n", path));
    ctx.push_str("## Static Analysis Results\n\n");

    if !parse_result.exports.is_empty() {
        ctx.push_str("### Exports\n");
        for export in &parse_result.exports {
            ctx.push_str(&format!("- `{}` ({})", export.name, export.kind));
            if let Some(sig) = &export.signature {
                ctx.push_str(&format!(": `{}`", sig));
            }
            if !export.description.is_empty() {
                ctx.push_str(&format!(" — {}", export.description));
            }
            ctx.push('\n');
        }
        ctx.push('\n');
    }

    if !parse_result.imports.is_empty() {
        ctx.push_str("### Dependencies\n");
        for import in &parse_result.imports {
            let ext = if import.is_external {
                " (external)"
            } else {
                ""
            };
            ctx.push_str(&format!("- `{}`{}\n", import.source, ext));
        }
        ctx.push('\n');
    }

    ctx
}

/// Analyze a single module with LLM
async fn analyze_module_with_llm(path: &str, content: &str, static_context: &str) -> Result<String> {
    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path);

    // Get API key from environment
    let api_key = std::env::var("ANTHROPIC_API_KEY")
        .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;

    let system_prompt = r#"You are a code analysis expert. Analyze the source code and produce clear documentation.

Provide:
1. **Purpose**: One sentence explaining what this module does
2. **Key Components**: Brief description of important functions/types (max 5)
3. **Usage**: How other code would use this module

Be concise. Max 500 words. Output in markdown."#;

    let user_prompt = format!(
        "Analyze `{}`:\n\n{}\n\n```\n{}\n```",
        filename,
        static_context,
        // Truncate very long files
        if content.len() > 30000 {
            &content[..30000]
        } else {
            content
        }
    );

    let client = reqwest::Client::new();
    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": format!("{}\n\n{}", system_prompt, user_prompt)}
            ]
        }))
        .send()
        .await?;

    if !response.status().is_success() {
        let status = response.status();
        let body = response.text().await?;
        anyhow::bail!("Anthropic API error {}: {}", status, body);
    }

    let json: serde_json::Value = response.json().await?;
    let text = json["content"][0]["text"]
        .as_str()
        .unwrap_or("")
        .to_string();

    Ok(text)
}

/// Cross-reference modules to find dependencies and gaps
pub async fn cross_reference(analysis: &Analysis) -> Result<CrossReference> {
    info!("Cross-referencing {} modules", analysis.modules.len());

    let mut crossref = CrossReference::default();
    let mut all_exports: HashMap<String, String> = HashMap::new();
    let mut used_exports: HashSet<String> = HashSet::new();
    let mut external_deps: HashSet<String> = HashSet::new();

    for module in &analysis.modules {
        for export in &module.exports {
            all_exports.insert(export.name.clone(), module.path.clone());
        }
    }

    for module in &analysis.modules {
        let mut deps = Vec::new();

        for import in &module.imports {
            if import.is_external {
                external_deps.insert(import.source.clone());
            } else {
                for item in &import.items {
                    if all_exports.contains_key(item) {
                        deps.push(all_exports[item].clone());
                        used_exports.insert(item.clone());
                    }
                }
            }
        }

        deps.sort();
        deps.dedup();
        crossref.dependencies.insert(module.path.clone(), deps);
    }

    for module in &analysis.modules {
        for export in &module.exports {
            if export.name == "main" || export.name.contains("test") {
                continue;
            }

            if !used_exports.contains(&export.name) && export.description.is_empty() {
                crossref.gaps.push(Gap {
                    kind: GapKind::MissingDocumentation,
                    description: format!(
                        "Public {} `{}` has no documentation",
                        export.kind, export.name
                    ),
                    location: Some(format!("{}:{}", module.path, export.line_number)),
                });
            }
        }
    }

    crossref.external_deps = external_deps.into_iter().collect();
    crossref.external_deps.sort();

    Ok(crossref)
}

/// Cross-reference with LLM to generate architecture overview
pub async fn cross_reference_with_llm(
    analysis: &Analysis,
    _provider: &dyn LlmProvider,
) -> Result<CrossReference> {
    let mut crossref = cross_reference(analysis).await?;

    // Build a summary of all modules for the LLM
    let mut modules_summary = String::new();
    let mut count = 0;
    for module in &analysis.modules {
        if count >= 50 {
            modules_summary.push_str("\n... and more modules\n");
            break;
        }
        let filename = std::path::Path::new(&module.path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&module.path);

        modules_summary.push_str(&format!("- **{}**: {}\n", filename, module.summary));
        count += 1;
    }

    // Generate architecture overview
    let api_key = match std::env::var("ANTHROPIC_API_KEY") {
        Ok(k) => k,
        Err(_) => {
            warn!("ANTHROPIC_API_KEY not set, skipping architecture overview");
            return Ok(crossref);
        }
    };

    let prompt = format!(
        r#"Based on these modules, write a brief architecture overview (max 300 words):

{}

Include: System purpose, core components, data flow, entry points."#,
        modules_summary
    );

    let client = reqwest::Client::new();
    let response = client
        .post("https://api.anthropic.com/v1/messages")
        .header("x-api-key", &api_key)
        .header("anthropic-version", "2023-06-01")
        .header("content-type", "application/json")
        .json(&serde_json::json!({
            "model": "claude-sonnet-4-20250514",
            "max_tokens": 1024,
            "messages": [
                {"role": "user", "content": prompt}
            ]
        }))
        .send()
        .await;

    match response {
        Ok(resp) if resp.status().is_success() => {
            if let Ok(json) = resp.json::<serde_json::Value>().await {
                if let Some(text) = json["content"][0]["text"].as_str() {
                    crossref.architecture_overview = Some(text.to_string());
                }
            }
        }
        Ok(resp) => {
            warn!(
                "Failed to generate architecture overview: {}",
                resp.status()
            );
        }
        Err(e) => {
            warn!("Failed to generate architecture overview: {}", e);
        }
    }

    Ok(crossref)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_analysis_total_exports() {
        let analysis = Analysis {
            modules: vec![
                ModuleAnalysis {
                    path: "a.rs".into(),
                    language: Language::Rust,
                    exports: vec![Export {
                        name: "foo".into(),
                        kind: ExportKind::Function,
                        signature: None,
                        description: "".into(),
                        line_number: 1,
                    }],
                    imports: vec![],
                    summary: "".into(),
                    has_deep_analysis: false,
                },
                ModuleAnalysis {
                    path: "b.rs".into(),
                    language: Language::Rust,
                    exports: vec![
                        Export {
                            name: "bar".into(),
                            kind: ExportKind::Function,
                            signature: None,
                            description: "".into(),
                            line_number: 1,
                        },
                        Export {
                            name: "baz".into(),
                            kind: ExportKind::Function,
                            signature: None,
                            description: "".into(),
                            line_number: 2,
                        },
                    ],
                    imports: vec![],
                    summary: "".into(),
                    has_deep_analysis: false,
                },
            ],
        };

        assert_eq!(analysis.total_exports(), 3);
    }

    #[test]
    fn test_export_kind_display() {
        assert_eq!(format!("{}", ExportKind::Function), "fn");
        assert_eq!(format!("{}", ExportKind::Struct), "struct");
        assert_eq!(format!("{}", ExportKind::Trait), "trait/interface");
    }
}
