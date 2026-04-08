use std::path::PathBuf;

use serde::{Deserialize, Serialize};

/// Phil configuration stored at `~/.phil/config.toml`.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    #[serde(default)]
    pub model: ModelConfig,
    #[serde(default)]
    pub daemon: DaemonConfig,
    #[serde(default)]
    pub defaults: DefaultsConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub github: Option<GitHubConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    /// Name of the active model (from the registry). Default: "phi4-mini"
    #[serde(default = "default_model_name")]
    pub active: String,
    /// Override: explicit path to a .gguf file (bypasses registry)
    #[serde(default)]
    pub path: Option<String>,
}

impl Default for ModelConfig {
    fn default() -> Self {
        Self {
            active: default_model_name(),
            path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonConfig {
    /// Idle timeout in seconds before daemon auto-exits
    #[serde(default = "default_idle_timeout")]
    pub idle_timeout: u64,
    /// Disable the daemon entirely
    #[serde(default)]
    pub disabled: bool,
}

impl Default for DaemonConfig {
    fn default() -> Self {
        Self {
            idle_timeout: default_idle_timeout(),
            disabled: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DefaultsConfig {
    /// Default temperature for inference
    #[serde(default = "default_temperature")]
    pub temperature: f32,
    /// Default max tokens
    #[serde(default = "default_max_tokens")]
    pub max_tokens: u32,
}

impl Default for DefaultsConfig {
    fn default() -> Self {
        Self {
            temperature: default_temperature(),
            max_tokens: default_max_tokens(),
        }
    }
}

fn default_model_name() -> String { "phi4-mini".into() }
fn default_idle_timeout() -> u64 { 300 }
fn default_temperature() -> f32 { 0.1 }
fn default_max_tokens() -> u32 { 2048 }

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubConfig {
    /// GitHub Personal Access Token with models:read scope
    #[serde(default)]
    pub token: String,
}

/// Returns the path to the config file: `~/.phil/config.toml`
pub fn config_path() -> Result<PathBuf, crate::ModelError> {
    let home = dirs::home_dir().ok_or(crate::ModelError::NoHomeDir)?;
    Ok(home.join(".phil").join("config.toml"))
}

/// Load config from disk. Returns defaults if file doesn't exist.
pub fn load_config() -> Result<Config, crate::ModelError> {
    let path = config_path()?;
    if !path.exists() {
        return Ok(Config::default());
    }
    let content = std::fs::read_to_string(&path)?;
    let config: Config = toml::from_str(&content)
        .map_err(|e| crate::ModelError::Download(format!("invalid config: {e}")))?;
    Ok(config)
}

/// Save config to disk.
pub fn save_config(config: &Config) -> Result<(), crate::ModelError> {
    let path = config_path()?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let content = toml::to_string_pretty(config)
        .map_err(|e| crate::ModelError::Download(format!("serialize config: {e}")))?;
    std::fs::write(&path, content)?;
    Ok(())
}

/// Scaffold a default config file. Returns the path.
pub fn init_config() -> Result<PathBuf, crate::ModelError> {
    let path = config_path()?;
    if path.exists() {
        return Ok(path);
    }
    save_config(&Config::default())?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_valid() {
        let cfg = Config::default();
        assert_eq!(cfg.model.active, "phi4-mini");
        assert_eq!(cfg.daemon.idle_timeout, 300);
        assert!(!cfg.daemon.disabled);
        assert_eq!(cfg.defaults.temperature, 0.1);
        assert_eq!(cfg.defaults.max_tokens, 2048);
    }

    #[test]
    fn parse_minimal_config() {
        let toml_str = r#"
[model]
active = "phi4-mini"
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.model.active, "phi4-mini");
        assert_eq!(cfg.daemon.idle_timeout, 300); // default
    }

    #[test]
    fn parse_full_config() {
        let toml_str = r#"
[model]
active = "llama3"
path = "/custom/model.gguf"

[daemon]
idle_timeout = 600
disabled = false

[defaults]
temperature = 0.5
max_tokens = 4096
"#;
        let cfg: Config = toml::from_str(toml_str).unwrap();
        assert_eq!(cfg.model.active, "llama3");
        assert_eq!(cfg.model.path, Some("/custom/model.gguf".into()));
        assert_eq!(cfg.daemon.idle_timeout, 600);
        assert_eq!(cfg.defaults.temperature, 0.5);
        assert_eq!(cfg.defaults.max_tokens, 4096);
    }

    #[test]
    fn roundtrip_config() {
        let cfg = Config::default();
        let serialized = toml::to_string_pretty(&cfg).unwrap();
        let deserialized: Config = toml::from_str(&serialized).unwrap();
        assert_eq!(deserialized.model.active, cfg.model.active);
        assert_eq!(deserialized.defaults.temperature, cfg.defaults.temperature);
    }
}
