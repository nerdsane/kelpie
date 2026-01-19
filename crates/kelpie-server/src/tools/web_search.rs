//! Web Search Tool for Letta Compatibility
//!
//! TigerStyle: Tavily API integration with comprehensive error handling.
//!
//! Implements Letta's web_search prebuilt tool using Tavily API.
//! Requires TAVILY_API_KEY environment variable.

use crate::tools::{BuiltinToolHandler, UnifiedToolRegistry};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

// =============================================================================
// TigerStyle Constants
// =============================================================================

/// Maximum number of search results
const SEARCH_RESULTS_MAX: u32 = 10;

/// Default number of search results
const SEARCH_RESULTS_DEFAULT: u32 = 5;

/// API request timeout in seconds
const API_TIMEOUT_SECONDS: u64 = 10;

/// Tavily API endpoint
const TAVILY_API_URL: &str = "https://api.tavily.com/search";

// =============================================================================
// Types
// =============================================================================

/// Tavily search request
#[derive(Debug, Serialize)]
struct TavilySearchRequest {
    /// API key
    api_key: String,
    /// Search query
    query: String,
    /// Number of results (default: 5, max: 10)
    #[serde(skip_serializing_if = "Option::is_none")]
    max_results: Option<u32>,
    /// Search depth: "basic" or "advanced"
    #[serde(skip_serializing_if = "Option::is_none")]
    search_depth: Option<String>,
}

/// Tavily search response
#[derive(Debug, Deserialize)]
struct TavilySearchResponse {
    /// Search results
    results: Vec<TavilyResult>,
}

/// Individual search result
#[derive(Debug, Deserialize, Serialize)]
struct TavilyResult {
    /// Result title
    title: String,
    /// Result URL
    url: String,
    /// Content snippet
    content: String,
    /// Relevance score (0.0-1.0)
    score: f64,
}

// =============================================================================
// Registration
// =============================================================================

/// Register web_search tool with the unified registry
pub async fn register_web_search_tool(registry: &UnifiedToolRegistry) {
    let handler: BuiltinToolHandler = Arc::new(|input: &Value| {
        let input = input.clone();
        Box::pin(async move { execute_web_search(&input).await })
    });

    registry
        .register_builtin(
            "web_search",
            "Search the web using Tavily API. Returns a list of relevant web pages with titles, URLs, and content snippets. Requires TAVILY_API_KEY environment variable.",
            json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "Search query"
                    },
                    "num_results": {
                        "type": "integer",
                        "description": "Number of results to return (default: 5, max: 10)",
                        "minimum": 1,
                        "maximum": 10,
                        "default": 5
                    },
                    "search_depth": {
                        "type": "string",
                        "description": "Search depth: 'basic' for quick results, 'advanced' for comprehensive search",
                        "enum": ["basic", "advanced"],
                        "default": "basic"
                    }
                },
                "required": ["query"]
            }),
            handler,
        )
        .await;

    tracing::info!("Registered prebuilt tool: web_search");
}

// =============================================================================
// Execution
// =============================================================================

/// Execute web search
async fn execute_web_search(input: &Value) -> String {
    // Extract query parameter
    let query = match input.get("query").and_then(|v| v.as_str()) {
        Some(q) if !q.trim().is_empty() => q.trim().to_string(),
        Some(_) => return "Error: query cannot be empty".to_string(),
        None => return "Error: missing required parameter 'query'".to_string(),
    };

    // Extract optional num_results parameter
    let num_results = input
        .get("num_results")
        .and_then(|v| v.as_u64())
        .map(|n| n as u32)
        .unwrap_or(SEARCH_RESULTS_DEFAULT);

    // Validate num_results
    if num_results > SEARCH_RESULTS_MAX {
        return format!(
            "Error: num_results cannot exceed {} (got: {})",
            SEARCH_RESULTS_MAX, num_results
        );
    }

    if num_results == 0 {
        return "Error: num_results must be at least 1".to_string();
    }

    // Extract optional search_depth parameter
    let search_depth = input.get("search_depth").and_then(|v| v.as_str()).map(|s| {
        match s {
            "basic" | "advanced" => s.to_string(),
            _ => "basic".to_string(), // Default to basic if invalid
        }
    });

    // Get API key from environment
    let api_key = match std::env::var("TAVILY_API_KEY") {
        Ok(key) if !key.is_empty() => key,
        Ok(_) => return "Error: TAVILY_API_KEY environment variable is empty".to_string(),
        Err(_) => return "Error: TAVILY_API_KEY environment variable not set. Please set your Tavily API key.".to_string(),
    };

    // Execute search
    match perform_tavily_search(&api_key, &query, num_results, search_depth).await {
        Ok(results) => format_search_results(&results),
        Err(e) => format!("Error: {}", e),
    }
}

/// Perform Tavily API search
async fn perform_tavily_search(
    api_key: &str,
    query: &str,
    max_results: u32,
    search_depth: Option<String>,
) -> Result<Vec<TavilyResult>, String> {
    // Build request
    let request = TavilySearchRequest {
        api_key: api_key.to_string(),
        query: query.to_string(),
        max_results: Some(max_results),
        search_depth,
    };

    // Create HTTP client with timeout
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(API_TIMEOUT_SECONDS))
        .build()
        .map_err(|e| format!("Failed to create HTTP client: {}", e))?;

    // Send request
    let response = client
        .post(TAVILY_API_URL)
        .json(&request)
        .send()
        .await
        .map_err(|e| {
            if e.is_timeout() {
                format!("Request timed out after {}s", API_TIMEOUT_SECONDS)
            } else if e.is_connect() {
                "Failed to connect to Tavily API. Please check your internet connection."
                    .to_string()
            } else {
                format!("HTTP request failed: {}", e)
            }
        })?;

    // Check status code
    let status = response.status();
    if !status.is_success() {
        let error_body = response
            .text()
            .await
            .unwrap_or_else(|_| "Unable to read error response".to_string());

        return Err(match status.as_u16() {
            401 => "Authentication failed. Please check your TAVILY_API_KEY.".to_string(),
            429 => "Rate limit exceeded. Please try again later.".to_string(),
            500..=599 => format!(
                "Tavily API server error ({}). Please try again later.",
                status
            ),
            _ => format!("API request failed with status {}: {}", status, error_body),
        });
    }

    // Parse response
    let search_response: TavilySearchResponse = response
        .json()
        .await
        .map_err(|e| format!("Failed to parse API response: {}", e))?;

    Ok(search_response.results)
}

/// Format search results for display
fn format_search_results(results: &[TavilyResult]) -> String {
    if results.is_empty() {
        return "No results found".to_string();
    }

    let results_json = serde_json::to_string_pretty(&results)
        .unwrap_or_else(|_| "Error formatting results".to_string());

    format!("Found {} results:\n{}", results.len(), results_json)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::assertions_on_constants)] // TigerStyle: validate constants
    fn test_search_results_validation() {
        // Valid range
        assert!(SEARCH_RESULTS_DEFAULT <= SEARCH_RESULTS_MAX);
        assert!(SEARCH_RESULTS_DEFAULT > 0);
    }

    #[tokio::test]
    async fn test_web_search_missing_query() {
        let input = json!({});
        let result = execute_web_search(&input).await;
        assert!(result.contains("Error: missing required parameter 'query'"));
    }

    #[tokio::test]
    async fn test_web_search_empty_query() {
        let input = json!({
            "query": "   "
        });
        let result = execute_web_search(&input).await;
        assert!(result.contains("Error: query cannot be empty"));
    }

    #[tokio::test]
    async fn test_web_search_num_results_too_large() {
        let input = json!({
            "query": "test",
            "num_results": 100
        });
        let result = execute_web_search(&input).await;
        assert!(result.contains("Error: num_results cannot exceed"));
    }

    #[tokio::test]
    async fn test_web_search_num_results_zero() {
        let input = json!({
            "query": "test",
            "num_results": 0
        });
        let result = execute_web_search(&input).await;
        assert!(result.contains("Error: num_results must be at least 1"));
    }

    #[tokio::test]
    async fn test_web_search_no_api_key() {
        // Temporarily clear API key
        std::env::remove_var("TAVILY_API_KEY");

        let input = json!({
            "query": "test"
        });
        let result = execute_web_search(&input).await;
        assert!(result.contains("Error: TAVILY_API_KEY environment variable not set"));
    }

    #[test]
    fn test_format_empty_results() {
        let results: Vec<TavilyResult> = vec![];
        let output = format_search_results(&results);
        assert_eq!(output, "No results found");
    }

    #[test]
    fn test_format_single_result() {
        let results = vec![TavilyResult {
            title: "Test Title".to_string(),
            url: "https://example.com".to_string(),
            content: "Test content".to_string(),
            score: 0.95,
        }];
        let output = format_search_results(&results);
        assert!(output.contains("Found 1 results"));
        assert!(output.contains("Test Title"));
        assert!(output.contains("https://example.com"));
    }
}
