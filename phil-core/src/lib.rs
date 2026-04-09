mod model;
mod inference;
pub mod agent;
pub mod apple;
pub mod config;
pub mod daemon;
pub mod github;
pub mod pack;

pub use model::{ModelManager, ModelError, ModelEntry, model_registry, find_model};
pub use inference::{PhilInference, InferenceError, CompletionParams};
pub use daemon::{DaemonRequest, DaemonResponse};
pub use pack::{Pack, PackError, PackMeta};
pub use github::{GitHubError, GitHubModel, github_models, find_github_model};
pub use apple::{apple_available, apple_complete, AppleError};
pub use agent::{ToolDef, ToolCall, AgentStep, packs_as_tools, parse_model_output, run_agent_loop, build_agent_prompt, append_tool_result};
