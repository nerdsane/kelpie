//! Common test utilities for kelpie-server DST tests
//!
//! This module provides shared test infrastructure to avoid code duplication
//! across DST test files.

// TLA+ invariant verification helpers
pub mod invariants;
pub use invariants::InvariantViolation;

// TLA+ bug pattern scenario constructors
pub mod tla_scenarios;

// Simulated HTTP client with fault injection (DST only)
#[cfg(feature = "dst")]
pub mod sim_http;
