use anyhow::Result;
use std::collections::HashMap;
use std::fs::{self, File};
use std::io::Write;
use std::path::Path;
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
    /// Brief summary of what this module does
    pub summary: String,
    /// Whether deep analysis was written to disk
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
            ExportKind::Trait => write!(f, "trait"),
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
    /// Module dependencies: who imports what
    pub dependencies: HashMap<String, Vec<String>>,
    /// Potential gaps or issues found
    pub gaps: Vec<Gap>,
    /// External dependencies used
    pub external_deps: Vec<String>,
    /// LLM-generated architecture overview
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

        // Read file content
        let content = match fs::read_to_string(&file.path) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read {}: {}", file.path, e);
                continue;
            }
        };

        // Parse with tree-sitter
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

/// Run full analysis with LLM assistance - streams output to disk
pub async fn analyze_streaming(
    inventory: &FileInventory,
    provider: &dyn LlmProvider,
    output_path: &Path,
    _parallelism: usize,
) -> Result<Analysis> {
    info!(
        "Running streaming LLM analysis on {} source files",
        inventory.source_files.len()
    );

    // Create modules directory upfront
    let modules_dir = output_path.join("modules");
    fs::create_dir_all(&modules_dir)?;

    let mut analysis = Analysis::default();
    let total_files = inventory.source_files.len();

    for (idx, file) in inventory.source_files.iter().enumerate() {
        info!("[{}/{}] Analyzing: {}", idx + 1, total_files, file.path);

        // Analyze single file and stream to disk
        let module = analyze_single_file_streaming(file, provider, &modules_dir).await?;
        analysis.modules.push(module);

        // Memory hint to the allocator
        #[cfg(unix)]
        unsafe {
            libc::malloc_trim(0);
        }
    }

    Ok(analysis)
}

/// Analyze a single file and write output immediately
async fn analyze_single_file_streaming(
    file: &SourceFile,
    provider: &dyn LlmProvider,
    modules_dir: &Path,
) -> Result<ModuleAnalysis> {
    // Read file content
    let content = match fs::read_to_string(&file.path) {
        Ok(c) => c,
        Err(e) => {
            warn!("Failed to read {}: {}", file.path, e);
            return Ok(ModuleAnalysis {
                path: file.path.clone(),
                language: file.language,
                exports: vec![],
                imports: vec![],
                summary: format!("Failed to read: {}", e),
                has_deep_analysis: false,
            });
        }
    };

    // Parse with tree-sitter
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

    // Build static context
    let static_context = build_static_context_from_parse(&file.path, &parse_result);

    // Get LLM analysis (skip very large files)
    let (summary, deep_analysis) = if content.len() > 50_000 {
        warn!("Skipping LLM analysis for {} (file too large)", file.path);
        (
            format!("{:?} file (too large for LLM analysis)", file.language),
            None,
        )
    } else {
        match analyze_module_with_llm(provider, &file.path, &content, &static_context).await {
            Ok(deep) => {
                let summary = deep.lines().next().unwrap_or("").to_string();
                (summary, Some(deep))
            }
            Err(e) => {
                warn!("LLM analysis failed for {}: {}", file.path, e);
                (
                    format!("{:?} file with {} exports", file.language, parse_result.exports.len()),
                    None,
                )
            }
        }
    };

    // Write module markdown immediately
    let safe_name = file.path.replace(['/', '.'], "_");
    let module_path = modules_dir.join(format!("{}.md", safe_name));
    
    write_module_markdown(
        &module_path,
        &file.path,
        file.language,
        &parse_result,
        deep_analysis.as_deref(),
    )?;

    // Return lightweight analysis (no deep_analysis in memory)
    Ok(ModuleAnalysis {
        path: file.path.clone(),
        language: file.language,
        exports: parse_result.exports,
        imports: parse_result.imports,
        summary,
        has_deep_analysis: deep_analysis.is_some(),
    })
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

    // Deep analysis (if available)
    if let Some(deep) = deep_analysis {
        writeln!(file, "## Analysis\n")?;
        writeln!(file, "{}\n", deep)?;
    }

    // Exports table
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

        // Detailed exports
        writeln!(file, "\n## Export Details\n")?;

        for export in &parse_result.exports {
            writeln!(file, "### `{}`\n", export.name)?;
            writeln!(file, "**Kind:** {} | **Line:** {}\n", export.kind, export.line_number)?;

            if let Some(sig) = &export.signature {
                writeln!(file, "```rust\n{}\n```\n", sig)?;
            }

            if !export.description.is_empty() {
                writeln!(file, "{}\n", export.description)?;
            }
        }
    }

    // Imports
    if !parse_result.imports.is_empty() {
        writeln!(file, "## Dependencies\n")?;

        let external: Vec<_> = parse_result.imports.iter().filter(|i| i.is_external).collect();
        let internal: Vec<_> = parse_result.imports.iter().filter(|i| !i.is_external).collect();

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
            let ext = if import.is_external { " (external)" } else { "" };
            ctx.push_str(&format!("- `{}`{}\n", import.source, ext));
        }
        ctx.push('\n');
    }

    ctx
}

/// Analyze a single module with LLM
async fn analyze_module_with_llm(
    provider: &dyn LlmProvider,
    path: &str,
    content: &str,
    static_context: &str,
) -> Result<String> {
    let filename = std::path::Path::new(path)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(path);

    let system_prompt = r#"You are a code analysis expert. Your job is to analyze source code and produce clear, structured documentation that helps other developers (and AI assistants) understand the codebase.

For each module, provide:
1. **Purpose**: One sentence explaining what this module does
2. **Key Components**: Brief description of each important function/type
3. **Data Flow**: How data moves through this module
4. **Dependencies**: What this module relies on and why
5. **Usage**: How other code would use this module

Be concise but thorough. Focus on INTENT and BEHAVIOR, not just listing code.
Output in markdown format. Keep response under 2000 tokens."#;

    let user_prompt = format!(
        r#"Analyze this source file: `{}`

{}

## Source Code

```
{}
```

Provide a deep analysis of this module."#,
        filename, static_context, content
    );

    let messages = vec![
        Message {
            role: Role::System,
            content: system_prompt.to_string(),
        },
        Message {
            role: Role::User,
            content: user_prompt,
        },
    ];

    let config = LlmConfig {
        max_tokens: 2048,
        temperature: 0.0,
    };

    provider.complete(messages, config).await
}

/// Cross-reference modules to find dependencies and gaps
pub async fn cross_reference(analysis: &Analysis) -> Result<CrossReference> {
    info!("Cross-referencing {} modules", analysis.modules.len());

    let mut crossref = CrossReference::default();
    let mut all_exports: HashMap<String, String> = HashMap::new();
    let mut used_exports: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut external_deps: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Build export map
    for module in &analysis.modules {
        for export in &module.exports {
            all_exports.insert(export.name.clone(), module.path.clone());
        }
    }

    // Build dependency map and track usage
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

    // Find gaps
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
    provider: &dyn LlmProvider,
) -> Result<CrossReference> {
    let mut crossref = cross_reference(analysis).await?;

    // Build a summary of all modules for the LLM
    let mut modules_summary = String::new();
    for module in &analysis.modules {
        let filename = std::path::Path::new(&module.path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&module.path);

        modules_summary.push_str(&format!("### {}\n", filename));
        modules_summary.push_str(&module.summary);
        modules_summary.push_str("\n\n");
    }

    // Generate architecture overview
    let system_prompt = r#"You are a software architect. Analyze the module summaries and produce a high-level architecture overview.

Include:
1. **System Purpose**: What does this codebase do overall?
2. **Core Components**: The main modules and their roles
3. **Data Flow**: How data moves through the system
4. **Entry Points**: Where does execution start?
5. **Extension Points**: Where can the system be extended?

Be concise. Write for developers who need to understand the codebase quickly."#;

    let user_prompt = format!(
        "Here are the analyzed modules:\n\n{}\n\nGenerate an architecture overview.",
        modules_summary
    );

    let messages = vec![
        Message {
            role: Role::System,
            content: system_prompt.to_string(),
        },
        Message {
            role: Role::User,
            content: user_prompt,
        },
    ];

    let config = LlmConfig {
        max_tokens: 2048,
        temperature: 0.0,
    };

    match provider.complete(messages, config).await {
        Ok(overview) => {
            crossref.architecture_overview = Some(overview);
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
        assert_eq!(format!("{}", ExportKind::Trait), "trait");
    }
}
