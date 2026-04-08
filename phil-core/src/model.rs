use std::path::PathBuf;

use futures_util::StreamExt;
use indicatif::{ProgressBar, ProgressStyle};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

use crate::config;

const MODEL_FILENAME: &str = "Phi-4-mini-instruct-Q4_K_M.gguf";

/// A known model in the registry.
#[derive(Debug, Clone)]
pub struct ModelEntry {
    /// Short name used in config and CLI
    pub name: &'static str,
    /// Human description
    pub description: &'static str,
    /// GGUF filename
    pub filename: &'static str,
    /// Download URL
    pub url: &'static str,
    /// Expected SHA256 (empty = skip verification)
    pub sha256: &'static str,
    /// Approximate size for display
    pub size: &'static str,
}

/// Built-in model registry.
pub fn model_registry() -> Vec<ModelEntry> {
    vec![
        ModelEntry {
            name: "phi4-mini",
            description: "Microsoft Phi-4-mini-instruct Q4_K_M (default)",
            filename: "Phi-4-mini-instruct-Q4_K_M.gguf",
            url: "https://huggingface.co/unsloth/Phi-4-mini-instruct-GGUF/resolve/main/Phi-4-mini-instruct-Q4_K_M.gguf",
            sha256: "88c00229914083cd112853aab84ed51b87bdf6b9ce42f532d8c85c7c63b1730a",
            size: "2.5GB",
        },
        ModelEntry {
            name: "phi4-mini-fp16",
            description: "Microsoft Phi-4-mini-instruct FP16 (higher quality, larger)",
            filename: "Phi-4-mini-instruct-F16.gguf",
            url: "https://huggingface.co/unsloth/Phi-4-mini-instruct-GGUF/resolve/main/Phi-4-mini-instruct-F16.gguf",
            sha256: "",
            size: "7.6GB",
        },
        ModelEntry {
            name: "phi4-mini-q8",
            description: "Microsoft Phi-4-mini-instruct Q8_0 (balanced quality/size)",
            filename: "Phi-4-mini-instruct-Q8_0.gguf",
            url: "https://huggingface.co/unsloth/Phi-4-mini-instruct-GGUF/resolve/main/Phi-4-mini-instruct-Q8_0.gguf",
            sha256: "",
            size: "4.1GB",
        },
        ModelEntry {
            name: "qwen3-0.6b",
            description: "Qwen3 0.6B Q4_K_M (tiny, fast, low quality)",
            filename: "Qwen3-0.6B-Q4_K_M.gguf",
            url: "https://huggingface.co/unsloth/Qwen3-0.6B-GGUF/resolve/main/Qwen3-0.6B-Q4_K_M.gguf",
            sha256: "",
            size: "0.5GB",
        },
        ModelEntry {
            name: "qwen3-1.7b",
            description: "Qwen3 1.7B Q4_K_M (small, fast)",
            filename: "Qwen3-1.7B-Q4_K_M.gguf",
            url: "https://huggingface.co/unsloth/Qwen3-1.7B-GGUF/resolve/main/Qwen3-1.7B-Q4_K_M.gguf",
            sha256: "",
            size: "1.2GB",
        },
        ModelEntry {
            name: "qwen3-4b",
            description: "Qwen3 4B Q4_K_M (good quality, moderate size)",
            filename: "Qwen3-4B-Q4_K_M.gguf",
            url: "https://huggingface.co/unsloth/Qwen3-4B-GGUF/resolve/main/Qwen3-4B-Q4_K_M.gguf",
            sha256: "",
            size: "2.7GB",
        },
    ]
}

/// Look up a model by name in the registry.
pub fn find_model(name: &str) -> Option<ModelEntry> {
    model_registry().into_iter().find(|m| m.name == name)
}

#[derive(Debug, thiserror::Error)]
pub enum ModelError {
    #[error("failed to create model directory: {0}")]
    DirCreate(std::io::Error),
    #[error("model download failed: {0}")]
    Download(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("sha256 mismatch: expected {expected}, got {actual}")]
    Sha256Mismatch { expected: String, actual: String },
    #[error("could not determine home directory")]
    NoHomeDir,
}

pub struct ModelManager {
    models_dir: PathBuf,
}

impl ModelManager {
    pub fn new() -> Result<Self, ModelError> {
        let home = dirs::home_dir().ok_or(ModelError::NoHomeDir)?;
        let models_dir = home.join(".phil").join("models");
        Ok(Self { models_dir })
    }

    pub fn model_path(&self) -> PathBuf {
        self.models_dir.join(MODEL_FILENAME)
    }

    /// Resolve the model path from config: explicit path > active model name > default.
    pub fn resolve_model_path(&self) -> Result<PathBuf, ModelError> {
        let cfg = config::load_config()?;
        if let Some(ref p) = cfg.model.path {
            return Ok(PathBuf::from(p));
        }
        if let Some(entry) = find_model(&cfg.model.active) {
            return Ok(self.models_dir.join(entry.filename));
        }
        // Fallback to default
        Ok(self.model_path())
    }

    /// Ensures the active model exists locally. Downloads if missing.
    pub async fn ensure_model(&self) -> Result<PathBuf, ModelError> {
        let path = self.resolve_model_path()?;
        if path.exists() {
            return Ok(path);
        }

        tokio::fs::create_dir_all(&self.models_dir)
            .await
            .map_err(ModelError::DirCreate)?;

        // Find the model entry to get URL
        let cfg = config::load_config()?;
        let entry = find_model(&cfg.model.active)
            .unwrap_or_else(|| find_model("phi4-mini").unwrap());

        self.download_model_entry(&entry, &path).await?;
        Ok(path)
    }

    /// Install a model by name from the registry.
    pub async fn install_model(&self, name: &str) -> Result<PathBuf, ModelError> {
        let entry = find_model(name)
            .ok_or_else(|| ModelError::Download(format!("unknown model: {name}. Run `phil model ls` to see available models.")))?;

        tokio::fs::create_dir_all(&self.models_dir)
            .await
            .map_err(ModelError::DirCreate)?;

        let path = self.models_dir.join(entry.filename);
        if path.exists() {
            eprintln!("Model {name} already installed at {}", path.display());
            return Ok(path);
        }

        self.download_model_entry(&entry, &path).await?;
        Ok(path)
    }

    /// List installed models (files in models_dir).
    pub fn list_installed(&self) -> Result<Vec<String>, ModelError> {
        if !self.models_dir.exists() {
            return Ok(vec![]);
        }
        let mut names = vec![];
        for entry in std::fs::read_dir(&self.models_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "gguf") {
                if let Some(name) = path.file_name() {
                    names.push(name.to_string_lossy().to_string());
                }
            }
        }
        names.sort();
        Ok(names)
    }

    async fn download_model_entry(&self, entry: &ModelEntry, dest: &PathBuf) -> Result<(), ModelError> {
        eprintln!("Downloading {} (~{})...", entry.description, entry.size);
        eprintln!("From: {}", entry.url);
        eprintln!("To:   {}", dest.display());

        let client = reqwest::Client::new();
        let resp = client
            .get(entry.url)
            .send()
            .await?
            .error_for_status()
            .map_err(|e| ModelError::Download(e.to_string()))?;

        let total_size = resp.content_length().unwrap_or(0);

        let pb = ProgressBar::new(total_size);
        pb.set_style(
            ProgressStyle::default_bar()
                .template("{spinner:.green} [{bar:40.cyan/blue}] {bytes}/{total_bytes} ({eta})")
                .unwrap()
                .progress_chars("#>-"),
        );

        let tmp_path = dest.with_extension("gguf.tmp");
        let mut file = tokio::fs::File::create(&tmp_path).await?;
        let mut hasher = Sha256::new();
        let mut stream = resp.bytes_stream();

        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
            pb.inc(chunk.len() as u64);
        }

        file.flush().await?;
        drop(file);
        pb.finish_with_message("Download complete");

        // Verify SHA256 if known
        if !entry.sha256.is_empty() {
            let hash = format!("{:x}", hasher.finalize());
            if hash != entry.sha256 {
                tokio::fs::remove_file(&tmp_path).await?;
                return Err(ModelError::Sha256Mismatch {
                    expected: entry.sha256.to_string(),
                    actual: hash,
                });
            }
        }

        tokio::fs::rename(&tmp_path, dest).await?;
        eprintln!("Model ready at {}", dest.display());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_manager_creates_with_correct_path() {
        let mgr = ModelManager::new().unwrap();
        let path = mgr.model_path();
        assert!(path.to_string_lossy().contains(".phil/models/"));
        assert!(path
            .file_name()
            .unwrap()
            .to_string_lossy()
            .ends_with(".gguf"));
    }

    #[test]
    fn model_path_contains_filename() {
        let mgr = ModelManager::new().unwrap();
        let path = mgr.model_path();
        assert_eq!(
            path.file_name().unwrap().to_string_lossy(),
            MODEL_FILENAME
        );
    }

    #[test]
    fn model_manager_models_dir_under_home() {
        let mgr = ModelManager::new().unwrap();
        let home = dirs::home_dir().unwrap();
        assert!(mgr.models_dir.starts_with(&home));
    }

    #[tokio::test]
    async fn ensure_model_returns_path_if_exists() {
        // Create a temp dir that mimics ~/.phil/models/ with a fake model file
        let tmp = tempfile::tempdir().unwrap();
        let models_dir = tmp.path().join("models");
        std::fs::create_dir_all(&models_dir).unwrap();
        let fake_model = models_dir.join(MODEL_FILENAME);
        std::fs::write(&fake_model, b"fake").unwrap();

        let mgr = ModelManager {
            models_dir: models_dir.clone(),
        };
        let result = mgr.ensure_model().await.unwrap();
        assert_eq!(result, fake_model);
    }
}
