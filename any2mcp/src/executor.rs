use std::process::Command;

use crate::manifest::{ArgType, ToolDef};

#[derive(Debug, thiserror::Error)]
pub enum ExecError {
    #[error("tool `{name}` not found in manifest")]
    ToolNotFound { name: String },
    #[error("missing required argument `{arg}`")]
    MissingArg { arg: String },
    #[error("command execution failed: {0}")]
    Exec(std::io::Error),
    #[error("invalid argument value for `{name}`: {reason}")]
    InvalidArg { name: String, reason: String },
}

/// Execute a tool from the manifest with the given arguments.
/// Returns (stdout, stderr, exit_code).
pub fn execute_tool(
    binary: &str,
    tool: &ToolDef,
    args: &serde_json::Map<String, serde_json::Value>,
) -> Result<(String, String, i32), ExecError> {
    let mut cmd = Command::new(binary);

    // Add subcommand parts
    for sub in &tool.subcommand {
        cmd.arg(sub);
    }

    // Process arguments
    for arg_def in &tool.args {
        let value = args.get(&arg_def.name);

        // Check required args
        if arg_def.required && value.is_none() {
            return Err(ExecError::MissingArg {
                arg: arg_def.name.clone(),
            });
        }

        let Some(value) = value else { continue };

        match &arg_def.flag {
            Some(flag) => {
                // Flag-based argument
                match arg_def.arg_type {
                    ArgType::Bool => {
                        if value.as_bool().unwrap_or(false) {
                            cmd.arg(flag);
                        }
                    }
                    ArgType::String | ArgType::Int | ArgType::Float => {
                        let val_str = value_to_string(value, &arg_def.name)?;
                        cmd.arg(flag);
                        cmd.arg(&val_str);
                    }
                }
            }
            None => {
                // Positional argument
                let val_str = value_to_string(value, &arg_def.name)?;
                cmd.arg(&val_str);
            }
        }
    }

    let output = cmd.output().map_err(ExecError::Exec)?;

    let stdout = String::from_utf8_lossy(&output.stdout).to_string();
    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let exit_code = output.status.code().unwrap_or(-1);

    Ok((stdout, stderr, exit_code))
}

fn value_to_string(value: &serde_json::Value, name: &str) -> Result<String, ExecError> {
    match value {
        serde_json::Value::String(s) => {
            // Reject values that could be shell injection
            validate_arg_value(s, name)?;
            Ok(s.clone())
        }
        serde_json::Value::Number(n) => Ok(n.to_string()),
        serde_json::Value::Bool(b) => Ok(b.to_string()),
        _ => Err(ExecError::InvalidArg {
            name: name.to_string(),
            reason: "unsupported value type".to_string(),
        }),
    }
}

/// Validate argument values to prevent command injection.
/// Since we use Command::arg (not shell), the main risk is mitigated,
/// but we still reject suspicious patterns.
fn validate_arg_value(value: &str, name: &str) -> Result<(), ExecError> {
    // Reject null bytes
    if value.contains('\0') {
        return Err(ExecError::InvalidArg {
            name: name.to_string(),
            reason: "value contains null bytes".to_string(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ArgDef, ArgType};
    use serde_json::json;

    fn echo_tool(args: Vec<ArgDef>) -> ToolDef {
        ToolDef {
            name: "test_echo".to_string(),
            description: "test".to_string(),
            subcommand: vec![],
            args,
        }
    }

    // ── execute_tool ─────────────────────────────────────────────

    #[test]
    fn execute_simple_positional_arg() {
        let tool = echo_tool(vec![ArgDef {
            name: "message".to_string(),
            description: "text".to_string(),
            flag: None,
            required: true,
            arg_type: ArgType::String,
        }]);
        let mut args = serde_json::Map::new();
        args.insert("message".to_string(), json!("hello world"));

        let (stdout, _, code) = execute_tool("echo", &tool, &args).unwrap();
        assert_eq!(code, 0);
        assert_eq!(stdout.trim(), "hello world");
    }

    #[test]
    fn execute_with_subcommand() {
        // `echo` ignores subcommands, so "sub hello" → "sub hello"
        let tool = ToolDef {
            name: "test".to_string(),
            description: "test".to_string(),
            subcommand: vec!["sub".to_string()],
            args: vec![ArgDef {
                name: "msg".to_string(),
                description: "text".to_string(),
                flag: None,
                required: false,
                arg_type: ArgType::String,
            }],
        };
        let mut args = serde_json::Map::new();
        args.insert("msg".to_string(), json!("hello"));

        let (stdout, _, _) = execute_tool("echo", &tool, &args).unwrap();
        assert_eq!(stdout.trim(), "sub hello");
    }

    #[test]
    fn execute_bool_flag_true() {
        let tool = echo_tool(vec![ArgDef {
            name: "verbose".to_string(),
            description: "be verbose".to_string(),
            flag: Some("--verbose".to_string()),
            required: false,
            arg_type: ArgType::Bool,
        }]);
        let mut args = serde_json::Map::new();
        args.insert("verbose".to_string(), json!(true));

        let (stdout, _, _) = execute_tool("echo", &tool, &args).unwrap();
        assert_eq!(stdout.trim(), "--verbose");
    }

    #[test]
    fn execute_bool_flag_false_omitted() {
        let tool = echo_tool(vec![ArgDef {
            name: "verbose".to_string(),
            description: "be verbose".to_string(),
            flag: Some("--verbose".to_string()),
            required: false,
            arg_type: ArgType::Bool,
        }]);
        let mut args = serde_json::Map::new();
        args.insert("verbose".to_string(), json!(false));

        let (stdout, _, _) = execute_tool("echo", &tool, &args).unwrap();
        assert_eq!(stdout.trim(), "");
    }

    #[test]
    fn execute_string_flag() {
        let tool = echo_tool(vec![ArgDef {
            name: "name".to_string(),
            description: "name".to_string(),
            flag: Some("--name".to_string()),
            required: false,
            arg_type: ArgType::String,
        }]);
        let mut args = serde_json::Map::new();
        args.insert("name".to_string(), json!("Alice"));

        let (stdout, _, _) = execute_tool("echo", &tool, &args).unwrap();
        assert_eq!(stdout.trim(), "--name Alice");
    }

    #[test]
    fn execute_missing_required_arg_errors() {
        let tool = echo_tool(vec![ArgDef {
            name: "input".to_string(),
            description: "required".to_string(),
            flag: None,
            required: true,
            arg_type: ArgType::String,
        }]);
        let args = serde_json::Map::new();

        let result = execute_tool("echo", &tool, &args);
        assert!(result.is_err());
        let err = result.unwrap_err().to_string();
        assert!(err.contains("missing required argument"));
    }

    #[test]
    fn execute_optional_arg_omitted() {
        let tool = echo_tool(vec![ArgDef {
            name: "opt".to_string(),
            description: "optional".to_string(),
            flag: Some("--opt".to_string()),
            required: false,
            arg_type: ArgType::String,
        }]);
        let args = serde_json::Map::new();

        let (stdout, _, code) = execute_tool("echo", &tool, &args).unwrap();
        assert_eq!(code, 0);
        assert_eq!(stdout.trim(), "");
    }

    #[test]
    fn execute_nonexistent_binary_errors() {
        let tool = echo_tool(vec![]);
        let args = serde_json::Map::new();

        let result = execute_tool("nonexistent_binary_xyz_42", &tool, &args);
        assert!(result.is_err());
    }

    #[test]
    fn execute_number_arg() {
        let tool = echo_tool(vec![ArgDef {
            name: "count".to_string(),
            description: "count".to_string(),
            flag: Some("--count".to_string()),
            required: false,
            arg_type: ArgType::Int,
        }]);
        let mut args = serde_json::Map::new();
        args.insert("count".to_string(), json!(42));

        let (stdout, _, _) = execute_tool("echo", &tool, &args).unwrap();
        assert_eq!(stdout.trim(), "--count 42");
    }

    // ── validate_arg_value ───────────────────────────────────────

    #[test]
    fn validate_rejects_null_bytes() {
        let result = validate_arg_value("hello\0world", "test");
        assert!(result.is_err());
    }

    #[test]
    fn validate_allows_normal_strings() {
        assert!(validate_arg_value("hello world", "test").is_ok());
        assert!(validate_arg_value("--flag", "test").is_ok());
        assert!(validate_arg_value("path/to/file", "test").is_ok());
        assert!(validate_arg_value("", "test").is_ok());
    }

    // ── value_to_string ──────────────────────────────────────────

    #[test]
    fn value_to_string_types() {
        assert_eq!(value_to_string(&json!("hello"), "t").unwrap(), "hello");
        assert_eq!(value_to_string(&json!(42), "t").unwrap(), "42");
        assert_eq!(value_to_string(&json!(3.14), "t").unwrap(), "3.14");
        assert_eq!(value_to_string(&json!(true), "t").unwrap(), "true");
    }

    #[test]
    fn value_to_string_rejects_object() {
        let result = value_to_string(&json!({"a": 1}), "test");
        assert!(result.is_err());
    }

    #[test]
    fn value_to_string_rejects_array() {
        let result = value_to_string(&json!([1, 2]), "test");
        assert!(result.is_err());
    }

    #[test]
    fn value_to_string_rejects_null() {
        let result = value_to_string(&json!(null), "test");
        assert!(result.is_err());
    }

    // ── Proptests ────────────────────────────────────────────────

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn validate_never_panics(s in "\\PC{0,200}") {
                let _ = validate_arg_value(&s, "prop");
            }

            #[test]
            fn validate_accepts_no_null_bytes(s in "[^\x00]{0,200}") {
                prop_assert!(validate_arg_value(&s, "prop").is_ok());
            }

            #[test]
            fn validate_rejects_strings_with_null(
                prefix in "[^\x00]{0,50}",
                suffix in "[^\x00]{0,50}"
            ) {
                let with_null = format!("{prefix}\0{suffix}");
                prop_assert!(validate_arg_value(&with_null, "prop").is_err());
            }

            #[test]
            fn echo_roundtrip_string_arg(s in "[a-zA-Z0-9 _-]{1,50}") {
                let tool = echo_tool(vec![ArgDef {
                    name: "msg".to_string(),
                    description: "text".to_string(),
                    flag: None,
                    required: true,
                    arg_type: ArgType::String,
                }]);
                let mut args = serde_json::Map::new();
                args.insert("msg".to_string(), json!(s.clone()));

                let (stdout, _, code) = execute_tool("echo", &tool, &args).unwrap();
                prop_assert_eq!(code, 0);
                prop_assert_eq!(stdout.trim(), s.trim());
            }
        }
    }
}
