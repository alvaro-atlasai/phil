use std::process::Command;

use phil_core::{CompletionParams, InferenceError, ModelManager, PhilInference};

use crate::manifest::{Manifest, ToolDef};

#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("failed to run `{binary} --help`: {reason}")]
    HelpFailed { binary: String, reason: String },
    #[error("model error: {0}")]
    Model(#[from] phil_core::ModelError),
    #[error("inference error: {0}")]
    Inference(#[from] InferenceError),
    #[error("json parse error: {0}")]
    Json(String),
}

/// Run `<binary> --help` and optionally recurse into subcommands.
fn capture_help(binary: &str, subcommand: &[&str]) -> Result<String, ParseError> {
    let mut cmd = Command::new(binary);
    for sub in subcommand {
        cmd.arg(sub);
    }
    cmd.arg("--help");

    let output = cmd.output().map_err(|e| ParseError::HelpFailed {
        binary: binary.to_string(),
        reason: e.to_string(),
    })?;

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    // Some tools print help to stderr
    let text = if stdout.trim().is_empty() {
        stderr.to_string()
    } else {
        stdout.to_string()
    };

    if text.trim().is_empty() {
        return Err(ParseError::HelpFailed {
            binary: binary.to_string(),
            reason: "no help output produced".to_string(),
        });
    }

    Ok(text)
}

/// Extract subcommand names from the top-level help text.
pub(crate) fn extract_subcommand_names(help_text: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_commands = false;

    for line in help_text.lines() {
        let trimmed = line.trim();

        // Detect "Commands:", "COMMANDS:", "Available commands:", "SUBCOMMANDS:" sections
        if trimmed.ends_with(':')
            && (trimmed.to_lowercase().contains("command")
                || trimmed.to_lowercase().contains("subcommand"))
        {
            in_commands = true;
            continue;
        }

        // End of commands section on blank line or new section header
        if in_commands {
            if trimmed.is_empty() {
                in_commands = false;
                continue;
            }
            // New section header
            if trimmed.ends_with(':') && !trimmed.starts_with(' ') {
                in_commands = false;
                continue;
            }
            // Extract the first word as the subcommand name
            if let Some(name) = trimmed.split_whitespace().next() {
                // Skip `help` subcommand and anything starting with `-`
                if name != "help" && !name.starts_with('-') {
                    names.push(name.to_string());
                }
            }
        }
    }

    names
}

const PARSER_SYSTEM_PROMPT: &str = r#"You are a CLI analysis tool. Given a CLI tool's --help output, extract the tool's capabilities as structured JSON.

For each command or subcommand, output a JSON object with these fields:
- "name": a short snake_case tool name (e.g. "list_branches", "add_remote")
- "description": a one-sentence description of what the command does
- "subcommand": array of subcommand words (e.g. ["remote", "add"])
- "args": array of argument objects, each with:
  - "name": parameter name (snake_case)
  - "description": what the argument does
  - "flag": the CLI flag (e.g. "--branch" or "-b"), null for positional args
  - "required": boolean
  - "arg_type": one of "string", "bool", "int", "float"

Return a JSON array of tool objects. Only include the most useful/common commands, not every possible flag.
Focus on the 10-15 most important commands and their key arguments."#;

/// Parse a binary's --help output using Phi-4-mini to generate a manifest.
pub async fn parse_binary(binary: &str) -> Result<Manifest, ParseError> {
    eprintln!("Analyzing `{binary}`...");

    // Capture top-level help
    let top_help = capture_help(binary, &[])?;

    // Look for subcommands
    let subcommands = extract_subcommand_names(&top_help);

    // Collect help texts
    let mut all_help = format!("=== {binary} --help ===\n{top_help}\n");

    // Limit to first 15 subcommands to avoid overwhelming the model
    for sub in subcommands.iter().take(15) {
        if let Ok(sub_help) = capture_help(binary, &[sub]) {
            // Truncate long help texts
            let truncated: String = sub_help.chars().take(2000).collect();
            all_help.push_str(&format!("\n=== {binary} {sub} --help ===\n{truncated}\n"));
        }
    }

    // Truncate total to fit in context
    let all_help: String = all_help.chars().take(12000).collect();

    // Load model and run inference
    eprintln!("Loading model for analysis...");
    let mgr = ModelManager::new()?;
    let model_path = mgr.ensure_model().await?;
    let inference = PhilInference::load(&model_path)?;

    let user_input = format!(
        "Analyze this CLI tool and extract its commands as JSON:\n\n{all_help}"
    );

    let schema = serde_json::json!({
        "type": "array",
        "items": {
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "description": {"type": "string"},
                "subcommand": {"type": "array", "items": {"type": "string"}},
                "args": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "name": {"type": "string"},
                            "description": {"type": "string"},
                            "flag": {"type": ["string", "null"]},
                            "required": {"type": "boolean"},
                            "arg_type": {"type": "string", "enum": ["string", "bool", "int", "float"]}
                        }
                    }
                }
            }
        }
    });

    eprintln!("Generating tool manifest...");
    let params = CompletionParams {
        max_tokens: 4096,
        temperature: 0.1,
        ..Default::default()
    };

    let tools_json = inference.complete_json(
        PARSER_SYSTEM_PROMPT,
        &user_input,
        &schema,
        &params,
    )?;

    // Parse the JSON into ToolDefs
    let tools: Vec<ToolDef> = serde_json::from_value(tools_json)
        .map_err(|e| ParseError::Json(e.to_string()))?;

    // Build description from top-level help (first non-empty line)
    let description = top_help
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("CLI tool")
        .trim()
        .to_string();

    Ok(Manifest {
        binary: binary.to_string(),
        description,
        tools,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── extract_subcommand_names ─────────────────────────────────

    #[test]
    fn extracts_commands_section() {
        let help = r#"Usage: mytool [OPTIONS] [COMMAND]

Commands:
  init      Initialize a new project
  build     Build the project
  test      Run tests
  help      Print this message

Options:
  -h, --help  Print help
"#;
        let names = extract_subcommand_names(help);
        assert_eq!(names, vec!["init", "build", "test"]);
    }

    #[test]
    fn extracts_uppercase_commands() {
        let help = r#"COMMANDS:
  deploy    Deploy to production
  rollback  Rollback deployment
"#;
        let names = extract_subcommand_names(help);
        assert_eq!(names, vec!["deploy", "rollback"]);
    }

    #[test]
    fn extracts_subcommands_section() {
        let help = r#"SUBCOMMANDS:
  add       Add a dependency
  remove    Remove a dependency
"#;
        let names = extract_subcommand_names(help);
        assert_eq!(names, vec!["add", "remove"]);
    }

    #[test]
    fn extracts_available_commands() {
        let help = r#"Available commands:
  start     Start the server
  stop      Stop the server
"#;
        let names = extract_subcommand_names(help);
        assert_eq!(names, vec!["start", "stop"]);
    }

    #[test]
    fn stops_at_blank_line() {
        let help = r#"Commands:
  first     First command
  second    Second command

Options:
  --verbose  Be verbose
"#;
        let names = extract_subcommand_names(help);
        assert_eq!(names, vec!["first", "second"]);
    }

    #[test]
    fn stops_at_new_section() {
        let help = r#"Commands:
  alpha     Alpha command
  beta      Beta command
Options:
  --help    Show help
"#;
        let names = extract_subcommand_names(help);
        assert_eq!(names, vec!["alpha", "beta"]);
    }

    #[test]
    fn skips_flags() {
        let help = r#"Commands:
  run       Run something
  --flag    Not a command
  build     Build something
"#;
        let names = extract_subcommand_names(help);
        assert_eq!(names, vec!["run", "build"]);
    }

    #[test]
    fn skips_help_command() {
        let help = r#"Commands:
  deploy    Deploy
  help      Show help
"#;
        let names = extract_subcommand_names(help);
        assert_eq!(names, vec!["deploy"]);
    }

    #[test]
    fn empty_help_returns_empty() {
        assert!(extract_subcommand_names("").is_empty());
        assert!(extract_subcommand_names("Usage: tool [OPTIONS]").is_empty());
    }

    #[test]
    fn no_commands_section_returns_empty() {
        let help = r#"mytool v1.0.0

Options:
  --help     Show help
  --version  Show version
"#;
        assert!(extract_subcommand_names(help).is_empty());
    }

    // ── capture_help ─────────────────────────────────────────────

    #[test]
    fn capture_help_nonexistent_binary() {
        let result = capture_help("nonexistent_binary_xyz_42", &[]);
        assert!(result.is_err());
    }

    #[test]
    fn capture_help_real_binary() {
        // `echo` should exist on all systems
        let result = capture_help("echo", &[]);
        // echo --help either works or prints "--help"
        assert!(result.is_ok());
    }

    // ── Proptests ────────────────────────────────────────────────

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn extract_subcommands_never_panics(s in "\\PC{0,500}") {
                let _ = extract_subcommand_names(&s);
            }

            #[test]
            fn extract_subcommands_never_returns_help(s in "\\PC{0,500}") {
                let names = extract_subcommand_names(&s);
                for name in &names {
                    prop_assert_ne!(name, "help");
                }
            }

            #[test]
            fn extract_subcommands_never_returns_flags(s in "\\PC{0,500}") {
                let names = extract_subcommand_names(&s);
                for name in &names {
                    prop_assert!(!name.starts_with('-'));
                }
            }

            #[test]
            fn commands_section_extracts_first_words(
                cmds in prop::collection::vec("[a-z]{2,10}    [a-zA-Z ]{5,30}", 1..8)
            ) {
                let help = format!("Commands:\n{}\n", cmds.join("\n"));
                let names = extract_subcommand_names(&help);
                // Each command line's first word should appear (except "help")
                for cmd_line in &cmds {
                    let first_word = cmd_line.split_whitespace().next().unwrap();
                    if first_word != "help" && !first_word.starts_with('-') {
                        prop_assert!(
                            names.contains(&first_word.to_string()),
                            "Expected {:?} in {:?}",
                            first_word,
                            names
                        );
                    }
                }
            }
        }
    }
}
