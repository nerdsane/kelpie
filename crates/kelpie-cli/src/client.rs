//! Kelpie HTTP Client
//!
//! TigerStyle: HTTP client for Kelpie server API with explicit error handling.

use anyhow::{anyhow, Context, Result};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::time::Duration;

/// Default server URL
pub const DEFAULT_SERVER_URL: &str = "http://localhost:8283";

/// Default request timeout in seconds
pub const REQUEST_TIMEOUT_SECONDS: u64 = 60;

/// Kelpie API client
pub struct KelpieClient {
    client: reqwest::Client,
    base_url: String,
}

impl KelpieClient {
    /// Create a new client with the given base URL
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(REQUEST_TIMEOUT_SECONDS))
            .build()
            .context("Failed to create HTTP client")?;

        Ok(Self {
            client,
            base_url: base_url.into().trim_end_matches('/').to_string(),
        })
    }

    /// Create a client with default URL
    #[allow(dead_code)]
    pub fn default_url() -> Result<Self> {
        Self::new(DEFAULT_SERVER_URL)
    }

    /// Get server health status
    pub async fn health(&self) -> Result<HealthResponse> {
        self.get("/v1/health").await
    }

    /// List all agents
    pub async fn list_agents(&self) -> Result<ListAgentsResponse> {
        self.get("/v1/agents").await
    }

    /// Get agent by ID
    pub async fn get_agent(&self, agent_id: &str) -> Result<AgentResponse> {
        self.get(&format!("/v1/agents/{}", agent_id)).await
    }

    /// Create a new agent
    pub async fn create_agent(&self, request: &CreateAgentRequest) -> Result<AgentResponse> {
        self.post("/v1/agents", request).await
    }

    /// Delete an agent
    pub async fn delete_agent(&self, agent_id: &str) -> Result<()> {
        self.delete(&format!("/v1/agents/{}", agent_id)).await
    }

    /// Send a message to an agent (non-streaming)
    pub async fn send_message(&self, agent_id: &str, content: &str) -> Result<SendMessageResponse> {
        let request = SendMessageRequest {
            messages: vec![MessageInput {
                role: "user".to_string(),
                content: content.to_string(),
            }],
        };
        self.post(&format!("/v1/agents/{}/messages", agent_id), &request)
            .await
    }

    /// Send a message with streaming response
    pub async fn send_message_stream(
        &self,
        agent_id: &str,
        content: &str,
    ) -> Result<reqwest::Response> {
        let request = SendMessageRequest {
            messages: vec![MessageInput {
                role: "user".to_string(),
                content: content.to_string(),
            }],
        };

        let url = format!("{}/v1/agents/{}/messages/stream", self.base_url, agent_id);
        let response = self
            .client
            .post(&url)
            .json(&request)
            .send()
            .await
            .context("Failed to send streaming request")?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Server returned error {}: {}",
                status,
                body.chars().take(200).collect::<String>()
            ));
        }

        Ok(response)
    }

    /// GET request helper
    async fn get<T: DeserializeOwned>(&self, path: &str) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .client
            .get(&url)
            .send()
            .await
            .with_context(|| format!("GET {} failed", url))?;

        self.handle_response(response).await
    }

    /// POST request helper
    async fn post<T: DeserializeOwned, R: Serialize>(&self, path: &str, body: &R) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .client
            .post(&url)
            .json(body)
            .send()
            .await
            .with_context(|| format!("POST {} failed", url))?;

        self.handle_response(response).await
    }

    /// DELETE request helper
    async fn delete(&self, path: &str) -> Result<()> {
        let url = format!("{}{}", self.base_url, path);
        let response = self
            .client
            .delete(&url)
            .send()
            .await
            .with_context(|| format!("DELETE {} failed", url))?;

        if !response.status().is_success() {
            let status = response.status();
            let body = response.text().await.unwrap_or_default();
            return Err(anyhow!(
                "Server returned error {}: {}",
                status,
                body.chars().take(200).collect::<String>()
            ));
        }

        Ok(())
    }

    /// Handle response and deserialize JSON
    async fn handle_response<T: DeserializeOwned>(&self, response: reqwest::Response) -> Result<T> {
        let status = response.status();
        let body = response
            .text()
            .await
            .context("Failed to read response body")?;

        if !status.is_success() {
            return Err(anyhow!(
                "Server returned error {}: {}",
                status,
                body.chars().take(200).collect::<String>()
            ));
        }

        serde_json::from_str(&body).with_context(|| {
            format!(
                "Failed to parse response: {}",
                body.chars().take(100).collect::<String>()
            )
        })
    }
}

// =============================================================================
// API Types
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthResponse {
    pub status: String,
    #[serde(default)]
    pub version: String,
    #[serde(default)]
    pub agent_count: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListAgentsResponse {
    pub agents: Vec<AgentSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentSummary {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub agent_type: String,
    #[serde(default)]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResponse {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub agent_type: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CreateAgentRequest {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agent_type: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memory_blocks: Option<Vec<MemoryBlockInput>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryBlockInput {
    pub label: String,
    pub value: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SendMessageRequest {
    pub messages: Vec<MessageInput>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MessageInput {
    pub role: String,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SendMessageResponse {
    pub messages: Vec<MessageOutput>,
    #[serde(default)]
    pub usage: Option<UsageStats>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageOutput {
    #[serde(default)]
    pub id: String,
    pub role: String,
    pub content: String,
    #[serde(default)]
    pub message_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageStats {
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
    #[serde(default)]
    pub total_tokens: u32,
}
