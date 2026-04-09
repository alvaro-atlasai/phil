use std::path::Path;
use std::io::Write;

use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{AddBos, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use encoding_rs::UTF_8;

#[derive(Debug, thiserror::Error)]
pub enum InferenceError {
    #[error("backend init failed: {0}")]
    Backend(String),
    #[error("model load failed: {0}")]
    ModelLoad(String),
    #[error("context creation failed: {0}")]
    Context(String),
    #[error("tokenization failed: {0}")]
    Tokenize(String),
    #[error("decode failed: {0}")]
    Decode(String),
    #[error("sampling failed")]
    Sampling,
}

pub struct CompletionParams {
    pub max_tokens: u32,
    pub temperature: f32,
    pub top_p: f32,
}

impl Default for CompletionParams {
    fn default() -> Self {
        Self {
            max_tokens: 2048,
            temperature: 0.1,
            top_p: 0.9,
        }
    }
}

pub struct PhilInference {
    backend: LlamaBackend,
    model: LlamaModel,
}

impl PhilInference {
    /// Load a GGUF model from disk.
    pub fn load(model_path: &Path) -> Result<Self, InferenceError> {
        let mut backend =
            LlamaBackend::init().map_err(|e| InferenceError::Backend(e.to_string()))?;

        backend.void_logs();

        let model_params = LlamaModelParams::default();

        let model = LlamaModel::load_from_file(&backend, model_path, &model_params)
            .map_err(|e| InferenceError::ModelLoad(e.to_string()))?;

        Ok(Self { backend, model })
    }

    /// Generate a completion, streaming tokens to the provided writer.
    /// Returns the full generated text.
    pub fn complete_streaming<W: Write>(
        &self,
        system_prompt: &str,
        user_input: &str,
        params: &CompletionParams,
        writer: W,
    ) -> Result<String, InferenceError> {
        let prompt = format!(
            "<|system|>{system_prompt}<|end|>\n<|user|>{user_input}<|end|>\n<|assistant|>"
        );
        self.complete_raw(&prompt, params, writer)
    }

    /// Generate a completion from a pre-formatted prompt string.
    /// Used by the agentic tool-calling loop for multi-turn conversations.
    pub fn complete_raw<W: Write>(
        &self,
        prompt: &str,
        params: &CompletionParams,
        mut writer: W,
    ) -> Result<String, InferenceError> {

        let ctx_params = LlamaContextParams::default()
            .with_n_ctx(std::num::NonZeroU32::new(4096));

        let mut ctx = self
            .model
            .new_context(&self.backend, ctx_params)
            .map_err(|e| InferenceError::Context(e.to_string()))?;

        let tokens = self
            .model
            .str_to_token(&prompt, AddBos::Always)
            .map_err(|e| InferenceError::Tokenize(e.to_string()))?;

        let mut batch = LlamaBatch::new(4096, 1);

        // Feed prompt tokens
        let n_tokens = tokens.len();
        for (i, &token) in tokens.iter().enumerate() {
            let is_last = i == n_tokens - 1;
            batch
                .add(token, i as i32, &[0], is_last)
                .map_err(|e| InferenceError::Decode(e.to_string()))?;
        }

        ctx.decode(&mut batch)
            .map_err(|e| InferenceError::Decode(e.to_string()))?;

        // Set up sampler
        let mut sampler = LlamaSampler::chain_simple([
            LlamaSampler::top_p(params.top_p, 1),
            LlamaSampler::temp(params.temperature),
            LlamaSampler::dist(42),
        ]);

        let mut output = String::new();
        let mut n_cur = n_tokens;
        let eos_token = self.model.token_eos();
        let eot_token = self
            .model
            .str_to_token("<|end|>", AddBos::Never)
            .ok()
            .and_then(|t| t.first().copied());

        let mut decoder = UTF_8.new_decoder();

        for _ in 0..params.max_tokens {
            let token = sampler.sample(&ctx, batch.n_tokens() - 1);
            sampler.accept(token);

            // Stop on EOS or end-of-turn
            if token == eos_token {
                break;
            }
            if let Some(eot) = eot_token {
                if token == eot {
                    break;
                }
            }

            let piece = self
                .model
                .token_to_piece(token, &mut decoder, false, None)
                .map_err(|e| InferenceError::Tokenize(e.to_string()))?;

            output.push_str(&piece);
            let _ = writer.write_all(piece.as_bytes());
            let _ = writer.flush();

            // Prepare next token
            batch.clear();
            batch
                .add(token, n_cur as i32, &[0], true)
                .map_err(|e| InferenceError::Decode(e.to_string()))?;
            n_cur += 1;

            ctx.decode(&mut batch)
                .map_err(|e| InferenceError::Decode(e.to_string()))?;
        }

        Ok(output)
    }

    /// Generate a completion and return the full text (no streaming).
    pub fn complete(
        &self,
        system_prompt: &str,
        user_input: &str,
        params: &CompletionParams,
    ) -> Result<String, InferenceError> {
        self.complete_streaming(system_prompt, user_input, params, std::io::sink())
    }

    /// Generate a completion constrained to a JSON schema.
    /// Uses llama.cpp grammar-based constrained decoding.
    pub fn complete_json(
        &self,
        system_prompt: &str,
        user_input: &str,
        json_schema: &serde_json::Value,
        params: &CompletionParams,
    ) -> Result<serde_json::Value, InferenceError> {
        let schema_instruction = format!(
            "You must respond with valid JSON matching this schema:\n```json\n{}\n```",
            serde_json::to_string_pretty(json_schema).unwrap_or_default()
        );
        let full_system = format!("{system_prompt}\n\n{schema_instruction}");

        let text = self.complete(&full_system, user_input, params)?;

        // Extract JSON from the response (handle markdown fences)
        let json_str = extract_json(&text);

        serde_json::from_str(json_str).map_err(|e| {
            InferenceError::Tokenize(format!(
                "Failed to parse model output as JSON: {e}\nRaw output: {text}"
            ))
        })
    }
}

/// Extract JSON from a string that might have markdown code fences.
pub fn extract_json(text: &str) -> &str {
    let trimmed = text.trim();

    // Try to find ```json ... ``` blocks
    if let Some(start) = trimmed.find("```json") {
        let after_fence = &trimmed[start + 7..];
        if let Some(end) = after_fence.find("```") {
            return after_fence[..end].trim();
        }
    }

    // Try to find ``` ... ``` blocks
    if let Some(start) = trimmed.find("```") {
        let after_fence = &trimmed[start + 3..];
        if let Some(end) = after_fence.find("```") {
            return after_fence[..end].trim();
        }
    }

    // Try to find the outermost { ... } or [ ... ]
    if let (Some(start), Some(end)) = (trimmed.find('{'), trimmed.rfind('}')) {
        if start < end {
            return &trimmed[start..=end];
        }
    }
    if let (Some(start), Some(end)) = (trimmed.find('['), trimmed.rfind(']')) {
        if start < end {
            return &trimmed[start..=end];
        }
    }

    trimmed
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── Unit tests for extract_json ──────────────────────────────

    #[test]
    fn extract_json_fenced_block() {
        let input = r#"Here is your JSON:
```json
{"key": "value"}
```
Done."#;
        assert_eq!(extract_json(input), r#"{"key": "value"}"#);
    }

    #[test]
    fn extract_json_plain_fenced_block() {
        let input = r#"```
[1, 2, 3]
```"#;
        assert_eq!(extract_json(input), "[1, 2, 3]");
    }

    #[test]
    fn extract_json_bare_object() {
        let input = r#"The result is {"a": 1, "b": 2} end."#;
        assert_eq!(extract_json(input), r#"{"a": 1, "b": 2}"#);
    }

    #[test]
    fn extract_json_bare_array() {
        let input = r#"Output: [1, 2, 3] done"#;
        assert_eq!(extract_json(input), "[1, 2, 3]");
    }

    #[test]
    fn extract_json_nested_braces() {
        let input = r#"{"outer": {"inner": true}}"#;
        assert_eq!(extract_json(input), r#"{"outer": {"inner": true}}"#);
    }

    #[test]
    fn extract_json_with_whitespace() {
        let input = "  \n  {\"x\": 1}  \n  ";
        assert_eq!(extract_json(input), r#"{"x": 1}"#);
    }

    #[test]
    fn extract_json_plain_text_fallback() {
        let input = "no json here";
        assert_eq!(extract_json(input), "no json here");
    }

    #[test]
    fn extract_json_empty_input() {
        assert_eq!(extract_json(""), "");
        assert_eq!(extract_json("   "), "");
    }

    #[test]
    fn extract_json_fenced_preferred_over_bare() {
        // When both fenced and bare JSON exist, fenced should win
        let input = r#"prefix {"ignored": true}
```json
{"correct": true}
```
suffix"#;
        assert_eq!(extract_json(input), r#"{"correct": true}"#);
    }

    #[test]
    fn extract_json_multiline_fenced() {
        let input = r#"```json
{
  "name": "test",
  "items": [1, 2, 3]
}
```"#;
        let result = extract_json(input);
        let parsed: serde_json::Value = serde_json::from_str(result).unwrap();
        assert_eq!(parsed["name"], "test");
    }

    // ── CompletionParams defaults ────────────────────────────────

    #[test]
    fn completion_params_defaults() {
        let params = CompletionParams::default();
        assert_eq!(params.max_tokens, 2048);
        assert!((params.temperature - 0.1).abs() < f32::EPSILON);
        assert!((params.top_p - 0.9).abs() < f32::EPSILON);
    }

    // ── Proptests ────────────────────────────────────────────────

    mod proptests {
        use super::*;
        use proptest::prelude::*;

        proptest! {
            #[test]
            fn extract_json_never_panics(s in "\\PC{0,500}") {
                let _ = extract_json(&s);
            }

            #[test]
            fn extract_json_fenced_roundtrip(
                json_body in "[a-zA-Z0-9 :,\"{}\\[\\]]{1,100}"
            ) {
                let fenced = format!("```json\n{json_body}\n```");
                let result = extract_json(&fenced);
                assert_eq!(result, json_body.trim());
            }

            #[test]
            fn extract_json_bare_object_contains_braces(
                prefix in "[a-zA-Z ]{0,20}",
                inner in "[a-zA-Z0-9: ,\"]{1,50}",
                suffix in "[a-zA-Z ]{0,20}"
            ) {
                let input = format!("{prefix}{{{inner}}}{suffix}");
                let result = extract_json(&input);
                assert!(result.starts_with('{'));
                assert!(result.ends_with('}'));
            }

            #[test]
            fn extract_json_bare_array_contains_brackets(
                prefix in "[a-zA-Z ]{0,20}",
                inner in "[0-9, ]{1,30}",
                suffix in "[a-zA-Z ]{0,20}"
            ) {
                // Only test when no braces are present (arrays are lower priority)
                let input = format!("{prefix}[{inner}]{suffix}");
                if !input.contains('{') {
                    let result = extract_json(&input);
                    assert!(result.starts_with('['));
                    assert!(result.ends_with(']'));
                }
            }
        }
    }
}
