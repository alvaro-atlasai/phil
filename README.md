# phil

A unix tool that operates on **meaning**, not structure. Like `sed`/`awk`/`jq`, but semantic.

Single static Rust binary. Zero config. Local Phi-4-mini with auto-managed daemon for low-latency pipeline use (~160ms/call).

## Install

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
curl api.example.com/users | phil "names of active users, one per line"
kubectl get pods -o json | phil "which pods are crashlooping and why?"
```

Nobody remembers `jq '.[] | select(.status == "active") | .name'`. Phil replaces jq for ad-hoc exploration.

### Git commit messages

```bash
git diff --staged | phil "conventional commit message, subject only"
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
cat schema.sql | phil "5 realistic INSERT statements" | sqlite3 test.db
```

### Line-by-line processing with `--each`

`--each` processes each stdin line as a separate inference call — like a semantic `sed`:

```bash
# Classify log lines
tail -f app.log | phil --each "classify: CRITICAL/WARN/OK — one word"

# Translate line by line
cat words.txt | phil --each "translate to Spanish, just the translation"

# Filter suspicious URLs
cat urls.txt | phil --each "is this URL suspicious? YES or NO" | paste - urls.txt | grep YES
```

~165ms per line with the daemon running. Only viable because the daemon eliminates model reload.

### Pipeline-scale transforms

```bash
find . -name '*.log' | xargs -I{} sh -c 'phil "one-line summary" < {} > {}.summary'
ls *.py | xargs -I{} sh -c 'phil "list public functions, one per line" < {}'
```

### Generate commands instead of memorizing them

```bash
cat payload.json | phil "curl command to POST this to https://api.example.com/v2/users with bearer token \$TOKEN"
```

## Packs

Packs are reusable prompt configs — a single TOML file that turns phil into a specialized tool.

```bash
git diff --staged | phil @commit
# → feat(packs): add reusable prompt configuration system

target/release/phil @explain "what does set -euo pipefail do"
# → -e: exit on error  -u: error on unset vars  -o pipefail: fail pipeline on any error

cat data.csv | phil @json
# → [{"name": "John", "age": 30}, ...]

gh --help | phil @mcp
# → YAML manifest for any2mcp
```

### Built-in packs

```
$ phil pack ls
  @commit   Conventional commit from staged diff
  @explain  Explain a command or concept concisely
  @json     Convert any input to JSON
  @mcp      Generate any2mcp manifest from --help output
  @review   Code review from a diff
  @tldr     Summarize man pages or docs
```

### Create your own

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
      --each               Process each stdin line separately (semantic sed)
      --no-daemon          Skip the daemon, load model directly
  -h, --help               Print help
  -V, --version            Print version

Subcommands:
  phil pack ls             List all packs
  phil pack init <name>    Create a new pack from template
  phil pack add <url>      Install a pack from URL
  phil pack show <name>    Show pack details
```

## Performance

Benchmarks on Apple M4 Pro:

| Mode | Latency |
|------|---------|
| Direct (`--no-daemon`) | ~550ms |
| Daemon cold start | ~590ms |
| Daemon warm | **~160ms** |

The daemon eliminates model reload, making phil viable inside `xargs`/`find` loops.

## any2mcp

Companion tool that auto-generates [MCP](https://modelcontextprotocol.io/) server wrappers from any CLI's `--help` output:

```bash
cargo install --path any2mcp
any2mcp init gh       # scans `gh --help`, generates tool manifest
any2mcp serve gh      # starts MCP stdio server for `gh`
```

Useful for exposing arbitrary CLIs to AI agents.

## Architecture

```
phil-core/     Shared inference engine (llama-cpp-2 + Phi-4-mini)
phil/          CLI binary — pipes, prompts, daemon
any2mcp/       CLI-to-MCP generator
```

Both binaries are self-contained (~8MB each) with llama.cpp statically linked. No Python, no Docker, no API keys.
