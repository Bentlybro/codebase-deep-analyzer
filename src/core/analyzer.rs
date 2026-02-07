use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use tracing::{debug, info, warn};

use super::discovery::{FileInventory, Language};
use super::parser;
use crate::llm::{LlmConfig, LlmProvider, Message, Role};

/// Result of analyzing a codebase
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
    /// Detailed LLM-generated analysis (if available)
    pub deep_analysis: Option<String>,
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
            deep_analysis: None,
        });
    }

    Ok(analysis)
}

/// Run full analysis with LLM assistance
pub async fn analyze(
    inventory: &FileInventory,
    provider: &dyn LlmProvider,
    parallelism: usize,
) -> Result<Analysis> {
    info!(
        "Running LLM-assisted analysis on {} source files (parallelism: {})",
        inventory.source_files.len(),
        parallelism
    );

    // First, run static analysis to get the structure
    let mut analysis = analyze_static(inventory).await?;

    // Now enhance each module with LLM analysis
    for module in &mut analysis.modules {
        // Read the source file
        let content = match fs::read_to_string(&module.path) {
            Ok(c) => c,
            Err(e) => {
                warn!("Failed to read {} for LLM analysis: {}", module.path, e);
                continue;
            }
        };

        // Skip very large files (>50KB) to avoid token limits
        if content.len() > 50_000 {
            warn!("Skipping LLM analysis for {} (file too large)", module.path);
            continue;
        }

        // Build context from static analysis
        let static_context = build_static_context(module);

        // Generate LLM analysis
        match analyze_module_with_llm(provider, &module.path, &content, &static_context).await {
            Ok(deep) => {
                module.deep_analysis = Some(deep.clone());
                // Update summary with first line of deep analysis
                if let Some(first_line) = deep.lines().next() {
                    module.summary = first_line.to_string();
                }
            }
            Err(e) => {
                warn!("LLM analysis failed for {}: {}", module.path, e);
            }
        }
    }

    Ok(analysis)
}

/// Build a context string from static analysis results
fn build_static_context(module: &ModuleAnalysis) -> String {
    let mut ctx = String::new();

    ctx.push_str("## Static Analysis Results\n\n");

    if !module.exports.is_empty() {
        ctx.push_str("### Exports\n");
        for export in &module.exports {
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

    if !module.imports.is_empty() {
        ctx.push_str("### Dependencies\n");
        for import in &module.imports {
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
Output in markdown format."#;

    let user_prompt = format!(
        r#"Analyze this source file: `{}`

{}

## Source Code

```
{}
```

Provide a deep analysis of this module. Focus on what it does, how it works, and how it fits into the larger codebase."#,
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
    let mut all_exports: HashMap<String, String> = HashMap::new(); // name -> path
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
                // Try to find the source module
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

    // Find gaps: exports not used anywhere
    for module in &analysis.modules {
        for export in &module.exports {
            // Skip main and test functions
            if export.name == "main" || export.name.contains("test") {
                continue;
            }

            if !used_exports.contains(&export.name) {
                // Check if it has documentation
                if export.description.is_empty() {
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
    // First do static cross-reference
    let mut crossref = cross_reference(analysis).await?;

    // Build a summary of all modules for the LLM
    let mut modules_summary = String::new();
    for module in &analysis.modules {
        let filename = std::path::Path::new(&module.path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or(&module.path);

        modules_summary.push_str(&format!("### {}\n", filename));
        if let Some(deep) = &module.deep_analysis {
            // Just first paragraph
            if let Some(para) = deep.split("\n\n").next() {
                modules_summary.push_str(para);
            }
        } else {
            modules_summary.push_str(&module.summary);
        }
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
        r#"Here are the analyzed modules:

{}

Generate an architecture overview for this codebase."#,
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
                    deep_analysis: None,
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
                    deep_analysis: None,
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

    #[test]
    fn test_build_static_context() {
        let module = ModuleAnalysis {
            path: "test.rs".into(),
            language: Language::Rust,
            exports: vec![Export {
                name: "test_fn".into(),
                kind: ExportKind::Function,
                signature: Some("pub fn test_fn()".into()),
                description: "A test function".into(),
                line_number: 1,
            }],
            imports: vec![Import {
                source: "std".into(),
                items: vec!["fs".into()],
                is_external: true,
            }],
            summary: "".into(),
            deep_analysis: None,
        };

        let ctx = build_static_context(&module);
        assert!(ctx.contains("test_fn"));
        assert!(ctx.contains("pub fn test_fn()"));
        assert!(ctx.contains("A test function"));
        assert!(ctx.contains("std"));
        assert!(ctx.contains("(external)"));
    }
}
