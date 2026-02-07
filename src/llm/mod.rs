mod anthropic;
mod openai;
mod ollama;

use anyhow::Result;
pub use async_trait::async_trait;

pub use anthropic::AnthropicProvider;
pub use openai::OpenAiProvider;
pub use ollama::OllamaProvider;

/// Message for LLM conversation
#[derive(Debug, Clone)]
pub struct Message {
    pub role: Role,
    pub content: String,
}

#[derive(Debug, Clone, Copy)]
pub enum Role {
    System,
    User,
    Assistant,
}

/// Configuration for LLM request
#[derive(Debug, Clone)]
pub struct LlmConfig {
    pub max_tokens: usize,
    pub temperature: f32,
}

impl Default for LlmConfig {
    fn default() -> Self {
        Self {
            max_tokens: 4096,
            temperature: 0.0,
        }
    }
}

/// Trait for LLM providers
#[async_trait]
pub trait LlmProvider: Send + Sync {
    /// Get the provider name
    fn name(&self) -> &str;
    
    /// Send a message and get a response
    async fn complete(&self, messages: Vec<Message>, config: LlmConfig) -> Result<String>;
}

/// Get an LLM provider by name
pub fn get_provider(name: &str, model: Option<&str>) -> Result<Box<dyn LlmProvider>> {
    match name.to_lowercase().as_str() {
        "anthropic" | "claude" => {
            Ok(Box::new(AnthropicProvider::new(model)?))
        }
        "openai" | "gpt" => {
            Ok(Box::new(OpenAiProvider::new(model)?))
        }
        "ollama" | "local" => {
            Ok(Box::new(OllamaProvider::new(model)?))
        }
        _ => {
            anyhow::bail!("Unknown LLM provider: {}. Supported: anthropic, openai, ollama", name)
        }
    }
}
