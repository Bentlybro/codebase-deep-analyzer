//! Tree-sitter based code parsing
//!
//! This module provides language-agnostic code parsing using tree-sitter.
//! It extracts exports, imports, and other structural information from source files.

use anyhow::Result;
use streaming_iterator::StreamingIterator;
use tree_sitter::{Node, Parser, Query, QueryCursor};

use super::analyzer::{Export, ExportKind, Import};
use super::discovery::Language;

/// Parse a source file and extract structural information
pub fn parse_file(content: &str, language: Language) -> Result<ParseResult> {
    match language {
        Language::Rust => parse_rust(content),
        Language::TypeScript | Language::JavaScript => parse_js_ts(content, language),
        _ => Ok(ParseResult {
            exports: vec![],
            imports: vec![],
        }),
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
                    "vis" => is_pub = text.contains("pub"),
                    "name" => {
                        name = text.to_string();
                        line_number = node.start_position().row + 1;
                    }
                    "func" => {
                        kind = ExportKind::Function;
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

                    let is_external = !path.starts_with("crate::")
                        && !path.starts_with("self::")
                        && !path.starts_with("super::");

                    let parts: Vec<&str> = path.split("::").collect();
                    let source = parts.first().unwrap_or(&"").to_string();
                    let items: Vec<String> = if parts.len() > 1 {
                        parts[1..].iter().map(|s: &&str| (*s).to_string()).collect()
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

/// Parse TypeScript/JavaScript using AST walking
fn parse_js_ts(content: &str, lang: Language) -> Result<ParseResult> {
    let mut parser = Parser::new();

    let ts_lang: tree_sitter::Language = if lang == Language::TypeScript {
        tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into()
    } else {
        tree_sitter_javascript::LANGUAGE.into()
    };

    parser.set_language(&ts_lang)?;

    let tree = parser
        .parse(content, None)
        .ok_or_else(|| anyhow::anyhow!("Failed to parse JS/TS file"))?;

    let mut exports = Vec::new();
    let mut imports = Vec::new();
    let lines: Vec<&str> = content.lines().collect();

    // Walk the AST to find exports and imports
    walk_node(
        tree.root_node(),
        content,
        &lines,
        &mut exports,
        &mut imports,
    );

    Ok(ParseResult { exports, imports })
}

/// Recursively walk AST nodes to extract exports/imports
fn walk_node(
    node: Node,
    content: &str,
    lines: &[&str],
    exports: &mut Vec<Export>,
    imports: &mut Vec<Import>,
) {
    let kind = node.kind();

    match kind {
        "export_statement" => {
            if let Some(export) = extract_export_from_node(node, content, lines) {
                exports.push(export);
            }
        }
        "import_statement" => {
            if let Some(import) = extract_import_from_node(node, content) {
                imports.push(import);
            }
        }
        _ => {}
    }

    // Recurse into children
    let mut cursor = node.walk();
    for child in node.children(&mut cursor) {
        walk_node(child, content, lines, exports, imports);
    }
}

/// Extract export info from an export_statement node
fn extract_export_from_node(node: Node, content: &str, lines: &[&str]) -> Option<Export> {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        let child_kind = child.kind();

        match child_kind {
            "function_declaration" | "function" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = name_node.utf8_text(content.as_bytes()).ok()?;
                    let line = name_node.start_position().row + 1;
                    let sig = lines
                        .get(node.start_position().row)
                        .map(|s| s.trim().to_string());
                    let desc = extract_jsdoc_comment(content, line);

                    return Some(Export {
                        name: name.to_string(),
                        kind: ExportKind::Function,
                        signature: sig,
                        description: desc.unwrap_or_default(),
                        line_number: line,
                    });
                }
            }
            "class_declaration" | "class" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = name_node.utf8_text(content.as_bytes()).ok()?;
                    let line = name_node.start_position().row + 1;
                    let desc = extract_jsdoc_comment(content, line);

                    return Some(Export {
                        name: name.to_string(),
                        kind: ExportKind::Class,
                        signature: None,
                        description: desc.unwrap_or_default(),
                        line_number: line,
                    });
                }
            }
            "lexical_declaration" => {
                // const/let declarations
                let mut decl_cursor = child.walk();
                for decl_child in child.children(&mut decl_cursor) {
                    if decl_child.kind() == "variable_declarator" {
                        if let Some(name_node) = decl_child.child_by_field_name("name") {
                            let name = name_node.utf8_text(content.as_bytes()).ok()?;
                            let line = name_node.start_position().row + 1;
                            let desc = extract_jsdoc_comment(content, line);

                            return Some(Export {
                                name: name.to_string(),
                                kind: ExportKind::Const,
                                signature: None,
                                description: desc.unwrap_or_default(),
                                line_number: line,
                            });
                        }
                    }
                }
            }
            "type_alias_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = name_node.utf8_text(content.as_bytes()).ok()?;
                    let line = name_node.start_position().row + 1;

                    return Some(Export {
                        name: name.to_string(),
                        kind: ExportKind::Type,
                        signature: None,
                        description: String::new(),
                        line_number: line,
                    });
                }
            }
            "interface_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = name_node.utf8_text(content.as_bytes()).ok()?;
                    let line = name_node.start_position().row + 1;

                    return Some(Export {
                        name: name.to_string(),
                        kind: ExportKind::Trait,
                        signature: None,
                        description: String::new(),
                        line_number: line,
                    });
                }
            }
            "enum_declaration" => {
                if let Some(name_node) = child.child_by_field_name("name") {
                    let name = name_node.utf8_text(content.as_bytes()).ok()?;
                    let line = name_node.start_position().row + 1;

                    return Some(Export {
                        name: name.to_string(),
                        kind: ExportKind::Enum,
                        signature: None,
                        description: String::new(),
                        line_number: line,
                    });
                }
            }
            _ => {}
        }
    }

    None
}

/// Extract import info from an import_statement node
fn extract_import_from_node(node: Node, content: &str) -> Option<Import> {
    let mut cursor = node.walk();

    for child in node.children(&mut cursor) {
        if child.kind() == "string" || child.kind().contains("string") {
            let source_raw = child.utf8_text(content.as_bytes()).ok()?;
            let source = source_raw.trim_matches(|c| c == '"' || c == '\'' || c == '`');

            let is_external =
                !source.starts_with('.') && !source.starts_with('/') && !source.starts_with("@/");

            return Some(Import {
                source: source.to_string(),
                items: vec![],
                is_external,
            });
        }
    }

    None
}

/// Extract doc comments (Rust style ///)
pub fn extract_doc_comment(content: &str, line: usize) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    if line == 0 || line > lines.len() {
        return None;
    }

    let mut doc_lines = Vec::new();
    let mut current = line - 1;

    while current > 0 {
        let prev_idx = current - 1;
        let prev_line = lines.get(prev_idx)?;
        let trimmed = prev_line.trim();

        if trimmed.starts_with("///") {
            doc_lines.push(trimmed.trim_start_matches("///").trim());
            current -= 1;
        } else if trimmed.starts_with("//!")
            || trimmed.starts_with("#[")
            || (trimmed.is_empty() && !doc_lines.is_empty())
        {
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

/// Extract JSDoc comments (JS/TS style /** */)
fn extract_jsdoc_comment(content: &str, line: usize) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    if line == 0 || line > lines.len() {
        return None;
    }

    let mut doc_lines = Vec::new();
    let mut current = line - 1;
    let mut in_block = false;

    while current > 0 {
        let prev_idx = current - 1;
        let prev_line = lines.get(prev_idx)?;
        let trimmed = prev_line.trim();

        if trimmed.ends_with("*/") && !in_block {
            in_block = true;
            let text = trimmed.trim_end_matches("*/").trim();
            if !text.is_empty() && !text.starts_with("/*") {
                doc_lines.push(text.trim_start_matches('*').trim());
            }
            current -= 1;
        } else if in_block {
            if trimmed.starts_with("/**") || trimmed.starts_with("/*") {
                let text = trimmed
                    .trim_start_matches("/**")
                    .trim_start_matches("/*")
                    .trim();
                if !text.is_empty() {
                    doc_lines.push(text);
                }
                break;
            } else {
                let text = trimmed.trim_start_matches('*').trim();
                if !text.is_empty() {
                    doc_lines.push(text);
                }
                current -= 1;
            }
        } else if trimmed.is_empty() {
            current -= 1;
        } else {
            break;
        }
    }

    doc_lines.reverse();
    if doc_lines.is_empty() {
        None
    } else {
        let desc: Vec<&str> = doc_lines
            .iter()
            .take_while(|l| !l.starts_with('@'))
            .map(|s| s.as_ref())
            .collect();
        if desc.is_empty() {
            None
        } else {
            Some(desc.join(" ").trim().to_string())
        }
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
        assert!(result.imports[0].is_external);
    }

    #[test]
    fn test_parse_typescript_exports() {
        let content = r#"
export function greet(name: string): string {
    return `Hello, ${name}!`;
}

export class MyClass {
    constructor() {}
}

export const MY_CONST = 42;
"#;
        let result = parse_js_ts(content, Language::TypeScript).unwrap();
        assert!(result.exports.len() >= 2); // At least function and class

        let names: Vec<&str> = result.exports.iter().map(|e| e.name.as_str()).collect();
        assert!(names.contains(&"greet") || names.contains(&"MyClass"));
    }

    #[test]
    fn test_parse_typescript_imports() {
        let content = r#"
import { foo } from './local';
import bar from 'external-package';
"#;
        let result = parse_js_ts(content, Language::TypeScript).unwrap();
        assert!(result.imports.len() >= 1);
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
        assert!(doc.unwrap().contains("doc comment"));
    }
}
