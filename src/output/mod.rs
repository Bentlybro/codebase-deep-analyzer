mod markdown;
mod json;

use anyhow::Result;
use clap::ValueEnum;
use std::path::Path;

use crate::core::{Analysis, CrossReference};

#[derive(Debug, Clone, Copy, ValueEnum, Default)]
pub enum Format {
    #[default]
    Markdown,
    Json,
}

/// Generate output documentation
pub fn generate(
    analysis: &Analysis,
    crossref: &CrossReference,
    output_path: &Path,
    format: Format,
) -> Result<()> {
    std::fs::create_dir_all(output_path)?;
    
    match format {
        Format::Markdown => markdown::generate(analysis, crossref, output_path),
        Format::Json => json::generate(analysis, crossref, output_path),
    }
}
