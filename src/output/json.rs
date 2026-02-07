use anyhow::Result;
use serde::Serialize;
use std::fs;
use std::path::Path;

use crate::core::{Analysis, CrossReference};
use crate::core::analyzer::{ExportKind, GapKind};

#[derive(Serialize)]
struct JsonOutput {
    version: &'static str,
    modules: Vec<JsonModule>,
    cross_reference: JsonCrossRef,
    statistics: JsonStats,
}

#[derive(Serialize)]
struct JsonModule {
    path: String,
    summary: String,
    exports: Vec<JsonExport>,
    imports: Vec<JsonImport>,
}

#[derive(Serialize)]
struct JsonExport {
    name: String,
    kind: String,
    signature: Option<String>,
    description: String,
    line: usize,
}

#[derive(Serialize)]
struct JsonImport {
    source: String,
    items: Vec<String>,
    external: bool,
}

#[derive(Serialize)]
struct JsonCrossRef {
    dependencies: Vec<JsonDependency>,
    gaps: Vec<JsonGap>,
}

#[derive(Serialize)]
struct JsonDependency {
    module: String,
    depends_on: Vec<String>,
}

#[derive(Serialize)]
struct JsonGap {
    kind: String,
    description: String,
    location: Option<String>,
}

#[derive(Serialize)]
struct JsonStats {
    total_modules: usize,
    total_exports: usize,
    dependencies_mapped: usize,
    potential_gaps: usize,
}

pub fn generate(analysis: &Analysis, crossref: &CrossReference, output_path: &Path) -> Result<()> {
    let output = JsonOutput {
        version: "1.0",
        modules: analysis.modules.iter().map(|m| JsonModule {
            path: m.path.clone(),
            summary: m.summary.clone(),
            exports: m.exports.iter().map(|e| JsonExport {
                name: e.name.clone(),
                kind: match e.kind {
                    ExportKind::Function => "function",
                    ExportKind::Class => "class",
                    ExportKind::Type => "type",
                    ExportKind::Const => "const",
                    ExportKind::Enum => "enum",
                    ExportKind::Trait => "trait",
                    ExportKind::Struct => "struct",
                    ExportKind::Module => "module",
                }.to_string(),
                signature: e.signature.clone(),
                description: e.description.clone(),
                line: e.line_number,
            }).collect(),
            imports: m.imports.iter().map(|i| JsonImport {
                source: i.source.clone(),
                items: i.items.clone(),
                external: i.is_external,
            }).collect(),
        }).collect(),
        cross_reference: JsonCrossRef {
            dependencies: crossref.dependencies.iter().map(|(k, v)| JsonDependency {
                module: k.clone(),
                depends_on: v.clone(),
            }).collect(),
            gaps: crossref.gaps.iter().map(|g| JsonGap {
                kind: match g.kind {
                    GapKind::UnusedExport => "unused_export",
                    GapKind::MissingDocumentation => "missing_docs",
                    GapKind::DeadCode => "dead_code",
                    GapKind::UntestedFunction => "untested",
                    GapKind::UndocumentedCommand => "undocumented_command",
                }.to_string(),
                description: g.description.clone(),
                location: g.location.clone(),
            }).collect(),
        },
        statistics: JsonStats {
            total_modules: analysis.modules.len(),
            total_exports: analysis.total_exports(),
            dependencies_mapped: crossref.dependencies.len(),
            potential_gaps: crossref.gaps.len(),
        },
    };
    
    let json_path = output_path.join("analysis.json");
    let json = serde_json::to_string_pretty(&output)?;
    fs::write(&json_path, json)?;
    
    Ok(())
}
