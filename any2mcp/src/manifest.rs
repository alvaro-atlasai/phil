use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Manifest {
    pub binary: String,
    pub description: String,
    pub tools: Vec<ToolDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    /// The subcommand path, e.g. ["remote", "add"] for `git remote add`
    #[serde(default)]
    pub subcommand: Vec<String>,
    #[serde(default)]
    pub args: Vec<ArgDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArgDef {
    pub name: String,
    pub description: String,
    /// The actual flag/option, e.g. "--branch" or "-b"
    pub flag: Option<String>,
    #[serde(default)]
    pub required: bool,
    #[serde(default = "default_arg_type")]
    pub arg_type: ArgType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ArgType {
    String,
    Bool,
    Int,
    Float,
}

fn default_arg_type() -> ArgType {
    ArgType::String
}

impl Manifest {
    pub fn from_yaml(yaml: &str) -> Result<Self, serde_yaml::Error> {
        serde_yaml::from_str(yaml)
    }

    pub fn to_yaml(&self) -> Result<String, serde_yaml::Error> {
        serde_yaml::to_string(self)
    }

    pub fn load(path: &Path) -> Result<Self, ManifestError> {
        let content = std::fs::read_to_string(path)?;
        Ok(Self::from_yaml(&content)?)
    }

    pub fn save(&self, path: &Path) -> Result<(), ManifestError> {
        let yaml = self.to_yaml()?;
        std::fs::write(path, yaml)?;
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("yaml error: {0}")]
    Yaml(#[from] serde_yaml::Error),
}

#[cfg(test)]
mod tests {
    use super::*;
    fn sample_manifest() -> Manifest {
        Manifest {
            binary: "git".to_string(),
            description: "The stupid content tracker".to_string(),
            tools: vec![
                ToolDef {
                    name: "status".to_string(),
                    description: "Show the working tree status".to_string(),
                    subcommand: vec!["status".to_string()],
                    args: vec![ArgDef {
                        name: "short".to_string(),
                        description: "Give output in short format".to_string(),
                        flag: Some("--short".to_string()),
                        required: false,
                        arg_type: ArgType::Bool,
                    }],
                },
                ToolDef {
                    name: "clone_repo".to_string(),
                    description: "Clone a repository".to_string(),
                    subcommand: vec!["clone".to_string()],
                    args: vec![
                        ArgDef {
                            name: "url".to_string(),
                            description: "Repository URL".to_string(),
                            flag: None,
                            required: true,
                            arg_type: ArgType::String,
                        },
                        ArgDef {
                            name: "branch".to_string(),
                            description: "Branch to clone".to_string(),
                            flag: Some("--branch".to_string()),
                            required: false,
                            arg_type: ArgType::String,
                        },
                    ],
                },
            ],
        }
    }

    #[test]
    fn yaml_roundtrip() {
        let manifest = sample_manifest();
        let yaml = manifest.to_yaml().unwrap();
        let parsed = Manifest::from_yaml(&yaml).unwrap();

        assert_eq!(parsed.binary, "git");
        assert_eq!(parsed.tools.len(), 2);
        assert_eq!(parsed.tools[0].name, "status");
        assert_eq!(parsed.tools[1].args.len(), 2);
        assert!(parsed.tools[1].args[0].required);
        assert!(!parsed.tools[1].args[1].required);
    }

    #[test]
    fn yaml_deserialize_minimal() {
        let yaml = r#"
binary: echo
description: Print text
tools:
  - name: print
    description: Print arguments
    args: []
"#;
        let manifest = Manifest::from_yaml(yaml).unwrap();
        assert_eq!(manifest.binary, "echo");
        assert_eq!(manifest.tools.len(), 1);
        assert!(manifest.tools[0].subcommand.is_empty());
    }

    #[test]
    fn yaml_deserialize_defaults() {
        // subcommand and args should default to empty, arg_type to string
        let yaml = r#"
binary: test
description: test tool
tools:
  - name: do_thing
    description: does a thing
    args:
      - name: input
        description: the input
"#;
        let manifest = Manifest::from_yaml(yaml).unwrap();
        let arg = &manifest.tools[0].args[0];
        assert!(!arg.required);
        assert!(matches!(arg.arg_type, ArgType::String));
        assert!(arg.flag.is_none());
    }

    #[test]
    fn save_and_load_roundtrip() {
        let manifest = sample_manifest();
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();

        manifest.save(&path).unwrap();
        let loaded = Manifest::load(&path).unwrap();

        assert_eq!(loaded.binary, manifest.binary);
        assert_eq!(loaded.tools.len(), manifest.tools.len());
        assert_eq!(loaded.tools[0].name, manifest.tools[0].name);
    }

    #[test]
    fn load_nonexistent_file_errors() {
        let result = Manifest::load(Path::new("/nonexistent/path/manifest.yaml"));
        assert!(result.is_err());
    }

    #[test]
    fn from_yaml_invalid_errors() {
        let result = Manifest::from_yaml("{{{{not valid yaml");
        assert!(result.is_err());
    }

    #[test]
    fn arg_types_serialize_lowercase() {
        let yaml = sample_manifest().to_yaml().unwrap();
        assert!(yaml.contains("bool"));
        assert!(yaml.contains("string"));
    }

    // ── Proptests ────────────────────────────────────────────────

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        fn arb_arg_type() -> impl Strategy<Value = ArgType> {
            prop_oneof![
                Just(ArgType::String),
                Just(ArgType::Bool),
                Just(ArgType::Int),
                Just(ArgType::Float),
            ]
        }

        fn arb_arg_def() -> impl Strategy<Value = ArgDef> {
            (
                "[a-z_]{1,20}",
                "[a-zA-Z0-9 ]{1,50}",
                prop::option::of("--[a-z]{1,15}"),
                any::<bool>(),
                arb_arg_type(),
            )
                .prop_map(|(name, description, flag, required, arg_type)| ArgDef {
                    name,
                    description,
                    flag,
                    required,
                    arg_type,
                })
        }

        fn arb_tool_def() -> impl Strategy<Value = ToolDef> {
            (
                "[a-z_]{1,20}",
                "[a-zA-Z0-9 ]{1,50}",
                prop::collection::vec("[a-z]{1,10}", 0..3),
                prop::collection::vec(arb_arg_def(), 0..5),
            )
                .prop_map(|(name, description, subcommand, args)| ToolDef {
                    name,
                    description,
                    subcommand,
                    args,
                })
        }

        fn arb_manifest() -> impl Strategy<Value = Manifest> {
            (
                "[a-z]{1,15}",
                "[a-zA-Z0-9 ]{1,50}",
                prop::collection::vec(arb_tool_def(), 0..10),
            )
                .prop_map(|(binary, description, tools)| Manifest {
                    binary,
                    description,
                    tools,
                })
        }

        proptest! {
            #[test]
            fn yaml_roundtrip_arbitrary(manifest in arb_manifest()) {
                let yaml = manifest.to_yaml().unwrap();
                let parsed = Manifest::from_yaml(&yaml).unwrap();
                prop_assert_eq!(parsed.binary, manifest.binary);
                prop_assert_eq!(parsed.tools.len(), manifest.tools.len());
                for (orig, parsed) in manifest.tools.iter().zip(parsed.tools.iter()) {
                    prop_assert_eq!(&orig.name, &parsed.name);
                    prop_assert_eq!(orig.args.len(), parsed.args.len());
                }
            }

            #[test]
            fn save_load_roundtrip_arbitrary(manifest in arb_manifest()) {
                let tmp = tempfile::NamedTempFile::new().unwrap();
                manifest.save(tmp.path()).unwrap();
                let loaded = Manifest::load(tmp.path()).unwrap();
                prop_assert_eq!(loaded.binary, manifest.binary);
                prop_assert_eq!(loaded.tools.len(), manifest.tools.len());
            }
        }
    }
}
