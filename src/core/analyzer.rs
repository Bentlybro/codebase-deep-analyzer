use anyhow::Result;
use std::collections::HashMap;
use std::fs;
use tracing::{debug, info, warn};

use super::discovery::{FileInventory, Language};
use super::parser;
use crate::llm::LlmProvider;

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
    pub summary: String,
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
}

#[derive(Debug)]
pub struct Gap {
    pub kind: GapKind,
    pub description: String,
    pub location: Option<String>,
}

#[derive(Debug)]
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
        });
    }

    Ok(analysis)
}

/// Run full analysis with LLM assistance
pub async fn analyze(
    inventory: &FileInventory,
    _provider: &dyn LlmProvider,
    _parallelism: usize,
) -> Result<Analysis> {
    info!(
        "Running LLM-assisted analysis on {} source files",
        inventory.source_files.len()
    );

    // TODO: Implement LLM-assisted analysis
    // For now, fall back to static analysis
    // Future: Use LLM to generate better summaries, understand intent, etc.

    analyze_static(inventory).await
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
