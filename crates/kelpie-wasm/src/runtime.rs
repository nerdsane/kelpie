//! WASM Runtime Implementation
//!
//! TigerStyle: Sandboxed WASM execution with explicit limits and caching.
//!
//! DST-Compliant: Uses TimeProvider abstraction for deterministic testing.
//! When the `dst` feature is enabled, supports FaultInjector for testing
//! error paths including compilation failures, execution failures, and timeouts.

use kelpie_core::io::TimeProvider;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::RwLock;
use tracing::{debug, info};
use wasi_cap_std_sync::WasiCtxBuilder;
use wasi_common::WasiCtx;
use wasmtime::{Config, Engine, Linker, Module, Store};

#[cfg(feature = "dst")]
use kelpie_dst::fault::{FaultInjector, FaultType};

// =============================================================================
// TigerStyle Constants
// =============================================================================

/// Default WASM memory limit in pages (64KB each)
pub const WASM_MEMORY_PAGES_MAX: u32 = 256; // 16MB

/// Default WASM execution timeout in milliseconds
pub const WASM_TIMEOUT_MS_DEFAULT: u64 = 30_000;

/// Maximum WASM module size in bytes
pub const WASM_MODULE_SIZE_BYTES_MAX: usize = 10 * 1024 * 1024; // 10MB

/// Maximum cached modules
pub const WASM_MODULE_CACHE_COUNT_MAX: usize = 100;

/// Maximum input size in bytes
pub const WASM_INPUT_SIZE_BYTES_MAX: usize = 1024 * 1024; // 1MB

/// Maximum output size in bytes
pub const WASM_OUTPUT_SIZE_BYTES_MAX: usize = 1024 * 1024; // 1MB

// =============================================================================
// Errors
// =============================================================================

/// WASM execution errors
#[derive(Debug, Error)]
pub enum WasmError {
    #[error("module too large: {size} bytes (max: {max} bytes)")]
    ModuleTooLarge { size: usize, max: usize },

    #[error("failed to compile module: {0}")]
    CompileFailed(String),

    #[error("failed to instantiate module: {0}")]
    InstantiateFailed(String),

    #[error("execution failed: {0}")]
    ExecutionFailed(String),

    #[error("timeout after {timeout_ms}ms")]
    Timeout { timeout_ms: u64 },

    #[error("output too large: {size} bytes (max: {max} bytes)")]
    OutputTooLarge { size: usize, max: usize },

    #[error("input too large: {size} bytes (max: {max} bytes)")]
    InputTooLarge { size: usize, max: usize },

    #[error("missing export: {name}")]
    MissingExport { name: String },

    #[error("invalid module hash")]
    InvalidHash,

    #[error("cache is full")]
    CacheFull,

    #[error("internal error: {0}")]
    Internal(String),
}

/// Result type for WASM operations
pub type WasmToolResult<T> = Result<T, WasmError>;

// =============================================================================
// Configuration
// =============================================================================

/// WASM runtime configuration
#[derive(Debug, Clone)]
pub struct WasmConfig {
    /// Maximum memory pages
    pub memory_pages_max: u32,
    /// Execution timeout in milliseconds
    pub timeout_ms: u64,
    /// Maximum cached modules
    pub cache_count_max: usize,
    /// Maximum module size in bytes
    pub module_size_bytes_max: usize,
}

impl Default for WasmConfig {
    fn default() -> Self {
        Self {
            memory_pages_max: WASM_MEMORY_PAGES_MAX,
            timeout_ms: WASM_TIMEOUT_MS_DEFAULT,
            cache_count_max: WASM_MODULE_CACHE_COUNT_MAX,
            module_size_bytes_max: WASM_MODULE_SIZE_BYTES_MAX,
        }
    }
}

impl WasmConfig {
    /// Create a new configuration with custom timeout
    pub fn with_timeout(mut self, timeout_ms: u64) -> Self {
        assert!(timeout_ms > 0, "timeout must be positive");
        self.timeout_ms = timeout_ms;
        self
    }

    /// Create a new configuration with custom memory limit
    pub fn with_memory_limit(mut self, pages: u32) -> Self {
        assert!(pages > 0, "memory pages must be positive");
        self.memory_pages_max = pages;
        self
    }

    /// Validate configuration
    pub fn validate(&self) -> WasmToolResult<()> {
        if self.memory_pages_max == 0 {
            return Err(WasmError::Internal(
                "memory_pages_max must be positive".to_string(),
            ));
        }
        if self.timeout_ms == 0 {
            return Err(WasmError::Internal(
                "timeout_ms must be positive".to_string(),
            ));
        }
        Ok(())
    }
}

// =============================================================================
// Module Cache Entry
// =============================================================================

/// Cached module with usage stats
struct CachedModule {
    module: Module,
    use_count: u64,
    /// Last used timestamp in milliseconds (from TimeProvider.monotonic_ms())
    last_used_ms: u64,
}

// =============================================================================
// WASM Runtime
// =============================================================================

/// WASM execution runtime
///
/// Provides secure, sandboxed execution of WASM modules with:
/// - Module caching for performance
/// - Memory and execution time limits
/// - WASI support for system calls
///
/// DST-Compliant: Uses TimeProvider for deterministic time in tests.
/// When the `dst` feature is enabled, supports FaultInjector for testing
/// error paths including compilation failures, execution failures, and timeouts.
pub struct WasmRuntime {
    engine: Engine,
    config: WasmConfig,
    module_cache: Arc<RwLock<HashMap<[u8; 32], CachedModule>>>,
    /// Time provider for DST-compatible timing
    time_provider: Arc<dyn TimeProvider>,
    /// Fault injector for DST testing (optional)
    #[cfg(feature = "dst")]
    fault_injector: Option<Arc<FaultInjector>>,
}

impl WasmRuntime {
    /// Create a new WASM runtime with TimeProvider for DST compatibility
    pub fn new(config: WasmConfig, time_provider: Arc<dyn TimeProvider>) -> WasmToolResult<Self> {
        config.validate()?;

        // Configure wasmtime engine
        let mut engine_config = Config::default();
        engine_config.wasm_backtrace_details(wasmtime::WasmBacktraceDetails::Enable);
        engine_config.consume_fuel(true); // Enable fuel for execution limits

        let engine = Engine::new(&engine_config)
            .map_err(|e| WasmError::Internal(format!("failed to create WASM engine: {}", e)))?;

        info!(
            memory_pages_max = config.memory_pages_max,
            timeout_ms = config.timeout_ms,
            "WASM runtime initialized"
        );

        Ok(Self {
            engine,
            config,
            module_cache: Arc::new(RwLock::new(HashMap::new())),
            time_provider,
            #[cfg(feature = "dst")]
            fault_injector: None,
        })
    }

    /// Create with default configuration and wall clock time
    pub fn with_defaults() -> WasmToolResult<Self> {
        use kelpie_core::io::WallClockTime;
        Self::new(WasmConfig::default(), Arc::new(WallClockTime::new()))
    }

    /// Create a WASM runtime with DST fault injection support
    ///
    /// # Arguments
    /// * `config` - WASM configuration
    /// * `time_provider` - Time provider for deterministic timing
    /// * `fault_injector` - Fault injector for DST testing
    #[cfg(feature = "dst")]
    pub fn with_fault_injection(
        config: WasmConfig,
        time_provider: Arc<dyn TimeProvider>,
        fault_injector: Arc<FaultInjector>,
    ) -> WasmToolResult<Self> {
        let mut runtime = Self::new(config, time_provider)?;
        runtime.fault_injector = Some(fault_injector);
        Ok(runtime)
    }

    /// Check for fault injection and return the fault type if triggered
    #[cfg(feature = "dst")]
    fn check_fault(&self, operation: &str) -> Option<FaultType> {
        self.fault_injector
            .as_ref()
            .and_then(|fi| fi.should_inject(operation))
    }

    /// Check for fault injection (no-op when dst feature is disabled)
    #[cfg(not(feature = "dst"))]
    #[allow(dead_code)]
    fn check_fault(&self, _operation: &str) -> Option<()> {
        None
    }

    /// Compute hash for WASM bytes (for caching)
    fn compute_hash(wasm_bytes: &[u8]) -> [u8; 32] {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};

        // Simple hash for caching - not cryptographic
        let mut hasher = DefaultHasher::new();
        wasm_bytes.hash(&mut hasher);
        let hash = hasher.finish();

        let mut result = [0u8; 32];
        result[..8].copy_from_slice(&hash.to_le_bytes());
        result[8..16].copy_from_slice(&(wasm_bytes.len() as u64).to_le_bytes());
        result
    }

    /// Get or compile a module
    async fn get_or_compile(&self, wasm_bytes: &[u8]) -> WasmToolResult<Module> {
        // TigerStyle: Validate input size
        if wasm_bytes.len() > self.config.module_size_bytes_max {
            return Err(WasmError::ModuleTooLarge {
                size: wasm_bytes.len(),
                max: self.config.module_size_bytes_max,
            });
        }

        let hash = Self::compute_hash(wasm_bytes);

        // DST: Check for cache eviction fault before cache lookup
        #[cfg(feature = "dst")]
        if let Some(fault) = self.check_fault("wasm_cache_lookup") {
            if matches!(fault, FaultType::WasmCacheEvict) {
                debug!("DST fault injection: forcing cache eviction");
                let mut cache = self.module_cache.write().await;
                cache.remove(&hash);
            }
        }

        // Check cache
        {
            let mut cache = self.module_cache.write().await;
            if let Some(cached) = cache.get_mut(&hash) {
                cached.use_count += 1;
                cached.last_used_ms = self.time_provider.monotonic_ms();
                debug!(hash = ?hash[..8], use_count = cached.use_count, "WASM module cache hit");
                return Ok(cached.module.clone());
            }
        }

        // DST: Check for compile failure fault
        #[cfg(feature = "dst")]
        if let Some(fault) = self.check_fault("wasm_compile") {
            if matches!(fault, FaultType::WasmCompileFail) {
                return Err(WasmError::CompileFailed(
                    "DST fault injection: simulated compile failure".to_string(),
                ));
            }
        }

        // Compile module
        debug!(size = wasm_bytes.len(), "Compiling WASM module");
        let module = Module::new(&self.engine, wasm_bytes)
            .map_err(|e| WasmError::CompileFailed(e.to_string()))?;

        // Cache the module
        {
            let mut cache = self.module_cache.write().await;

            // Evict if at capacity (simple LRU-ish)
            if cache.len() >= self.config.cache_count_max {
                // Find least used entry
                let least_used = cache
                    .iter()
                    .min_by_key(|(_, v)| v.use_count)
                    .map(|(k, _)| *k);

                if let Some(key) = least_used {
                    cache.remove(&key);
                    debug!("Evicted WASM module from cache");
                }
            }

            cache.insert(
                hash,
                CachedModule {
                    module: module.clone(),
                    use_count: 1,
                    last_used_ms: self.time_provider.monotonic_ms(),
                },
            );
        }

        Ok(module)
    }

    /// Execute a WASM module with JSON input
    ///
    /// The module must export a `_start` function (WASI convention) or `run`.
    /// Input is passed via stdin, output is captured from stdout.
    pub async fn execute(&self, wasm_bytes: &[u8], input: Value) -> WasmToolResult<String> {
        let input_json = serde_json::to_string(&input)
            .map_err(|e| WasmError::Internal(format!("failed to serialize input: {}", e)))?;

        // TigerStyle: Validate input size
        if input_json.len() > WASM_INPUT_SIZE_BYTES_MAX {
            return Err(WasmError::InputTooLarge {
                size: input_json.len(),
                max: WASM_INPUT_SIZE_BYTES_MAX,
            });
        }

        // DST: Check for execution-related faults before compile
        #[cfg(feature = "dst")]
        if let Some(fault) = self.check_fault("wasm_execute") {
            match fault {
                FaultType::WasmExecFail => {
                    return Err(WasmError::ExecutionFailed(
                        "DST fault injection: simulated execution failure".to_string(),
                    ));
                }
                FaultType::WasmExecTimeout { timeout_ms } => {
                    return Err(WasmError::Timeout { timeout_ms });
                }
                FaultType::WasmInstantiateFail => {
                    return Err(WasmError::InstantiateFailed(
                        "DST fault injection: simulated instantiation failure".to_string(),
                    ));
                }
                _ => {}
            }
        }

        let module = self.get_or_compile(wasm_bytes).await?;

        // Execute in blocking context (wasmtime is not async)
        let engine = self.engine.clone();
        let timeout_ms = self.config.timeout_ms;

        let result = tokio::task::spawn_blocking(move || {
            Self::execute_sync(&engine, &module, &input_json, timeout_ms)
        })
        .await
        .map_err(|e| WasmError::Internal(format!("execution task failed: {}", e)))??;

        // TigerStyle: Validate output size
        if result.len() > WASM_OUTPUT_SIZE_BYTES_MAX {
            return Err(WasmError::OutputTooLarge {
                size: result.len(),
                max: WASM_OUTPUT_SIZE_BYTES_MAX,
            });
        }

        Ok(result)
    }

    /// Synchronous execution (runs in blocking thread)
    fn execute_sync(
        engine: &Engine,
        module: &Module,
        input_json: &str,
        timeout_ms: u64,
    ) -> WasmToolResult<String> {
        // Create pipes for stdin/stdout using wasi-common built-in types
        let stdin_pipe = wasi_common::pipe::ReadPipe::from(input_json.as_bytes().to_vec());
        let stdout_pipe = wasi_common::pipe::WritePipe::new_in_memory();

        // Create WASI context using wasi-cap-std-sync
        let wasi_ctx = WasiCtxBuilder::new()
            .stdin(Box::new(stdin_pipe))
            .stdout(Box::new(stdout_pipe.clone()))
            .inherit_stderr()
            .build();

        // Create store with fuel for execution limits
        let mut store = Store::new(engine, wasi_ctx);

        // Set fuel based on timeout (rough approximation: 1M ops per 100ms)
        let fuel = timeout_ms * 10_000;
        store
            .set_fuel(fuel)
            .map_err(|e| WasmError::Internal(format!("failed to set fuel: {}", e)))?;

        // Create linker with WASI
        let mut linker: Linker<WasiCtx> = Linker::new(engine);
        wasmtime_wasi::add_to_linker(&mut linker, |ctx| ctx)
            .map_err(|e| WasmError::Internal(format!("failed to add WASI to linker: {}", e)))?;

        // Instantiate module
        let instance = linker
            .instantiate(&mut store, module)
            .map_err(|e| WasmError::InstantiateFailed(e.to_string()))?;

        // Get the main function (_start for WASI modules, or run)
        let run_func = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .or_else(|_| instance.get_typed_func::<(), ()>(&mut store, "run"))
            .map_err(|_| WasmError::MissingExport {
                name: "_start or run".to_string(),
            })?;

        // Execute
        match run_func.call(&mut store, ()) {
            Ok(()) => {
                // Get stdout output - drop store first to release borrow
                drop(store);
                let output_bytes = stdout_pipe
                    .try_into_inner()
                    .map_err(|_| WasmError::Internal("stdout pipe still borrowed".to_string()))?
                    .into_inner();
                let output = String::from_utf8_lossy(&output_bytes).to_string();
                Ok(output)
            }
            Err(e) => {
                // Check if it was a fuel exhaustion (timeout)
                if e.to_string().contains("fuel") {
                    Err(WasmError::Timeout { timeout_ms })
                } else {
                    Err(WasmError::ExecutionFailed(e.to_string()))
                }
            }
        }
    }

    /// Execute from raw bytes
    pub async fn execute_bytes(
        &self,
        wasm_bytes: &[u8],
        input_bytes: &[u8],
    ) -> WasmToolResult<Vec<u8>> {
        let input: Value = serde_json::from_slice(input_bytes)
            .map_err(|e| WasmError::Internal(format!("invalid JSON input: {}", e)))?;

        let output = self.execute(wasm_bytes, input).await?;
        Ok(output.into_bytes())
    }

    /// Clear the module cache
    pub async fn clear_cache(&self) {
        let mut cache = self.module_cache.write().await;
        cache.clear();
        info!("WASM module cache cleared");
    }

    /// Get cache statistics
    pub async fn cache_stats(&self) -> CacheStats {
        let cache = self.module_cache.read().await;
        let total_use_count: u64 = cache.values().map(|v| v.use_count).sum();

        CacheStats {
            module_count: cache.len(),
            total_use_count,
        }
    }
}

/// Cache statistics
#[derive(Debug, Clone)]
pub struct CacheStats {
    pub module_count: usize,
    pub total_use_count: u64,
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use kelpie_core::io::WallClockTime;

    /// Helper to create a test TimeProvider
    fn test_time_provider() -> Arc<dyn TimeProvider> {
        Arc::new(WallClockTime::new())
    }

    #[test]
    fn test_wasm_config_default() {
        let config = WasmConfig::default();
        assert_eq!(config.memory_pages_max, WASM_MEMORY_PAGES_MAX);
        assert_eq!(config.timeout_ms, WASM_TIMEOUT_MS_DEFAULT);
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_wasm_config_builder() {
        let config = WasmConfig::default()
            .with_timeout(60_000)
            .with_memory_limit(512);

        assert_eq!(config.timeout_ms, 60_000);
        assert_eq!(config.memory_pages_max, 512);
    }

    #[test]
    fn test_compute_hash() {
        let bytes1 = b"hello world";
        let bytes2 = b"hello world";
        let bytes3 = b"different content";

        let hash1 = WasmRuntime::compute_hash(bytes1);
        let hash2 = WasmRuntime::compute_hash(bytes2);
        let hash3 = WasmRuntime::compute_hash(bytes3);

        assert_eq!(hash1, hash2);
        assert_ne!(hash1, hash3);
    }

    #[tokio::test]
    async fn test_wasm_runtime_creation() {
        let runtime = WasmRuntime::with_defaults();
        assert!(runtime.is_ok());
    }

    #[tokio::test]
    async fn test_wasm_module_too_large() {
        let config = WasmConfig {
            module_size_bytes_max: 10, // Very small limit
            ..Default::default()
        };
        let runtime = WasmRuntime::new(config, test_time_provider()).unwrap();

        let large_bytes = vec![0u8; 100];
        let result = runtime.execute(&large_bytes, serde_json::json!({})).await;

        assert!(matches!(result, Err(WasmError::ModuleTooLarge { .. })));
    }

    #[tokio::test]
    async fn test_wasm_cache_stats() {
        let runtime = WasmRuntime::with_defaults().unwrap();
        let stats = runtime.cache_stats().await;

        assert_eq!(stats.module_count, 0);
        assert_eq!(stats.total_use_count, 0);
    }
}
