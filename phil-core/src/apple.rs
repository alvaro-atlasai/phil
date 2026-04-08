//! Apple Intelligence backend via the `phil-apple` helper binary.
//!
//! Uses macOS 26+ FoundationModels framework through a thin Swift helper.
//! The helper communicates via JSON-line protocol over stdin/stdout.

use std::io::Write;
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize)]
struct AppleRequest {
    #[serde(skip_serializing_if = "Option::is_none")]
    system: Option<String>,
    prompt: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    temperature: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    max_tokens: Option<u32>,
}

#[derive(Debug, Deserialize)]
struct AppleResponse {
    text: Option<String>,
    error: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PingResponse {
    status: Option<String>,
}

#[derive(Debug, thiserror::Error)]
pub enum AppleError {
    #[error("phil-apple not found — Apple Intelligence requires macOS 26+ and the phil-apple helper")]
    NotFound,
    #[error("Apple Intelligence not available: {0}")]
    Unavailable(String),
    #[error("phil-apple error: {0}")]
    Process(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// Check if Apple Intelligence is available by pinging the helper binary.
pub fn apple_available() -> bool {
    // Only check on macOS
    if !cfg!(target_os = "macos") {
        return false;
    }

    let helper = find_helper();
    let Some(helper_path) = helper else {
        return false;
    };

    match Command::new(&helper_path).arg("--ping").output() {
        Ok(output) => {
            if !output.status.success() {
                return false;
            }
            let stdout = String::from_utf8_lossy(&output.stdout);
            if let Ok(resp) = serde_json::from_str::<PingResponse>(&stdout) {
                resp.status.as_deref() == Some("ok")
            } else {
                false
            }
        }
        Err(_) => false,
    }
}

/// Complete a prompt using Apple Intelligence.
pub fn apple_complete(
    system_prompt: &str,
    user_input: &str,
    temperature: f32,
    max_tokens: u32,
) -> Result<String, AppleError> {
    let helper_path = find_helper().ok_or(AppleError::NotFound)?;

    let req = AppleRequest {
        system: if system_prompt.is_empty() {
            None
        } else {
            Some(system_prompt.to_string())
        },
        prompt: user_input.to_string(),
        temperature: Some(temperature),
        max_tokens: Some(max_tokens),
    };

    let req_json =
        serde_json::to_string(&req).map_err(|e| AppleError::Process(format!("serialize: {e}")))?;

    let mut child = Command::new(&helper_path)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;

    // Write JSON request and close stdin
    if let Some(ref mut stdin) = child.stdin {
        writeln!(stdin, "{req_json}")?;
    }
    drop(child.stdin.take());

    let output = child.wait_with_output()?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(AppleError::Process(format!(
            "exit code {}: {}",
            output.status.code().unwrap_or(-1),
            stderr.trim()
        )));
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    // Take the last non-empty line (in case of build output etc.)
    let response_line = stdout
        .lines()
        .filter(|l| !l.is_empty())
        .last()
        .ok_or_else(|| AppleError::Process("empty response".into()))?;

    let resp: AppleResponse = serde_json::from_str(response_line)
        .map_err(|e| AppleError::Process(format!("parse response: {e}")))?;

    if let Some(error) = resp.error {
        if error.contains("unavailable") || error.contains("not enabled") {
            return Err(AppleError::Unavailable(error));
        }
        return Err(AppleError::Process(error));
    }

    resp.text
        .ok_or_else(|| AppleError::Process("no text in response".into()))
}

/// Find the phil-apple helper binary. Checks:
/// 1. Next to the phil binary itself
/// 2. In PATH
fn find_helper() -> Option<String> {
    // Check next to the current executable
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let sibling = dir.join("phil-apple");
            if sibling.exists() {
                return Some(sibling.to_string_lossy().into_owned());
            }
        }
    }

    // Check in PATH
    if let Ok(output) = Command::new("which").arg("phil-apple").output() {
        if output.status.success() {
            let path = String::from_utf8_lossy(&output.stdout).trim().to_string();
            if !path.is_empty() {
                return Some(path);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_helper_does_not_panic() {
        // Just ensure it doesn't panic — availability depends on the system
        let _ = find_helper();
    }

    #[test]
    fn apple_available_returns_bool() {
        let _ = apple_available();
    }
}
