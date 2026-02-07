use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;

use super::{LlmConfig, LlmProvider, Message, Role};

#[allow(dead_code)]
const DEFAULT_MODEL: &str = "llama3";
#[allow(dead_code)]
const DEFAULT_URL: &str = "http://localhost:11434";

#[allow(dead_code)]
pub struct OllamaProvider {
    client: Client,
    base_url: String,
    model: String,
}

impl OllamaProvider {
    pub fn new(model: Option<&str>) -> Result<Self> {
        let base_url = env::var("OLLAMA_URL").unwrap_or_else(|_| DEFAULT_URL.to_string());

        Ok(Self {
            client: Client::new(),
            base_url,
            model: model.unwrap_or(DEFAULT_MODEL).to_string(),
        })
    }
}

#[allow(dead_code)]
#[derive(Serialize)]
struct ApiRequest {
    model: String,
    messages: Vec<ApiMessage>,
    stream: bool,
    options: Options,
}

#[allow(dead_code)]
#[derive(Serialize)]
struct Options {
    num_predict: usize,
    temperature: f32,
}

#[allow(dead_code)]
#[derive(Serialize)]
struct ApiMessage {
    role: String,
    content: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct ApiResponse {
    message: ResponseMessage,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct ResponseMessage {
    content: String,
}

#[async_trait]
impl LlmProvider for OllamaProvider {
    fn name(&self) -> &str {
        "ollama"
    }

    async fn complete(&self, messages: Vec<Message>, config: LlmConfig) -> Result<String> {
        let api_messages: Vec<ApiMessage> = messages
            .into_iter()
            .map(|msg| ApiMessage {
                role: match msg.role {
                    Role::System => "system".to_string(),
                    Role::User => "user".to_string(),
                    Role::Assistant => "assistant".to_string(),
                },
                content: msg.content,
            })
            .collect();

        let request = ApiRequest {
            model: self.model.clone(),
            messages: api_messages,
            stream: false,
            options: Options {
                num_predict: config.max_tokens,
                temperature: config.temperature,
            },
        };

        let url = format!("{}/api/chat", self.base_url);

        let response = self.client.post(&url).json(&request).send().await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("Ollama API error {}: {}", status, body);
        }

        let api_response: ApiResponse = response.json().await?;

        Ok(api_response.message.content)
    }
}
