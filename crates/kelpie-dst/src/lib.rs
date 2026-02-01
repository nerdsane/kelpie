//! Kelpie DST - Deterministic Simulation Testing
//!
//! TigerBeetle/FoundationDB-style deterministic simulation testing framework.
//!
//! # Overview
//!
//! DST enables testing distributed systems with:
//! - Deterministic time control (SimClock)
//! - Reproducible random numbers (DeterministicRng)
//! - Fault injection (FaultInjector)
//! - Simulated storage and network
//!
//! # Example
//!
//! ```rust,ignore
//! use kelpie_dst::{Simulation, SimConfig, FaultConfig, FaultType};
//!
//! #[test]
//! fn test_with_faults() {
//!     let config = SimConfig::from_env_or_random();
//!     let mut sim = Simulation::new(config)
//!         .with_fault(FaultConfig::new(FaultType::StorageWriteFail, 0.1));
//!
//!     sim.run(|env| async move {
//!         // Test code using env.clock(), env.rng(), env.storage()
//!         Ok(())
//!     }).unwrap();
//! }
//! ```
//!
//! # TigerStyle
//!
//! - All operations are deterministic given the same seed
//! - Always log the seed for reproducibility
//! - Explicit fault types and probabilities

pub mod agent;
pub mod clock;
pub mod fault;
pub mod http;
pub mod invariants;
pub mod liveness;
pub mod llm;
pub mod network;
pub mod rng;
pub mod sandbox;
pub mod sandbox_io;
pub mod simulation;
pub mod storage;
pub mod teleport;
pub mod time;
pub mod vm;

pub use agent::{AgentTestConfig, AgentTestState, BlockTestState, SimAgentEnv};
pub use clock::SimClock;
pub use fault::{FaultConfig, FaultInjector, FaultInjectorBuilder, FaultType};
pub use http::{MockResponse, RecordedRequest, SimHttpClient};
pub use invariants::{
    AtomicVisibility, ConsistentHolder, Durability, FencingTokenMonotonic, Invariant,
    InvariantChecker, InvariantCheckingSimulation, InvariantViolation, LeaseInfo, LeaseUniqueness,
    NoSplitBrain, NodeInfo, NodeState, NodeStatus, PlacementConsistency, ReadYourWrites,
    SingleActivation, Snapshot, SnapshotConsistency, SystemState, Transaction, TransactionState,
    WalEntry, WalEntryStatus,
};
pub use kelpie_core::teleport::{Architecture, SnapshotKind, TeleportPackage, VmSnapshotBlob};
pub use llm::{
    SimChatMessage, SimCompletionResponse, SimLlmClient, SimToolCall, SimToolDefinition,
};
pub use network::SimNetwork;
pub use rng::DeterministicRng;
pub use sandbox::{SimSandbox, SimSandboxFactory};
pub use sandbox_io::{SimSandboxIO, SimSandboxIOFactory};
pub use simulation::{SimConfig, SimEnvironment, Simulation};
pub use storage::SimStorage;
pub use teleport::SimTeleportStorage;
pub use time::{RealTime, SimTime};
pub use vm::{SimVm, SimVmFactory};

// Liveness property verification
pub use liveness::{
    verify_eventually, verify_leads_to, BoundedLiveness, LivenessResult, LivenessViolation,
    SystemStateSnapshot, LIVENESS_CHECK_INTERVAL_MS_DEFAULT, LIVENESS_STEPS_MAX,
    LIVENESS_TIMEOUT_MS_DEFAULT,
};
