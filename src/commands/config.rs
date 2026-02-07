use anyhow::Result;
use directories::ProjectDirs;
use std::fs;
use tracing::info;

const DEFAULT_CONFIG: &str = r#"# CDA Configuration
# https://github.com/Bentlybro/codebase-deep-analyzer

[llm]
# LLM provider: anthropic, openai, ollama
provider = "anthropic"

# Model to use (provider-specific)
# anthropic: claude-sonnet-4-20250514, claude-opus-4-20250514
# openai: gpt-4o, gpt-4-turbo
# ollama: llama3, codellama
# model = "claude-sonnet-4-20250514"

[analysis]
# Number of parallel workers for module analysis
parallelism = 4

# File patterns to ignore (in addition to .gitignore)
ignore_patterns = [
    "node_modules",
    "target",
    "dist",
    ".git",
    "__pycache__",
    "*.min.js",
    "*.map",
]

# Maximum file size to analyze (in bytes)
max_file_size = 1048576  # 1MB

[output]
# Default output format: markdown, json
format = "markdown"

# Include source code snippets in output
include_snippets = true

# Maximum snippet length (lines)
max_snippet_lines = 20
"#;

pub fn run(init: bool) -> Result<()> {
    let dirs = ProjectDirs::from("dev", "bentlybro", "cda")
        .ok_or_else(|| anyhow::anyhow!("Could not determine config directory"))?;

    let config_path = dirs.config_dir().join("config.toml");

    if init {
        fs::create_dir_all(dirs.config_dir())?;
        fs::write(&config_path, DEFAULT_CONFIG)?;
        info!("Created config file at: {}", config_path.display());
    } else if config_path.exists() {
        let content = fs::read_to_string(&config_path)?;
        println!("Config file: {}\n", config_path.display());
        println!("{}", content);
    } else {
        println!("No config file found.");
        println!("Run `cda config --init` to create one at:");
        println!("  {}", config_path.display());
        println!("\nOr use environment variables:");
        println!("  CDA_PROVIDER=anthropic");
        println!("  CDA_MODEL=claude-sonnet-4-20250514");
        println!("  ANTHROPIC_API_KEY=sk-...");
    }

    Ok(())
}
