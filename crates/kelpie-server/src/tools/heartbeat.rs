//! Heartbeat/Pause Mechanism Tools
//!
//! TigerStyle: Control flow tools for agent loop management.
//!
//! Implements the Letta-compatible pause_heartbeats tool that allows agents
//! to signal they want to pause their autonomous iteration loop.

use crate::tools::{
    BuiltinToolHandler, UnifiedToolRegistry, HEARTBEAT_PAUSE_MINUTES_DEFAULT,
    HEARTBEAT_PAUSE_MINUTES_MAX, HEARTBEAT_PAUSE_MINUTES_MIN, MS_PER_MINUTE,
};
use kelpie_core::io::{TimeProvider, WallClockTime};
use serde_json::json;
use std::sync::Arc;

// =============================================================================
// Clock Abstraction for DST Support
// =============================================================================

/// Clock source for time operations
///
/// Supports both real system time (production) and simulated time (DST testing).
#[derive(Clone, Default)]
pub enum ClockSource {
    /// Real system clock (production)
    #[default]
    Real,
    /// Simulated clock function (DST testing)
    Sim(Arc<dyn Fn() -> u64 + Send + Sync>),
}

impl ClockSource {
    /// Get current time in milliseconds since epoch
    pub fn now_ms(&self) -> u64 {
        match self {
            ClockSource::Real => WallClockTime::new().now_ms(),
            ClockSource::Sim(clock_fn) => clock_fn(),
        }
    }
}

// =============================================================================
// Tool Registration
// =============================================================================

/// Register the pause_heartbeats tool with the unified registry
///
/// This tool allows agents to pause their autonomous loop for a specified duration.
/// When called, it returns a ToolSignal::PauseHeartbeats that the agent loop
/// should check and act upon.
pub async fn register_heartbeat_tools(registry: &UnifiedToolRegistry) {
    register_pause_heartbeats(registry, ClockSource::Real).await;

    tracing::info!("Registered heartbeat tools: pause_heartbeats");
}

/// Register pause_heartbeats with a custom clock source (for DST testing)
pub async fn register_pause_heartbeats_with_clock(
    registry: &UnifiedToolRegistry,
    clock: ClockSource,
) {
    register_pause_heartbeats(registry, clock).await;
}

async fn register_pause_heartbeats(registry: &UnifiedToolRegistry, clock: ClockSource) {
    let handler: BuiltinToolHandler = Arc::new(move |input| {
        let clock = clock.clone();
        let input = input.clone();
        Box::pin(async move {
            // Extract minutes parameter
            let minutes = input
                .get("minutes")
                .and_then(|v| v.as_u64())
                .unwrap_or(HEARTBEAT_PAUSE_MINUTES_DEFAULT);

            // TigerStyle: Clamp to valid range with explicit bounds
            let minutes = minutes.clamp(HEARTBEAT_PAUSE_MINUTES_MIN, HEARTBEAT_PAUSE_MINUTES_MAX);

            // TigerStyle: Assertions
            debug_assert!(minutes >= HEARTBEAT_PAUSE_MINUTES_MIN);
            debug_assert!(minutes <= HEARTBEAT_PAUSE_MINUTES_MAX);

            // Calculate pause end time
            let now_ms = clock.now_ms();
            let pause_until_ms = now_ms.saturating_add(minutes.saturating_mul(MS_PER_MINUTE));

            // TigerStyle: Postcondition
            debug_assert!(pause_until_ms >= now_ms);

            // Return special format that the agent loop will parse
            // Format: "PAUSE_HEARTBEATS:minutes:pause_until_ms"
            // This allows the handler to communicate the signal through the string return
            format!(
                "PAUSE_HEARTBEATS:{}:{}\nHeartbeats paused for {} minute{}. I'll stop thinking for now.",
                minutes,
                pause_until_ms,
                minutes,
                if minutes == 1 { "" } else { "s" }
            )
        })
    });

    registry
        .register_builtin(
            "pause_heartbeats",
            "Temporarily disable heartbeat messages and end the current step. \
             Use this when you have completed your current task and don't need to \
             continue thinking. The system will automatically resume after the \
             specified duration. Call this to signal 'I'm done for now'.",
            json!({
                "type": "object",
                "properties": {
                    "minutes": {
                        "type": "integer",
                        "description": "Number of minutes to pause heartbeats (1-60). Default is 2 minutes.",
                        "minimum": 1,
                        "maximum": 60,
                        "default": 2
                    }
                },
                "required": []
            }),
            handler,
        )
        .await;
}

/// Parse a pause heartbeats response from tool output
///
/// Returns (minutes, pause_until_ms) if the output contains a pause signal.
pub fn parse_pause_signal(output: &str) -> Option<(u64, u64)> {
    if output.starts_with("PAUSE_HEARTBEATS:") {
        let parts: Vec<&str> = output.lines().next()?.split(':').collect();
        if parts.len() >= 3 {
            let minutes = parts[1].parse().ok()?;
            let pause_until_ms = parts[2].parse().ok()?;
            return Some((minutes, pause_until_ms));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_pause_signal() {
        let output = "PAUSE_HEARTBEATS:5:1704067200000\nHeartbeats paused for 5 minutes.";
        let result = parse_pause_signal(output);
        assert_eq!(result, Some((5, 1704067200000)));
    }

    #[test]
    fn test_parse_pause_signal_invalid() {
        let output = "Some other output";
        assert_eq!(parse_pause_signal(output), None);

        let output = "PAUSE_HEARTBEATS:invalid";
        assert_eq!(parse_pause_signal(output), None);
    }

    #[test]
    fn test_clock_source_real() {
        let clock = ClockSource::Real;
        let now = clock.now_ms();
        // Should be after year 2024
        assert!(now > 1704067200000);
    }

    #[test]
    fn test_clock_source_sim() {
        let clock = ClockSource::Sim(Arc::new(|| 12345));
        assert_eq!(clock.now_ms(), 12345);
    }

    #[tokio::test]
    async fn test_register_pause_heartbeats() {
        let registry = UnifiedToolRegistry::new();
        register_heartbeat_tools(&registry).await;

        assert!(registry.has_tool("pause_heartbeats").await);

        let tool = registry.get_tool("pause_heartbeats").await.unwrap();
        assert_eq!(tool.source, crate::tools::ToolSource::Builtin);
    }

    #[tokio::test]
    async fn test_pause_heartbeats_execution() {
        let registry = UnifiedToolRegistry::new();

        // Use simulated clock starting at 0
        let clock = ClockSource::Sim(Arc::new(|| 0));
        register_pause_heartbeats_with_clock(&registry, clock).await;

        // Execute with default minutes
        let result = registry.execute("pause_heartbeats", &json!({})).await;
        assert!(result.success);
        assert!(result.output.contains("PAUSE_HEARTBEATS:2:"));

        // Parse the signal
        let (minutes, pause_until_ms) = parse_pause_signal(&result.output).unwrap();
        assert_eq!(minutes, 2);
        assert_eq!(pause_until_ms, 2 * MS_PER_MINUTE);
    }

    #[tokio::test]
    async fn test_pause_heartbeats_custom_duration() {
        let registry = UnifiedToolRegistry::new();

        let clock = ClockSource::Sim(Arc::new(|| 0));
        register_pause_heartbeats_with_clock(&registry, clock).await;

        let result = registry
            .execute("pause_heartbeats", &json!({ "minutes": 10 }))
            .await;
        assert!(result.success);

        let (minutes, pause_until_ms) = parse_pause_signal(&result.output).unwrap();
        assert_eq!(minutes, 10);
        assert_eq!(pause_until_ms, 10 * MS_PER_MINUTE);
    }

    #[tokio::test]
    async fn test_pause_heartbeats_clamping() {
        let registry = UnifiedToolRegistry::new();

        let clock = ClockSource::Sim(Arc::new(|| 0));
        register_pause_heartbeats_with_clock(&registry, clock).await;

        // Test below minimum
        let result = registry
            .execute("pause_heartbeats", &json!({ "minutes": 0 }))
            .await;
        let (minutes, _) = parse_pause_signal(&result.output).unwrap();
        assert_eq!(minutes, HEARTBEAT_PAUSE_MINUTES_MIN);

        // Test above maximum
        let result = registry
            .execute("pause_heartbeats", &json!({ "minutes": 100 }))
            .await;
        let (minutes, _) = parse_pause_signal(&result.output).unwrap();
        assert_eq!(minutes, HEARTBEAT_PAUSE_MINUTES_MAX);
    }
}
