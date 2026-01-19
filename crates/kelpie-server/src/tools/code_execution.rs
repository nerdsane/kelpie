//! Code Execution Tool for Letta Compatibility
//!
//! TigerStyle: ProcessSandbox integration with multi-language support.
//!
//! Implements Letta's run_code prebuilt tool using ProcessSandbox.
//! Supports Python, JavaScript, TypeScript, R, and Java execution.

use crate::tools::{BuiltinToolHandler, UnifiedToolRegistry};
use kelpie_sandbox::{ExecOptions, ProcessSandbox, Sandbox, SandboxConfig};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::{Duration, Instant};

// =============================================================================
// TigerStyle Constants
// =============================================================================

/// Maximum code size in bytes (1 MiB)
const CODE_SIZE_BYTES_MAX: usize = 1024 * 1024;

/// Maximum output size in bytes (1 MiB)
const OUTPUT_SIZE_BYTES_MAX: u64 = 1024 * 1024;

/// Default execution timeout in seconds
const EXECUTION_TIMEOUT_SECONDS_DEFAULT: u64 = 30;

/// Maximum execution timeout in seconds
const EXECUTION_TIMEOUT_SECONDS_MAX: u64 = 300;

/// Minimum execution timeout in seconds
const EXECUTION_TIMEOUT_SECONDS_MIN: u64 = 1;

// =============================================================================
// Language Configuration
// =============================================================================

/// Supported programming languages
const SUPPORTED_LANGUAGES: &[&str] = &["python", "javascript", "typescript", "r", "java"];

/// Get command and args for executing code in a given language
fn get_execution_command(language: &str, code: &str) -> Result<(String, Vec<String>), String> {
    match language.to_lowercase().as_str() {
        "python" => Ok((
            "python3".to_string(),
            vec!["-c".to_string(), code.to_string()],
        )),
        "javascript" | "js" => Ok(("node".to_string(), vec!["-e".to_string(), code.to_string()])),
        "typescript" | "ts" => {
            // TypeScript requires compilation - use ts-node if available
            Ok((
                "ts-node".to_string(),
                vec!["-e".to_string(), code.to_string()],
            ))
        }
        "r" => Ok((
            "Rscript".to_string(),
            vec!["-e".to_string(), code.to_string()],
        )),
        "java" => {
            // Java requires a more complex setup - return error for now
            Err("Java execution requires compilation and is not yet supported in sandbox mode. Use a precompiled class or consider using JavaScript/Python instead.".to_string())
        }
        _ => Err(format!(
            "Unsupported language: '{}'. Supported languages: {}",
            language,
            SUPPORTED_LANGUAGES.join(", ")
        )),
    }
}

// =============================================================================
// Registration
// =============================================================================

/// Register run_code tool with the unified registry
pub async fn register_run_code_tool(registry: &UnifiedToolRegistry) {
    let handler: BuiltinToolHandler = Arc::new(|input: &Value| {
        let input = input.clone();
        Box::pin(async move { execute_code(&input).await })
    });

    registry
        .register_builtin(
            "run_code",
            "Execute code in a sandboxed environment. Supports Python, JavaScript, TypeScript, and R. Returns stdout, stderr, exit code, and execution time.",
            json!({
                "type": "object",
                "properties": {
                    "language": {
                        "type": "string",
                        "description": "Programming language (python, javascript, typescript, r)",
                        "enum": ["python", "javascript", "typescript", "r", "js", "ts"]
                    },
                    "code": {
                        "type": "string",
                        "description": "Code to execute"
                    },
                    "timeout_seconds": {
                        "type": "integer",
                        "description": "Execution timeout in seconds (default: 30, min: 1, max: 300)",
                        "minimum": 1,
                        "maximum": 300,
                        "default": 30
                    }
                },
                "required": ["language", "code"]
            }),
            handler,
        )
        .await;

    tracing::info!("Registered prebuilt tool: run_code");
}

// =============================================================================
// Execution
// =============================================================================

/// Execute code in a sandboxed environment
async fn execute_code(input: &Value) -> String {
    // Extract language parameter
    let language = match input.get("language").and_then(|v| v.as_str()) {
        Some(lang) if !lang.trim().is_empty() => lang.trim().to_lowercase(),
        Some(_) => return "Error: language cannot be empty".to_string(),
        None => return "Error: missing required parameter 'language'".to_string(),
    };

    // Extract code parameter
    let code = match input.get("code").and_then(|v| v.as_str()) {
        Some(c) if !c.trim().is_empty() => c.to_string(),
        Some(_) => return "Error: code cannot be empty".to_string(),
        None => return "Error: missing required parameter 'code'".to_string(),
    };

    // Validate code size
    if code.len() > CODE_SIZE_BYTES_MAX {
        return format!(
            "Error: code too large ({} bytes, max {} bytes)",
            code.len(),
            CODE_SIZE_BYTES_MAX
        );
    }

    // Extract optional timeout parameter
    let timeout_seconds = input
        .get("timeout_seconds")
        .and_then(|v| v.as_u64())
        .unwrap_or(EXECUTION_TIMEOUT_SECONDS_DEFAULT);

    // Validate timeout
    if timeout_seconds > EXECUTION_TIMEOUT_SECONDS_MAX {
        return format!(
            "Error: timeout_seconds cannot exceed {} (got: {})",
            EXECUTION_TIMEOUT_SECONDS_MAX, timeout_seconds
        );
    }

    if timeout_seconds < EXECUTION_TIMEOUT_SECONDS_MIN {
        return format!(
            "Error: timeout_seconds must be at least {} (got: {})",
            EXECUTION_TIMEOUT_SECONDS_MIN, timeout_seconds
        );
    }

    // Get execution command for the language
    let (command, args) = match get_execution_command(&language, &code) {
        Ok((cmd, args)) => (cmd, args),
        Err(e) => return format!("Error: {}", e),
    };

    // Execute in sandbox
    match execute_in_sandbox(&command, &args, timeout_seconds).await {
        Ok(result) => format_execution_result(&result),
        Err(e) => format!("Error: {}", e),
    }
}

// =============================================================================
// Sandbox Execution
// =============================================================================

/// Execution result
struct ExecutionResult {
    stdout: String,
    stderr: String,
    exit_code: i32,
    execution_time_ms: u64,
    success: bool,
}

/// Execute command in ProcessSandbox
async fn execute_in_sandbox(
    command: &str,
    args: &[String],
    timeout_seconds: u64,
) -> Result<ExecutionResult, String> {
    // Create and start sandbox
    let config = SandboxConfig::default();
    let mut sandbox = ProcessSandbox::new(config);

    if let Err(e) = sandbox.start().await {
        return Err(format!("Failed to start sandbox: {}", e));
    }

    // Configure execution options
    let exec_opts = ExecOptions::new()
        .with_timeout(Duration::from_secs(timeout_seconds))
        .with_max_output(OUTPUT_SIZE_BYTES_MAX);

    // Convert Vec<String> to Vec<&str> for exec()
    let args_refs: Vec<&str> = args.iter().map(|s| s.as_str()).collect();

    // Execute command and measure time
    let start_time = Instant::now();
    let output = sandbox
        .exec(command, &args_refs, exec_opts)
        .await
        .map_err(|e| format!("Sandbox execution failed: {}", e))?;
    let execution_time_ms = start_time.elapsed().as_millis() as u64;

    // Build result
    let result = ExecutionResult {
        stdout: output.stdout_string(),
        stderr: output.stderr_string(),
        exit_code: output.status.code,
        execution_time_ms,
        success: output.is_success(),
    };

    Ok(result)
}

/// Format execution result for display
fn format_execution_result(result: &ExecutionResult) -> String {
    let result_json = json!({
        "success": result.success,
        "exit_code": result.exit_code,
        "execution_time_ms": result.execution_time_ms,
        "stdout": result.stdout,
        "stderr": result.stderr,
    });

    serde_json::to_string_pretty(&result_json)
        .unwrap_or_else(|_| "Error formatting result".to_string())
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::assertions_on_constants)] // TigerStyle: validate constants
    fn test_constants_valid() {
        assert!(EXECUTION_TIMEOUT_SECONDS_DEFAULT >= EXECUTION_TIMEOUT_SECONDS_MIN);
        assert!(EXECUTION_TIMEOUT_SECONDS_DEFAULT <= EXECUTION_TIMEOUT_SECONDS_MAX);
        assert!(CODE_SIZE_BYTES_MAX > 0);
        assert!(OUTPUT_SIZE_BYTES_MAX > 0);
    }

    #[tokio::test]
    async fn test_run_code_missing_language() {
        let input = json!({
            "code": "print('hello')"
        });
        let result = execute_code(&input).await;
        assert!(result.contains("Error: missing required parameter 'language'"));
    }

    #[tokio::test]
    async fn test_run_code_empty_language() {
        let input = json!({
            "language": "   ",
            "code": "print('hello')"
        });
        let result = execute_code(&input).await;
        assert!(result.contains("Error: language cannot be empty"));
    }

    #[tokio::test]
    async fn test_run_code_missing_code() {
        let input = json!({
            "language": "python"
        });
        let result = execute_code(&input).await;
        assert!(result.contains("Error: missing required parameter 'code'"));
    }

    #[tokio::test]
    async fn test_run_code_empty_code() {
        let input = json!({
            "language": "python",
            "code": "   "
        });
        let result = execute_code(&input).await;
        assert!(result.contains("Error: code cannot be empty"));
    }

    #[tokio::test]
    async fn test_run_code_code_too_large() {
        let large_code = "x".repeat(CODE_SIZE_BYTES_MAX + 1);
        let input = json!({
            "language": "python",
            "code": large_code
        });
        let result = execute_code(&input).await;
        assert!(result.contains("Error: code too large"));
    }

    #[tokio::test]
    async fn test_run_code_timeout_too_large() {
        let input = json!({
            "language": "python",
            "code": "print('hello')",
            "timeout_seconds": 1000
        });
        let result = execute_code(&input).await;
        assert!(result.contains("Error: timeout_seconds cannot exceed"));
    }

    #[tokio::test]
    async fn test_run_code_timeout_too_small() {
        let input = json!({
            "language": "python",
            "code": "print('hello')",
            "timeout_seconds": 0
        });
        let result = execute_code(&input).await;
        assert!(result.contains("Error: timeout_seconds must be at least"));
    }

    #[tokio::test]
    async fn test_run_code_unsupported_language() {
        let input = json!({
            "language": "cobol",
            "code": "DISPLAY 'hello'."
        });
        let result = execute_code(&input).await;
        assert!(result.contains("Error: Unsupported language: 'cobol'"));
    }

    #[test]
    fn test_get_execution_command_python() {
        let (cmd, args) = get_execution_command("python", "print('test')").expect("should work");
        assert_eq!(cmd, "python3");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "-c");
        assert_eq!(args[1], "print('test')");
    }

    #[test]
    fn test_get_execution_command_javascript() {
        let (cmd, args) =
            get_execution_command("javascript", "console.log('test')").expect("should work");
        assert_eq!(cmd, "node");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "-e");
    }

    #[test]
    fn test_get_execution_command_js_alias() {
        let (cmd, _args) = get_execution_command("js", "console.log('test')").expect("should work");
        assert_eq!(cmd, "node");
    }

    #[test]
    fn test_get_execution_command_typescript() {
        let (cmd, _args) =
            get_execution_command("typescript", "console.log('test')").expect("should work");
        assert_eq!(cmd, "ts-node");
    }

    #[test]
    fn test_get_execution_command_r() {
        let (cmd, args) = get_execution_command("r", "print('test')").expect("should work");
        assert_eq!(cmd, "Rscript");
        assert_eq!(args.len(), 2);
        assert_eq!(args[0], "-e");
    }

    #[test]
    fn test_get_execution_command_java_not_supported() {
        let result = get_execution_command("java", "System.out.println(\"test\");");
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .contains("Java execution requires compilation"));
    }

    #[test]
    fn test_get_execution_command_case_insensitive() {
        let (cmd, _) = get_execution_command("PYTHON", "print('test')").expect("should work");
        assert_eq!(cmd, "python3");
    }

    // Integration tests that actually execute code (require runtime)
    #[tokio::test]
    #[ignore] // Requires python3 installed
    async fn test_run_code_python_success() {
        let input = json!({
            "language": "python",
            "code": "print('Hello from Python')"
        });
        let result = execute_code(&input).await;
        assert!(result.contains("\"success\": true"));
        assert!(result.contains("Hello from Python"));
    }

    #[tokio::test]
    #[ignore] // Requires python3 installed
    async fn test_run_code_python_stderr() {
        let input = json!({
            "language": "python",
            "code": "import sys; sys.stderr.write('error message\\n')"
        });
        let result = execute_code(&input).await;
        assert!(result.contains("error message"));
    }

    #[tokio::test]
    #[ignore] // Requires node installed
    async fn test_run_code_javascript_success() {
        let input = json!({
            "language": "javascript",
            "code": "console.log('Hello from JavaScript')"
        });
        let result = execute_code(&input).await;
        assert!(result.contains("\"success\": true"));
        assert!(result.contains("Hello from JavaScript"));
    }
}
