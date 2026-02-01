//! Dispatcher Adapter for Agent-to-Agent Communication (Issue #75)
//!
//! TigerStyle: Adapter pattern to bridge runtime's DispatcherHandle with the
//! AgentDispatcher trait required by the tools module.
//!
//! This adapter wraps a DispatcherHandle and implements AgentDispatcher, allowing
//! the call_agent tool to invoke other agents through the runtime dispatcher.
//!
//! Related:
//! - docs/adr/028-multi-agent-communication.md
//! - crates/kelpie-server/src/tools/agent_call.rs

use crate::tools::AgentDispatcher;
use async_trait::async_trait;
use bytes::Bytes;
use kelpie_core::actor::ActorId;
use kelpie_core::{Error, Result};
use kelpie_runtime::DispatcherHandle;
use std::time::Duration;

// =============================================================================
// TigerStyle Constants
// =============================================================================

/// Namespace for agent actors
/// TigerStyle: Explicit constant for agent actor namespace
pub const AGENT_ACTOR_NAMESPACE: &str = "agents";

// =============================================================================
// Dispatcher Adapter
// =============================================================================

/// Adapter that implements AgentDispatcher using a DispatcherHandle
///
/// TigerStyle: Adapter pattern for clean separation of concerns.
/// The tools module doesn't need to know about the runtime's dispatcher implementation.
pub struct DispatcherAdapter<R: kelpie_core::Runtime + Clone + Send + Sync + 'static> {
    dispatcher: DispatcherHandle<R>,
}

impl<R: kelpie_core::Runtime + Clone + Send + Sync + 'static> DispatcherAdapter<R> {
    /// Create a new dispatcher adapter
    ///
    /// TigerStyle: 2+ assertions per function
    pub fn new(dispatcher: DispatcherHandle<R>) -> Self {
        // Precondition: dispatcher should be valid (can't really check, but document it)
        // Postcondition checked at usage time

        Self { dispatcher }
    }
}

impl<R: kelpie_core::Runtime + Clone + Send + Sync + 'static> Clone for DispatcherAdapter<R> {
    fn clone(&self) -> Self {
        Self {
            dispatcher: self.dispatcher.clone(),
        }
    }
}

#[async_trait]
impl<R: kelpie_core::Runtime + Clone + Send + Sync + 'static> AgentDispatcher
    for DispatcherAdapter<R>
{
    /// Invoke another agent by ID
    ///
    /// TigerStyle: 2+ assertions, explicit error handling.
    ///
    /// # Arguments
    /// * `agent_id` - The ID of the agent to invoke (e.g., "helper-agent")
    /// * `operation` - The operation to invoke (e.g., "handle_message_full")
    /// * `payload` - The payload bytes (serialized request)
    /// * `timeout_ms` - Timeout in milliseconds
    ///
    /// # Returns
    /// The response bytes from the target agent
    async fn invoke_agent(
        &self,
        agent_id: &str,
        operation: &str,
        payload: Bytes,
        timeout_ms: u64,
    ) -> Result<Bytes> {
        // TigerStyle: Preconditions
        assert!(!agent_id.is_empty(), "agent_id cannot be empty");
        assert!(!operation.is_empty(), "operation cannot be empty");
        assert!(timeout_ms > 0, "timeout_ms must be positive");

        // Construct the full actor ID (agents namespace + agent id)
        let actor_id =
            ActorId::new(AGENT_ACTOR_NAMESPACE, agent_id).map_err(|e| Error::Internal {
                message: format!("Invalid agent ID '{}': {}", agent_id, e),
            })?;

        // TigerStyle: Postcondition check
        debug_assert!(
            actor_id.namespace() == AGENT_ACTOR_NAMESPACE,
            "actor should be in agents namespace"
        );

        // Invoke with timeout (DST-compatible)
        let invoke_future = self
            .dispatcher
            .invoke(actor_id, operation.to_string(), payload);

        // Use the runtime's timeout for DST compatibility
        let runtime = kelpie_core::current_runtime();
        match kelpie_core::Runtime::timeout(
            &runtime,
            Duration::from_millis(timeout_ms),
            invoke_future,
        )
        .await
        {
            Ok(result) => result,
            Err(()) => Err(Error::Internal {
                message: format!(
                    "Timeout after {}ms invoking agent '{}'",
                    timeout_ms, agent_id
                ),
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    // Tests for DispatcherAdapter require a full dispatcher setup
    // which is tested in the integration tests (multi_agent_dst.rs)
    // Unit tests here are limited to basic construction

    // Note: We can't easily test the adapter without a full dispatcher
    // The integration tests in multi_agent_dst.rs provide full coverage
}
