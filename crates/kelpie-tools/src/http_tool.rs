//! HTTP Tool Definitions
//!
//! TigerStyle: Declarative HTTP tools with URL templates and response extraction.
//!
//! Allows defining simple API-calling tools via URL templates without custom code.
//!
//! # Example
//!
//! ```rust,ignore
//! use kelpie_tools::http_tool::{HttpToolDefinition, HttpMethod};
//!
//! let weather_tool = HttpToolDefinition::new(
//!     "get_weather",
//!     "Get weather for a city",
//!     HttpMethod::Get,
//!     "https://api.weather.com/v1/current?q={city}",
//! );
//! ```

use crate::error::{ToolError, ToolResult};
use crate::traits::{Tool, ToolCapability, ToolInput, ToolMetadata, ToolOutput, ToolParam};
use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::time::Duration;

// =============================================================================
// TigerStyle Constants
// =============================================================================

/// Default HTTP timeout in milliseconds
pub const HTTP_TIMEOUT_MS_DEFAULT: u64 = 30_000;

/// Maximum response body size in bytes
pub const HTTP_RESPONSE_BODY_BYTES_MAX: u64 = 5 * 1024 * 1024; // 5MB

/// Maximum URL length in bytes
pub const HTTP_URL_BYTES_MAX: usize = 8192;

/// Maximum number of headers
pub const HTTP_HEADERS_COUNT_MAX: usize = 50;

// =============================================================================
// HTTP Method
// =============================================================================

/// HTTP request method
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum HttpMethod {
    Get,
    Post,
    Put,
    Patch,
    Delete,
}

impl std::fmt::Display for HttpMethod {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HttpMethod::Get => write!(f, "GET"),
            HttpMethod::Post => write!(f, "POST"),
            HttpMethod::Put => write!(f, "PUT"),
            HttpMethod::Patch => write!(f, "PATCH"),
            HttpMethod::Delete => write!(f, "DELETE"),
        }
    }
}

// =============================================================================
// HTTP Tool Definition
// =============================================================================

/// Definition for an HTTP-based tool
///
/// HTTP tools make API calls using URL templates with variable substitution.
/// Variables in the URL template are enclosed in braces: `{variable_name}`
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HttpToolDefinition {
    /// Tool name (unique identifier)
    pub name: String,
    /// Human-readable description
    pub description: String,
    /// HTTP method to use
    pub method: HttpMethod,
    /// URL template with placeholders like `{query}`
    pub url_template: String,
    /// Static headers to include
    pub headers: HashMap<String, String>,
    /// Request body template (for POST/PUT/PATCH)
    pub body_template: Option<String>,
    /// JSONPath expression to extract result from response
    pub response_path: Option<String>,
    /// Expected parameters (derived from URL template)
    pub parameters: Vec<ToolParam>,
    /// Timeout in milliseconds
    pub timeout_ms: u64,
}

impl HttpToolDefinition {
    /// Create a new HTTP tool definition
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        method: HttpMethod,
        url_template: impl Into<String>,
    ) -> Self {
        let url_template = url_template.into();
        let parameters = Self::extract_parameters(&url_template);

        Self {
            name: name.into(),
            description: description.into(),
            method,
            url_template,
            headers: HashMap::new(),
            body_template: None,
            response_path: None,
            parameters,
            timeout_ms: HTTP_TIMEOUT_MS_DEFAULT,
        }
    }

    /// Add a static header
    pub fn with_header(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        assert!(
            self.headers.len() < HTTP_HEADERS_COUNT_MAX,
            "too many headers"
        );
        self.headers.insert(key.into(), value.into());
        self
    }

    /// Set a request body template
    pub fn with_body_template(mut self, template: impl Into<String>) -> Self {
        let template = template.into();
        // Extract additional parameters from body template
        let body_params = Self::extract_parameters(&template);
        for param in body_params {
            if !self.parameters.iter().any(|p| p.name == param.name) {
                self.parameters.push(param);
            }
        }
        self.body_template = Some(template);
        self
    }

    /// Set a JSONPath for response extraction
    pub fn with_response_path(mut self, path: impl Into<String>) -> Self {
        self.response_path = Some(path.into());
        self
    }

    /// Set custom timeout
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout_ms = timeout.as_millis() as u64;
        self
    }

    /// Extract parameter names from a template string
    ///
    /// Parameters are delimited by `{` and `}`
    fn extract_parameters(template: &str) -> Vec<ToolParam> {
        let mut params = Vec::new();
        let chars = template.chars();
        let mut current_param = String::new();
        let mut in_param = false;

        for c in chars {
            match c {
                '{' if !in_param => {
                    in_param = true;
                    current_param.clear();
                }
                '}' if in_param => {
                    in_param = false;
                    if !current_param.is_empty()
                        && !params.iter().any(|p: &ToolParam| p.name == current_param)
                    {
                        params.push(ToolParam::string(
                            current_param.clone(),
                            format!("Parameter: {}", current_param),
                        ));
                    }
                }
                _ if in_param => {
                    current_param.push(c);
                }
                _ => {}
            }
        }

        params
    }

    /// Substitute variables in a template string
    fn substitute_template(template: &str, params: &HashMap<String, Value>) -> ToolResult<String> {
        let mut result = template.to_string();

        for (key, value) in params {
            let placeholder = format!("{{{}}}", key);
            let replacement = match value {
                Value::String(s) => s.clone(),
                Value::Number(n) => n.to_string(),
                Value::Bool(b) => b.to_string(),
                _ => value.to_string(),
            };
            result = result.replace(&placeholder, &replacement);
        }

        // Check for any remaining placeholders
        if result.contains('{') && result.contains('}') {
            // Find the first unsubstituted placeholder
            let start = result.find('{');
            let end = result.find('}');
            if let (Some(s), Some(e)) = (start, end) {
                if s < e {
                    let missing = &result[s + 1..e];
                    return Err(ToolError::MissingParameter {
                        tool: "http_tool".to_string(),
                        param: missing.to_string(),
                    });
                }
            }
        }

        Ok(result)
    }
}

// =============================================================================
// HTTP Tool Implementation
// =============================================================================

/// Runtime HTTP tool that can be executed
pub struct HttpTool {
    definition: HttpToolDefinition,
    metadata: ToolMetadata,
    client: Client,
}

impl HttpTool {
    /// Create a new HTTP tool from a definition
    pub fn new(definition: HttpToolDefinition) -> ToolResult<Self> {
        // TigerStyle: Validate definition
        assert!(!definition.name.is_empty(), "tool name cannot be empty");
        assert!(
            definition.url_template.len() <= HTTP_URL_BYTES_MAX,
            "URL template too long"
        );
        assert!(
            definition.headers.len() <= HTTP_HEADERS_COUNT_MAX,
            "too many headers"
        );

        let metadata = ToolMetadata {
            name: definition.name.clone(),
            description: definition.description.clone(),
            version: "1.0.0".to_string(),
            parameters: definition.parameters.clone(),
            capabilities: ToolCapability::network(),
            timeout_ms: definition.timeout_ms,
        };

        let client = Client::builder()
            .timeout(Duration::from_millis(definition.timeout_ms))
            .build()
            .map_err(|e| ToolError::Internal {
                message: format!("failed to create HTTP client: {}", e),
            })?;

        Ok(Self {
            definition,
            metadata,
            client,
        })
    }

    /// Execute the HTTP request
    async fn execute_request(&self, params: &HashMap<String, Value>) -> ToolResult<String> {
        // Substitute URL template
        let url = HttpToolDefinition::substitute_template(&self.definition.url_template, params)?;

        // TigerStyle: Validate URL
        assert!(url.len() <= HTTP_URL_BYTES_MAX, "resolved URL too long");

        // Build request
        let mut request = match self.definition.method {
            HttpMethod::Get => self.client.get(&url),
            HttpMethod::Post => self.client.post(&url),
            HttpMethod::Put => self.client.put(&url),
            HttpMethod::Patch => self.client.patch(&url),
            HttpMethod::Delete => self.client.delete(&url),
        };

        // Add headers
        for (key, value) in &self.definition.headers {
            request = request.header(key, value);
        }

        // Add body if present
        if let Some(body_template) = &self.definition.body_template {
            let body = HttpToolDefinition::substitute_template(body_template, params)?;
            request = request.body(body);
        }

        // Execute request
        let response = request
            .send()
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool: self.definition.name.clone(),
                reason: format!("HTTP request failed: {}", e),
            })?;

        let status = response.status();
        let body = response
            .text()
            .await
            .map_err(|e| ToolError::ExecutionFailed {
                tool: self.definition.name.clone(),
                reason: format!("failed to read response body: {}", e),
            })?;

        // TigerStyle: Validate response size
        if body.len() as u64 > HTTP_RESPONSE_BODY_BYTES_MAX {
            return Err(ToolError::ExecutionFailed {
                tool: self.definition.name.clone(),
                reason: format!(
                    "response body too large: {} bytes (max: {} bytes)",
                    body.len(),
                    HTTP_RESPONSE_BODY_BYTES_MAX
                ),
            });
        }

        if !status.is_success() {
            return Err(ToolError::ExecutionFailed {
                tool: self.definition.name.clone(),
                reason: format!("HTTP {} error: {}", status.as_u16(), body),
            });
        }

        // Extract using JSONPath if specified
        if let Some(path) = &self.definition.response_path {
            // Simple JSONPath extraction (supports basic paths like "$.data.result")
            let json: Value =
                serde_json::from_str(&body).map_err(|e| ToolError::ExecutionFailed {
                    tool: self.definition.name.clone(),
                    reason: format!("failed to parse JSON response: {}", e),
                })?;

            let extracted =
                extract_json_path(&json, path).ok_or_else(|| ToolError::ExecutionFailed {
                    tool: self.definition.name.clone(),
                    reason: format!("JSONPath '{}' not found in response", path),
                })?;

            return Ok(extracted.to_string());
        }

        Ok(body)
    }
}

#[async_trait]
impl Tool for HttpTool {
    fn metadata(&self) -> &ToolMetadata {
        &self.metadata
    }

    async fn execute(&self, input: ToolInput) -> ToolResult<ToolOutput> {
        let params: HashMap<String, Value> = input
            .params
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();

        match self.execute_request(&params).await {
            Ok(result) => Ok(ToolOutput::success(result)),
            Err(e) => Ok(ToolOutput::failure(e.to_string())),
        }
    }
}

// =============================================================================
// JSONPath Extraction
// =============================================================================

/// Simple JSONPath extraction
///
/// Supports paths like:
/// - `$.field` - root field
/// - `$.parent.child` - nested field
/// - `$[0]` - array index
/// - `$.array[0].field` - combined
fn extract_json_path(json: &Value, path: &str) -> Option<Value> {
    // TigerStyle: Preconditions
    assert!(!path.is_empty(), "JSONPath cannot be empty");

    // Remove leading $. if present
    let path = path.strip_prefix("$.").unwrap_or(path);
    let path = path.strip_prefix('$').unwrap_or(path);

    let mut current = json.clone();

    for segment in path.split('.') {
        if segment.is_empty() {
            continue;
        }

        // Check for array index
        if let Some(bracket_pos) = segment.find('[') {
            let field = &segment[..bracket_pos];
            let index_str = segment[bracket_pos + 1..].trim_end_matches(']');

            // Access field first if present
            if !field.is_empty() {
                current = current.get(field)?.clone();
            }

            // Then access array index
            let index: usize = index_str.parse().ok()?;
            current = current.get(index)?.clone();
        } else {
            // Simple field access
            current = current.get(segment)?.clone();
        }
    }

    Some(current)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_extract_parameters() {
        let params = HttpToolDefinition::extract_parameters(
            "https://api.example.com/search?q={query}&limit={limit}",
        );
        assert_eq!(params.len(), 2);
        assert_eq!(params[0].name, "query");
        assert_eq!(params[1].name, "limit");
    }

    #[test]
    fn test_extract_parameters_empty() {
        let params = HttpToolDefinition::extract_parameters("https://api.example.com/status");
        assert_eq!(params.len(), 0);
    }

    #[test]
    fn test_substitute_template() {
        let mut params = HashMap::new();
        params.insert("city".to_string(), Value::String("Tokyo".to_string()));
        params.insert("units".to_string(), Value::String("metric".to_string()));

        let result = HttpToolDefinition::substitute_template(
            "https://api.weather.com/v1/current?q={city}&units={units}",
            &params,
        )
        .unwrap();

        assert_eq!(
            result,
            "https://api.weather.com/v1/current?q=Tokyo&units=metric"
        );
    }

    #[test]
    fn test_substitute_template_missing() {
        let params = HashMap::new();
        let result =
            HttpToolDefinition::substitute_template("https://api.example.com?q={query}", &params);

        assert!(result.is_err());
        if let Err(ToolError::MissingParameter { param, .. }) = result {
            assert_eq!(param, "query");
        } else {
            panic!("expected MissingParameter error");
        }
    }

    #[test]
    fn test_extract_json_path_simple() {
        let json: Value = serde_json::json!({
            "data": {
                "result": "hello"
            }
        });

        let result = extract_json_path(&json, "$.data.result").unwrap();
        assert_eq!(result, Value::String("hello".to_string()));
    }

    #[test]
    fn test_extract_json_path_array() {
        let json: Value = serde_json::json!({
            "items": ["a", "b", "c"]
        });

        let result = extract_json_path(&json, "$.items[1]").unwrap();
        assert_eq!(result, Value::String("b".to_string()));
    }

    #[test]
    fn test_extract_json_path_nested() {
        let json: Value = serde_json::json!({
            "users": [
                {"name": "Alice"},
                {"name": "Bob"}
            ]
        });

        let result = extract_json_path(&json, "$.users[0].name").unwrap();
        assert_eq!(result, Value::String("Alice".to_string()));
    }

    #[test]
    fn test_http_tool_definition_builder() {
        let tool = HttpToolDefinition::new(
            "search",
            "Search the web",
            HttpMethod::Get,
            "https://api.example.com/search?q={query}",
        )
        .with_header("Authorization", "Bearer token")
        .with_timeout(Duration::from_secs(60));

        assert_eq!(tool.name, "search");
        assert_eq!(tool.method, HttpMethod::Get);
        assert_eq!(tool.parameters.len(), 1);
        assert_eq!(tool.parameters[0].name, "query");
        assert_eq!(tool.headers.len(), 1);
        assert_eq!(tool.timeout_ms, 60_000);
    }

    #[test]
    fn test_http_tool_creation() {
        let definition = HttpToolDefinition::new(
            "test_tool",
            "A test tool",
            HttpMethod::Get,
            "https://httpbin.org/get?arg={value}",
        );

        let tool = HttpTool::new(definition).unwrap();
        assert_eq!(tool.name(), "test_tool");
        assert!(tool.metadata().capabilities.requires_network);
    }
}
