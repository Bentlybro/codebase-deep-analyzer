//! Tree-sitter based code parsing
//! 
//! This module provides language-agnostic code parsing using tree-sitter.
//! It extracts exports, imports, and other structural information from source files.

use anyhow::Result;

use super::discovery::Language;
use super::analyzer::{Export, Import, ExportKind};

/// Parse a source file and extract structural information
pub fn parse_file(content: &str, language: Language) -> Result<ParseResult> {
    // TODO: Implement tree-sitter parsing for each language
    // 
    // Example for Rust:
    // 1. Create parser with tree_sitter_rust::language()
    // 2. Parse content to get tree
    // 3. Walk tree to find:
    //    - pub fn, pub struct, pub enum, pub type, pub const
    //    - use statements
    //    - mod declarations
    // 4. Extract names, signatures, doc comments
    
    Ok(ParseResult {
        exports: vec![],
        imports: vec![],
    })
}

pub struct ParseResult {
    pub exports: Vec<Export>,
    pub imports: Vec<Import>,
}

/// Extract doc comments for a given line range
pub fn extract_doc_comment(content: &str, line: usize) -> Option<String> {
    let lines: Vec<&str> = content.lines().collect();
    
    if line == 0 || line > lines.len() {
        return None;
    }
    
    let mut doc_lines = Vec::new();
    let mut current = line - 1; // 0-indexed, look at line before
    
    // Walk backwards to collect doc comments
    while current > 0 {
        let prev_line = lines.get(current - 1)?;
        let trimmed = prev_line.trim();
        
        if trimmed.starts_with("///") {
            doc_lines.push(trimmed.trim_start_matches("///").trim());
            current -= 1;
        } else if trimmed.starts_with("//!") {
            doc_lines.push(trimmed.trim_start_matches("//!").trim());
            current -= 1;
        } else if trimmed.starts_with('#') && trimmed.contains('[') {
            // Skip attributes
            current -= 1;
        } else if trimmed.is_empty() {
            // Skip empty lines within doc block
            current -= 1;
        } else {
            break;
        }
    }
    
    doc_lines.reverse();
    
    if doc_lines.is_empty() {
        None
    } else {
        Some(doc_lines.join("\n"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        assert!(doc.contains("This is a doc comment"));
        assert!(doc.contains("with multiple lines"));
    }
}
