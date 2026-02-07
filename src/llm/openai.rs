use anyhow::Result;
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::env;

use super::{LlmConfig, LlmProvider, Message, Role};

#[allow(dead_code)]
const DEFAULT_MODEL: &str = "gpt-4o";
#[allow(dead_code)]
const API_URL: &str = "https://api.openai.com/v1/chat/completions";

#[allow(dead_code)]
pub struct OpenAiProvider {
    client: Client,
    api_key: String,
    model: String,
}

impl OpenAiProvider {
    pub fn new(model: Option<&str>) -> Result<Self> {
        let api_key =
            env::var("OPENAI_API_KEY").map_err(|_| anyhow::anyhow!("OPENAI_API_KEY not set"))?;

        Ok(Self {
            client: Client::new(),
            api_key,
            model: model.unwrap_or(DEFAULT_MODEL).to_string(),
        })
    }
}

#[allow(dead_code)]
#[derive(Serialize)]
struct ApiRequest {
    model: String,
    max_tokens: usize,
    messages: Vec<ApiMessage>,
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
    choices: Vec<Choice>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct Choice {
    message: ChoiceMessage,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct ChoiceMessage {
    content: String,
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    fn name(&self) -> &str {
        "openai"
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
            max_tokens: config.max_tokens,
            messages: api_messages,
            temperature: config.temperature,
        };

        let response = self
            .client
            .post(API_URL)
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&request)
            .send()
            .await?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await?;
            anyhow::bail!("OpenAI API error {}: {}", status, body);
        }

        let api_response: ApiResponse = response.json().await?;

        api_response
            .choices
            .first()
            .map(|c| c.message.content.clone())
            .ok_or_else(|| anyhow::anyhow!("No response from OpenAI"))
    }
}
