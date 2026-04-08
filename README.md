# phil

**Pipe anything through AI.**

A Unix tool that operates on **meaning**, not structure. Like `sed`/`awk`/`jq`, but semantic. Talk to your files, logs, APIs, cloud — anything that flows through a pipe.

Single static Rust binary. Zero config. Local Phi-4-mini with auto-managed daemon (~160ms/call). Optional cloud models via GitHub Models API.

![phil pipe demo](demos/pipe-magic.gif)

## Install

```bash
curl -fsSL https://raw.githubusercontent.com/alvaro-atlasai/phil/main/install.sh | sh
```

Or build from source:

```bash
cargo install --path phil
```

First run downloads Phi-4-mini (~2.5GB) to `~/.phil/models/`. After that, it's fully offline.

## Usage

```bash
phil "your question or instruction"
cat file | phil "do something with this"
```

A background daemon auto-starts on first call and keeps the model loaded in memory. Shuts down after 5 minutes idle. Use `--no-daemon` to skip it.

## Examples

### Fuzzy jq — ask questions about JSON

```bash
curl -s wttr.in/London?format=j1 | phil "current temperature and wind speed, one line"
curl -s api.github.com/orgs/charmbracelet/repos | phil "repo names and star counts, most stars first"
```

Nobody remembers `jq '.[] | select(.status == "active") | .name'`. Phil replaces jq for ad-hoc exploration.

![fuzzy jq demo](demos/fuzzy-jq.gif)

### Git commit messages

```bash
git diff --staged | phil @commit
# → feat(daemon): add auto-managed unix socket for persistent model
```

### Log triage — not grep, understanding

```bash
tail -100 /var/log/app.log | phil "is this healthy? if not, one-line diagnosis"
# → Unhealthy: connection pool exhausted, 47 timeout errors in last 30s
```

Grep finds patterns. Phil finds **problems**.

### Format translation

```bash
cat config.yaml | phil "convert to TOML"
cat data.csv | phil "as JSON array"
echo 'name = "phil"' | phil "convert to JSON"
```

### Line-by-line processing with `--each`

`--each` processes each stdin line as a separate inference call — like a semantic `sed`:

```bash
printf "bonjour\nhola\nguten tag\nciao" | phil --each "what language? one word"
# → French
# → Spanish
# → German
# → Italian
```

![each mode demo](demos/each-mode.gif)

~160ms per line with the daemon running. Only viable because the daemon eliminates model reload.

### Generate and execute shell commands with `--do`

Natural language → shell command, with confirmation before execution:

```bash
phil --do "create a new rust project called hello-world"
#   cargo init hello-world
# Run this? [Y/n/e(dit)] y

phil --do "find all files larger than 10MB"
#   find . -size +10M -type f
# Run this? [Y/n/e(dit)]
```

![do mode demo](demos/do-mode.gif)

### Redact PII from cloud output

```bash
az group show -n myapp -o json | phil "redact all UUIDs and emails with '***', keep JSON"
```

![azure redact demo](demos/azure-deploy.gif)

## Packs

Packs are reusable prompt configs — a single TOML file that turns phil into a specialized tool.

```bash
git diff --staged | phil @commit      # conventional commit message
phil @explain "set -euo pipefail"     # concise explanation
cat data.csv | phil @json             # convert to JSON
gh --help | phil @mcp                 # generate MCP manifest
```

### Built-in packs

```
$ phil pack ls
  @az       Generate Azure CLI commands from natural language
  @commit   Conventional commit from staged diff
  @docker   Docker commands and Dockerfile help
  @explain  Explain a command or concept concisely
  @fabric   Microsoft Fabric / Power BI helper
  @json     Convert any input to JSON
  @k8s      Kubernetes troubleshooting and kubectl commands
  @mcp      Generate any2mcp manifest from --help output
  @review   Code review from a diff
  @tf       Terraform helper — generate HCL from descriptions
  @tldr     Summarize man pages or docs
```

### Create your own — use it, pack it, reuse it

See something useful? Turn it into a pack with one command:

```bash
phil pack gen "redact PII — replace UUIDs, emails, IPs with ***"
# → Generated @pii-redactor → ~/.phil/packs/pii-redactor.toml

# Now reuse it forever:
cat output.json | phil @pii-redactor
```

![pack flow demo](demos/pack-flow.gif)

Or create one manually:

```bash
phil pack init mypack
# Creates ~/.phil/packs/mypack.toml — edit the system prompt, then:
cat input | phil @mypack
```

A pack is just TOML:

```toml
name = "sql"
description = "Natural language to SQL"
system = """
Convert the input to a SQL query. Output only valid SQL.
No markdown fences. No explanation.
"""
temperature = 0.1
max_tokens = 512
```

### Share packs

```bash
# Install from a URL (gist, raw GitHub, etc.)
phil pack add https://gist.githubusercontent.com/.../sql.toml

# Or just copy the .toml file to ~/.phil/packs/
```

User packs in `~/.phil/packs/` override built-ins with the same name.

## Cloud Models

Use GitHub Models API for heavier tasks (GPT-4o, Llama 3.3 70B, etc.):

```bash
phil auth github                      # one-time setup with PAT
phil model use gpt-4o                 # switch to cloud
phil --do "deploy this to Azure"      # benefits from stronger reasoning
phil model use phi4-mini              # switch back to local
```

```
$ phil model ls
Local models:
  phi4-mini       2.3GB  ✓ active     Phi-4-mini-instruct (Q4_K_M)
  phi4-mini-q8    4.1GB    available   Phi-4-mini-instruct (Q8_0)
  qwen3-1.7b     1.4GB    available   Qwen3 1.7B (Q4_K_M)
  ...

GitHub Models (remote):
  gpt-4o          cloud    ready       GPT-4o [openai]
  o4-mini         cloud    ready       o4-mini reasoning [openai]
  llama-3.3-70b   cloud    ready       Llama 3.3 70B [meta]
  ...
```

Local for speed and privacy. Cloud for power. Same `phil` command either way.

## Performance

![speed demo](demos/speed.gif)

| Mode | Latency |
|------|---------|
| Direct (`--no-daemon`) | ~550ms |
| Daemon cold start | ~590ms |
| Daemon warm | **~160ms** |

The daemon eliminates model reload, making phil viable inside `xargs`/`find` loops.

## any2mcp

Companion tool: turn **any CLI** into an [MCP](https://modelcontextprotocol.io/) server. AI agents (Claude, VS Code Copilot, Cursor) can then call the CLI's commands as tools.

```bash
cargo install --path any2mcp
```

Pre-built manifests for Azure CLI and Fabric in [`examples/`](examples/):

```bash
any2mcp examples/az.yaml       # serve Azure CLI as MCP
any2mcp examples/fabric.yaml   # serve Fabric CLI as MCP
```

Or generate a manifest from any CLI:

```bash
gh --help | phil @mcp > gh.yaml
any2mcp gh.yaml
```

Configure in your MCP client (Claude Desktop, VS Code, etc.):

```json
{
  "mcpServers": {
    "azure-cli": { "command": "any2mcp", "args": ["az.yaml"] }
  }
}
```

## Options

```
phil [OPTIONS] <PROMPT>
phil @pack [additional prompt]

Options:
  -s, --system <SYSTEM>    Custom system prompt
      --raw                Don't add a system prompt
      --model <MODEL>      Path to a custom GGUF model file
      --max-tokens <N>     Maximum tokens to generate [default: 2048]
      --temperature <F>    Sampling temperature (0.0–1.0) [default: 0.1]
      --each               Process each stdin line separately
      --do                 Generate and execute shell commands
      --no-daemon          Skip the daemon, load model directly
  -h, --help               Print help
  -V, --version            Print version

Subcommands:
  pack ls|init|add|show|gen    Manage packs
  model ls|install|use         Manage models
  config show|init|set         Manage configuration
  auth <provider>              Authenticate (github)
```

## Architecture

```
phil-core/     Shared library: inference, daemon, packs, config, models, GitHub API
phil/          CLI binary — pipes, prompts, packs, --do, daemon
any2mcp/       Turn any CLI into an MCP server
```

Both binaries are self-contained (~8MB each) with llama.cpp statically linked. No Python, no Docker, no API keys required for local use.

## License

MIT
