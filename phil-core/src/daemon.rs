use std::io::Write;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::Notify;

use crate::inference::{CompletionParams, PhilInference};

const IDLE_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes

#[derive(Debug, Serialize, Deserialize)]
pub struct DaemonRequest {
    pub system_prompt: String,
    pub user_input: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub top_p: f32,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DaemonResponse {
    pub ok: bool,
    pub text: Option<String>,
    pub error: Option<String>,
}

/// Returns the path to the Unix socket.
pub fn socket_path() -> Result<PathBuf, crate::ModelError> {
    let home = dirs::home_dir().ok_or(crate::ModelError::NoHomeDir)?;
    Ok(home.join(".phil").join("phil.sock"))
}

/// Returns the path to the daemon PID file.
pub fn pid_path() -> Result<PathBuf, crate::ModelError> {
    let home = dirs::home_dir().ok_or(crate::ModelError::NoHomeDir)?;
    Ok(home.join(".phil").join("phil.pid"))
}

/// Check if a daemon is already running and responsive.
pub async fn daemon_is_running() -> bool {
    let sock = match socket_path() {
        Ok(s) => s,
        Err(_) => return false,
    };
    if !sock.exists() {
        return false;
    }
    // Try to connect
    match UnixStream::connect(&sock).await {
        Ok(mut stream) => {
            // Send a ping
            let ping = DaemonRequest {
                system_prompt: String::new(),
                user_input: "__ping__".to_string(),
                max_tokens: 0,
                temperature: 0.0,
                top_p: 0.0,
            };
            let json = serde_json::to_string(&ping).unwrap();
            let _ = stream.write_all(json.as_bytes()).await;
            let _ = stream.write_all(b"\n").await;
            let _ = stream.shutdown().await;
            true
        }
        Err(_) => {
            // Stale socket file
            let _ = std::fs::remove_file(&sock);
            false
        }
    }
}

/// Run the daemon server. Loads the model once and handles requests over a Unix socket.
/// Auto-exits after IDLE_TIMEOUT of inactivity.
pub async fn run_daemon(model_path: PathBuf) -> Result<(), Box<dyn std::error::Error>> {
    let sock_path = socket_path()?;
    let pid_file = pid_path()?;

    // Ensure parent dir exists
    if let Some(parent) = sock_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }

    // Clean up stale socket
    if sock_path.exists() {
        std::fs::remove_file(&sock_path)?;
    }

    // Write PID file
    std::fs::write(&pid_file, std::process::id().to_string())?;

    // Load model (one-time cost)
    eprintln!("phil daemon: loading model...");
    let inference = Arc::new(Mutex::new(PhilInference::load(&model_path)?));
    eprintln!("phil daemon: ready at {}", sock_path.display());

    let listener = UnixListener::bind(&sock_path)?;
    let last_activity = Arc::new(Mutex::new(Instant::now()));
    let shutdown = Arc::new(Notify::new());

    // Idle timeout watcher
    {
        let last_activity = Arc::clone(&last_activity);
        let shutdown = Arc::clone(&shutdown);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(Duration::from_secs(30)).await;
                let elapsed = last_activity.lock().unwrap().elapsed();
                if elapsed >= IDLE_TIMEOUT {
                    eprintln!("phil daemon: idle timeout, shutting down");
                    shutdown.notify_one();
                    return;
                }
            }
        });
    }

    loop {
        tokio::select! {
            _ = shutdown.notified() => {
                break;
            }
            result = listener.accept() => {
                match result {
                    Ok((stream, _)) => {
                        *last_activity.lock().unwrap() = Instant::now();
                        let inference = Arc::clone(&inference);
                        // Serialize inference — only one GPU context at a time
                        tokio::task::spawn_blocking(move || {
                            let _guard = inference.lock().unwrap();
                            handle_connection(stream, &_guard);
                        });
                    }
                    Err(e) => {
                        eprintln!("phil daemon: accept error: {e}");
                    }
                }
            }
        }
    }

    // Cleanup
    let _ = std::fs::remove_file(&sock_path);
    let _ = std::fs::remove_file(&pid_file);
    eprintln!("phil daemon: stopped");
    Ok(())
}

fn handle_connection(stream: UnixStream, inference: &PhilInference) {
    let std_stream = match stream.into_std() {
        Ok(s) => s,
        Err(_) => return,
    };
    std_stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .ok();

    let reader = std::io::BufReader::new(&std_stream);
    let mut writer = &std_stream;

    use std::io::BufRead;
    for line in reader.lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };
        if line.trim().is_empty() {
            continue;
        }

        let req: DaemonRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = DaemonResponse {
                    ok: false,
                    text: None,
                    error: Some(format!("invalid request: {e}")),
                };
                let _ = serde_json::to_writer(&mut writer, &resp);
                let _ = writer.write_all(b"\n");
                break;
            }
        };

        // Handle ping
        if req.user_input == "__ping__" {
            let resp = DaemonResponse {
                ok: true,
                text: Some("pong".to_string()),
                error: None,
            };
            let _ = serde_json::to_writer(&mut writer, &resp);
            let _ = writer.write_all(b"\n");
            break;
        }

        let params = CompletionParams {
            max_tokens: req.max_tokens,
            temperature: req.temperature,
            top_p: req.top_p,
        };

        let resp = match inference.complete(&req.system_prompt, &req.user_input, &params) {
            Ok(text) => DaemonResponse {
                ok: true,
                text: Some(text),
                error: None,
            },
            Err(e) => DaemonResponse {
                ok: false,
                text: None,
                error: Some(e.to_string()),
            },
        };

        let _ = serde_json::to_writer(&mut writer, &resp);
        let _ = writer.write_all(b"\n");
        break; // one request per connection
    }
}

/// Send a request to the running daemon and return the response text.
pub async fn daemon_complete(req: &DaemonRequest) -> Result<String, String> {
    let sock = socket_path().map_err(|e| e.to_string())?;

    // Retry on transient connection errors (e.g. daemon busy with ping)
    let mut last_err = String::new();
    for _ in 0..3 {
        match daemon_complete_once(req, &sock).await {
            Ok(text) => return Ok(text),
            Err(e) if e.contains("not connected") || e.contains("Connection refused") || e.contains("Broken pipe") => {
                last_err = e;
                tokio::time::sleep(Duration::from_millis(100)).await;
                continue;
            }
            Err(e) => return Err(e),
        }
    }
    Err(last_err)
}

async fn daemon_complete_once(req: &DaemonRequest, sock: &std::path::Path) -> Result<String, String> {
    let mut stream = UnixStream::connect(sock)
        .await
        .map_err(|e| format!("failed to connect to daemon: {e}"))?;

    let json = serde_json::to_string(req).unwrap();
    stream
        .write_all(json.as_bytes())
        .await
        .map_err(|e| format!("write failed: {e}"))?;
    stream
        .write_all(b"\n")
        .await
        .map_err(|e| format!("write failed: {e}"))?;
    stream
        .shutdown()
        .await
        .map_err(|e| format!("shutdown failed: {e}"))?;

    let mut reader = BufReader::new(stream);
    let mut response_line = String::new();
    reader
        .read_line(&mut response_line)
        .await
        .map_err(|e| format!("read failed: {e}"))?;

    let resp: DaemonResponse =
        serde_json::from_str(&response_line).map_err(|e| format!("invalid response: {e}"))?;

    if resp.ok {
        Ok(resp.text.unwrap_or_default())
    } else {
        Err(resp.error.unwrap_or_else(|| "unknown error".to_string()))
    }
}
