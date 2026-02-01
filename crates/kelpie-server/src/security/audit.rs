//! Audit Logging
//!
//! TigerStyle: Comprehensive audit logging for forensics and compliance.
//!
//! Captures:
//! - All tool executions with inputs/outputs
//! - Agent state changes
//! - Authentication events
//! - API requests (configurable)
//!
//! Log format: Structured JSON for easy parsing and analysis.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::sync::Arc;
use tokio::sync::RwLock;

// =============================================================================
// TigerStyle Constants
// =============================================================================

/// Maximum audit entries to keep in memory
pub const AUDIT_ENTRIES_COUNT_MAX: usize = 10_000;

/// Maximum input/output size in bytes to log (truncated if larger)
pub const AUDIT_DATA_SIZE_BYTES_MAX: usize = 10_000;

// =============================================================================
// Types
// =============================================================================

/// Audit event type
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AuditEvent {
    /// Tool was executed
    ToolExecution {
        /// Tool name
        tool_name: String,
        /// Agent ID that executed the tool
        agent_id: String,
        /// Tool input (may be truncated)
        input: String,
        /// Tool output (may be truncated)
        output: String,
        /// Execution duration in milliseconds
        duration_ms: u64,
        /// Whether execution succeeded
        success: bool,
        /// Error message if failed
        error: Option<String>,
    },
    /// Agent was created
    AgentCreated {
        agent_id: String,
        agent_name: String,
    },
    /// Agent was deleted
    AgentDeleted { agent_id: String },
    /// Agent state was updated
    AgentUpdated {
        agent_id: String,
        fields_changed: Vec<String>,
    },
    /// Message sent to agent
    MessageSent {
        agent_id: String,
        message_preview: String,
    },
    /// Authentication attempt
    AuthAttempt {
        /// Whether authentication succeeded
        success: bool,
        /// Source IP (if available)
        source_ip: Option<String>,
        /// Reason for failure (if failed)
        reason: Option<String>,
    },
    /// API request (if verbose logging enabled)
    ApiRequest {
        method: String,
        path: String,
        status_code: u16,
        duration_ms: u64,
    },
    /// MCP tool registered
    McpToolRegistered {
        server_name: String,
        tool_name: String,
    },
    /// Custom tool registered
    CustomToolRegistered { tool_name: String, source: String },
    /// Proposal created
    ProposalCreated {
        proposal_id: String,
        proposal_type: String,
        agent_id: String,
    },
    /// Proposal approved
    ProposalApproved {
        proposal_id: String,
        user_id: String,
    },
    /// Proposal rejected
    ProposalRejected {
        proposal_id: String,
        user_id: String,
        reason: Option<String>,
    },
}

/// A single audit log entry
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    /// Unique entry ID
    pub id: u64,
    /// Timestamp
    pub timestamp: DateTime<Utc>,
    /// Event type and details
    pub event: AuditEvent,
    /// Additional context (optional)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}

impl AuditEntry {
    /// Create a new audit entry
    fn new(id: u64, event: AuditEvent, context: Option<serde_json::Value>) -> Self {
        Self {
            id,
            timestamp: Utc::now(),
            event,
            context,
        }
    }
}

/// Audit log store
#[derive(Debug)]
pub struct AuditLog {
    /// Entries in chronological order
    entries: VecDeque<AuditEntry>,
    /// Next entry ID
    next_id: u64,
    /// Maximum entries to keep
    max_entries: usize,
    /// Whether verbose logging is enabled (API requests, etc.)
    verbose: bool,
}

impl AuditLog {
    /// Create a new audit log
    pub fn new() -> Self {
        Self::with_capacity(AUDIT_ENTRIES_COUNT_MAX)
    }

    /// Create with custom capacity
    pub fn with_capacity(max_entries: usize) -> Self {
        let verbose = std::env::var("KELPIE_AUDIT_VERBOSE")
            .map(|v| v.to_lowercase() == "true" || v == "1")
            .unwrap_or(false);

        Self {
            entries: VecDeque::with_capacity(max_entries.min(1000)),
            next_id: 0,
            max_entries,
            verbose,
        }
    }

    /// Log an event
    pub fn log(&mut self, event: AuditEvent) {
        self.log_with_context(event, None);
    }

    /// Log an event with additional context
    pub fn log_with_context(&mut self, event: AuditEvent, context: Option<serde_json::Value>) {
        // Skip verbose events if not enabled
        if !self.verbose && matches!(event, AuditEvent::ApiRequest { .. }) {
            return;
        }

        let entry = AuditEntry::new(self.next_id, event, context);
        self.next_id += 1;

        // Add entry, removing oldest if at capacity
        if self.entries.len() >= self.max_entries {
            self.entries.pop_front();
        }
        self.entries.push_back(entry.clone());

        // Also log to tracing for external collection
        tracing::info!(
            target: "audit",
            entry_id = entry.id,
            event = ?entry.event,
            "Audit log entry"
        );
    }

    /// Log a tool execution
    pub fn log_tool_execution(
        &mut self,
        tool_name: &str,
        agent_id: &str,
        input: &str,
        output: &str,
        duration_ms: u64,
        success: bool,
        error: Option<String>,
    ) {
        // Truncate input/output if too large
        let truncate = |s: &str| -> String {
            if s.len() > AUDIT_DATA_SIZE_BYTES_MAX {
                format!("{}...[truncated]", &s[..AUDIT_DATA_SIZE_BYTES_MAX])
            } else {
                s.to_string()
            }
        };

        self.log(AuditEvent::ToolExecution {
            tool_name: tool_name.to_string(),
            agent_id: agent_id.to_string(),
            input: truncate(input),
            output: truncate(output),
            duration_ms,
            success,
            error,
        });
    }

    /// Get recent entries
    pub fn recent(&self, count: usize) -> Vec<&AuditEntry> {
        self.entries.iter().rev().take(count).collect()
    }

    /// Get entries since a given ID
    pub fn since(&self, id: u64) -> Vec<&AuditEntry> {
        self.entries.iter().filter(|e| e.id > id).collect()
    }

    /// Get entries in a time range
    pub fn in_range(&self, start: DateTime<Utc>, end: DateTime<Utc>) -> Vec<&AuditEntry> {
        self.entries
            .iter()
            .filter(|e| e.timestamp >= start && e.timestamp <= end)
            .collect()
    }

    /// Get tool executions for an agent
    pub fn tool_executions_for_agent(&self, agent_id: &str) -> Vec<&AuditEntry> {
        self.entries
            .iter()
            .filter(|e| matches!(&e.event, AuditEvent::ToolExecution { agent_id: aid, .. } if aid == agent_id))
            .collect()
    }

    /// Get statistics
    pub fn stats(&self) -> AuditStats {
        let mut stats = AuditStats::default();

        for entry in &self.entries {
            match &entry.event {
                AuditEvent::ToolExecution { success, .. } => {
                    stats.tool_executions_total += 1;
                    if *success {
                        stats.tool_executions_success += 1;
                    } else {
                        stats.tool_executions_failed += 1;
                    }
                }
                AuditEvent::AuthAttempt { success, .. } => {
                    stats.auth_attempts_total += 1;
                    if !*success {
                        stats.auth_attempts_failed += 1;
                    }
                }
                AuditEvent::AgentCreated { .. } => stats.agents_created += 1,
                AuditEvent::AgentDeleted { .. } => stats.agents_deleted += 1,
                _ => {}
            }
        }

        stats.entries_total = self.entries.len();
        stats
    }

    /// Export entries as JSON lines
    pub fn export_jsonl(&self) -> String {
        self.entries
            .iter()
            .map(|e| serde_json::to_string(e).unwrap_or_default())
            .collect::<Vec<_>>()
            .join("\n")
    }
}

impl Default for AuditLog {
    fn default() -> Self {
        Self::new()
    }
}

/// Audit statistics
#[derive(Debug, Default, Serialize)]
pub struct AuditStats {
    pub entries_total: usize,
    pub tool_executions_total: u64,
    pub tool_executions_success: u64,
    pub tool_executions_failed: u64,
    pub auth_attempts_total: u64,
    pub auth_attempts_failed: u64,
    pub agents_created: u64,
    pub agents_deleted: u64,
}

/// Thread-safe audit log
pub type SharedAuditLog = Arc<RwLock<AuditLog>>;

/// Create a new shared audit log
pub fn new_shared_log() -> SharedAuditLog {
    Arc::new(RwLock::new(AuditLog::new()))
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_audit_log_basic() {
        let mut log = AuditLog::with_capacity(100);

        log.log_tool_execution(
            "shell",
            "agent_123",
            "ls -la",
            "file1.txt\nfile2.txt",
            50,
            true,
            None,
        );

        assert_eq!(log.entries.len(), 1);
        let stats = log.stats();
        assert_eq!(stats.tool_executions_total, 1);
        assert_eq!(stats.tool_executions_success, 1);
    }

    #[test]
    fn test_audit_log_capacity() {
        let mut log = AuditLog::with_capacity(5);

        for i in 0..10 {
            log.log(AuditEvent::AgentCreated {
                agent_id: format!("agent_{}", i),
                agent_name: format!("Agent {}", i),
            });
        }

        // Should have at most 5 entries
        assert_eq!(log.entries.len(), 5);
        // Should be the last 5
        assert_eq!(
            log.entries.front().unwrap().id,
            5,
            "oldest entry should be id 5"
        );
    }

    #[test]
    fn test_audit_log_truncation() {
        let mut log = AuditLog::new();

        let large_input = "x".repeat(AUDIT_DATA_SIZE_BYTES_MAX + 1000);
        log.log_tool_execution("shell", "agent", &large_input, "ok", 10, true, None);

        if let AuditEvent::ToolExecution { input, .. } = &log.entries[0].event {
            assert!(input.ends_with("...[truncated]"));
            assert!(input.len() <= AUDIT_DATA_SIZE_BYTES_MAX + 20);
        } else {
            panic!("Expected ToolExecution event");
        }
    }
}
