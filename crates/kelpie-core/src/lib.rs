//! Kelpie Core
//!
//! Core types, errors, and constants for the Kelpie virtual actor system.
//!
//! # Overview
//!
//! Kelpie is a distributed virtual actor system with linearizability guarantees,
//! designed as infrastructure for AI agent orchestration and general stateful
//! distributed systems.
//!
//! # TigerStyle
//!
//! This crate follows [TigerStyle](https://github.com/tigerbeetle/tigerbeetle/blob/main/docs/TIGER_STYLE.md)
//! engineering principles:
//! - Safety > Performance > Developer Experience
//! - Explicit limits with big-endian naming (e.g., `ACTOR_ID_LENGTH_BYTES_MAX`)
//! - 2+ assertions per function
//! - No recursion (bounded iteration only)

pub mod actor;
pub mod config;
pub mod constants;
pub mod error;
pub mod http;
pub mod io;
pub mod metrics;
pub mod runtime;
pub mod telemetry;
pub mod teleport;

pub use actor::{
    Actor, ActorContext, ActorId, ActorRef, ArcContextKV, BufferedKVOp, BufferingContextKV,
    ContextKV, NoOpKV,
};
pub use config::KelpieConfig;
pub use constants::*;
pub use error::{Error, Result};
pub use io::{IoContext, RngProvider, StdRngProvider, TimeProvider, WallClockTime};
pub use runtime::{
    current_runtime, CurrentRuntime, Instant, JoinError, JoinHandle, Runtime, TokioRuntime,
};

#[cfg(madsim)]
pub use runtime::MadsimRuntime;
pub use telemetry::{init_telemetry, TelemetryConfig, TelemetryGuard};
pub use teleport::{
    Architecture, SnapshotKind, TeleportPackage, TeleportSnapshotError, TeleportStorage,
    TeleportStorageError, TeleportStorageResult, VmSnapshotBlob,
};
