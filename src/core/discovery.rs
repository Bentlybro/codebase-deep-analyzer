use anyhow::Result;
use ignore::WalkBuilder;
use std::path::Path;
use tracing::debug;

/// Categorized inventory of files in a codebase
#[derive(Debug, Default)]
pub struct FileInventory {
    pub root: String,
    pub source_files: Vec<SourceFile>,
    pub config_files: Vec<String>,
    pub doc_files: Vec<String>,
    pub test_files: Vec<String>,
}

#[derive(Debug)]
pub struct SourceFile {
    pub path: String,
    pub language: Language,
    pub size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Rust,
    TypeScript,
    JavaScript,
    Python,
    Go,
    Java,
    CSharp,
    Cpp,
    C,
    Ruby,
    Shell,
    Unknown,
}

impl Language {
    fn from_extension(ext: &str) -> Self {
        match ext.to_lowercase().as_str() {
            "rs" => Language::Rust,
            "ts" | "tsx" => Language::TypeScript,
            "js" | "jsx" | "mjs" | "cjs" => Language::JavaScript,
            "py" => Language::Python,
            "go" => Language::Go,
            "java" => Language::Java,
            "cs" => Language::CSharp,
            "cpp" | "cc" | "cxx" | "hpp" => Language::Cpp,
            "c" | "h" => Language::C,
            "rb" => Language::Ruby,
            "sh" | "bash" | "zsh" => Language::Shell,
            _ => Language::Unknown,
        }
    }
}

impl FileInventory {
    pub fn total_files(&self) -> usize {
        self.source_files.len() + self.config_files.len() + self.doc_files.len() + self.test_files.len()
    }
}

/// Discover all files in a codebase, respecting .gitignore
pub async fn discover(path: &Path, module: Option<&str>) -> Result<FileInventory> {
    let search_path = if let Some(m) = module {
        path.join(m)
    } else {
        path.to_path_buf()
    };

    let mut inventory = FileInventory {
        root: path.display().to_string(),
        ..Default::default()
    };

    let walker = WalkBuilder::new(&search_path)
        .hidden(false)
        .git_ignore(true)
        .git_global(true)
        .git_exclude(true)
        .build();

    for entry in walker.flatten() {
        let path = entry.path();
        
        if !path.is_file() {
            continue;
        }

        let path_str = path.display().to_string();
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        let extension = path.extension().and_then(|e| e.to_str()).unwrap_or("");

        // Skip binary and generated files
        if is_binary_extension(extension) {
            continue;
        }

        // Categorize the file
        if is_config_file(file_name, extension) {
            debug!("Config file: {}", path_str);
            inventory.config_files.push(path_str);
        } else if is_doc_file(file_name, extension) {
            debug!("Doc file: {}", path_str);
            inventory.doc_files.push(path_str);
        } else if is_test_file(&path_str, file_name) {
            debug!("Test file: {}", path_str);
            inventory.test_files.push(path_str);
        } else if is_source_file(extension) {
            let metadata = path.metadata()?;
            debug!("Source file: {} ({} bytes)", path_str, metadata.len());
            inventory.source_files.push(SourceFile {
                path: path_str,
                language: Language::from_extension(extension),
                size: metadata.len(),
            });
        }
    }

    Ok(inventory)
}

fn is_binary_extension(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "png" | "jpg" | "jpeg" | "gif" | "ico" | "webp" | "svg" |
        "pdf" | "doc" | "docx" | "xls" | "xlsx" |
        "zip" | "tar" | "gz" | "rar" | "7z" |
        "exe" | "dll" | "so" | "dylib" | "o" | "a" |
        "wasm" | "class" | "pyc" | "pyo" |
        "mp3" | "mp4" | "wav" | "avi" | "mov" |
        "ttf" | "otf" | "woff" | "woff2" | "eot"
    )
}

fn is_config_file(name: &str, ext: &str) -> bool {
    matches!(
        name.to_lowercase().as_str(),
        "package.json" | "cargo.toml" | "pyproject.toml" | "go.mod" |
        "tsconfig.json" | "webpack.config.js" | "vite.config.ts" |
        ".eslintrc" | ".prettierrc" | "dockerfile" | "docker-compose.yml" |
        "makefile" | "justfile" | ".env.example"
    ) || matches!(
        ext.to_lowercase().as_str(),
        "toml" | "yaml" | "yml"
    ) && !name.contains("test")
}

fn is_doc_file(name: &str, ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "md" | "rst" | "txt" | "adoc"
    ) || matches!(
        name.to_lowercase().as_str(),
        "readme" | "changelog" | "contributing" | "license" | "authors"
    )
}

fn is_test_file(path: &str, name: &str) -> bool {
    let path_lower = path.to_lowercase();
    let name_lower = name.to_lowercase();
    
    path_lower.contains("/test/") ||
    path_lower.contains("/tests/") ||
    path_lower.contains("/__tests__/") ||
    path_lower.contains("/spec/") ||
    name_lower.contains("_test.") ||
    name_lower.contains(".test.") ||
    name_lower.contains("_spec.") ||
    name_lower.contains(".spec.")
}

fn is_source_file(ext: &str) -> bool {
    matches!(
        ext.to_lowercase().as_str(),
        "rs" | "ts" | "tsx" | "js" | "jsx" | "mjs" | "cjs" |
        "py" | "go" | "java" | "cs" | "cpp" | "cc" | "cxx" | "c" | "h" | "hpp" |
        "rb" | "php" | "swift" | "kt" | "scala" | "clj" |
        "sh" | "bash" | "zsh" | "ps1" |
        "sql" | "graphql" | "proto"
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_language_detection() {
        assert_eq!(Language::from_extension("rs"), Language::Rust);
        assert_eq!(Language::from_extension("ts"), Language::TypeScript);
        assert_eq!(Language::from_extension("py"), Language::Python);
        assert_eq!(Language::from_extension("unknown"), Language::Unknown);
    }

    #[test]
    fn test_is_test_file() {
        assert!(is_test_file("/src/tests/foo.rs", "foo.rs"));
        assert!(is_test_file("/src/foo_test.rs", "foo_test.rs"));
        assert!(is_test_file("/src/foo.test.ts", "foo.test.ts"));
        assert!(!is_test_file("/src/foo.rs", "foo.rs"));
    }
}
