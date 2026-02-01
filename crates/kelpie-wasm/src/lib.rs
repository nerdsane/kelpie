//! Kelpie WASM Runtime
//!
//! WASM tool execution for polyglot tool support.
//!
//! TigerStyle: Secure sandboxed execution with explicit resource limits.
//!
//! # Overview
//!
//! Provides:
//! - wasmtime integration for WASM module execution
//! - Module caching for performance
//! - Memory and execution time limits
//! - JSON input/output for tool interface

mod runtime;

pub use runtime::{WasmConfig, WasmError, WasmRuntime, WasmToolResult};
