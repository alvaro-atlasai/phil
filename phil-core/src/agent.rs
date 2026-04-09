//! Agentic tool-calling loop for phil.
//!
//! Packs become tools that the model can call autonomously. Uses Phi-4-mini's
//! native `<|tool|>...<|/tool|>` format for function calling.

use serde::{Deserialize, Serialize};
use std::io::Write;

use crate::inference::{CompletionParams, InferenceError, PhilInference};
use crate::pack;

/// A tool definition exposed to the model.
#[derive(Debug, Clone, Serialize)]
pub struct ToolDef {
    pub name: String,
    pub description: String,
    pub parameters: serde_json::Value,
}

/// A parsed tool call from the model's output.
#[derive(Debug, Clone, Deserialize)]
pub struct ToolCall {
    pub name: String,
    #[serde(default)]
    pub arguments: serde_json::Value,
}

/// Result of one agent step.
#[derive(Debug)]
pub enum AgentStep {
    /// Model produced a final text response.
    Done(String),
    /// Model requested tool calls.
    ToolCalls(Vec<ToolCall>),
}

/// Maximum number of tool-calling rounds before we force a final answer.
const MAX_ROUNDS: usize = 5;

/// Convert available packs into tool definitions for the model.
pub fn packs_as_tools() -> Vec<ToolDef> {
    let metas = pack::list_packs_meta().unwrap_or_default();
    metas
        .iter()
        .map(|m| ToolDef {
            name: m.name.clone(),
            description: m.description.clone(),
            parameters: serde_json::json!({
                "input": {
                    "description": "The text input to process with this pack",
                    "type": "str"
                }
            }),
        })
        .collect()
}

/// Format tool definitions as JSON for Phi-4's `<|tool|>...<|/tool|>` block.
fn format_tools_json(tools: &[ToolDef]) -> String {
    let defs: Vec<serde_json::Value> = tools
        .iter()
        .map(|t| {
            serde_json::json!({
                "name": t.name,
                "description": t.description,
                "parameters": t.parameters,
            })
        })
        .collect();
    serde_json::to_string(&defs).unwrap_or_else(|_| "[]".into())
}

/// Execute a tool call by running the corresponding pack.
/// Returns the pack's output text.
pub fn execute_tool_call(call: &ToolCall, stderr: &mut impl Write) -> Result<String, String> {
    let input = match &call.arguments {
        serde_json::Value::Object(map) => {
            map.get("input")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string()
        }
        serde_json::Value::String(s) => s.clone(),
        _ => String::new(),
    };

    let _ = writeln!(stderr, "  → @{} {:?}", call.name, truncate(&input, 60));

    // Load the pack to verify it exists
    let _pack = pack::load_pack(&call.name)
        .map_err(|e| format!("pack @{} not found: {e}", call.name))?;

    // Execute via the appropriate backend — for now, we return the pack's
    // system prompt applied to the input. The actual execution happens in the
    // caller (CLI layer) which has access to the model/daemon/cloud backends.
    // We return a marker that the CLI layer intercepts.
    Ok(format!("__PACK_EXEC__:{}:{}", call.name, input))
}

/// Parse the model's output to detect tool calls vs final text.
pub fn parse_model_output(output: &str) -> AgentStep {
    let trimmed = output.trim();

    // Phi-4-mini outputs tool calls as: <|tool_call|>{"name":"...","arguments":{...}}
    // or sometimes as JSON: {"name": "...", "arguments": {...}}
    if let Some(rest) = trimmed.strip_prefix("<|tool_call|>") {
        if let Some(calls) = parse_tool_calls(rest) {
            return AgentStep::ToolCalls(calls);
        }
    }

    // Also try parsing if the model outputs JSON directly with a "name" field
    // that matches a known pack
    if trimmed.starts_with('{') || trimmed.starts_with('[') {
        if let Some(calls) = parse_tool_calls(trimmed) {
            return AgentStep::ToolCalls(calls);
        }
    }

    AgentStep::Done(trimmed.to_string())
}

/// Try to parse one or more tool calls from a string.
fn parse_tool_calls(text: &str) -> Option<Vec<ToolCall>> {
    let text = text.trim();

    // Try as a single tool call object
    if let Ok(call) = serde_json::from_str::<ToolCall>(text) {
        if !call.name.is_empty() {
            return Some(vec![call]);
        }
    }

    // Try as an array of tool calls
    if let Ok(calls) = serde_json::from_str::<Vec<ToolCall>>(text) {
        if !calls.is_empty() && calls.iter().all(|c| !c.name.is_empty()) {
            return Some(calls);
        }
    }

    // Try to extract JSON from the text (model might add prose around it)
    let json_str = crate::inference::extract_json(text);
    if json_str != text {
        if let Ok(call) = serde_json::from_str::<ToolCall>(json_str) {
            if !call.name.is_empty() {
                return Some(vec![call]);
            }
        }
    }

    None
}

/// Build the initial prompt with tools for the agentic loop.
pub fn build_agent_prompt(system: &str, tools: &[ToolDef], user_input: &str) -> String {
    let tools_json = format_tools_json(tools);
    format!(
        "<|system|>{system}\n\
         You have access to the following tools. Call a tool when it would help answer the user's request. \
         When calling a tool, output ONLY a JSON object like: {{\"name\": \"tool_name\", \"arguments\": {{\"input\": \"...\"}}}}\n\
         When you have the final answer (after getting tool results or if no tool is needed), respond with plain text.\
         <|tool|>{tools_json}<|/tool|><|end|>\n\
         <|user|>{user_input}<|end|>\n\
         <|assistant|>"
    )
}

/// Append a tool result to an existing conversation prompt and ask for next step.
pub fn append_tool_result(prompt: &mut String, tool_name: &str, result: &str) {
    // Remove trailing <|assistant|> to continue the conversation
    // The model's previous output (tool call) is implicitly part of the context
    prompt.push_str(&format!(
        "<|end|>\n<|tool_response|>\nname: {tool_name}\n{result}\n<|end|>\n<|assistant|>"
    ));
}

/// Run the full agentic loop: model calls tools, we execute them, feed results back.
///
/// `execute_fn` is called for each tool call and should return the tool's output text.
/// This allows the CLI layer to route execution through the daemon, cloud, or Apple backend.
pub fn run_agent_loop<F, W>(
    inference: &PhilInference,
    system: &str,
    user_input: &str,
    params: &CompletionParams,
    tools: &[ToolDef],
    mut execute_fn: F,
    mut stderr: W,
) -> Result<String, InferenceError>
where
    F: FnMut(&ToolCall) -> Result<String, String>,
    W: Write,
{
    let mut prompt = build_agent_prompt(system, tools, user_input);

    for round in 0..MAX_ROUNDS {
        let output = inference.complete_raw(&prompt, params, std::io::sink())?;

        match parse_model_output(&output) {
            AgentStep::Done(text) => return Ok(text),
            AgentStep::ToolCalls(calls) => {
                for call in &calls {
                    let _ = writeln!(stderr, "  [{}/{}] calling @{}...", round + 1, MAX_ROUNDS, call.name);
                    match execute_fn(call) {
                        Ok(result) => {
                            let display = truncate(&result, 200);
                            let _ = writeln!(stderr, "  ← {display}");
                            append_tool_result(&mut prompt, &call.name, &result);
                        }
                        Err(e) => {
                            let _ = writeln!(stderr, "  ✗ {e}");
                            append_tool_result(&mut prompt, &call.name, &format!("Error: {e}"));
                        }
                    }
                }
            }
        }
    }

    // If we exhaust rounds, force a final answer
    prompt.push_str("\n[You have used all available tool calls. Provide your final answer now based on the information gathered.]\n");
    inference.complete_raw(&prompt, params, std::io::sink())
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        s.replace('\n', " ")
    } else {
        format!("{}...", s[..max].replace('\n', " "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tool_call_with_prefix() {
        let output = r#"<|tool_call|>{"name": "commit", "arguments": {"input": "feat: add tool calling"}}"#;
        match parse_model_output(output) {
            AgentStep::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "commit");
            }
            _ => panic!("expected tool call"),
        }
    }

    #[test]
    fn parse_tool_call_bare_json() {
        let output = r#"{"name": "json", "arguments": {"input": "name,age\nAlice,30"}}"#;
        match parse_model_output(output) {
            AgentStep::ToolCalls(calls) => {
                assert_eq!(calls.len(), 1);
                assert_eq!(calls[0].name, "json");
            }
            _ => panic!("expected tool call"),
        }
    }

    #[test]
    fn parse_plain_text() {
        let output = "The capital of France is Paris.";
        match parse_model_output(output) {
            AgentStep::Done(text) => assert_eq!(text, "The capital of France is Paris."),
            _ => panic!("expected done"),
        }
    }

    #[test]
    fn packs_as_tools_not_empty() {
        let tools = packs_as_tools();
        assert!(!tools.is_empty());
        // Every tool should have a name and description
        for t in &tools {
            assert!(!t.name.is_empty());
            assert!(!t.description.is_empty());
        }
    }

    #[test]
    fn build_prompt_contains_tools() {
        let tools = vec![ToolDef {
            name: "test".into(),
            description: "A test tool".into(),
            parameters: serde_json::json!({"input": {"type": "str"}}),
        }];
        let prompt = build_agent_prompt("You are helpful.", &tools, "hello");
        assert!(prompt.contains("<|tool|>"));
        assert!(prompt.contains("<|/tool|>"));
        assert!(prompt.contains("\"test\""));
    }
}
