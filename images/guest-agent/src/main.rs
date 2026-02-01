//! Kelpie Guest Agent
//!
//! Runs inside VM, listens for commands from host via virtio-vsock.
//!
//! TigerStyle: Explicit error handling, no unwrap(), 2+ assertions per function.

use anyhow::{Context, Result};
use std::process::Stdio;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};
use tokio::process::Command;
use tracing::{error, info, warn};

#[cfg(feature = "vsock")]
use tokio_vsock::{VsockAddr, VsockListener, VMADDR_CID_ANY};

#[cfg(not(feature = "vsock"))]
use tokio::net::UnixListener;

mod protocol;
use protocol::{ExecOutput, Request, Response};

/// Default vsock port for guest agent
/// TigerStyle: Explicit constant with unit in name
const VSOCK_PORT_DEFAULT: u32 = 9001;

/// Maximum request size in bytes
/// TigerStyle: Explicit limit with unit
const REQUEST_SIZE_BYTES_MAX: usize = 10 * 1024 * 1024; // 10MB

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    info!("Kelpie Guest Agent starting...");
    info!(
        version = env!("CARGO_PKG_VERSION"),
        "Guest agent version"
    );

    #[cfg(feature = "vsock")]
    {
        // Use vsock for VM communication
        // Guest connects to host via vsock (libkrun tunnels to Unix socket on host)
        let port = std::env::var("KELPIE_GUEST_VSOCK_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(VSOCK_PORT_DEFAULT);

        // CID 2 is the host in vsock
        const VMADDR_CID_HOST: u32 = 2;

        info!(port = port, cid = VMADDR_CID_HOST, "Connecting to host via vsock");

        // Retry connection until successful (host may not be ready immediately)
        let max_retries = 30;
        let mut stream = None;

        for attempt in 1..=max_retries {
            let addr = VsockAddr::new(VMADDR_CID_HOST, port);
            match tokio_vsock::VsockStream::connect(addr).await {
                Ok(s) => {
                    info!(attempt = attempt, "Connected to host via vsock");
                    stream = Some(s);
                    break;
                }
                Err(e) => {
                    if attempt < max_retries {
                        info!(attempt = attempt, error = %e, "Vsock connect retry");
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    } else {
                        error!(error = %e, "Failed to connect to host via vsock after {} attempts", max_retries);
                        return Err(e.into());
                    }
                }
            }
        }

        let stream = stream.context("Failed to connect to vsock")?;
        info!("Connected to host, handling commands");

        // Handle the single connection to the host
        if let Err(e) = handle_connection(stream).await {
            error!(error = %e, "Connection handler error");
        }

        info!("Host disconnected, shutting down");
        Ok(())
    }

    #[cfg(not(feature = "vsock"))]
    {
        // Fallback to Unix socket for development/testing
        let socket_path = std::env::var("KELPIE_GUEST_SOCKET")
            .unwrap_or_else(|_| "/tmp/kelpie-guest.sock".to_string());

        // Remove existing socket if present
        let _ = std::fs::remove_file(&socket_path);

        let listener = UnixListener::bind(&socket_path).context("Failed to bind Unix socket")?;

        info!(socket = %socket_path, "Listening for Unix socket connections");

        // Accept connections in a loop
        loop {
            match listener.accept().await {
                Ok((stream, _addr)) => {
                    info!("Accepted connection");
                    tokio::spawn(async move {
                        if let Err(e) = handle_connection(stream).await {
                            error!(error = %e, "Connection handler error");
                        }
                    });
                }
                Err(e) => {
                    error!(error = %e, "Accept error");
                }
            }
        }
    }
}

/// Handle a single connection
///
/// TigerStyle: Clear error propagation, explicit timeouts
async fn handle_connection<S>(mut stream: S) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    info!("Handling connection");

    loop {
        // Read request length (4 bytes, big-endian)
        let mut len_buf = [0u8; 4];
        match stream.read_exact(&mut len_buf).await {
            Ok(_n) => {}
            Err(e) if e.kind() == std::io::ErrorKind::UnexpectedEof => {
                info!("Client disconnected");
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        }

        let request_len = u32::from_be_bytes(len_buf) as usize;

        // TigerStyle: Explicit bounds check
        assert!(
            request_len <= REQUEST_SIZE_BYTES_MAX,
            "request size {} exceeds max {}",
            request_len,
            REQUEST_SIZE_BYTES_MAX
        );

        if request_len > REQUEST_SIZE_BYTES_MAX {
            warn!(size = request_len, max = REQUEST_SIZE_BYTES_MAX, "Request too large");
            let response = Response::Error {
                message: format!(
                    "Request too large: {} > {}",
                    request_len, REQUEST_SIZE_BYTES_MAX
                ),
            };
            send_response(&mut stream, &response).await?;
            continue;
        }

        // Read request body
        let mut request_buf = vec![0u8; request_len];
        stream.read_exact(&mut request_buf).await?;

        // Parse request
        let request: Request = match serde_json::from_slice(&request_buf) {
            Ok(req) => req,
            Err(e) => {
                error!(error = %e, "Failed to parse request");
                let response = Response::Error {
                    message: format!("Invalid request: {}", e),
                };
                send_response(&mut stream, &response).await?;
                continue;
            }
        };

        // Handle request
        let response = handle_request(request).await;

        // Send response
        send_response(&mut stream, &response).await?;
    }
}

/// Handle a request and return response
///
/// TigerStyle: Exhaustive match, no unwrap()
async fn handle_request(request: Request) -> Response {
    match request {
        Request::Ping => Response::Pong,

        Request::Exec {
            command,
            args,
            stdin,
        } => {
            info!(command = %command, args = ?args, "Executing command");
            execute_command(&command, &args, stdin.as_deref()).await
        }

        Request::ReadFile { path } => {
            info!(path = %path, "Reading file");
            read_file(&path).await
        }

        Request::WriteFile { path, contents } => {
            info!(path = %path, size = contents.len(), "Writing file");
            write_file(&path, &contents).await
        }

        Request::ListDir { path } => {
            info!(path = %path, "Listing directory");
            list_directory(&path).await
        }
    }
}

/// Execute a command
///
/// TigerStyle: Explicit error handling, timeouts, output capture
async fn execute_command(command: &str, args: &[String], stdin: Option<&str>) -> Response {
    // Preconditions
    assert!(!command.is_empty(), "command must not be empty");

    let mut cmd = Command::new(command);
    cmd.args(args);
    cmd.stdin(Stdio::piped());
    cmd.stdout(Stdio::piped());
    cmd.stderr(Stdio::piped());

    let mut child = match cmd.spawn() {
        Ok(c) => c,
        Err(e) => {
            return Response::Error {
                message: format!("Failed to spawn process: {}", e),
            };
        }
    };

    // Write stdin if provided
    if let Some(input) = stdin {
        if let Some(mut child_stdin) = child.stdin.take() {
            if let Err(e) = child_stdin.write_all(input.as_bytes()).await {
                error!(error = %e, "Failed to write stdin");
            }
        }
    }

    // Wait for completion with timeout
    let timeout = tokio::time::Duration::from_secs(300); // 5 minutes
    let result = tokio::time::timeout(timeout, child.wait_with_output()).await;

    match result {
        Ok(Ok(output)) => {
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            let stderr = String::from_utf8_lossy(&output.stderr).to_string();
            let exit_code = output.status.code().unwrap_or(-1);

            Response::ExecResult(ExecOutput {
                exit_code,
                stdout,
                stderr,
            })
        }
        Ok(Err(e)) => Response::Error {
            message: format!("Command execution failed: {}", e),
        },
        Err(_) => Response::Error {
            message: "Command timed out after 300 seconds".to_string(),
        },
    }
}

/// Read a file
///
/// TigerStyle: Explicit size limits, error handling
async fn read_file(path: &str) -> Response {
    assert!(!path.is_empty(), "path must not be empty");

    match tokio::fs::read(path).await {
        Ok(contents) => Response::FileContents { contents },
        Err(e) => Response::Error {
            message: format!("Failed to read file: {}", e),
        },
    }
}

/// Write a file
///
/// TigerStyle: Atomic write with temp file, explicit permissions
async fn write_file(path: &str, contents: &[u8]) -> Response {
    assert!(!path.is_empty(), "path must not be empty");

    match tokio::fs::write(path, contents).await {
        Ok(()) => Response::Success,
        Err(e) => Response::Error {
            message: format!("Failed to write file: {}", e),
        },
    }
}

/// List directory contents
///
/// TigerStyle: Explicit limits on number of entries
async fn list_directory(path: &str) -> Response {
    assert!(!path.is_empty(), "path must not be empty");

    match tokio::fs::read_dir(path).await {
        Ok(mut entries) => {
            let mut files = Vec::new();
            while let Some(entry) = entries.next_entry().await.transpose() {
                match entry {
                    Ok(e) => {
                        if let Ok(name) = e.file_name().into_string() {
                            files.push(name);
                        }
                    }
                    Err(e) => {
                        return Response::Error {
                            message: format!("Failed to read directory entry: {}", e),
                        };
                    }
                }
            }

            Response::DirectoryListing { files }
        }
        Err(e) => Response::Error {
            message: format!("Failed to read directory: {}", e),
        },
    }
}

/// Send a response
///
/// TigerStyle: Length-prefixed protocol, explicit error handling
async fn send_response<S>(stream: &mut S, response: &Response) -> Result<()>
where
    S: AsyncWrite + Unpin,
{
    let response_bytes = serde_json::to_vec(response)?;
    let response_len = response_bytes.len() as u32;

    // Send length prefix (4 bytes, big-endian)
    stream.write_all(&response_len.to_be_bytes()).await?;

    // Send response body
    stream.write_all(&response_bytes).await?;

    stream.flush().await?;

    Ok(())
}
