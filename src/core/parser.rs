//! Tree-sitter based code parsing
//!
//! This module provides language-agnostic code parsing using tree-sitter.
//! It extracts exports, imports, and other structural information from source files.

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

use super::analyzer::{Export, ExportKind, Import};
use super::discovery::Language;

/// Parse a source file and extract structural information
pub fn parse_file(content: &str, language: Language) -> Result<ParseResult> {
    match language {
        Language::Rust => parse_rust(content),
        _ => {
            // For unsupported languages, return empty result
            Ok(ParseResult {
                exports: vec![],
                imports: vec![],
            })
        }
    }
}

pub struct ParseResult {
    pub exports: Vec<Export>,
    pub imports: Vec<Import>,
}

/// Parse a Rust source file
fn parse_rust(content: &str) -> Result<ParseResult> {
    let mut parser = Parser::new();
    let language = tree_sitter_rust::LANGUAGE;
    parser.set_language(&language.into())?;

    let tree = parser
        .parse(content, None)
        .ok_or_else(|| anyhow::anyhow!("Failed to parse Rust file"))?;

    let mut exports = Vec::new();
    let mut imports = Vec::new();

    // Query for public items
    let export_query = Query::new(
        &language.into(),
        r#"
        (function_item
          (visibility_modifier) @vis
          name: (identifier) @name
        ) @func

        (struct_item
          (visibility_modifier) @vis
          name: (type_identifier) @name
        ) @struct

        (enum_item
          (visibility_modifier) @vis
          name: (type_identifier) @name
        ) @enum

        (type_item
          (visibility_modifier) @vis
          name: (type_identifier) @name
        ) @type

        (const_item
          (visibility_modifier) @vis
          name: (identifier) @name
        ) @const

        (trait_item
          (visibility_modifier) @vis
          name: (type_identifier) @name
        ) @trait

        (mod_item
          (visibility_modifier) @vis
          name: (identifier) @name
        ) @mod
        "#,
    )?;

    // Query for use statements
    let import_query = Query::new(
        &language.into(),
        r#"
        (use_declaration
          argument: (_) @path
        ) @use
        "#,
    )?;

    let mut cursor = QueryCursor::new();
    let lines: Vec<&str> = content.lines().collect();

    // Extract exports using StreamingIterator
    {
        let mut matches = cursor.matches(&export_query, tree.root_node(), content.as_bytes());
        while let Some(match_) = {
            matches.advance();
            matches.get()
        } {
            let mut name = String::new();
            let mut kind = ExportKind::Function;
            let mut is_pub = false;
            let mut line_number = 0;
            let mut signature = None;

            for capture in match_.captures {
                let capture_name = export_query.capture_names()[capture.index as usize];
                let node = capture.node;
                let text = node.utf8_text(content.as_bytes()).unwrap_or("");

                match capture_name {
                    "vis" => {
                        is_pub = text.contains("pub");
                    }
                    "name" => {
                        name = text.to_string();
                        line_number = node.start_position().row + 1;
                    }
                    "func" => {
                        kind = ExportKind::Function;
                        // Extract full signature (first line)
                        let start = node.start_position().row;
                        if let Some(line) = lines.get(start) {
                            signature = Some(line.trim().to_string());
                        }
                    }
                    "struct" => kind = ExportKind::Struct,
                    "enum" => kind = ExportKind::Enum,
                    "type" => kind = ExportKind::Type,
                    "const" => kind = ExportKind::Const,
                    "trait" => kind = ExportKind::Trait,
                    "mod" => kind = ExportKind::Module,
                    _ => {}
                }
            }

            if is_pub && !name.is_empty() {
                let description = extract_doc_comment(content, line_number).unwrap_or_default();
                exports.push(Export {
                    name,
                    kind,
                    signature,
                    description,
                    line_number,
                });
            }
        }
    }

    // Extract imports using StreamingIterator
    {
        let mut cursor2 = QueryCursor::new();
        let mut matches = cursor2.matches(&import_query, tree.root_node(), content.as_bytes());
        while let Some(match_) = {
            matches.advance();
            matches.get()
        } {
            for capture in match_.captures {
                let capture_name = import_query.capture_names()[capture.index as usize];
                if capture_name == "path" {
                    let node = capture.node;
                    let path = node.utf8_text(content.as_bytes()).unwrap_or("");

                    // Determine if external
                    let is_external = !path.starts_with("crate::")
                        && !path.starts_with("self::")
                        && !path.starts_with("super::");

                    // Parse the path to extract source and items
                    let parts: Vec<&str> = path.split("::").collect();
                    let source = parts.first().unwrap_or(&"").to_string();
                    let items: Vec<String> = if parts.len() > 1 {
                        parts[1..]
                            .iter()
                            .map(|s: &&str| (*s).to_string())
                            .collect()
                    } else {
                        vec![]
                    };

                    imports.push(Import {
                        source,
                        items,
                        is_external,
                    });
                }
            }
        }
    }

    Ok(ParseResult { exports, imports })
}

/// Extract doc comments for a given line
pub fn extract_doc_comment(content: &str, line: usize) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();

    if line == 0 || line > lines.len() {
        return None;
    }

    let mut doc_lines = Vec::new();
    let mut current = line - 1; // 0-indexed, look at line before the item

    // Walk backwards to collect doc comments
    while current > 0 {
        let prev_idx = current - 1;
        let prev_line = lines.get(prev_idx)?;
        let trimmed = prev_line.trim();

        if trimmed.starts_with("///") {
            doc_lines.push(trimmed.trim_start_matches("///").trim());
            current -= 1;
        } else if trimmed.starts_with("//!") {
            // Module-level doc, skip
            current -= 1;
        } else if trimmed.starts_with("#[") {
            // Skip attributes
            current -= 1;
        } else if trimmed.is_empty() && !doc_lines.is_empty() {
            // Empty line in the middle of doc block
            current -= 1;
        } else {
            break;
        }
    }

    doc_lines.reverse();

    if doc_lines.is_empty() {
        None
    } else {
        Some(doc_lines.join(" ").trim().to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_rust_function() {
        let content = r#"
/// This is a public function
pub fn hello_world(name: &str) -> String {
    format!("Hello, {}!", name)
}

fn private_func() {}
"#;
        let result = parse_rust(content).unwrap();
        assert_eq!(result.exports.len(), 1);
        assert_eq!(result.exports[0].name, "hello_world");
        assert!(matches!(result.exports[0].kind, ExportKind::Function));
        assert!(result.exports[0].description.contains("public function"));
    }

    #[test]
    fn test_parse_rust_struct() {
        let content = r#"
/// A test struct
pub struct TestStruct {
    pub field: String,
}

struct PrivateStruct {}
"#;
        let result = parse_rust(content).unwrap();
        assert_eq!(result.exports.len(), 1);
        assert_eq!(result.exports[0].name, "TestStruct");
        assert!(matches!(result.exports[0].kind, ExportKind::Struct));
    }

    #[test]
    fn test_parse_rust_imports() {
        let content = r#"
use std::collections::HashMap;
use crate::core::analyzer;
use super::discovery::Language;
"#;
        let result = parse_rust(content).unwrap();
        assert_eq!(result.imports.len(), 3);

        // std is external
        assert!(result.imports[0].is_external);
        assert_eq!(result.imports[0].source, "std");

        // crate:: is internal
        assert!(!result.imports[1].is_external);

        // super:: is internal
        assert!(!result.imports[2].is_external);
    }

    #[test]
    fn test_extract_doc_comment() {
        let content = r#"
/// This is a doc comment
/// with multiple lines
pub fn foo() {}
"#;
        let doc = extract_doc_comment(content, 4);
        assert!(doc.is_some());
        let doc = doc.unwrap();
        assert!(doc.contains("doc comment"));
        assert!(doc.contains("multiple lines"));
    }
}
