//! Security Module
//!
//! TigerStyle: Authentication, authorization, and audit logging for Kelpie server.
//!
//! Features:
//! - API key authentication middleware
//! - Rate limiting (future)
//! - Audit logging for tool executions

pub mod audit;
pub mod auth;

pub use audit::{AuditEntry, AuditEvent, AuditLog};
pub use auth::{ApiKeyAuth, ApiKeyAuthLayer};
