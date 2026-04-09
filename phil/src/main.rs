use std::io::{self, BufRead, Read, Write};
use clap::{Parser, Subcommand};
use phil_core::{CompletionParams, DaemonRequest, ModelManager, Pack, PhilInference};
use phil_core::{daemon, config, github, apple, pack, model_registry, find_model, find_github_model};

const DEFAULT_SYSTEM_PROMPT: &str =
    "You are a concise command-line assistant. Respond directly without preamble. \
     When given data, analyze it as requested. Be brief and precise.";
const DO_SYSTEM_PROMPT: &str = "\
You are a shell command generator. Given a natural language task, output ONLY the shell command(s) to accomplish it.

Rules:
- Output raw shell commands, no markdown fences, no explanation
- Use && to chain multiple commands on one line when possible
- For multi-step tasks that need separate lines, output one command per line
- Use common CLI tools (git, npm, cargo, pip, docker, curl, etc.)
- Prefer safe defaults (e.g. mkdir -p, set -e for scripts)
- If the task is ambiguous, pick the most common interpretation
- Target the current OS (macOS/Linux)
- Never output dangerous commands (rm -rf /, etc.) without explicit paths
- If asked to create a project, use standard scaffolding tools (npm init, cargo init, etc.)";
#[derive(Parser)]
#[command(
    name = "phil",
    about = "Pipe-native AI CLI powered by local Phi-4-mini",
    version,
    after_help = "Examples:\n  \
        cat error.log | phil \"what went wrong?\"\n  \
        git diff --staged | phil @commit\n  \
        echo '{\"name\":\"John\"}' | phil \"convert to YAML\"\n  \
        phil @explain \"what is set -euo pipefail?\"\n  \
        cat urls.txt | phil @suspicious --each\n  \
        phil --do \"setup a new node project with express\"\n  \
        phil pack ls"
)]
struct Cli {
    /// The prompt or @pack name, followed by optional additional text
    #[arg(trailing_var_arg = true)]
    prompt: Vec<String>,

    /// Custom system prompt
    #[arg(short, long)]
    system: Option<String>,

    /// Don't add a system prompt
    #[arg(long)]
    raw: bool,

    /// Path to a custom GGUF model file
    #[arg(long)]
    model: Option<String>,

    /// Maximum tokens to generate
    #[arg(long, default_value = "2048")]
    max_tokens: u32,

    /// Sampling temperature (0.0 = deterministic, 1.0 = creative)
    #[arg(long, default_value = "0.1")]
    temperature: f32,

    /// Skip the daemon, load model directly in this process
    #[arg(long)]
    no_daemon: bool,

    /// Process each stdin line separately (like semantic sed)
    #[arg(long)]
    each: bool,

    /// Generate and execute shell commands from natural language
    #[arg(long, alias = "do")]
    execute: bool,

    /// Show what phil would do without calling the model
    #[arg(long)]
    dry_run: bool,

    /// Agentic mode: let the model autonomously call packs as tools
    #[arg(long)]
    agent: bool,

    /// Run as the background daemon (internal use)
    #[arg(long, hide = true)]
    daemon: bool,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Manage packs (reusable prompt configurations)
    Pack {
        #[command(subcommand)]
        action: PackAction,
    },
    /// Manage models
    Model {
        #[command(subcommand)]
        action: ModelAction,
    },
    /// Show or initialize config
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Authenticate with a remote provider
    Auth {
        /// Provider name (currently: github)
        provider: String,
    },
}

#[derive(Subcommand)]
enum PackAction {
    /// List all available packs
    Ls,
    /// Create a new pack from a template
    Init {
        /// Name for the new pack
        name: String,
    },
    /// Install a pack from a URL
    Add {
        /// URL to a .toml pack file (e.g. a GitHub raw URL or gist)
        url: String,
    },
    /// Show details of a specific pack
    Show {
        /// Pack name
        name: String,
    },
    /// Generate a pack using the LLM from a description
    Gen {
        /// Description of what the pack should do
        description: String,
    },
    /// Export all packs as an any2mcp YAML manifest (MCP server)
    Export,
}

#[derive(Subcommand)]
enum ModelAction {
    /// List available and installed models
    Ls,
    /// Download and install a model
    Install {
        /// Model name (e.g. phi4-mini, qwen3-4b)
        name: String,
    },
    /// Set the active model
    Use {
        /// Model name
        name: String,
    },
}

#[derive(Subcommand)]
enum ConfigAction {
    /// Show current configuration
    Show,
    /// Initialize config file with defaults
    Init,
    /// Set a config value
    Set {
        /// Key (e.g. model.active, defaults.temperature)
        key: String,
        /// Value
        value: String,
    },
}

#[tokio::main]
async fn main() {
    if let Err(e) = run().await {
        eprintln!("phil: {e}");
        std::process::exit(1);
    }
}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    let cli = Cli::parse();

    // Hidden daemon mode — used when auto-spawning
    if cli.daemon {
        let model_path = match &cli.model {
            Some(p) => std::path::PathBuf::from(p),
            None => {
                let mgr = ModelManager::new()?;
                mgr.ensure_model().await?
            }
        };
        return daemon::run_daemon(model_path).await;
    }

    // Handle subcommands
    if let Some(cmd) = cli.command {
        return handle_command(cmd).await;
    }

    let raw_prompt = if cli.prompt.is_empty() {
        return Err("no prompt provided. Usage: phil \"your question\" or phil @pack".into());
    } else {
        cli.prompt.join(" ")
    };
    let raw_prompt = raw_prompt.as_str();

    // Resolve @pack syntax
    let (prompt, system_prompt, max_tokens, temperature, force_each) = if let Some(pack_name) = raw_prompt.split_whitespace().next().and_then(|w| w.strip_prefix('@')) {
        let loaded = pack::load_pack(pack_name)?;
        let sys = if cli.raw { String::new() } else {
            cli.system.unwrap_or(loaded.system)
        };
        // Pack defaults for max_tokens/temperature, CLI flags override
        let mt = loaded.max_tokens.unwrap_or(cli.max_tokens);
        let temp = loaded.temperature.unwrap_or(cli.temperature);
        // Everything after @pack becomes the user prompt
        let rest = raw_prompt[pack_name.len() + 1..].trim();
        let user_prompt = if rest.is_empty() {
            loaded.description.clone()
        } else {
            rest.to_string()
        };
        (user_prompt, sys, mt, temp, loaded.each)
    } else {
        let sys = if cli.raw {
            String::new()
        } else {
            cli.system.unwrap_or_else(|| DEFAULT_SYSTEM_PROMPT.to_string())
        };
        (raw_prompt.to_string(), sys, cli.max_tokens, cli.temperature, false)
    };

    let use_each = cli.each || force_each;

    // Check if active model is a GitHub (cloud) model
    let github_model = if cli.model.is_none() {
        let cfg = config::load_config()?;
        find_github_model(&cfg.model.active)
    } else {
        None
    };

    // Check if active model is Apple Intelligence
    let use_apple = if cli.model.is_none() && github_model.is_none() {
        let cfg = config::load_config()?;
        cfg.model.active == "apple"
    } else {
        false
    };

    // --dry-run: show execution plan and exit
    if cli.dry_run {
        let backend = if let Some(ref gh) = github_model {
            format!("github  ({}, cloud via GitHub Models API)", gh.name)
        } else if use_apple {
            "apple   (Apple Intelligence, on-device)".to_string()
        } else if cli.model.is_some() {
            format!("custom  ({})", cli.model.as_deref().unwrap())
        } else {
            let cfg = config::load_config()?;
            format!("local   ({}, daemon={})", cfg.model.active, if cli.no_daemon { "off" } else { "on" })
        };
        let mode = if cli.execute {
            "--do (generate + execute shell command)"
        } else if cli.agent {
            "--agent (agentic loop, packs as tools)"
        } else if use_each {
            "--each (per-line inference)"
        } else {
            "completion"
        };
        let sys = if cli.execute { DO_SYSTEM_PROMPT } else { &system_prompt };
        eprintln!("backend:  {backend}");
        eprintln!("mode:     {mode}");
        eprintln!("tokens:   {max_tokens}");
        eprintln!("temp:     {temperature}");
        eprintln!("system:   {}", if sys.is_empty() { "(none — --raw)" } else { &sys[..sys.len().min(120)] });
        if sys.len() > 120 {
            eprintln!("          ... ({} chars total)", sys.len());
        }
        eprintln!("prompt:   {prompt}");
        if atty::isnt(atty::Stream::Stdin) && !use_each {
            eprintln!("stdin:    (piped data will be prepended to prompt)");
        }
        if cli.agent {
            let tools = phil_core::agent::packs_as_tools();
            let names: Vec<&str> = tools.iter().map(|t| t.name.as_str()).collect();
            eprintln!("tools:    {} packs — {}", tools.len(), names.join(", "));
        }
        // For --do, fall through to actually generate the command (just don't execute)
        if !cli.execute {
            return Ok(());
        }
    }

    // --each mode: process each stdin line separately
    if use_each {
        if atty::is(atty::Stream::Stdin) {
            return Err("--each requires piped stdin".into());
        }
        if let Some(ref gh) = github_model {
            return run_each_github(&prompt, &system_prompt, gh, max_tokens, temperature).await;
        }
        if use_apple {
            return run_each_apple(&prompt, &system_prompt, max_tokens, temperature).await;
        }
        return run_each(&prompt, &system_prompt, cli.model.as_deref(), max_tokens, temperature, cli.no_daemon).await;
    }

    // --do mode: generate shell command and execute with confirmation
    if cli.execute {
        return run_do(&prompt, &github_model, use_apple, cli.model.as_deref(), max_tokens, temperature, cli.no_daemon, cli.dry_run).await;
    }

    // --agent mode: agentic loop with packs as tools
    if cli.agent {
        return run_agent(&prompt, &system_prompt, &github_model, use_apple, cli.model.as_deref(), max_tokens, temperature, cli.no_daemon).await;
    }

    // Read stdin if piped
    let stdin_data = if atty::isnt(atty::Stream::Stdin) {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        if buf.is_empty() {
            None
        } else {
            Some(buf)
        }
    } else {
        None
    };

    // Build user message: stdin data + prompt
    let user_input = match stdin_data {
        Some(data) => format!("--- INPUT DATA ---\n{data}\n--- END DATA ---\n\n{prompt}"),
        None => prompt.to_string(),
    };

    // GitHub cloud model — call API directly, no daemon needed
    if let Some(ref gh) = github_model {
        let token = github::load_token()?;
        let text = github::complete(&token, gh.api_name, &system_prompt, &user_input, temperature, max_tokens).await?;
        print!("{text}");
        if !text.ends_with('\n') {
            println!();
        }
        return Ok(());
    }

    // Apple Intelligence — call phil-apple helper
    if use_apple {
        match apple::apple_complete(&system_prompt, &user_input, temperature, max_tokens) {
            Ok(text) => {
                print!("{text}");
                if !text.ends_with('\n') {
                    println!();
                }
                return Ok(());
            }
            Err(apple::AppleError::Unavailable(msg)) => {
                eprintln!("phil: Apple Intelligence unavailable ({msg})");
                eprintln!("      Install a local model: phil model install phi4-mini");
                eprintln!("      Or use cloud models:   phil auth github");
                return Err("Apple Intelligence not available".into());
            }
            Err(e) => return Err(e.into()),
        }
    }

    // Try daemon mode (unless --no-daemon)
    if !cli.no_daemon && cli.model.is_none() {
        // If no local model is installed yet and Apple is available, use Apple
        // instead of triggering a 2.5GB download on first run
        if !daemon::daemon_is_running().await {
            let mgr = ModelManager::new()?;
            let has_local_model = mgr.resolve_model_path().map(|p| p.exists()).unwrap_or(false);
            if !has_local_model && apple::apple_available() {
                eprintln!("phil: no local model installed — using Apple Intelligence (zero download)");
                eprintln!("      For larger context: phil model install phi4-mini");
                match apple::apple_complete(&system_prompt, &user_input, temperature, max_tokens) {
                    Ok(text) => {
                        print!("{text}");
                        if !text.ends_with('\n') {
                            println!();
                        }
                        return Ok(());
                    }
                    Err(e) => {
                        eprintln!("phil: Apple fallback failed ({e}), downloading local model...");
                    }
                }
            }
        }

        // Ensure daemon is running
        if let Err(e) = ensure_daemon_running().await {
            eprintln!("phil: {e}, falling back to direct mode");
            return run_direct(&user_input, &system_prompt, cli.model.as_deref(), cli.max_tokens, cli.temperature).await;
        }

        let req = DaemonRequest {
            system_prompt: system_prompt.clone(),
            user_input: user_input.clone(),
            max_tokens: cli.max_tokens,
            temperature: cli.temperature,
            top_p: 0.9,
        };

        match daemon::daemon_complete(&req).await {
            Ok(text) => {
                print!("{text}");
                if !text.ends_with('\n') {
                    println!();
                }
                return Ok(());
            }
            Err(e) => {
                eprintln!("phil: daemon error ({e}), falling back to direct mode");
            }
        }
    }

    run_direct(&user_input, &system_prompt, cli.model.as_deref(), cli.max_tokens, cli.temperature).await
}

/// Process each stdin line as a separate inference call.
async fn run_each(
    prompt: &str,
    system_prompt: &str,
    model: Option<&str>,
    max_tokens: u32,
    temperature: f32,
    no_daemon: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let use_daemon = !no_daemon && model.is_none();

    // Ensure daemon is ready if we'll use it
    if use_daemon {
        ensure_daemon_running().await?;
    }

    // For direct mode, load model once and reuse
    let inference = if !use_daemon {
        let model_path = match model {
            Some(p) => std::path::PathBuf::from(p),
            None => {
                let mgr = ModelManager::new()?;
                mgr.ensure_model().await?
            }
        };
        eprintln!("Loading model...");
        let inf = PhilInference::load(&model_path)?;
        eprintln!("Ready.\n");
        Some(inf)
    } else {
        None
    };

    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.is_empty() {
            println!();
            continue;
        }

        let user_input = format!("--- INPUT DATA ---\n{line}\n--- END DATA ---\n\n{prompt}");

        if use_daemon {
            let req = DaemonRequest {
                system_prompt: system_prompt.to_string(),
                user_input,
                max_tokens,
                temperature,
                top_p: 0.9,
            };
            match daemon::daemon_complete(&req).await {
                Ok(text) => {
                    let text = text.trim_end();
                    println!("{text}");
                }
                Err(e) => {
                    eprintln!("phil: daemon error on line: {e}");
                    println!("ERROR");
                }
            }
        } else {
            let inf = inference.as_ref().unwrap();
            let params = CompletionParams {
                max_tokens,
                temperature,
                ..Default::default()
            };
            match inf.complete(system_prompt, &user_input, &params) {
                Ok(text) => {
                    let text = text.trim_end();
                    println!("{text}");
                }
                Err(e) => {
                    eprintln!("phil: inference error on line: {e}");
                    println!("ERROR");
                }
            }
        }
    }

    Ok(())
}

/// Process each stdin line via GitHub Models API.
async fn run_each_github(
    prompt: &str,
    system_prompt: &str,
    gh: &phil_core::GitHubModel,
    max_tokens: u32,
    temperature: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    let token = github::load_token()?;
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.is_empty() {
            println!();
            continue;
        }
        let user_input = format!("--- INPUT DATA ---\n{line}\n--- END DATA ---\n\n{prompt}");
        match github::complete(&token, gh.api_name, system_prompt, &user_input, temperature, max_tokens).await {
            Ok(text) => {
                let text = text.trim_end();
                println!("{text}");
            }
            Err(e) => {
                eprintln!("phil: github api error on line: {e}");
                println!("ERROR");
            }
        }
    }
    Ok(())
}

/// Process each stdin line via Apple Intelligence.
async fn run_each_apple(
    prompt: &str,
    system_prompt: &str,
    max_tokens: u32,
    temperature: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line?;
        if line.is_empty() {
            println!();
            continue;
        }
        let user_input = format!("--- INPUT DATA ---\n{line}\n--- END DATA ---\n\n{prompt}");
        match apple::apple_complete(system_prompt, &user_input, temperature, max_tokens) {
            Ok(text) => {
                let text = text.trim_end();
                println!("{text}");
            }
            Err(e) => {
                eprintln!("phil: apple error on line: {e}");
                println!("ERROR");
            }
        }
    }
    Ok(())
}

/// Generate a shell command from natural language and execute it with user confirmation.
async fn run_do(
    prompt: &str,
    github_model: &Option<phil_core::GitHubModel>,
    use_apple: bool,
    model: Option<&str>,
    max_tokens: u32,
    temperature: f32,
    no_daemon: bool,
    dry_run: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    let system = DO_SYSTEM_PROMPT;

    // Generate the command using whichever backend is active
    let command_text = if let Some(ref gh) = github_model {
        let token = github::load_token()?;
        github::complete(&token, gh.api_name, system, prompt, temperature, max_tokens).await?
    } else if use_apple {
        apple::apple_complete(system, prompt, temperature, max_tokens)?
    } else if !no_daemon && model.is_none() {
        ensure_daemon_running().await?;
        let req = DaemonRequest {
            system_prompt: system.to_string(),
            user_input: prompt.to_string(),
            max_tokens,
            temperature,
            top_p: 0.9,
        };
        daemon::daemon_complete(&req).await?
    } else {
        let model_path = match model {
            Some(p) => std::path::PathBuf::from(p),
            None => {
                let mgr = ModelManager::new()?;
                mgr.ensure_model().await?
            }
        };
        let inference = PhilInference::load(&model_path)?;
        let params = CompletionParams {
            max_tokens,
            temperature,
            ..Default::default()
        };
        inference.complete(system, prompt, &params)?
    };

    let command_text = command_text.trim();
    if command_text.is_empty() {
        return Err("model returned empty command".into());
    }

    // Strip markdown fences if model included them despite instructions
    let command_text = command_text
        .strip_prefix("```sh\n").or_else(|| command_text.strip_prefix("```bash\n")).or_else(|| command_text.strip_prefix("```\n"))
        .and_then(|s| s.strip_suffix("\n```").or_else(|| s.strip_suffix("```")))
        .unwrap_or(command_text);

    // Display the command
    eprintln!("\n  \x1b[1;36m{}\x1b[0m\n", command_text);

    // --dry-run: show the generated command but don't execute
    if dry_run {
        // Also print to stdout (plain, no color) so it can be piped
        println!("{command_text}");
        return Ok(());
    }

    // Ask for confirmation
    eprint!("Run this? [Y/n/e(dit)] ");
    io::stderr().flush()?;

    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    let answer = answer.trim().to_lowercase();

    match answer.as_str() {
        "" | "y" | "yes" => {
            let status = std::process::Command::new("sh")
                .arg("-c")
                .arg(command_text)
                .status()?;
            if !status.success() {
                std::process::exit(status.code().unwrap_or(1));
            }
        }
        "e" | "edit" => {
            // Let user edit the command in-line
            eprint!("$ ");
            io::stderr().flush()?;
            let mut edited = String::new();
            io::stdin().read_line(&mut edited)?;
            let edited = edited.trim();
            if !edited.is_empty() {
                let status = std::process::Command::new("sh")
                    .arg("-c")
                    .arg(edited)
                    .status()?;
                if !status.success() {
                    std::process::exit(status.code().unwrap_or(1));
                }
            }
        }
        _ => {
            eprintln!("Aborted.");
        }
    }

    Ok(())
}

/// Agentic mode: model autonomously calls packs as tools.
async fn run_agent(
    prompt: &str,
    system_prompt: &str,
    github_model: &Option<phil_core::GitHubModel>,
    use_apple: bool,
    model: Option<&str>,
    max_tokens: u32,
    temperature: f32,
    no_daemon: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    use phil_core::agent;

    let tools = agent::packs_as_tools();
    if tools.is_empty() {
        return Err("no packs available as tools. Run `phil pack ls` to see available packs.".into());
    }

    // Read stdin if piped
    let stdin_data = if atty::isnt(atty::Stream::Stdin) {
        let mut buf = String::new();
        io::stdin().read_to_string(&mut buf)?;
        if buf.is_empty() { None } else { Some(buf) }
    } else {
        None
    };

    let user_input = match stdin_data {
        Some(data) => format!("--- INPUT DATA ---\n{data}\n--- END DATA ---\n\n{prompt}"),
        None => prompt.to_string(),
    };

    eprintln!("phil: agent mode — {} tools available", tools.len());

    // Helper: call whichever model backend is active
    let complete = |system: &str, input: &str| -> Result<String, Box<dyn std::error::Error>> {
        if let Some(ref gh) = github_model {
            let token = github::load_token()?;
            // We can't use async inside a sync closure easily, so block_on
            let rt = tokio::runtime::Handle::current();
            let text = rt.block_on(github::complete(&token, gh.api_name, system, input, temperature, max_tokens))?;
            Ok(text)
        } else if use_apple {
            Ok(apple::apple_complete(system, input, temperature, max_tokens)?)
        } else if !no_daemon && model.is_none() {
            let rt = tokio::runtime::Handle::current();
            rt.block_on(ensure_daemon_running())?;
            let req = DaemonRequest {
                system_prompt: system.to_string(),
                user_input: input.to_string(),
                max_tokens,
                temperature,
                top_p: 0.9,
            };
            let text = rt.block_on(daemon::daemon_complete(&req))?;
            Ok(text)
        } else {
            let model_path = match model {
                Some(p) => std::path::PathBuf::from(p),
                None => {
                    let mgr = ModelManager::new()?;
                    let rt = tokio::runtime::Handle::current();
                    rt.block_on(mgr.ensure_model())?
                }
            };
            let inference = PhilInference::load(&model_path)?;
            let params = CompletionParams { max_tokens, temperature, ..Default::default() };
            Ok(inference.complete(system, input, &params)?)
        }
    };

    // Helper: execute a pack with the active backend
    let execute_pack = |pack_name: &str, input: &str| -> Result<String, Box<dyn std::error::Error>> {
        let pack = phil_core::pack::load_pack(pack_name)?;
        let temp = pack.temperature.unwrap_or(temperature);
        let mt = pack.max_tokens.unwrap_or(max_tokens);

        if let Some(ref gh) = github_model {
            let token = github::load_token()?;
            let rt = tokio::runtime::Handle::current();
            Ok(rt.block_on(github::complete(&token, gh.api_name, &pack.system, input, temp, mt))?)
        } else if use_apple {
            Ok(apple::apple_complete(&pack.system, input, temp, mt)?)
        } else if !no_daemon && model.is_none() {
            let rt = tokio::runtime::Handle::current();
            let req = DaemonRequest {
                system_prompt: pack.system.clone(),
                user_input: input.to_string(),
                max_tokens: mt,
                temperature: temp,
                top_p: 0.9,
            };
            Ok(rt.block_on(daemon::daemon_complete(&req))?)
        } else {
            let model_path = match model {
                Some(p) => std::path::PathBuf::from(p),
                None => {
                    let mgr = ModelManager::new()?;
                    let rt = tokio::runtime::Handle::current();
                    rt.block_on(mgr.ensure_model())?
                }
            };
            let inference = PhilInference::load(&model_path)?;
            let params = CompletionParams { max_tokens: mt, temperature: temp, ..Default::default() };
            Ok(inference.complete(&pack.system, input, &params)?)
        }
    };

    // Build initial prompt with tools
    let tools_json = serde_json::to_string(
        &tools.iter().map(|t| serde_json::json!({
            "name": t.name,
            "description": t.description,
            "parameters": t.parameters,
        })).collect::<Vec<_>>()
    ).unwrap_or_else(|_| "[]".into());

    let agent_system = format!(
        "{system_prompt}\n\n\
         You have access to the following tools (packs). When a tool would help, call it by outputting ONLY a JSON object:\n\
         {{\"name\": \"tool_name\", \"arguments\": {{\"input\": \"the text to process\"}}}}\n\n\
         After receiving a tool result, use it to form your final answer as plain text.\n\
         Available tools: {tools_json}"
    );

    let max_rounds = 5;
    let mut conversation_context = user_input.clone();

    for round in 0..max_rounds {
        let output = complete(&agent_system, &conversation_context)?;
        let output = output.trim().to_string();

        match agent::parse_model_output(&output) {
            agent::AgentStep::Done(text) => {
                print!("{text}");
                if !text.ends_with('\n') {
                    println!();
                }
                return Ok(());
            }
            agent::AgentStep::ToolCalls(calls) => {
                for call in &calls {
                    eprintln!("  [{}/{}] @{}", round + 1, max_rounds, call.name);

                    let input = match &call.arguments {
                        serde_json::Value::Object(map) => {
                            map.get("input").and_then(|v| v.as_str()).unwrap_or("").to_string()
                        }
                        serde_json::Value::String(s) => s.clone(),
                        _ => String::new(),
                    };

                    // Use original stdin data as input if the tool call has no specific input
                    let tool_input = if input.is_empty() {
                        conversation_context.clone()
                    } else {
                        input
                    };

                    match execute_pack(&call.name, &tool_input) {
                        Ok(result) => {
                            let preview = if result.len() > 100 {
                                format!("{}...", &result[..100])
                            } else {
                                result.clone()
                            };
                            eprintln!("  ← {}", preview.replace('\n', " "));
                            // Feed result back: original context + tool result
                            conversation_context = format!(
                                "{user_input}\n\nI called @{name} and got this result:\n{result}\n\nUse this result to answer the original request.",
                                name = call.name,
                            );
                        }
                        Err(e) => {
                            eprintln!("  ✗ @{}: {e}", call.name);
                            conversation_context = format!(
                                "{user_input}\n\nI tried calling @{name} but it failed: {e}\n\nAnswer without using that tool.",
                                name = call.name,
                            );
                        }
                    }
                }
            }
        }
    }

    // Exhausted rounds — force final answer
    let output = complete(
        &agent_system,
        &format!("{conversation_context}\n\nProvide your final answer now based on the information gathered."),
    )?;
    print!("{}", output.trim());
    if !output.trim().ends_with('\n') {
        println!();
    }
    Ok(())
}

/// Ensure the daemon is running, starting it if needed.
async fn ensure_daemon_running() -> Result<(), Box<dyn std::error::Error>> {
    if daemon::daemon_is_running().await {
        return Ok(());
    }
    spawn_daemon()?;
    let start = std::time::Instant::now();
    loop {
        if daemon::daemon_is_running().await {
            return Ok(());
        }
        if start.elapsed() > std::time::Duration::from_secs(120) {
            return Err("daemon failed to start within 120s".into());
        }
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
    }
}

async fn run_direct(
    user_input: &str,
    system_prompt: &str,
    model: Option<&str>,
    max_tokens: u32,
    temperature: f32,
) -> Result<(), Box<dyn std::error::Error>> {
    let model_path = match model {
        Some(p) => std::path::PathBuf::from(p),
        None => {
            let mgr = ModelManager::new()?;
            mgr.ensure_model().await?
        }
    };

    eprintln!("Loading model...");
    let inference = PhilInference::load(&model_path)?;
    eprintln!("Ready.\n");

    let params = CompletionParams {
        max_tokens,
        temperature,
        ..Default::default()
    };

    let stdout = io::stdout();
    let writer = stdout.lock();

    inference.complete_streaming(system_prompt, user_input, &params, writer)?;

    // Ensure output ends with newline
    println!();

    Ok(())
}

/// Handle pack subcommands.
async fn handle_command(cmd: Commands) -> Result<(), Box<dyn std::error::Error>> {
    match cmd {
        Commands::Pack { action } => handle_pack_action(action).await,
        Commands::Model { action } => handle_model_action(action).await,
        Commands::Config { action } => handle_config_action(action),
        Commands::Auth { provider } => handle_auth(&provider).await,
    }
}

async fn handle_pack_action(action: PackAction) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        PackAction::Ls => {
            let packs = pack::list_packs_meta()?;
            if packs.is_empty() {
                println!("No packs installed. Run `phil pack init <name>` to create one.");
            } else {
                let max_name = packs.iter().map(|p| p.name.len()).max().unwrap_or(10);
                for p in &packs {
                    let tag = if p.builtin { " (built-in)" } else { "" };
                    println!("  @{:<width$}  {}{}", p.name, p.description, tag, width = max_name);
                }
            }
            Ok(())
        }
        PackAction::Init { name } => {
            let dir = pack::packs_dir()?;
            std::fs::create_dir_all(&dir)?;
            let path = dir.join(format!("{name}.toml"));
            if path.exists() {
                return Err(format!("pack already exists: {}", path.display()).into());
            }
            let content = pack::scaffold_pack(&name);
            std::fs::write(&path, &content)?;
            pack::rebuild_index().ok();
            println!("Created {}", path.display());
            println!("Edit the file to set your system prompt, then use: phil @{name}");
            Ok(())
        }
        PackAction::Add { url } => {
            let resp = reqwest::get(&url).await
                .map_err(|e| format!("failed to fetch {url}: {e}"))?;
            if !resp.status().is_success() {
                return Err(format!("HTTP {}: {url}", resp.status()).into());
            }
            let body = resp.text().await
                .map_err(|e| format!("failed to read response: {e}"))?;

            // Validate it parses as a pack
            let parsed: Pack = toml::from_str(&body)
                .map_err(|e| format!("not a valid pack TOML: {e}"))?;

            let dir = pack::packs_dir()?;
            std::fs::create_dir_all(&dir)?;
            let path = dir.join(format!("{}.toml", parsed.name));
            if path.exists() {
                return Err(format!("pack @{} already exists. Delete {} first.", parsed.name, path.display()).into());
            }
            std::fs::write(&path, &body)?;
            pack::rebuild_index().ok();
            println!("Installed @{} → {}", parsed.name, path.display());
            Ok(())
        }
        PackAction::Show { name } => {
            let p = pack::load_pack(&name)?;
            println!("@{}", p.name);
            println!("  {}", p.description);
            println!();
            println!("System prompt:");
            println!("  {}", p.system.replace('\n', "\n  "));
            if let Some(t) = p.temperature {
                println!("Temperature: {t}");
            }
            if let Some(m) = p.max_tokens {
                println!("Max tokens: {m}");
            }
            if p.each {
                println!("Each: true (processes stdin line-by-line)");
            }
            Ok(())
        }
        PackAction::Gen { description } => {
            // Use daemon or direct inference to generate the pack
            ensure_daemon_running().await.ok();
            let req = DaemonRequest {
                system_prompt: pack::PACK_GEN_SYSTEM.to_string(),
                user_input: pack::pack_gen_prompt(&description),
                max_tokens: 1024,
                temperature: 0.2,
                top_p: 0.9,
            };

            let toml_text = match daemon::daemon_complete(&req).await {
                Ok(text) => text,
                Err(_) => {
                    // Fallback to direct
                    let mgr = ModelManager::new()?;
                    let model_path = mgr.ensure_model().await?;
                    let inference = PhilInference::load(&model_path)?;
                    let params = CompletionParams {
                        max_tokens: 1024,
                        temperature: 0.2,
                        ..Default::default()
                    };
                    inference.complete(pack::PACK_GEN_SYSTEM, &pack::pack_gen_prompt(&description), &params)
                        .map_err(|e| format!("inference error: {e}"))?
                }
            };

            // Validate it parses
            let toml_text = toml_text.trim();
            match toml::from_str::<Pack>(toml_text) {
                Ok(parsed) => {
                    let dir = pack::packs_dir()?;
                    std::fs::create_dir_all(&dir)?;
                    let path = dir.join(format!("{}.toml", parsed.name));
                    if path.exists() {
                        return Err(format!("pack @{} already exists at {}", parsed.name, path.display()).into());
                    }
                    std::fs::write(&path, toml_text)?;
                    pack::rebuild_index().ok();
                    println!("Generated @{} → {}", parsed.name, path.display());
                    println!("\nPreview:");
                    println!("{toml_text}");
                }
                Err(e) => {
                    eprintln!("Warning: generated TOML didn't parse ({e}). Printing raw output:");
                    println!("{toml_text}");
                    eprintln!("\nYou can save this manually to ~/.phil/packs/<name>.toml");
                }
            }
            Ok(())
        }
        PackAction::Export => {
            let packs = pack::list_packs_meta()?;
            if packs.is_empty() {
                return Err("no packs available to export".into());
            }

            // Generate any2mcp-compatible YAML manifest
            println!("binary: phil");
            println!("description: Phil AI packs — reusable prompt-based tools powered by local/cloud LLMs");
            println!("tools:");
            for p in &packs {
                println!("  - name: {}", p.name);
                println!("    description: {}", p.description);
                println!("    subcommand: [\"@{}\"]", p.name);
                println!("    args:");
                println!("      - name: input");
                println!("        description: The text input to process (passed as trailing argument)");
                println!("        required: false");
            }

            eprintln!("\n# Save this manifest and serve it:");
            eprintln!("#   phil pack export > phil-packs.yaml");
            eprintln!("#   any2mcp serve --manifest phil-packs.yaml");
            eprintln!("#");
            eprintln!("# Or configure directly in your MCP client:");
            eprintln!("# {{\"mcpServers\": {{\"phil\": {{\"command\": \"any2mcp\", \"args\": [\"phil-packs.yaml\"]}}}}}}");
            Ok(())
        }
    }
}

fn is_builtin(name: &str) -> bool {
    pack::is_builtin(name)
}

async fn handle_model_action(action: ModelAction) -> Result<(), Box<dyn std::error::Error>> {
    let mgr = ModelManager::new()?;
    match action {
        ModelAction::Ls => {
            let registry = model_registry();
            let installed = mgr.list_installed()?;
            let cfg = config::load_config()?;
            let active = cfg.model.active;
            let has_github = cfg.github.as_ref().is_some_and(|g| !g.token.is_empty());
            let has_apple = apple::apple_available();

            // Compute max name across all models for alignment
            let gh_models = github::github_models();
            let max_name = registry.iter().map(|m| m.name.len())
                .chain(gh_models.iter().map(|m| m.name.len()))
                .chain(std::iter::once("apple".len()))
                .max().unwrap_or(10);

            // Apple Intelligence section
            if cfg!(target_os = "macos") {
                let is_active = active == "apple";
                let status = if is_active {
                    "✓ active"
                } else if has_apple {
                    "  ready"
                } else {
                    "  unavailable"
                };
                println!("Apple Intelligence (on-device, zero download):\n");
                println!("  {:<width$}  {:>6}  {:<12} Apple on-device model (macOS 26+, 4096 ctx)",
                    "apple", "built-in", status, width = max_name);
                println!();
            }

            println!("Local models:\n");
            for m in &registry {
                let is_installed = installed.iter().any(|f| f == m.filename);
                let is_active = m.name == active;
                let status = match (is_installed, is_active) {
                    (true, true) => "✓ active",
                    (true, false) => "✓ installed",
                    _ => "  available",
                };
                println!("  {:<width$}  {:>6}  {:<12} {}",
                    m.name, m.size, status, m.description,
                    width = max_name);
            }

            println!("\nGitHub Models (remote):{}\n",
                if has_github { "" } else { "  ⚠ run `phil auth github` to enable" });
            for m in &gh_models {
                let is_active = m.name == active;
                let status = if is_active { "✓ active" } else if has_github { "  ready" } else { "  locked" };
                println!("  {:<width$}  {:>6}  {:<12} {} [{}]",
                    m.name, "cloud", status, m.description, m.provider,
                    width = max_name);
            }
            Ok(())
        }
        ModelAction::Install { name } => {
            if name == "apple" {
                return Err("apple is a built-in on-device model — no install needed. Just run: phil model use apple".into());
            }
            if find_github_model(&name).is_some() {
                return Err(format!("{name} is a cloud model — no install needed. Just run: phil model use {name}").into());
            }
            mgr.install_model(&name).await?;
            println!("\nUse this model with: phil model use {name}");
            Ok(())
        }
        ModelAction::Use { name } => {
            // Check all registries: apple, local, GitHub
            let is_apple = name == "apple";
            let is_local = find_model(&name).is_some();
            let is_github = find_github_model(&name).is_some();

            if !is_apple && !is_local && !is_github {
                return Err(format!("unknown model: {name}. Run `phil model ls`.").into());
            }

            if is_apple && !apple::apple_available() {
                eprintln!("Warning: Apple Intelligence is not currently available on this system.");
                eprintln!("Requires macOS 26+ with Apple Intelligence enabled and phil-apple helper installed.");
            }

            if is_github {
                // Verify we have a token
                if github::load_token().is_err() {
                    return Err("no GitHub token configured. Run `phil auth github` first.".into());
                }
            }

            let mut cfg = config::load_config()?;
            cfg.model.active = name.clone();
            config::save_config(&cfg)?;
            if is_apple {
                println!("Active model set to: {name} (Apple Intelligence, on-device)");
            } else if is_github {
                println!("Active model set to: {name} (GitHub Models API)");
            } else {
                println!("Active model set to: {name} (local)");
                println!("Restart the daemon for changes to take effect: kill $(cat ~/.phil/phil.pid)");
            }
            Ok(())
        }
    }
}

fn handle_config_action(action: ConfigAction) -> Result<(), Box<dyn std::error::Error>> {
    match action {
        ConfigAction::Show => {
            let cfg = config::load_config()?;
            let toml_str = toml::to_string_pretty(&cfg)
                .map_err(|e| format!("serialize: {e}"))?;
            let path = config::config_path()?;
            if path.exists() {
                println!("# {}", path.display());
            } else {
                println!("# (using defaults, no config file yet)");
            }
            println!("{toml_str}");
            Ok(())
        }
        ConfigAction::Init => {
            let path = config::init_config()?;
            println!("Config file at: {}", path.display());
            Ok(())
        }
        ConfigAction::Set { key, value } => {
            let mut cfg = config::load_config()?;
            let display_val = value.clone();
            match key.as_str() {
                "model.active" => cfg.model.active = value,
                "model.path" => cfg.model.path = Some(value),
                "daemon.idle_timeout" => {
                    cfg.daemon.idle_timeout = value.parse()
                        .map_err(|_| "invalid number for idle_timeout")?;
                }
                "daemon.disabled" => {
                    cfg.daemon.disabled = value.parse()
                        .map_err(|_| "expected true or false")?;
                }
                "defaults.temperature" => {
                    cfg.defaults.temperature = value.parse()
                        .map_err(|_| "invalid number for temperature")?;
                }
                "defaults.max_tokens" => {
                    cfg.defaults.max_tokens = value.parse()
                        .map_err(|_| "invalid number for max_tokens")?;
                }
                _ => return Err(format!("unknown config key: {key}. Valid keys: model.active, model.path, daemon.idle_timeout, daemon.disabled, defaults.temperature, defaults.max_tokens").into()),
            }
            config::save_config(&cfg)?;
            println!("Set {key} = {display_val}");
            Ok(())
        }
    }
}

async fn handle_auth(provider: &str) -> Result<(), Box<dyn std::error::Error>> {
    match provider {
        "github" => {
            println!("GitHub Models authentication");
            println!("────────────────────────────");
            println!();
            println!("Phil uses GitHub Models API to access cloud models (GPT-4o, Llama, etc.).");
            println!("You need a GitHub Personal Access Token (PAT) with the `models:read` scope.");
            println!();
            println!("Create one at: https://github.com/settings/tokens?type=beta");
            println!("  → Fine-grained token → Resource owner: your account");
            println!("  → Permissions: Models → Read");
            println!();

            // Read token from stdin
            eprint!("Paste your token: ");
            let mut token = String::new();
            io::stdin().read_line(&mut token)?;
            let token = token.trim();

            if token.is_empty() {
                return Err("no token provided".into());
            }

            // Validate the token
            eprint!("Validating...");
            match github::validate_token(token).await {
                Ok(username) => {
                    eprintln!(" ✓ authenticated as {username}");
                    github::save_token(token)?;
                    println!("\nToken saved. You can now use GitHub models:");
                    println!("  phil model use gpt-4o");
                    println!("  phil model ls");
                    Ok(())
                }
                Err(e) => {
                    eprintln!(" ✗");
                    Err(format!("authentication failed: {e}").into())
                }
            }
        }
        _ => Err(format!("unknown provider: {provider}. Supported: github").into()),
    }
}

/// Spawn the daemon as a detached background process.
fn spawn_daemon() -> Result<(), Box<dyn std::error::Error>> {
    let exe = std::env::current_exe()?;
    eprintln!("phil: starting daemon...");
    let mut cmd = std::process::Command::new(exe);
    cmd.arg("--daemon");
    cmd.stdin(std::process::Stdio::null());
    cmd.stdout(std::process::Stdio::null());
    cmd.stderr(std::process::Stdio::null());

    // On Unix, detach the daemon from this process group
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }

    cmd.spawn()?;
    Ok(())
}
