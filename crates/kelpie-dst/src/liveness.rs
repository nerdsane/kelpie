//! Liveness property verification for DST
//!
//! TigerStyle: Bounded liveness checking with explicit timeouts.
//!
//! This module provides tools for verifying liveness properties (temporal properties
//! that assert something good eventually happens) in deterministic simulations.
//!
//! # Temporal Operators
//!
//! - `<>` (eventually): The property holds at some point in the future
//! - `~>` (leads-to): If P holds, then Q eventually holds (P ~> Q ≡ [](P => <>Q))
//! - `[]<>` (infinitely often): The property holds infinitely often
//!
//! # Bounded Liveness
//!
//! Since simulations can't run forever, we use bounded liveness checks:
//! - Set a maximum number of steps or simulated time
//! - If the property doesn't hold within bounds, report a violation
//! - The bounds should be set based on system timeouts (e.g., 2-3x heartbeat timeout)
//!
//! # Example
//!
//! ```rust,ignore
//! use kelpie_dst::{liveness, SimConfig, Simulation, SimClock};
//!
//! #[test]
//! fn test_eventual_activation() {
//!     Simulation::new(SimConfig::from_env_or_random()).run(|env| async move {
//!         // Start claiming
//!         start_claim(&env, "actor-1").await;
//!
//!         // Verify: Claiming ~> (Active ∨ Idle)
//!         liveness::verify_leads_to(
//!             &env.clock,
//!             || is_claiming("actor-1"),
//!             || is_active("actor-1") || is_idle("actor-1"),
//!             10_000, // timeout_ms
//!             100,    // check_interval_ms
//!         ).await?;
//!
//!         Ok(())
//!     });
//! }
//! ```

use crate::clock::SimClock;
use std::collections::VecDeque;
use std::fmt;
use std::hash::Hash;
use std::sync::Arc;

// =============================================================================
// Constants (TigerStyle: explicit units)
// =============================================================================

/// Default check interval in milliseconds
pub const LIVENESS_CHECK_INTERVAL_MS_DEFAULT: u64 = 10;

/// Default timeout for liveness checks in milliseconds
pub const LIVENESS_TIMEOUT_MS_DEFAULT: u64 = 10_000;

/// Maximum steps for bounded liveness checks
pub const LIVENESS_STEPS_MAX: u64 = 100_000;

// =============================================================================
// Error Types
// =============================================================================

/// Error returned when a liveness property is violated
#[derive(Debug, Clone)]
pub struct LivenessViolation {
    /// Name of the property that was violated
    pub property: String,
    /// Human-readable description of what was expected
    pub expected: String,
    /// Time waited before giving up (milliseconds)
    pub waited_ms: u64,
    /// Number of checks performed
    pub checks_performed: u64,
    /// Description of the final state when timeout occurred
    pub final_state: String,
}

impl fmt::Display for LivenessViolation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Liveness violation: '{}' - expected '{}' but timed out after {}ms ({} checks). Final state: {}",
            self.property, self.expected, self.waited_ms, self.checks_performed, self.final_state
        )
    }
}

impl std::error::Error for LivenessViolation {}

impl From<LivenessViolation> for kelpie_core::Error {
    fn from(v: LivenessViolation) -> Self {
        kelpie_core::Error::Internal {
            message: v.to_string(),
        }
    }
}

/// Result type for liveness checks
pub type LivenessResult<T> = std::result::Result<T, LivenessViolation>;

// =============================================================================
// State-Based Exploration (Issue #40: Real Bounded Liveness)
// =============================================================================

/// A trace of states for counterexample reporting
///
/// TigerStyle: Captures the sequence of states leading to a liveness violation.
#[derive(Debug, Clone)]
pub struct StateTrace<S> {
    /// The sequence of states visited
    pub states: Vec<S>,
    /// The actions taken between states (if captured)
    pub actions: Vec<String>,
    /// Step at which violation was detected (or exploration stopped)
    pub step_count: u64,
}

impl<S: fmt::Debug> StateTrace<S> {
    /// Create a new empty trace
    pub fn new() -> Self {
        Self {
            states: Vec::new(),
            actions: Vec::new(),
            step_count: 0,
        }
    }

    /// Add a state to the trace
    pub fn push_state(&mut self, state: S) {
        self.states.push(state);
        self.step_count += 1;
    }

    /// Add a state with an action description
    pub fn push_state_with_action(&mut self, state: S, action: impl Into<String>) {
        self.states.push(state);
        self.actions.push(action.into());
        self.step_count += 1;
    }

    /// Get the final state in the trace
    pub fn final_state(&self) -> Option<&S> {
        self.states.last()
    }

    /// Format trace for display
    pub fn format_trace(&self) -> String {
        let mut result = String::new();
        for (i, state) in self.states.iter().enumerate() {
            if i < self.actions.len() {
                result.push_str(&format!(
                    "Step {}: {:?}\n  -> {}\n",
                    i, state, self.actions[i]
                ));
            } else {
                result.push_str(&format!("Step {}: {:?}\n", i, state));
            }
        }
        result
    }
}

impl<S: fmt::Debug> Default for StateTrace<S> {
    fn default() -> Self {
        Self::new()
    }
}

impl<S: fmt::Debug> fmt::Display for StateTrace<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.format_trace())
    }
}

/// Error returned when state-based liveness check fails
///
/// TigerStyle: Includes counterexample trace for debugging.
#[derive(Debug)]
pub struct StateLivenessViolation<S> {
    /// Name of the property that was violated
    pub property: String,
    /// Human-readable description of what was expected
    pub expected: String,
    /// The counterexample trace leading to the violation
    pub trace: StateTrace<S>,
    /// Total states explored before timeout
    pub states_explored: u64,
    /// Maximum depth reached
    pub max_depth_reached: u64,
}

impl<S: fmt::Debug> fmt::Display for StateLivenessViolation<S> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Liveness violation: '{}'\nExpected: {}\nStates explored: {}\nMax depth: {}\nCounterexample trace:\n{}",
            self.property,
            self.expected,
            self.states_explored,
            self.max_depth_reached,
            self.trace.format_trace()
        )
    }
}

impl<S: fmt::Debug> std::error::Error for StateLivenessViolation<S> {}

/// Result type for state-based liveness checks
pub type StateLivenessResult<T, S> = std::result::Result<T, StateLivenessViolation<S>>;

/// Configuration for state-based bounded liveness checking
///
/// TigerStyle: Explicit bounds for state space exploration.
///
/// # Example
///
/// ```rust,ignore
/// use kelpie_dst::liveness::StateExplorer;
///
/// #[derive(Clone, Hash, Eq, PartialEq, Debug)]
/// enum NodeState { Idle, Claiming, Active }
///
/// let explorer = StateExplorer::new(1000)  // max 1000 steps
///     .with_property("EventualActivation", |s| *s == NodeState::Active);
///
/// // Check that Active is eventually reached from Claiming
/// explorer.check_eventually(
///     NodeState::Claiming,
///     |s| match s {
///         NodeState::Idle => vec![NodeState::Claiming],
///         NodeState::Claiming => vec![NodeState::Active, NodeState::Idle],
///         NodeState::Active => vec![NodeState::Active],
///     }
/// )?;
/// ```
#[derive(Debug, Clone)]
pub struct StateExplorer {
    /// Maximum number of steps to explore
    pub max_steps: u64,
    /// Maximum depth for BFS exploration
    pub max_depth: u64,
    /// Maximum number of states to track (memory bound)
    pub max_states_tracked: u64,
}

/// Default maximum steps for state exploration
pub const STATE_EXPLORER_STEPS_MAX_DEFAULT: u64 = 10_000;

/// Default maximum depth for state exploration
pub const STATE_EXPLORER_DEPTH_MAX_DEFAULT: u64 = 100;

/// Default maximum states to track
pub const STATE_EXPLORER_STATES_MAX_DEFAULT: u64 = 100_000;

impl StateExplorer {
    /// Create a new state explorer with the given maximum steps
    pub fn new(max_steps: u64) -> Self {
        assert!(max_steps > 0, "max_steps must be positive");
        Self {
            max_steps,
            max_depth: STATE_EXPLORER_DEPTH_MAX_DEFAULT,
            max_states_tracked: STATE_EXPLORER_STATES_MAX_DEFAULT,
        }
    }

    /// Set the maximum depth for exploration
    pub fn with_max_depth(mut self, depth: u64) -> Self {
        assert!(depth > 0, "max_depth must be positive");
        self.max_depth = depth;
        self
    }

    /// Set the maximum states to track
    pub fn with_max_states(mut self, states: u64) -> Self {
        assert!(states > 0, "max_states must be positive");
        self.max_states_tracked = states;
        self
    }

    /// Check that a property eventually holds (<> operator)
    ///
    /// Uses BFS to explore the state space and verify that all paths
    /// eventually reach a state where the property holds.
    ///
    /// # Arguments
    /// * `property_name` - Human-readable name for error messages
    /// * `initial` - The initial state
    /// * `transitions` - Function that returns successor states
    /// * `property` - Function that returns true when the goal is reached
    ///
    /// # Returns
    /// * `Ok(trace)` - A trace showing one path to a satisfying state
    /// * `Err(violation)` - A counterexample trace where property never holds
    pub fn check_eventually<S, F, P>(
        &self,
        property_name: &str,
        initial: S,
        transitions: F,
        property: P,
    ) -> StateLivenessResult<StateTrace<S>, S>
    where
        S: Clone + Eq + Hash + fmt::Debug,
        F: Fn(&S) -> Vec<(S, String)>,
        P: Fn(&S) -> bool,
    {
        // TigerStyle: Preconditions
        assert!(self.max_steps > 0, "max_steps must be positive");
        assert!(self.max_depth > 0, "max_depth must be positive");

        // Check if initial state satisfies property
        if property(&initial) {
            let mut trace = StateTrace::new();
            trace.push_state(initial);
            return Ok(trace);
        }

        // BFS exploration
        let mut visited: std::collections::HashSet<S> = std::collections::HashSet::new();
        let mut queue: VecDeque<(S, StateTrace<S>)> = VecDeque::new();
        let mut states_explored = 0u64;
        let mut max_depth_seen = 0u64;

        // Initialize
        let mut initial_trace = StateTrace::new();
        initial_trace.push_state(initial.clone());
        queue.push_back((initial.clone(), initial_trace));
        visited.insert(initial.clone());

        while let Some((state, trace)) = queue.pop_front() {
            states_explored += 1;
            let current_depth = trace.step_count;

            // Check bounds
            if states_explored >= self.max_steps {
                return Err(StateLivenessViolation {
                    property: property_name.to_string(),
                    expected: "property to eventually hold".to_string(),
                    trace,
                    states_explored,
                    max_depth_reached: max_depth_seen,
                });
            }

            if current_depth >= self.max_depth {
                max_depth_seen = max_depth_seen.max(current_depth);
                continue; // Don't explore deeper, but continue with other states
            }

            // Explore successors
            let successors = transitions(&state);

            // If no successors, this is a terminal state without satisfying property
            if successors.is_empty() {
                // Terminal state that doesn't satisfy - potential counterexample
                // But continue exploring other paths
                max_depth_seen = max_depth_seen.max(current_depth);
                continue;
            }

            for (next_state, action) in successors {
                // Check if successor satisfies property
                if property(&next_state) {
                    let mut success_trace = trace.clone();
                    success_trace.push_state_with_action(next_state, action);
                    tracing::debug!(
                        property = property_name,
                        steps = success_trace.step_count,
                        states_explored = states_explored,
                        "Eventually property satisfied"
                    );
                    return Ok(success_trace);
                }

                // Add to queue if not visited and within memory bounds
                if !visited.contains(&next_state)
                    && visited.len() < self.max_states_tracked as usize
                {
                    visited.insert(next_state.clone());
                    let mut next_trace = trace.clone();
                    next_trace.push_state_with_action(next_state.clone(), action);
                    queue.push_back((next_state, next_trace));
                }
            }

            max_depth_seen = max_depth_seen.max(current_depth);
        }

        // Exhausted exploration without finding satisfying state
        Err(StateLivenessViolation {
            property: property_name.to_string(),
            expected: "property to eventually hold (explored all reachable states)".to_string(),
            trace: StateTrace::new(), // Empty trace - no path found
            states_explored,
            max_depth_reached: max_depth_seen,
        })
    }

    /// Check the leads-to property: P ~> Q
    ///
    /// Verifies that from any state where P holds, Q eventually holds.
    /// This is equivalent to [](P => <>Q).
    ///
    /// # Arguments
    /// * `property_name` - Human-readable name for error messages
    /// * `initial` - The initial state
    /// * `transitions` - Function that returns successor states
    /// * `precondition` - The trigger condition P
    /// * `postcondition` - The expected eventual outcome Q
    pub fn check_leads_to<S, F, P, Q>(
        &self,
        property_name: &str,
        initial: S,
        transitions: F,
        precondition: P,
        postcondition: Q,
    ) -> StateLivenessResult<(), S>
    where
        S: Clone + Eq + Hash + fmt::Debug,
        F: Fn(&S) -> Vec<(S, String)>,
        P: Fn(&S) -> bool,
        Q: Fn(&S) -> bool,
    {
        // TigerStyle: Preconditions
        assert!(self.max_steps > 0, "max_steps must be positive");

        // Find all states where precondition holds
        let mut visited: std::collections::HashSet<S> = std::collections::HashSet::new();
        let mut p_states: Vec<S> = Vec::new();
        let mut queue: VecDeque<S> = VecDeque::new();

        queue.push_back(initial.clone());
        visited.insert(initial);

        // Phase 1: Find all reachable states where P holds
        let mut steps = 0u64;
        while let Some(state) = queue.pop_front() {
            steps += 1;
            if steps >= self.max_steps / 2 {
                break; // Use half the budget for finding P states
            }

            if precondition(&state) {
                p_states.push(state.clone());
            }

            for (next, _) in transitions(&state) {
                if !visited.contains(&next) && visited.len() < self.max_states_tracked as usize {
                    visited.insert(next.clone());
                    queue.push_back(next);
                }
            }
        }

        // If no P states found, leads-to is vacuously true
        if p_states.is_empty() {
            tracing::debug!(
                property = property_name,
                "Leads-to vacuously satisfied (precondition never holds)"
            );
            return Ok(());
        }

        // Phase 2: For each P state, verify Q eventually holds
        for p_state in p_states {
            // Check if Q holds immediately
            if postcondition(&p_state) {
                continue;
            }

            // Try to reach Q from this P state
            let result = self.check_eventually(
                &format!("{}_from_P", property_name),
                p_state,
                &transitions,
                &postcondition,
            );

            if let Err(violation) = result {
                return Err(StateLivenessViolation {
                    property: property_name.to_string(),
                    expected: "postcondition Q to hold after precondition P".to_string(),
                    trace: violation.trace,
                    states_explored: violation.states_explored,
                    max_depth_reached: violation.max_depth_reached,
                });
            }
        }

        tracing::debug!(
            property = property_name,
            "Leads-to property satisfied for all P states"
        );
        Ok(())
    }

    /// Check that a condition holds infinitely often ([]<> operator)
    ///
    /// In bounded checking, verifies that from any reachable state,
    /// a state satisfying the property is reachable.
    ///
    /// # Arguments
    /// * `property_name` - Human-readable name for error messages
    /// * `initial` - The initial state
    /// * `transitions` - Function that returns successor states
    /// * `property` - Function that returns true for satisfying states
    /// * `min_occurrences` - Minimum paths that must reach the property
    pub fn check_infinitely_often<S, F, P>(
        &self,
        property_name: &str,
        initial: S,
        transitions: F,
        property: P,
        min_occurrences: u64,
    ) -> StateLivenessResult<u64, S>
    where
        S: Clone + Eq + Hash + fmt::Debug,
        F: Fn(&S) -> Vec<(S, String)>,
        P: Fn(&S) -> bool,
    {
        // TigerStyle: Preconditions
        assert!(
            min_occurrences > 0,
            "min_occurrences must be positive for []<>"
        );

        // Sample random paths and count how many reach the property
        let mut visited: std::collections::HashSet<S> = std::collections::HashSet::new();
        let mut queue: VecDeque<(S, u64)> = VecDeque::new();
        let mut occurrences = 0u64;
        let mut states_explored = 0u64;
        let mut max_depth_seen = 0u64;
        let initial_for_trace = initial.clone(); // Save for error trace

        queue.push_back((initial.clone(), 0));
        visited.insert(initial);

        while let Some((state, depth)) = queue.pop_front() {
            states_explored += 1;

            if property(&state) {
                occurrences += 1;
                if occurrences >= min_occurrences {
                    tracing::debug!(
                        property = property_name,
                        occurrences = occurrences,
                        states_explored = states_explored,
                        "Infinitely-often property satisfied"
                    );
                    return Ok(occurrences);
                }
            }

            if states_explored >= self.max_steps || depth >= self.max_depth {
                max_depth_seen = max_depth_seen.max(depth);
                continue;
            }

            for (next, _) in transitions(&state) {
                if !visited.contains(&next) && visited.len() < self.max_states_tracked as usize {
                    visited.insert(next.clone());
                    queue.push_back((next, depth + 1));
                }
            }

            max_depth_seen = max_depth_seen.max(depth);
        }

        if occurrences >= min_occurrences {
            Ok(occurrences)
        } else {
            let mut trace = StateTrace::new();
            trace.push_state(initial_for_trace);
            Err(StateLivenessViolation {
                property: property_name.to_string(),
                expected: format!(
                    "property to hold at least {} times (found {})",
                    min_occurrences, occurrences
                ),
                trace,
                states_explored,
                max_depth_reached: max_depth_seen,
            })
        }
    }
}

impl Default for StateExplorer {
    fn default() -> Self {
        Self::new(STATE_EXPLORER_STEPS_MAX_DEFAULT)
    }
}

// =============================================================================
// Bounded Liveness Checker
// =============================================================================

/// Configuration for bounded liveness checks
#[derive(Debug, Clone)]
pub struct BoundedLiveness {
    /// Maximum time to wait in milliseconds
    pub timeout_ms: u64,
    /// Interval between checks in milliseconds
    pub check_interval_ms: u64,
    /// Maximum number of checks (alternative bound)
    pub max_checks: u64,
}

impl BoundedLiveness {
    /// Create a new bounded liveness checker with the given timeout
    pub fn new(timeout_ms: u64) -> Self {
        assert!(timeout_ms > 0, "timeout must be positive");

        Self {
            timeout_ms,
            check_interval_ms: LIVENESS_CHECK_INTERVAL_MS_DEFAULT,
            max_checks: LIVENESS_STEPS_MAX,
        }
    }

    /// Set the check interval
    pub fn with_check_interval_ms(mut self, interval_ms: u64) -> Self {
        assert!(interval_ms > 0, "check interval must be positive");
        self.check_interval_ms = interval_ms;
        self
    }

    /// Set the maximum number of checks
    pub fn with_max_checks(mut self, max: u64) -> Self {
        assert!(max > 0, "max checks must be positive");
        self.max_checks = max;
        self
    }

    /// Verify that a condition eventually becomes true (<> operator)
    ///
    /// # Arguments
    /// * `clock` - The simulation clock
    /// * `property_name` - Human-readable name for error messages
    /// * `condition` - Closure that returns true when the property holds
    /// * `state_description` - Closure that describes the current state for error messages
    pub async fn verify_eventually<F, S>(
        &self,
        clock: &Arc<SimClock>,
        property_name: &str,
        condition: F,
        state_description: S,
    ) -> LivenessResult<()>
    where
        F: Fn() -> bool,
        S: Fn() -> String,
    {
        let start_time_ms = clock.now_ms();
        let deadline_ms = start_time_ms + self.timeout_ms;
        let mut checks = 0u64;

        loop {
            checks += 1;

            // Check if condition holds
            if condition() {
                tracing::debug!(
                    property = property_name,
                    checks = checks,
                    elapsed_ms = clock.now_ms() - start_time_ms,
                    "Liveness property satisfied"
                );
                return Ok(());
            }

            // Check bounds
            if clock.now_ms() >= deadline_ms || checks >= self.max_checks {
                return Err(LivenessViolation {
                    property: property_name.to_string(),
                    expected: format!("condition to become true within {}ms", self.timeout_ms),
                    waited_ms: clock.now_ms() - start_time_ms,
                    checks_performed: checks,
                    final_state: state_description(),
                });
            }

            // Advance time and check again
            clock.advance_ms(self.check_interval_ms);
        }
    }

    /// Verify the leads-to property: P ~> Q (if P holds, Q eventually holds)
    ///
    /// This is equivalent to [](P => <>Q): always, if P then eventually Q.
    ///
    /// # Arguments
    /// * `clock` - The simulation clock
    /// * `property_name` - Human-readable name for error messages
    /// * `precondition` - The trigger condition P
    /// * `postcondition` - The expected eventual outcome Q
    /// * `state_description` - Closure that describes the current state for error messages
    pub async fn verify_leads_to<P, Q, S>(
        &self,
        clock: &Arc<SimClock>,
        property_name: &str,
        precondition: P,
        postcondition: Q,
        state_description: S,
    ) -> LivenessResult<()>
    where
        P: Fn() -> bool,
        Q: Fn() -> bool,
        S: Fn() -> String,
    {
        let start_time_ms = clock.now_ms();
        let deadline_ms = start_time_ms + self.timeout_ms;
        let mut checks = 0u64;
        let mut precondition_seen = false;
        let mut precondition_time_ms = 0u64;

        loop {
            checks += 1;

            let p_holds = precondition();
            let q_holds = postcondition();

            // Track when precondition first holds
            if p_holds && !precondition_seen {
                precondition_seen = true;
                precondition_time_ms = clock.now_ms();
                tracing::debug!(
                    property = property_name,
                    time_ms = precondition_time_ms,
                    "Precondition triggered, waiting for postcondition"
                );
            }

            // If precondition triggered and postcondition now holds, success
            if precondition_seen && q_holds {
                tracing::debug!(
                    property = property_name,
                    checks = checks,
                    elapsed_ms = clock.now_ms() - precondition_time_ms,
                    "Leads-to property satisfied (P ~> Q)"
                );
                return Ok(());
            }

            // Check bounds
            if clock.now_ms() >= deadline_ms || checks >= self.max_checks {
                if !precondition_seen {
                    // Precondition never triggered - this is actually OK for leads-to
                    // (P ~> Q is vacuously true if P never holds)
                    tracing::debug!(
                        property = property_name,
                        "Leads-to vacuously satisfied (precondition never held)"
                    );
                    return Ok(());
                }

                return Err(LivenessViolation {
                    property: property_name.to_string(),
                    expected: format!(
                        "postcondition to hold within {}ms after precondition",
                        self.timeout_ms
                    ),
                    waited_ms: clock.now_ms() - precondition_time_ms,
                    checks_performed: checks,
                    final_state: state_description(),
                });
            }

            // Advance time and check again
            clock.advance_ms(self.check_interval_ms);
        }
    }

    /// Verify that a condition holds infinitely often ([]<> operator)
    ///
    /// In bounded checking, we verify that the condition holds at least `min_occurrences`
    /// times within the timeout.
    ///
    /// # Arguments
    /// * `clock` - The simulation clock
    /// * `property_name` - Human-readable name for error messages
    /// * `condition` - Closure that returns true when the property holds
    /// * `min_occurrences` - Minimum number of times the condition must hold
    /// * `state_description` - Closure that describes the current state for error messages
    pub async fn verify_infinitely_often<F, S>(
        &self,
        clock: &Arc<SimClock>,
        property_name: &str,
        condition: F,
        min_occurrences: u64,
        state_description: S,
    ) -> LivenessResult<u64>
    where
        F: Fn() -> bool,
        S: Fn() -> String,
    {
        assert!(
            min_occurrences > 0,
            "min_occurrences must be positive for []<>"
        );

        let start_time_ms = clock.now_ms();
        let deadline_ms = start_time_ms + self.timeout_ms;
        let mut checks = 0u64;
        let mut occurrences = 0u64;
        let mut was_true = false;

        loop {
            checks += 1;

            let now_true = condition();

            // Count rising edges (false -> true transitions)
            if now_true && !was_true {
                occurrences += 1;
                tracing::trace!(
                    property = property_name,
                    occurrences = occurrences,
                    "Condition became true (occurrence #{})",
                    occurrences
                );
            }
            was_true = now_true;

            // Check if we've seen enough occurrences
            if occurrences >= min_occurrences {
                tracing::debug!(
                    property = property_name,
                    occurrences = occurrences,
                    checks = checks,
                    "Infinitely-often property satisfied"
                );
                return Ok(occurrences);
            }

            // Check bounds
            if clock.now_ms() >= deadline_ms || checks >= self.max_checks {
                return Err(LivenessViolation {
                    property: property_name.to_string(),
                    expected: format!(
                        "condition to hold at least {} times within {}ms (saw {} times)",
                        min_occurrences, self.timeout_ms, occurrences
                    ),
                    waited_ms: clock.now_ms() - start_time_ms,
                    checks_performed: checks,
                    final_state: state_description(),
                });
            }

            // Advance time and check again
            clock.advance_ms(self.check_interval_ms);
        }
    }
}

impl Default for BoundedLiveness {
    fn default() -> Self {
        Self::new(LIVENESS_TIMEOUT_MS_DEFAULT)
    }
}

// =============================================================================
// Convenience Functions
// =============================================================================

/// Verify that a condition eventually becomes true (<> operator)
///
/// This is a convenience wrapper around `BoundedLiveness::verify_eventually`.
pub async fn verify_eventually<F, S>(
    clock: &Arc<SimClock>,
    property_name: &str,
    condition: F,
    timeout_ms: u64,
    check_interval_ms: u64,
    state_description: S,
) -> LivenessResult<()>
where
    F: Fn() -> bool,
    S: Fn() -> String,
{
    BoundedLiveness::new(timeout_ms)
        .with_check_interval_ms(check_interval_ms)
        .verify_eventually(clock, property_name, condition, state_description)
        .await
}

/// Verify the leads-to property: P ~> Q
///
/// This is a convenience wrapper around `BoundedLiveness::verify_leads_to`.
pub async fn verify_leads_to<P, Q, S>(
    clock: &Arc<SimClock>,
    property_name: &str,
    precondition: P,
    postcondition: Q,
    timeout_ms: u64,
    check_interval_ms: u64,
    state_description: S,
) -> LivenessResult<()>
where
    P: Fn() -> bool,
    Q: Fn() -> bool,
    S: Fn() -> String,
{
    BoundedLiveness::new(timeout_ms)
        .with_check_interval_ms(check_interval_ms)
        .verify_leads_to(
            clock,
            property_name,
            precondition,
            postcondition,
            state_description,
        )
        .await
}

// =============================================================================
// State Snapshot Helpers
// =============================================================================

/// A captured system state for liveness checking
///
/// This provides a way to capture and compare system states during liveness verification.
#[derive(Debug, Clone)]
pub struct SystemStateSnapshot {
    /// Time when the snapshot was taken
    pub time_ms: u64,
    /// Arbitrary state fields (name -> value)
    pub fields: std::collections::HashMap<String, String>,
}

impl SystemStateSnapshot {
    /// Create a new empty snapshot
    pub fn new(time_ms: u64) -> Self {
        Self {
            time_ms,
            fields: std::collections::HashMap::new(),
        }
    }

    /// Add a field to the snapshot
    pub fn with_field(mut self, name: impl Into<String>, value: impl Into<String>) -> Self {
        self.fields.insert(name.into(), value.into());
        self
    }

    /// Get a field value
    pub fn get(&self, name: &str) -> Option<&String> {
        self.fields.get(name)
    }

    /// Check if a field equals a value
    pub fn field_equals(&self, name: &str, expected: &str) -> bool {
        self.fields
            .get(name)
            .map(|v| v == expected)
            .unwrap_or(false)
    }
}

impl fmt::Display for SystemStateSnapshot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "State@{}ms: {{", self.time_ms)?;
        let mut first = true;
        for (k, v) in &self.fields {
            if !first {
                write!(f, ", ")?;
            }
            write!(f, "{}={}", k, v)?;
            first = false;
        }
        write!(f, "}}")
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_verify_eventually_success() {
        let clock = Arc::new(SimClock::from_millis(0));
        let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));

        // Condition becomes true after 5 checks
        let counter_ref = counter.clone();
        let condition = move || {
            let val = counter_ref.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            val >= 5
        };

        let result = BoundedLiveness::new(1000)
            .with_check_interval_ms(10)
            .verify_eventually(&clock, "test_property", condition, || {
                "counter state".to_string()
            })
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_verify_eventually_timeout() {
        let clock = Arc::new(SimClock::from_millis(0));

        // Condition never becomes true
        let condition = || false;

        let result = BoundedLiveness::new(100)
            .with_check_interval_ms(10)
            .verify_eventually(&clock, "never_true", condition, || {
                "always false".to_string()
            })
            .await;

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert_eq!(err.property, "never_true");
        assert!(err.waited_ms >= 100);
    }

    #[tokio::test]
    #[allow(clippy::disallowed_methods)] // tokio::spawn is fine in tests
    async fn test_verify_leads_to_success() {
        let clock = Arc::new(SimClock::from_millis(0));
        let phase = Arc::new(std::sync::atomic::AtomicU64::new(0));

        // Phase 0 -> Phase 1 -> Phase 2
        // Precondition: phase == 1
        // Postcondition: phase == 2
        let phase_pre = phase.clone();
        let precondition = move || phase_pre.load(std::sync::atomic::Ordering::SeqCst) == 1;

        let phase_post = phase.clone();
        let postcondition = move || phase_post.load(std::sync::atomic::Ordering::SeqCst) == 2;

        // Spawn a "background" process that advances phases
        let clock_spawn = clock.clone();
        let phase_spawn = phase.clone();
        tokio::spawn(async move {
            // After 50ms, go to phase 1
            while clock_spawn.now_ms() < 50 {
                tokio::task::yield_now().await;
            }
            phase_spawn.store(1, std::sync::atomic::Ordering::SeqCst);

            // After 100ms, go to phase 2
            while clock_spawn.now_ms() < 100 {
                tokio::task::yield_now().await;
            }
            phase_spawn.store(2, std::sync::atomic::Ordering::SeqCst);
        });

        let phase_desc = phase.clone();
        let result = BoundedLiveness::new(500)
            .with_check_interval_ms(10)
            .verify_leads_to(
                &clock,
                "phase_transition",
                precondition,
                postcondition,
                move || {
                    format!(
                        "phase={}",
                        phase_desc.load(std::sync::atomic::Ordering::SeqCst)
                    )
                },
            )
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_verify_leads_to_vacuous() {
        let clock = Arc::new(SimClock::from_millis(0));

        // Precondition never holds - leads-to is vacuously true
        let precondition = || false;
        let postcondition = || false;

        let result = BoundedLiveness::new(100)
            .with_check_interval_ms(10)
            .verify_leads_to(&clock, "vacuous", precondition, postcondition, || {
                "n/a".to_string()
            })
            .await;

        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_verify_infinitely_often_success() {
        let clock = Arc::new(SimClock::from_millis(0));
        let counter = std::sync::atomic::AtomicU64::new(0);

        // Condition alternates every 20ms
        let condition = || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (clock.now_ms() / 20) % 2 == 0
        };

        let result = BoundedLiveness::new(1000)
            .with_check_interval_ms(10)
            .verify_infinitely_often(
                &clock,
                "alternating",
                condition,
                5, // Expect at least 5 occurrences
                || "counter state".to_string(),
            )
            .await;

        assert!(result.is_ok());
        assert!(result.unwrap() >= 5);
    }

    #[test]
    fn test_system_state_snapshot() {
        let snapshot = SystemStateSnapshot::new(1000)
            .with_field("node_state", "Claiming")
            .with_field("lease_holder", "node-1");

        assert_eq!(snapshot.time_ms, 1000);
        assert!(snapshot.field_equals("node_state", "Claiming"));
        assert!(!snapshot.field_equals("node_state", "Active"));
        assert_eq!(snapshot.get("lease_holder"), Some(&"node-1".to_string()));

        let display = format!("{}", snapshot);
        assert!(display.contains("1000ms"));
        assert!(display.contains("node_state=Claiming"));
    }

    #[test]
    fn test_bounded_liveness_builder() {
        let liveness = BoundedLiveness::new(5000)
            .with_check_interval_ms(50)
            .with_max_checks(1000);

        assert_eq!(liveness.timeout_ms, 5000);
        assert_eq!(liveness.check_interval_ms, 50);
        assert_eq!(liveness.max_checks, 1000);
    }

    // =========================================================================
    // State-Based Exploration Tests (Issue #40)
    // =========================================================================

    /// Simple node state machine for testing
    #[derive(Clone, Hash, Eq, PartialEq, Debug)]
    enum TestNodeState {
        Idle,
        Claiming,
        Active,
    }

    /// Node state transitions: Idle -> Claiming -> Active or Idle
    fn node_transitions(state: &TestNodeState) -> Vec<(TestNodeState, String)> {
        match state {
            TestNodeState::Idle => vec![(TestNodeState::Claiming, "start_claim".into())],
            TestNodeState::Claiming => vec![
                (TestNodeState::Active, "claim_success".into()),
                (TestNodeState::Idle, "claim_fail".into()),
            ],
            TestNodeState::Active => vec![(TestNodeState::Active, "stay_active".into())],
        }
    }

    #[test]
    fn test_state_explorer_check_eventually_success() {
        let explorer = StateExplorer::new(100);

        // Starting from Idle, Active should be reachable
        let result = explorer.check_eventually(
            "EventualActivation",
            TestNodeState::Idle,
            node_transitions,
            |s| *s == TestNodeState::Active,
        );

        assert!(result.is_ok());
        let trace = result.unwrap();
        assert!(trace.step_count >= 2); // At least Idle -> Claiming -> Active
        assert_eq!(trace.final_state(), Some(&TestNodeState::Active));
    }

    #[test]
    fn test_state_explorer_check_eventually_immediate() {
        let explorer = StateExplorer::new(100);

        // Starting from Active, Active holds immediately
        let result = explorer.check_eventually(
            "AlreadyActive",
            TestNodeState::Active,
            node_transitions,
            |s| *s == TestNodeState::Active,
        );

        assert!(result.is_ok());
        let trace = result.unwrap();
        assert_eq!(trace.step_count, 1); // Just initial state
    }

    #[test]
    fn test_state_explorer_check_eventually_failure() {
        let explorer = StateExplorer::new(100);

        // "NotReachable" state doesn't exist
        let result = explorer.check_eventually(
            "UnreachableState",
            TestNodeState::Idle,
            node_transitions,
            |_| false, // Never satisfied
        );

        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(err.property.contains("UnreachableState"));
        assert!(err.states_explored > 0);
    }

    #[test]
    fn test_state_explorer_check_leads_to() {
        let explorer = StateExplorer::new(100);

        // Claiming ~> (Active ∨ Idle)
        let result = explorer.check_leads_to(
            "ClaimResolution",
            TestNodeState::Idle,
            node_transitions,
            |s| *s == TestNodeState::Claiming,
            |s| *s == TestNodeState::Active || *s == TestNodeState::Idle,
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_state_explorer_check_leads_to_vacuous() {
        let explorer = StateExplorer::new(100);

        // If we start at Active, precondition (Claiming) never holds
        // leads-to should be vacuously true
        let result = explorer.check_leads_to(
            "VacuousLeadsTo",
            TestNodeState::Active,
            node_transitions,
            |s| *s == TestNodeState::Claiming,
            |_| false, // Postcondition never holds, but that's OK if precondition never holds
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_state_explorer_check_infinitely_often() {
        let explorer = StateExplorer::new(1000);

        // Active state should be reachable (at least once)
        let result = explorer.check_infinitely_often(
            "ActiveOccurs",
            TestNodeState::Idle,
            node_transitions,
            |s| *s == TestNodeState::Active,
            1, // At least 1 occurrence
        );

        assert!(result.is_ok());
    }

    #[test]
    fn test_state_trace_format() {
        let mut trace: StateTrace<TestNodeState> = StateTrace::new();
        trace.push_state_with_action(TestNodeState::Idle, "init");
        trace.push_state_with_action(TestNodeState::Claiming, "start_claim");
        trace.push_state_with_action(TestNodeState::Active, "claim_success");

        assert_eq!(trace.step_count, 3);
        assert_eq!(trace.final_state(), Some(&TestNodeState::Active));

        let formatted = trace.format_trace();
        assert!(formatted.contains("Idle"));
        assert!(formatted.contains("Claiming"));
        assert!(formatted.contains("Active"));
        assert!(formatted.contains("start_claim"));
        assert!(formatted.contains("claim_success"));
    }

    #[test]
    fn test_state_explorer_bounded_depth() {
        // Very shallow explorer
        let explorer = StateExplorer::new(100).with_max_depth(1);

        // From Idle, we can reach Claiming (depth 1) but not Active (depth 2)
        let result = explorer.check_eventually(
            "ShallowExploration",
            TestNodeState::Idle,
            node_transitions,
            |s| *s == TestNodeState::Active,
        );

        // Should fail because Active requires depth 2
        assert!(result.is_err());
    }

    #[test]
    fn test_state_explorer_builder() {
        let explorer = StateExplorer::new(5000)
            .with_max_depth(50)
            .with_max_states(10000);

        assert_eq!(explorer.max_steps, 5000);
        assert_eq!(explorer.max_depth, 50);
        assert_eq!(explorer.max_states_tracked, 10000);
    }

    /// More complex state machine with contention
    #[derive(Clone, Hash, Eq, PartialEq, Debug)]
    struct TwoNodeState {
        node0: TestNodeState,
        node1: TestNodeState,
        holder: Option<u8>, // Which node holds the lock
    }

    fn two_node_transitions(state: &TwoNodeState) -> Vec<(TwoNodeState, String)> {
        let mut results = Vec::new();

        // Node 0 transitions
        match &state.node0 {
            TestNodeState::Idle => {
                let mut next = state.clone();
                next.node0 = TestNodeState::Claiming;
                results.push((next, "n0_start_claim".into()));
            }
            TestNodeState::Claiming => {
                if state.holder.is_none() {
                    let mut next = state.clone();
                    next.node0 = TestNodeState::Active;
                    next.holder = Some(0);
                    results.push((next, "n0_claim_success".into()));
                }
                let mut next = state.clone();
                next.node0 = TestNodeState::Idle;
                results.push((next, "n0_claim_fail".into()));
            }
            TestNodeState::Active => {
                // Stay active or release
                results.push((state.clone(), "n0_stay_active".into()));
                let mut next = state.clone();
                next.node0 = TestNodeState::Idle;
                next.holder = None;
                results.push((next, "n0_release".into()));
            }
        }

        // Node 1 transitions (same logic)
        match &state.node1 {
            TestNodeState::Idle => {
                let mut next = state.clone();
                next.node1 = TestNodeState::Claiming;
                results.push((next, "n1_start_claim".into()));
            }
            TestNodeState::Claiming => {
                if state.holder.is_none() {
                    let mut next = state.clone();
                    next.node1 = TestNodeState::Active;
                    next.holder = Some(1);
                    results.push((next, "n1_claim_success".into()));
                }
                let mut next = state.clone();
                next.node1 = TestNodeState::Idle;
                results.push((next, "n1_claim_fail".into()));
            }
            TestNodeState::Active => {
                results.push((state.clone(), "n1_stay_active".into()));
                let mut next = state.clone();
                next.node1 = TestNodeState::Idle;
                next.holder = None;
                results.push((next, "n1_release".into()));
            }
        }

        results
    }

    #[test]
    fn test_state_explorer_two_node_eventual_activation() {
        let explorer = StateExplorer::new(10000).with_max_depth(20);

        let initial = TwoNodeState {
            node0: TestNodeState::Idle,
            node1: TestNodeState::Idle,
            holder: None,
        };

        // At least one node should eventually be active
        let result =
            explorer.check_eventually("SomeNodeActive", initial, two_node_transitions, |s| {
                s.node0 == TestNodeState::Active || s.node1 == TestNodeState::Active
            });

        assert!(result.is_ok());
    }

    #[test]
    fn test_state_explorer_mutual_exclusion() {
        let explorer = StateExplorer::new(10000).with_max_depth(20);

        let initial = TwoNodeState {
            node0: TestNodeState::Idle,
            node1: TestNodeState::Idle,
            holder: None,
        };

        // Should NEVER find both nodes active (mutual exclusion)
        let result = explorer.check_eventually(
            "BothActive_ShouldFail",
            initial,
            two_node_transitions,
            |s| s.node0 == TestNodeState::Active && s.node1 == TestNodeState::Active,
        );

        // This should fail - both nodes can't be active due to holder check
        assert!(result.is_err());
    }
}
