//! Agent group API endpoints
//!
//! TigerStyle: Letta-compatible agent group management and routing.

use crate::api::ApiError;
use axum::{
    extract::{Path, Query, State},
    routing::{get, post},
    Json, Router,
};
use chrono::Utc;
use kelpie_core::Runtime;
use kelpie_server::llm::ChatMessage;
use kelpie_server::models::{
    AgentGroup, CreateAgentGroupRequest, CreateMessageRequest, RoutingPolicy,
    UpdateAgentGroupRequest,
};
use kelpie_server::state::AppState;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::instrument;

/// Query parameters for listing agent groups
#[derive(Debug, Deserialize)]
pub struct ListGroupsQuery {
    pub name: Option<String>,
    /// Cursor for pagination (Kelpie's native parameter)
    pub cursor: Option<String>,
    /// Cursor for pagination (Letta SDK compatibility - alias for cursor)
    pub after: Option<String>,
    pub limit: Option<usize>,
}

/// Response for listing agent groups
#[derive(Debug, Serialize)]
pub struct ListGroupsResponse {
    pub groups: Vec<AgentGroup>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub next_cursor: Option<String>,
}

/// Response for group message routing
#[derive(Debug, Serialize)]
pub struct GroupMessageResponse {
    pub responses: Vec<GroupMessageItem>,
}

#[derive(Debug, Serialize)]
pub struct GroupMessageItem {
    pub agent_id: String,
    pub response: Value,
}

/// Create agent group routes
pub fn router<R: Runtime + 'static>() -> Router<AppState<R>> {
    Router::new()
        .route("/agent-groups", get(list_groups).post(create_group))
        .route(
            "/agent-groups/:group_id",
            get(get_group).patch(update_group).delete(delete_group),
        )
        .route("/agent-groups/:group_id/messages", post(send_group_message))
}

/// Create a new agent group
#[instrument(skip(state, request), level = "info")]
pub async fn create_group<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Json(request): Json<CreateAgentGroupRequest>,
) -> Result<Json<AgentGroup>, ApiError> {
    // Validate name if provided
    if let Some(ref name) = request.name {
        if name.trim().is_empty() {
            return Err(ApiError::bad_request("group name cannot be empty"));
        }
    }

    // Validate agent IDs
    for agent_id in &request.agent_ids {
        let exists = state
            .get_agent_async(agent_id)
            .await?
            .ok_or_else(|| ApiError::not_found("Agent", agent_id))?;
        let _ = exists;
    }

    let group = AgentGroup::from_request(request);
    state.add_agent_group(group.clone()).await?;

    tracing::info!(group_id = %group.id, "created agent group");
    Ok(Json(group))
}

/// List agent groups
#[instrument(skip(state, query), level = "info")]
pub async fn list_groups<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Query(query): Query<ListGroupsQuery>,
) -> Result<Json<ListGroupsResponse>, ApiError> {
    let (mut groups, _) = state.list_agent_groups(None)?;

    if let Some(name_filter) = query.name {
        groups.retain(|g| g.name.contains(&name_filter));
    }

    let limit = query.limit.unwrap_or(50).min(100);
    let cursor = query.cursor.as_deref().or(query.after.as_deref());
    let start_idx = if let Some(cursor) = cursor {
        groups
            .iter()
            .position(|g| g.id == cursor)
            .map(|i| i + 1)
            .unwrap_or(0)
    } else {
        0
    };

    let page: Vec<_> = groups.into_iter().skip(start_idx).take(limit + 1).collect();
    let (items, next_cursor) = if page.len() > limit {
        let items: Vec<_> = page.into_iter().take(limit).collect();
        let next_cursor = items.last().map(|g| g.id.clone());
        (items, next_cursor)
    } else {
        (page, None)
    };

    Ok(Json(ListGroupsResponse {
        groups: items,
        next_cursor,
    }))
}

/// Get agent group details
#[instrument(skip(state), fields(group_id = %group_id), level = "info")]
pub async fn get_group<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(group_id): Path<String>,
) -> Result<Json<AgentGroup>, ApiError> {
    let group = state
        .get_agent_group(&group_id)?
        .ok_or_else(|| ApiError::not_found("AgentGroup", &group_id))?;
    Ok(Json(group))
}

/// Update agent group
#[instrument(skip(state, request), fields(group_id = %group_id), level = "info")]
pub async fn update_group<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(group_id): Path<String>,
    Json(request): Json<UpdateAgentGroupRequest>,
) -> Result<Json<AgentGroup>, ApiError> {
    let mut group = state
        .get_agent_group(&group_id)?
        .ok_or_else(|| ApiError::not_found("AgentGroup", &group_id))?;

    // Validate agent IDs being added
    for agent_id in &request.add_agent_ids {
        let _ = state
            .get_agent_async(agent_id)
            .await?
            .ok_or_else(|| ApiError::not_found("Agent", agent_id))?;
    }

    group.apply_update(request);

    if group.last_routed_index >= group.agent_ids.len() {
        group.last_routed_index = 0;
    }

    state.update_agent_group(group.clone()).await?;
    Ok(Json(group))
}

/// Delete agent group
#[instrument(skip(state), fields(group_id = %group_id), level = "info")]
pub async fn delete_group<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(group_id): Path<String>,
) -> Result<(), ApiError> {
    state.delete_agent_group(&group_id).await?;
    Ok(())
}

/// Send message to agent group
#[instrument(skip(state, request), fields(group_id = %group_id), level = "info")]
async fn send_group_message<R: Runtime + 'static>(
    State(state): State<AppState<R>>,
    Path(group_id): Path<String>,
    Json(request): Json<CreateMessageRequest>,
) -> Result<Json<GroupMessageResponse>, ApiError> {
    let mut group = state
        .get_agent_group(&group_id)?
        .ok_or_else(|| ApiError::not_found("AgentGroup", &group_id))?;

    let (_, content) = request
        .effective_content()
        .ok_or_else(|| ApiError::bad_request("message content cannot be empty"))?;

    if group.agent_ids.is_empty() {
        return Err(ApiError::bad_request("agent group has no members"));
    }

    let content_with_context = apply_group_context(&group.shared_state, &content);

    let responses = match group.routing_policy {
        RoutingPolicy::RoundRobin => {
            let agent_id = select_round_robin(&mut group)?;
            let response =
                send_to_agent(&state, &agent_id, &content_with_context, request.clone()).await?;
            vec![GroupMessageItem { agent_id, response }]
        }
        RoutingPolicy::Broadcast => {
            let mut results = Vec::new();
            for agent_id in group.agent_ids.clone() {
                let response =
                    send_to_agent(&state, &agent_id, &content_with_context, request.clone())
                        .await?;
                results.push(GroupMessageItem { agent_id, response });
            }
            results
        }
        RoutingPolicy::Intelligent => {
            let agent_id = select_intelligent(&state, &group, &content).await?;
            let response =
                send_to_agent(&state, &agent_id, &content_with_context, request.clone()).await?;
            vec![GroupMessageItem { agent_id, response }]
        }
        // Letta compatibility - these types fall back to round_robin for now
        RoutingPolicy::Supervisor
        | RoutingPolicy::Dynamic
        | RoutingPolicy::Sleeptime
        | RoutingPolicy::VoiceSleeptime
        | RoutingPolicy::Swarm => {
            let agent_id = select_round_robin(&mut group)?;
            let response =
                send_to_agent(&state, &agent_id, &content_with_context, request.clone()).await?;
            vec![GroupMessageItem { agent_id, response }]
        }
    };

    for item in &responses {
        append_shared_state(&mut group, &item.agent_id, &item.response);
    }

    group.updated_at = Utc::now();
    state.update_agent_group(group).await?;

    Ok(Json(GroupMessageResponse { responses }))
}

fn select_round_robin(group: &mut AgentGroup) -> Result<String, ApiError> {
    if group.agent_ids.is_empty() {
        return Err(ApiError::bad_request("agent group has no members"));
    }

    let idx = group.last_routed_index % group.agent_ids.len();
    let agent_id = group.agent_ids[idx].clone();
    group.last_routed_index = (idx + 1) % group.agent_ids.len();
    Ok(agent_id)
}

async fn select_intelligent<R: Runtime + 'static>(
    state: &AppState<R>,
    group: &AgentGroup,
    content: &str,
) -> Result<String, ApiError> {
    let llm = state
        .llm()
        .ok_or_else(|| ApiError::internal("LLM not configured for intelligent routing"))?;

    let prompt = format!(
        "Select the best agent ID to handle this message.\nAgents: {}\nMessage: {}\nRespond with only the agent ID.",
        group.agent_ids.join(", "),
        content
    );

    let response = llm
        .complete(vec![ChatMessage {
            role: "user".to_string(),
            content: prompt,
        }])
        .await
        .map_err(|e| ApiError::internal(format!("LLM routing failed: {}", e)))?;

    let trimmed = response.content.trim();
    if group.agent_ids.contains(&trimmed.to_string()) {
        return Ok(trimmed.to_string());
    }

    for agent_id in &group.agent_ids {
        if response.content.contains(agent_id) {
            return Ok(agent_id.clone());
        }
    }

    Err(ApiError::bad_request("LLM did not select a valid agent ID"))
}

fn apply_group_context(shared_state: &Value, content: &str) -> String {
    if shared_state == &Value::Null || shared_state == &Value::Array(vec![]) {
        return content.to_string();
    }

    format!("Group context: {}\n\n{}", shared_state, content)
}

fn append_shared_state(group: &mut AgentGroup, agent_id: &str, response: &Value) {
    let entry = serde_json::json!({
        "agent_id": agent_id,
        "response": response,
        "timestamp": Utc::now().to_rfc3339(),
    });

    match group.shared_state.as_array_mut() {
        Some(arr) => arr.push(entry),
        None => {
            group.shared_state = serde_json::json!([entry]);
        }
    }
}

async fn send_to_agent<R: Runtime + 'static>(
    state: &AppState<R>,
    agent_id: &str,
    content: &str,
    request: CreateMessageRequest,
) -> Result<Value, ApiError> {
    let mut request = request;
    request.content = content.to_string();
    request.msg = None;
    request.messages = None;

    let response =
        crate::api::messages::handle_message_request(state.clone(), agent_id.to_string(), request)
            .await?;

    Ok(serde_json::to_value(response).unwrap_or_else(|_| {
        serde_json::json!({
            "error": "failed to serialize response"
        })
    }))
}
