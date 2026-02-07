use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;

use super::{LlmConfig, LlmProvider, Message, Role};

const DEFAULT_MODEL: &str = "claude-sonnet-4-20250514";
const API_URL: &str = "https://api.anthropic.com/v1/messages";

pub struct AnthropicProvider {
    client: Client,
    api_key: String,
    model: String,
}

impl AnthropicProvider {
    pub fn new(model: Option<&str>) -> Result<Self> {
        let api_key = env::var("ANTHROPIC_API_KEY")
            .map_err(|_| anyhow::anyhow!("ANTHROPIC_API_KEY not set"))?;
        
        Ok(Self {
            client: Client::new(),
            api_key,
            model: model.unwrap_or(DEFAULT_MODEL).to_string(),
        })
    }
}

#[derive(Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: usize,
    messages: Vec<ApiMessage>,
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    temperature: f32,
}

#[derive(Serialize)]
struct ApiMessage {
    role: String,
    content: String,
}

#[derive(Deserialize)]
struct ApiResponse {
    content: Vec<ContentBlock>,
}

#[derive(Deserialize)]
struct ContentBlock {
    text: String,
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    fn name(&self) -> &str {
        "anthropic"
    }
    
    async fn complete(&self, messages: Vec<Message>, config: LlmConfig) -> Result<String> {
        let mut system_prompt = None;
        let mut api_messages = Vec::new();
        
        for msg in messages {
            match msg.role {
                Role::System => {
                    system_prompt = Some(msg.content);
                }
                Role::User => {
                    api_messages.push(ApiMessage {
                        role: "user".to_string(),
                        content: msg.content,
                    });
                }
                Role::Assistant => {
                    api_messages.push(ApiMessage {
                        role: "assistant".to_string(),
                        content: msg.content,
                    });
                }
            }
        }
        
        let request = ApiRequest {
            model: self.model.clone(),
            max_tokens: config.max_tokens,
            messages: api_messages,
            system: system_prompt,
            temperature: config.temperature,
        };
        
        let response = self.client
            .post(API_URL)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json")
            .json(&request)
            .send()
            .await?;
        
        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("Anthropic API error {}: {}", status, body);
        }
        
        let api_response: ApiResponse = response.json().await?;
        
        Ok(api_response
            .content
            .into_iter()
            .map(|c| c.text)
            .collect::<Vec<_>>()
            .join(""))
    }
}
