use std::io::{self, BufRead, Write};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::executor;
use crate::manifest::Manifest;

#[derive(Debug, Serialize, Deserialize)]
struct JsonRpcRequest {
    jsonrpc: String,
    id: Option<Value>,
    method: String,
    #[serde(default)]
    params: Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcResponse {
    jsonrpc: String,
    id: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<JsonRpcError>,
}

#[derive(Debug, Serialize)]
struct JsonRpcError {
    code: i32,
    message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<Value>,
}

impl JsonRpcResponse {
    fn success(id: Value, result: Value) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: Some(result),
            error: None,
        }
    }

    fn error(id: Value, code: i32, message: String) -> Self {
        Self {
            jsonrpc: "2.0".to_string(),
            id,
            result: None,
            error: Some(JsonRpcError {
                code,
                message,
                data: None,
            }),
        }
    }
}

/// Run the MCP stdio server with the given manifest.
pub fn serve(manifest: &Manifest) -> Result<(), io::Error> {
    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut stdout = stdout.lock();

    eprintln!(
        "MCP server started for `{}` ({} tools)",
        manifest.binary,
        manifest.tools.len()
    );

    for line in stdin.lock().lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }

        let request: JsonRpcRequest = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                let resp = JsonRpcResponse::error(
                    Value::Null,
                    -32700,
                    format!("Parse error: {e}"),
                );
                write_response(&mut stdout, &resp)?;
                continue;
            }
        };

        let id = request.id.clone().unwrap_or(Value::Null);

        let response = match request.method.as_str() {
            "initialize" => handle_initialize(id, manifest),
            "initialized" => continue, // notification, no response
            "tools/list" => handle_tools_list(id, manifest),
            "tools/call" => handle_tools_call(id, manifest, &request.params),
            _ => JsonRpcResponse::error(id, -32601, format!("Method not found: {}", request.method)),
        };

        write_response(&mut stdout, &response)?;
    }

    Ok(())
}

fn write_response(writer: &mut impl Write, response: &JsonRpcResponse) -> io::Result<()> {
    let json = serde_json::to_string(response).unwrap();
    writeln!(writer, "{json}")?;
    writer.flush()
}

fn handle_initialize(id: Value, manifest: &Manifest) -> JsonRpcResponse {
    JsonRpcResponse::success(
        id,
        serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {
                "tools": {}
            },
            "serverInfo": {
                "name": format!("any2mcp-{}", manifest.binary),
                "version": env!("CARGO_PKG_VERSION")
            }
        }),
    )
}

fn handle_tools_list(id: Value, manifest: &Manifest) -> JsonRpcResponse {
    let tools: Vec<Value> = manifest
        .tools
        .iter()
        .map(|tool| {
            let properties: serde_json::Map<String, Value> = tool
                .args
                .iter()
                .map(|arg| {
                    let schema = match arg.arg_type {
                        crate::manifest::ArgType::String => serde_json::json!({
                            "type": "string",
                            "description": arg.description
                        }),
                        crate::manifest::ArgType::Bool => serde_json::json!({
                            "type": "boolean",
                            "description": arg.description
                        }),
                        crate::manifest::ArgType::Int => serde_json::json!({
                            "type": "integer",
                            "description": arg.description
                        }),
                        crate::manifest::ArgType::Float => serde_json::json!({
                            "type": "number",
                            "description": arg.description
                        }),
                    };
                    (arg.name.clone(), schema)
                })
                .collect();

            let required: Vec<String> = tool
                .args
                .iter()
                .filter(|a| a.required)
                .map(|a| a.name.clone())
                .collect();

            serde_json::json!({
                "name": tool.name,
                "description": tool.description,
                "inputSchema": {
                    "type": "object",
                    "properties": properties,
                    "required": required
                }
            })
        })
        .collect();

    JsonRpcResponse::success(id, serde_json::json!({ "tools": tools }))
}

fn handle_tools_call(id: Value, manifest: &Manifest, params: &Value) -> JsonRpcResponse {
    let tool_name = match params.get("name").and_then(|v| v.as_str()) {
        Some(n) => n,
        None => {
            return JsonRpcResponse::error(id, -32602, "Missing tool name".to_string());
        }
    };

    let tool = match manifest.tools.iter().find(|t| t.name == tool_name) {
        Some(t) => t,
        None => {
            return JsonRpcResponse::error(
                id,
                -32602,
                format!("Unknown tool: {tool_name}"),
            );
        }
    };

    let arguments = params
        .get("arguments")
        .and_then(|v| v.as_object())
        .cloned()
        .unwrap_or_default();

    match executor::execute_tool(&manifest.binary, tool, &arguments) {
        Ok((stdout, stderr, exit_code)) => {
            let mut content = Vec::new();

            if !stdout.is_empty() {
                content.push(serde_json::json!({
                    "type": "text",
                    "text": stdout
                }));
            }
            if !stderr.is_empty() {
                content.push(serde_json::json!({
                    "type": "text",
                    "text": format!("[stderr] {stderr}")
                }));
            }

            JsonRpcResponse::success(
                id,
                serde_json::json!({
                    "content": content,
                    "isError": exit_code != 0
                }),
            )
        }
        Err(e) => JsonRpcResponse::success(
            id,
            serde_json::json!({
                "content": [{
                    "type": "text",
                    "text": format!("Error: {e}")
                }],
                "isError": true
            }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::{ArgDef, ArgType, Manifest, ToolDef};

    fn test_manifest() -> Manifest {
        Manifest {
            binary: "echo".to_string(),
            description: "Test tool".to_string(),
            tools: vec![
                ToolDef {
                    name: "say_hello".to_string(),
                    description: "Prints a greeting".to_string(),
                    subcommand: vec![],
                    args: vec![ArgDef {
                        name: "message".to_string(),
                        description: "The message".to_string(),
                        flag: None,
                        required: true,
                        arg_type: ArgType::String,
                    }],
                },
                ToolDef {
                    name: "say_loud".to_string(),
                    description: "Prints loudly".to_string(),
                    subcommand: vec![],
                    args: vec![
                        ArgDef {
                            name: "text".to_string(),
                            description: "text".to_string(),
                            flag: None,
                            required: true,
                            arg_type: ArgType::String,
                        },
                        ArgDef {
                            name: "uppercase".to_string(),
                            description: "uppercase flag".to_string(),
                            flag: Some("--upper".to_string()),
                            required: false,
                            arg_type: ArgType::Bool,
                        },
                    ],
                },
            ],
        }
    }

    // ── handle_initialize ────────────────────────────────────────

    #[test]
    fn initialize_returns_capabilities() {
        let manifest = test_manifest();
        let resp = handle_initialize(Value::Number(1.into()), &manifest);
        let result = resp.result.unwrap();

        assert_eq!(result["protocolVersion"], "2024-11-05");
        assert!(result["capabilities"]["tools"].is_object());
        assert!(result["serverInfo"]["name"]
            .as_str()
            .unwrap()
            .contains("echo"));
    }

    #[test]
    fn initialize_preserves_id() {
        let manifest = test_manifest();
        let resp = handle_initialize(Value::String("abc".to_string()), &manifest);
        assert_eq!(resp.id, Value::String("abc".to_string()));
    }

    // ── handle_tools_list ────────────────────────────────────────

    #[test]
    fn tools_list_returns_all_tools() {
        let manifest = test_manifest();
        let resp = handle_tools_list(Value::Number(2.into()), &manifest);
        let result = resp.result.unwrap();
        let tools = result["tools"].as_array().unwrap();

        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["name"], "say_hello");
        assert_eq!(tools[1]["name"], "say_loud");
    }

    #[test]
    fn tools_list_has_input_schema() {
        let manifest = test_manifest();
        let resp = handle_tools_list(Value::Number(1.into()), &manifest);
        let result = resp.result.unwrap();
        let tool = &result["tools"][0];

        let schema = &tool["inputSchema"];
        assert_eq!(schema["type"], "object");
        assert!(schema["properties"]["message"].is_object());
        assert_eq!(schema["properties"]["message"]["type"], "string");
    }

    #[test]
    fn tools_list_required_fields() {
        let manifest = test_manifest();
        let resp = handle_tools_list(Value::Number(1.into()), &manifest);
        let result = resp.result.unwrap();

        let required = result["tools"][0]["inputSchema"]["required"]
            .as_array()
            .unwrap();
        assert_eq!(required, &[Value::String("message".to_string())]);
    }

    #[test]
    fn tools_list_bool_arg_type() {
        let manifest = test_manifest();
        let resp = handle_tools_list(Value::Number(1.into()), &manifest);
        let result = resp.result.unwrap();

        let props = &result["tools"][1]["inputSchema"]["properties"];
        assert_eq!(props["uppercase"]["type"], "boolean");
    }

    // ── handle_tools_call ────────────────────────────────────────

    #[test]
    fn tools_call_executes_tool() {
        let manifest = test_manifest();
        let params = serde_json::json!({
            "name": "say_hello",
            "arguments": {"message": "hi"}
        });
        let resp = handle_tools_call(Value::Number(3.into()), &manifest, &params);
        let result = resp.result.unwrap();

        assert_eq!(result["isError"], false);
        let content = result["content"].as_array().unwrap();
        assert!(!content.is_empty());
        assert!(content[0]["text"].as_str().unwrap().contains("hi"));
    }

    #[test]
    fn tools_call_missing_name_errors() {
        let manifest = test_manifest();
        let params = serde_json::json!({"arguments": {}});
        let resp = handle_tools_call(Value::Number(4.into()), &manifest, &params);

        assert!(resp.error.is_some());
        assert_eq!(resp.error.unwrap().code, -32602);
    }

    #[test]
    fn tools_call_unknown_tool_errors() {
        let manifest = test_manifest();
        let params = serde_json::json!({
            "name": "nonexistent",
            "arguments": {}
        });
        let resp = handle_tools_call(Value::Number(5.into()), &manifest, &params);

        assert!(resp.error.is_some());
        assert!(resp.error.unwrap().message.contains("nonexistent"));
    }

    #[test]
    fn tools_call_missing_required_arg_returns_error_content() {
        let manifest = test_manifest();
        let params = serde_json::json!({
            "name": "say_hello",
            "arguments": {}
        });
        let resp = handle_tools_call(Value::Number(6.into()), &manifest, &params);
        let result = resp.result.unwrap();

        assert_eq!(result["isError"], true);
    }

    // ── JsonRpcResponse ──────────────────────────────────────────

    #[test]
    fn success_response_format() {
        let resp = JsonRpcResponse::success(Value::Number(1.into()), serde_json::json!("ok"));
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 1);
        assert_eq!(json["result"], "ok");
        assert!(json.get("error").is_none());
    }

    #[test]
    fn error_response_format() {
        let resp = JsonRpcResponse::error(Value::Number(2.into()), -32600, "bad".to_string());
        let json = serde_json::to_value(&resp).unwrap();

        assert_eq!(json["jsonrpc"], "2.0");
        assert_eq!(json["id"], 2);
        assert_eq!(json["error"]["code"], -32600);
        assert_eq!(json["error"]["message"], "bad");
        assert!(json.get("result").is_none());
    }

    // ── write_response ───────────────────────────────────────────

    #[test]
    fn write_response_is_single_line_json() {
        let resp = JsonRpcResponse::success(Value::Number(1.into()), serde_json::json!(true));
        let mut buf = Vec::new();
        write_response(&mut buf, &resp).unwrap();

        let s = String::from_utf8(buf).unwrap();
        let lines: Vec<&str> = s.lines().collect();
        assert_eq!(lines.len(), 1);

        // Should be valid JSON
        let _: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    }

    // ── Request parsing (integration-style) ──────────────────────

    #[test]
    fn parse_initialize_request() {
        let json = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "initialize");
        assert_eq!(req.id, Some(Value::Number(1.into())));
    }

    #[test]
    fn parse_tools_call_request() {
        let json = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"say_hello","arguments":{"message":"hi"}}}"#;
        let req: JsonRpcRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.method, "tools/call");
        assert_eq!(req.params["name"], "say_hello");
    }

    // ── Proptests ────────────────────────────────────────────────

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn tools_call_with_arbitrary_message(msg in "[a-zA-Z0-9 ]{1,50}") {
                let manifest = test_manifest();
                let params = serde_json::json!({
                    "name": "say_hello",
                    "arguments": {"message": msg.clone()}
                });
                let resp = handle_tools_call(Value::Number(1.into()), &manifest, &params);
                let result = resp.result.unwrap();
                prop_assert!(!result["isError"].as_bool().unwrap_or(true));
                let text = result["content"][0]["text"].as_str().unwrap();
                prop_assert!(text.contains(&msg));
            }

            #[test]
            fn initialize_always_returns_protocol_version(id in 1..1000i64) {
                let manifest = test_manifest();
                let resp = handle_initialize(Value::Number(id.into()), &manifest);
                let result = resp.result.unwrap();
                prop_assert_eq!(result["protocolVersion"].as_str().unwrap(), "2024-11-05");
            }

            #[test]
            fn unknown_tool_always_errors(name in "[a-z]{10,30}") {
                let manifest = test_manifest();
                let params = serde_json::json!({
                    "name": name,
                    "arguments": {}
                });
                let resp = handle_tools_call(Value::Number(1.into()), &manifest, &params);
                // Should be either a JSON-RPC error or an isError result
                if let Some(err) = resp.error {
                    prop_assert!(err.code != 0);
                } else {
                    let result = resp.result.unwrap();
                    prop_assert!(result["isError"].as_bool().unwrap_or(false));
                }
            }
        }
    }
}
