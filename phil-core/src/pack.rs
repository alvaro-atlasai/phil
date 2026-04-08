use std::path::{Path, PathBuf};
use std::collections::BTreeMap;
use std::sync::OnceLock;

use serde::{Deserialize, Serialize};

/// A Phil pack — a reusable prompt configuration stored as a TOML file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Pack {
    /// Short identifier (e.g., "commit", "json")
    pub name: String,
    /// Brief description shown in `phil pack ls`
    pub description: String,
    /// The system prompt that defines the pack's behavior
    pub system: String,
    /// Sampling temperature override
    #[serde(default)]
    pub temperature: Option<f32>,
    /// Max tokens override
    #[serde(default)]
    pub max_tokens: Option<u32>,
    /// Whether --each is implied when stdin is piped
    #[serde(default)]
    pub each: bool,
}

/// Lightweight pack metadata for fast listing (no full system prompt).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackMeta {
    pub name: String,
    pub description: String,
    pub builtin: bool,
}

/// Cached pack index stored at `~/.phil/packs/.index.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct PackIndex {
    packs: Vec<PackMeta>,
}

/// Returns the packs directory: `~/.phil/packs/`
pub fn packs_dir() -> Result<PathBuf, crate::ModelError> {
    let home = dirs::home_dir().ok_or(crate::ModelError::NoHomeDir)?;
    Ok(home.join(".phil").join("packs"))
}

fn index_path() -> Result<PathBuf, crate::ModelError> {
    Ok(packs_dir()?.join(".index.json"))
}

use std::collections::HashSet;

/// Static set of builtin pack names — O(1) lookup instead of scanning all packs.
fn builtin_names() -> &'static HashSet<String> {
    static NAMES: OnceLock<HashSet<String>> = OnceLock::new();
    NAMES.get_or_init(|| {
        builtin_packs().into_iter().map(|p| p.name).collect()
    })
}

/// Check if a pack name is a built-in. O(1).
pub fn is_builtin(name: &str) -> bool {
    builtin_names().contains(name)
}

/// Load a pack by name. Checks `~/.phil/packs/{name}.toml` first, then built-ins.
/// This is the hot path — does NOT read the index, just one file lookup + one builtin check.
pub fn load_pack(name: &str) -> Result<Pack, PackError> {
    // Check user packs first
    let dir = packs_dir().map_err(|e| PackError::Load(e.to_string()))?;
    let user_file = dir.join(format!("{name}.toml"));
    if user_file.exists() {
        return load_pack_from_file(&user_file);
    }

    // Fall back to built-in
    builtin_pack(name).ok_or_else(|| PackError::NotFound(name.to_string()))
}

/// Load a pack from a TOML file on disk.
pub fn load_pack_from_file(path: &Path) -> Result<Pack, PackError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| PackError::Load(format!("{}: {e}", path.display())))?;
    let pack: Pack = toml::from_str(&content)
        .map_err(|e| PackError::Parse(format!("{}: {e}", path.display())))?;
    Ok(pack)
}

/// List all packs as lightweight metadata. Uses the cached index when fresh,
/// rebuilds it when stale or missing. O(1) file read in the common case.
pub fn list_packs_meta() -> Result<Vec<PackMeta>, PackError> {
    let dir = packs_dir().map_err(|e| PackError::Load(e.to_string()))?;
    let idx_path = index_path().map_err(|e| PackError::Load(e.to_string()))?;

    if is_index_fresh(&idx_path, &dir) {
        if let Ok(index) = load_index(&idx_path) {
            return Ok(index.packs);
        }
    }

    // Rebuild
    let meta = rebuild_index_inner(&dir)?;
    let _ = save_index(&idx_path, &meta); // best-effort cache
    Ok(meta)
}

/// Full pack list (legacy). Prefer `list_packs_meta()` for listing.
pub fn list_packs() -> Result<Vec<Pack>, PackError> {
    let mut packs: BTreeMap<String, Pack> = BTreeMap::new();

    for pack in builtin_packs() {
        packs.insert(pack.name.clone(), pack);
    }

    let dir = packs_dir().map_err(|e| PackError::Load(e.to_string()))?;
    if dir.exists() {
        let entries = std::fs::read_dir(&dir)
            .map_err(|e| PackError::Load(format!("{}: {e}", dir.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| PackError::Load(e.to_string()))?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "toml") {
                if let Ok(pack) = load_pack_from_file(&path) {
                    packs.insert(pack.name.clone(), pack);
                }
            }
        }
    }

    Ok(packs.into_values().collect())
}

/// Rebuild the index from disk. Called after mutations or when stale.
pub fn rebuild_index() -> Result<(), PackError> {
    let dir = packs_dir().map_err(|e| PackError::Load(e.to_string()))?;
    let idx_path = index_path().map_err(|e| PackError::Load(e.to_string()))?;
    let meta = rebuild_index_inner(&dir)?;
    save_index(&idx_path, &meta).map_err(|e| PackError::Load(e.to_string()))
}

fn rebuild_index_inner(dir: &Path) -> Result<Vec<PackMeta>, PackError> {
    let mut meta_map: BTreeMap<String, PackMeta> = BTreeMap::new();

    // Builtins first
    for p in builtin_packs() {
        meta_map.insert(p.name.clone(), PackMeta {
            name: p.name,
            description: p.description,
            builtin: true,
        });
    }

    // Overlay user packs — only parse the header fields we need.
    // We use a minimal struct to avoid parsing the full system prompt.
    if dir.exists() {
        let entries = std::fs::read_dir(dir)
            .map_err(|e| PackError::Load(format!("{}: {e}", dir.display())))?;
        for entry in entries {
            let entry = entry.map_err(|e| PackError::Load(e.to_string()))?;
            let path = entry.path();
            if path.extension().is_some_and(|e| e == "toml") {
                if let Ok(m) = extract_pack_meta(&path) {
                    meta_map.insert(m.name.clone(), m);
                }
            }
        }
    }

    Ok(meta_map.into_values().collect())
}

/// Extract only name+description from a pack file without parsing the full
/// system prompt into a String allocation. Uses a minimal deserialize target.
fn extract_pack_meta(path: &Path) -> Result<PackMeta, PackError> {
    #[derive(Deserialize)]
    struct Header {
        name: String,
        description: String,
    }
    let content = std::fs::read_to_string(path)
        .map_err(|e| PackError::Load(format!("{}: {e}", path.display())))?;
    let h: Header = toml::from_str(&content)
        .map_err(|e| PackError::Parse(format!("{}: {e}", path.display())))?;
    Ok(PackMeta {
        name: h.name,
        description: h.description,
        builtin: false,
    })
}

fn is_index_fresh(idx_path: &Path, dir: &Path) -> bool {
    let Ok(idx_meta) = std::fs::metadata(idx_path) else { return false };
    let Ok(dir_meta) = std::fs::metadata(dir) else { return false };
    let Ok(idx_mtime) = idx_meta.modified() else { return false };
    let Ok(dir_mtime) = dir_meta.modified() else { return false };
    idx_mtime >= dir_mtime
}

fn load_index(path: &Path) -> Result<PackIndex, PackError> {
    let content = std::fs::read_to_string(path)
        .map_err(|e| PackError::Load(e.to_string()))?;
    serde_json::from_str(&content)
        .map_err(|e| PackError::Parse(e.to_string()))
}

fn save_index(path: &Path, meta: &[PackMeta]) -> Result<(), PackError> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let index = PackIndex { packs: meta.to_vec() };
    let json = serde_json::to_string(&index)
        .map_err(|e| PackError::Parse(e.to_string()))?;
    std::fs::write(path, json)?;
    Ok(())
}

/// Scaffold a new pack TOML file.
pub fn scaffold_pack(name: &str) -> String {
    format!(
        r#"name = "{name}"
description = "TODO: describe what this pack does"
system = """
TODO: write the system prompt here.
Be specific about the output format you want.
"""
# temperature = 0.1
# max_tokens = 2048
# each = false
"#
    )
}

/// System prompt for LLM-powered pack generation.
pub const PACK_GEN_SYSTEM: &str = r#"You generate phil pack TOML files. A pack has:
- name: short lowercase identifier (letters, numbers, hyphens)
- description: one-line summary of what this pack does
- system: the system prompt that defines the pack's behavior. Be specific about output format and constraints.
- temperature: 0.0-1.0 (lower = more deterministic, use 0.1 for code/data transforms, 0.3 for creative)
- max_tokens: max output length (512 for short outputs, 2048 general, 4096 for long analysis)
- each: whether to process stdin line-by-line. IMPORTANT RULES FOR each:
  - each = true ONLY for tasks where each line is independent (translating sentences, classifying log lines)
  - each = false for EVERYTHING ELSE, especially: converting file formats (CSV, JSON, YAML), analyzing code/diffs, summarizing documents, any task needing full context

Output ONLY valid TOML. No markdown fences. No commentary.

EXAMPLE (format conversion — each=false because full input is needed):

name = "csv-to-json"
description = "Convert CSV to JSON"
system = """
Convert the entire CSV input to a JSON array of objects. Use the first row as keys. Output only valid JSON.
"""
temperature = 0.1
max_tokens = 4096
each = false

EXAMPLE (per-line task — each=true):

name = "classify-log"
description = "Classify log lines by severity"
system = """
Classify this log line as ERROR, WARN, or INFO. Output only the label.
"""
temperature = 0.0
max_tokens = 32
each = true
"#;

/// Build the user prompt for pack generation.
pub fn pack_gen_prompt(description: &str) -> String {
    format!("Generate a phil pack TOML for: {description}")
}

/// Returns a built-in pack by name, or None.
fn builtin_pack(name: &str) -> Option<Pack> {
    builtin_packs().into_iter().find(|p| p.name == name)
}

/// All built-in packs.
fn builtin_packs() -> Vec<Pack> {
    vec![
        Pack {
            name: "commit".into(),
            description: "Conventional commit from staged diff".into(),
            system: "You generate a single conventional commit message from a git diff.\n\
                     Format: type(scope): description\n\
                     Types: feat, fix, refactor, docs, style, test, chore, perf, ci, build\n\
                     One line only. No explanation. No quotes. Under 72 characters.".into(),
            temperature: Some(0.1),
            max_tokens: Some(100),
            each: false,
        },
        Pack {
            name: "explain".into(),
            description: "Explain a command or concept concisely".into(),
            system: "You explain commands, code, and concepts in plain English.\n\
                     Be concise. Use bullet points for multi-part explanations.\n\
                     When explaining a command, break down each flag/argument.".into(),
            temperature: Some(0.3),
            max_tokens: Some(1024),
            each: false,
        },
        Pack {
            name: "json".into(),
            description: "Convert any input to JSON".into(),
            system: "Convert the input data to valid JSON. Output only the JSON, nothing else.\n\
                     Infer the structure from the data. Use arrays for lists, objects for records.\n\
                     Preserve all data. No markdown fences. No commentary.".into(),
            temperature: Some(0.1),
            max_tokens: Some(4096),
            each: false,
        },
        Pack {
            name: "review".into(),
            description: "Code review from a diff".into(),
            system: "You are a senior code reviewer. Given a diff, provide:\n\
                     1. A one-line summary of the change\n\
                     2. Issues found (bugs, security, performance, style) — or \"LGTM\" if none\n\
                     3. Suggestions (if any)\n\
                     Be direct. No fluff. Focus on what matters.".into(),
            temperature: Some(0.3),
            max_tokens: Some(2048),
            each: false,
        },
        Pack {
            name: "tldr".into(),
            description: "Summarize man pages or docs".into(),
            system: "Summarize the input into a tldr-style cheat sheet.\n\
                     Format:\n\
                     # command-name\n\
                     > One-line description\n\n\
                     - Task description:\n\
                     `command --flags`\n\n\
                     Show the 5–8 most common use cases. No prose.".into(),
            temperature: Some(0.2),
            max_tokens: Some(1024),
            each: false,
        },
        Pack {
            name: "mcp".into(),
            description: "Generate any2mcp manifest from --help output".into(),
            system: "You analyze CLI --help output and generate an any2mcp YAML manifest.\n\
                     For each command/subcommand, extract:\n\
                     - name, description\n\
                     - arguments with name, type (string/number/boolean), description, required flag\n\n\
                     Output valid YAML in this format:\n\
                     binary: <name>\n\
                     tools:\n\
                       - name: <subcommand>\n\
                         description: <what it does>\n\
                         args:\n\
                           - name: <arg>\n\
                             type: string\n\
                             description: <what it is>\n\
                             required: true\n\n\
                     Only output the YAML. No markdown fences. No commentary.".into(),
            temperature: Some(0.1),
            max_tokens: Some(4096),
            each: false,
        },
        // --- DevOps packs ---
        Pack {
            name: "az".into(),
            description: "Generate Azure CLI commands from natural language".into(),
            system: "You translate natural language requests into Azure CLI (az) commands.\n\
                     Output only the az command(s), one per line. No explanation.\n\
                     Use --output table for queries, --output json for automation.\n\
                     If multiple steps are needed, chain with && or list them.\n\
                     Always use safe, read-only commands unless explicitly asked to create/delete.".into(),
            temperature: Some(0.1),
            max_tokens: Some(512),
            each: false,
        },
        Pack {
            name: "k8s".into(),
            description: "Kubernetes troubleshooting and kubectl commands".into(),
            system: "You are a Kubernetes expert. Given cluster state, logs, or a description:\n\
                     1. Diagnose the issue in one line\n\
                     2. Provide the kubectl command(s) to investigate or fix\n\
                     3. If given logs/events, identify the root cause\n\
                     Be direct. Output actionable commands. No fluff.".into(),
            temperature: Some(0.2),
            max_tokens: Some(1024),
            each: false,
        },
        Pack {
            name: "docker".into(),
            description: "Docker commands and Dockerfile help".into(),
            system: "You are a Docker expert.\n\
                     - When given a description, output the docker command(s)\n\
                     - When given a Dockerfile, review it for best practices\n\
                     - When given docker logs or errors, diagnose the issue\n\
                     Output commands or analysis directly. No markdown fences unless showing a Dockerfile.".into(),
            temperature: Some(0.2),
            max_tokens: Some(1024),
            each: false,
        },
        Pack {
            name: "tf".into(),
            description: "Terraform helper — generate HCL from descriptions".into(),
            system: "You generate Terraform HCL code from natural language descriptions.\n\
                     Output valid HCL only. Use current best practices:\n\
                     - Use variables for configurable values\n\
                     - Include sensible defaults\n\
                     - Add brief inline comments for non-obvious settings\n\
                     No markdown fences. No explanation outside the HCL.".into(),
            temperature: Some(0.1),
            max_tokens: Some(2048),
            each: false,
        },
        Pack {
            name: "fabric".into(),
            description: "Microsoft Fabric / Power BI helper".into(),
            system: "You are a Microsoft Fabric and Power BI expert.\n\
                     - Generate DAX formulas from natural language\n\
                     - Generate KQL queries for Fabric Real-Time Analytics\n\
                     - Help with Power Query M expressions\n\
                     - Explain Fabric concepts (lakehouse, warehouse, pipeline)\n\
                     Output the formula/query/code directly. Be precise with syntax.".into(),
            temperature: Some(0.2),
            max_tokens: Some(1024),
            each: false,
        },
    ]
}

#[derive(Debug, thiserror::Error)]
pub enum PackError {
    #[error("pack not found: {0}")]
    NotFound(String),
    #[error("failed to load pack: {0}")]
    Load(String),
    #[error("failed to parse pack: {0}")]
    Parse(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_packs_all_valid() {
        let packs = builtin_packs();
        assert!(packs.len() >= 11); // 6 original + 5 devops
        for pack in &packs {
            assert!(!pack.name.is_empty());
            assert!(!pack.description.is_empty());
            assert!(!pack.system.is_empty());
        }
    }

    #[test]
    fn builtin_pack_by_name() {
        assert!(builtin_pack("commit").is_some());
        assert!(builtin_pack("mcp").is_some());
        assert!(builtin_pack("nonexistent").is_none());
    }

    #[test]
    fn scaffold_contains_name() {
        let toml = scaffold_pack("mypack");
        assert!(toml.contains("name = \"mypack\""));
        assert!(toml.contains("description"));
        assert!(toml.contains("system"));
    }

    #[test]
    fn parse_scaffold_roundtrip() {
        // Fill in the TODOs so it's valid
        let toml_str = r#"
name = "test"
description = "a test pack"
system = "be helpful"
temperature = 0.5
max_tokens = 512
each = true
"#;
        let pack: Pack = toml::from_str(toml_str).unwrap();
        assert_eq!(pack.name, "test");
        assert_eq!(pack.temperature, Some(0.5));
        assert_eq!(pack.max_tokens, Some(512));
        assert!(pack.each);
    }

    #[test]
    fn parse_minimal_pack() {
        let toml_str = r#"
name = "minimal"
description = "just the basics"
system = "do stuff"
"#;
        let pack: Pack = toml::from_str(toml_str).unwrap();
        assert_eq!(pack.name, "minimal");
        assert_eq!(pack.temperature, None);
        assert_eq!(pack.max_tokens, None);
        assert!(!pack.each);
    }

    #[test]
    fn list_packs_includes_builtins() {
        let packs = list_packs().unwrap();
        let names: Vec<&str> = packs.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"commit"));
        assert!(names.contains(&"json"));
        assert!(names.contains(&"mcp"));
    }

    #[test]
    fn load_builtin_pack() {
        let pack = load_pack("commit").unwrap();
        assert_eq!(pack.name, "commit");
        assert!(pack.system.contains("conventional commit"));
    }

    #[test]
    fn load_nonexistent_pack() {
        let err = load_pack("zzz_nonexistent_zzz").unwrap_err();
        assert!(matches!(err, PackError::NotFound(_)));
    }

    #[test]
    fn is_builtin_known_names() {
        assert!(is_builtin("commit"));
        assert!(is_builtin("az"));
        assert!(is_builtin("k8s"));
        assert!(is_builtin("fabric"));
        assert!(!is_builtin("nonexistent"));
    }

    #[test]
    fn list_packs_meta_includes_builtins() {
        let meta = list_packs_meta().unwrap();
        let names: Vec<&str> = meta.iter().map(|m| m.name.as_str()).collect();
        assert!(names.contains(&"commit"));
        assert!(names.contains(&"json"));
        assert!(names.contains(&"az"));
        for m in &meta {
            if is_builtin(&m.name) {
                assert!(m.builtin);
            }
        }
    }

    #[test]
    fn index_roundtrip_in_tempdir() {
        let dir = tempfile::tempdir().unwrap();
        let idx_path = dir.path().join(".index.json");

        // Write a user pack
        let pack_content = r#"name = "custom"
description = "a custom pack"
system = "do custom things"
"#;
        std::fs::write(dir.path().join("custom.toml"), pack_content).unwrap();

        // Rebuild index from this dir
        let meta = rebuild_index_inner(dir.path()).unwrap();
        save_index(&idx_path, &meta).unwrap();

        // Load it back
        let loaded = load_index(&idx_path).unwrap();
        let custom = loaded.packs.iter().find(|m| m.name == "custom").unwrap();
        assert_eq!(custom.description, "a custom pack");
        assert!(!custom.builtin);
        // Builtins should also be present
        assert!(loaded.packs.iter().any(|m| m.name == "commit" && m.builtin));
    }

    #[test]
    fn index_staleness_check() {
        let dir = tempfile::tempdir().unwrap();
        let idx_path = dir.path().join(".index.json");

        // No index yet → not fresh
        assert!(!is_index_fresh(&idx_path, dir.path()));

        // Create index
        std::fs::write(&idx_path, "{}").unwrap();
        // Immediately after creation, dir hasn't changed → fresh
        assert!(is_index_fresh(&idx_path, dir.path()));

        // Touch a file in dir → dir mtime updates → stale
        std::thread::sleep(std::time::Duration::from_millis(50));
        std::fs::write(dir.path().join("new.toml"), "").unwrap();
        assert!(!is_index_fresh(&idx_path, dir.path()));
    }
}
