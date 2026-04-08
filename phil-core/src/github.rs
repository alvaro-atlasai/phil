//! GitHub Models API backend for remote inference.
//!
//! Uses the GitHub Models inference endpoint (OpenAI-compatible) at
//! `https://models.inference.ai.azure.com`. Authenticates with a GitHub PAT.

use serde::{Deserialize, Serialize};

const GITHUB_MODELS_ENDPOINT: &str = "https://models.inference.ai.azure.com/chat/completions";

/// A GitHub Models API chat completion request.
#[derive(Debug, Serialize)]
struct ChatRequest {
    model: String,
    messages: Vec<ChatMessage>,
    temperature: f32,
    max_tokens: u32,
    top_p: f32,
}

#[derive(Debug, Serialize, Deserialize)]
struct ChatMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct ChatResponse {
    choices: Vec<ChatChoice>,
}

#[derive(Debug, Deserialize)]
struct ChatChoice {
    message: ChatMessage,
}

#[derive(Debug, Deserialize)]
struct ErrorResponse {
    error: Option<ApiError>,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    message: String,
}

#[derive(Debug, thiserror::Error)]
pub enum GitHubError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("api error: {0}")]
    Api(String),
    #[error("no github token configured. Run: phil auth github")]
    NoToken,
    #[error("invalid token: {0}")]
    AuthFailed(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

/// The set of models available through GitHub Models.
pub fn github_models() -> Vec<GitHubModel> {
    vec![
        GitHubModel {
            name: "gpt-4o",
            api_name: "gpt-4o",
            description: "OpenAI GPT-4o (fast, strong reasoning)",
            provider: "OpenAI",
        },
        GitHubModel {
            name: "gpt-4o-mini",
            api_name: "gpt-4o-mini",
            description: "OpenAI GPT-4o-mini (fast, cheap)",
            provider: "OpenAI",
        },
        GitHubModel {
            name: "o4-mini",
            api_name: "o4-mini",
            description: "OpenAI o4-mini (reasoning model)",
            provider: "OpenAI",
        },
        GitHubModel {
            name: "llama-3.3-70b",
            api_name: "Meta-Llama-3.3-70B-Instruct",
            description: "Meta Llama 3.3 70B (open, strong)",
            provider: "Meta",
        },
        GitHubModel {
            name: "mistral-large",
            api_name: "Mistral-Large-2411",
            description: "Mistral Large (strong multilingual)",
            provider: "Mistral",
        },
        GitHubModel {
            name: "deepseek-r1",
            api_name: "DeepSeek-R1",
            description: "DeepSeek R1 (reasoning, open-weight)",
            provider: "DeepSeek",
        },
    ]
}

#[derive(Debug, Clone)]
pub struct GitHubModel {
    /// Short name used in phil CLI
    pub name: &'static str,
    /// API model identifier for the endpoint
    pub api_name: &'static str,
    /// Human description
    pub description: &'static str,
    /// Provider name
    pub provider: &'static str,
}

/// Look up a GitHub model by short name.
pub fn find_github_model(name: &str) -> Option<GitHubModel> {
    github_models().into_iter().find(|m| m.name == name)
}

/// Call the GitHub Models API for a chat completion.
pub async fn complete(
    token: &str,
    model_api_name: &str,
    system_prompt: &str,
    user_input: &str,
    temperature: f32,
    max_tokens: u32,
) -> Result<String, GitHubError> {
    let mut messages = Vec::new();
    if !system_prompt.is_empty() {
        messages.push(ChatMessage {
            role: "system".into(),
            content: system_prompt.into(),
        });
    }
    messages.push(ChatMessage {
        role: "user".into(),
        content: user_input.into(),
    });

    let req = ChatRequest {
        model: model_api_name.into(),
        messages,
        temperature,
        max_tokens,
        top_p: 0.9,
    };

    let client = reqwest::Client::new();
    let resp = client
        .post(GITHUB_MODELS_ENDPOINT)
        .header("Authorization", format!("Bearer {token}"))
        .json(&req)
        .send()
        .await?;

    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        if let Ok(err) = serde_json::from_str::<ErrorResponse>(&body) {
            if let Some(api_err) = err.error {
                if status.as_u16() == 401 {
                    return Err(GitHubError::AuthFailed(api_err.message));
                }
                return Err(GitHubError::Api(api_err.message));
            }
        }
        return Err(GitHubError::Api(format!("HTTP {status}: {body}")));
    }

    let chat_resp: ChatResponse = resp.json().await?;
    chat_resp
        .choices
        .into_iter()
        .next()
        .map(|c| c.message.content)
        .ok_or_else(|| GitHubError::Api("empty response from API".into()))
}

/// Validate a GitHub token by calling the /user endpoint.
pub async fn validate_token(token: &str) -> Result<String, GitHubError> {
    let client = reqwest::Client::new();
    let resp = client
        .get("https://api.github.com/user")
        .header("Authorization", format!("Bearer {token}"))
        .header("User-Agent", "phil-cli")
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;

    if !resp.status().is_success() {
        return Err(GitHubError::AuthFailed(
            format!("token validation failed (HTTP {}). Ensure you have a PAT with 'models:read' scope.", resp.status()),
        ));
    }

    #[derive(Deserialize)]
    struct User {
        login: String,
    }
    let user: User = resp.json().await?;
    Ok(user.login)
}

/// Load the stored GitHub token from config.
pub fn load_token() -> Result<String, GitHubError> {
    let cfg = crate::config::load_config()
        .map_err(|e| GitHubError::Io(std::io::Error::other(e.to_string())))?;
    cfg.github
        .and_then(|g| if g.token.is_empty() { None } else { Some(g.token) })
        .ok_or(GitHubError::NoToken)
}

/// Store a GitHub token in config.
pub fn save_token(token: &str) -> Result<(), GitHubError> {
    let mut cfg = crate::config::load_config()
        .map_err(|e| GitHubError::Io(std::io::Error::other(e.to_string())))?;
    cfg.github = Some(crate::config::GitHubConfig {
        token: token.to_string(),
    });
    crate::config::save_config(&cfg)
        .map_err(|e| GitHubError::Io(std::io::Error::other(e.to_string())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn github_models_all_have_names() {
        for m in github_models() {
            assert!(!m.name.is_empty());
            assert!(!m.api_name.is_empty());
            assert!(!m.description.is_empty());
        }
    }

    #[test]
    fn find_known_model() {
        assert!(find_github_model("gpt-4o").is_some());
        assert!(find_github_model("llama-3.3-70b").is_some());
        assert!(find_github_model("nonexistent").is_none());
    }

    #[test]
    fn find_returns_correct_api_name() {
        let m = find_github_model("llama-3.3-70b").unwrap();
        assert_eq!(m.api_name, "Meta-Llama-3.3-70B-Instruct");
    }
}
