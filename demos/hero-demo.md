# Phil Hero Demo — Script

> **Goal:** Show why phil is the tool you want on every machine.  
> **Tone:** Casual, fast, developer-to-developer.  
> **Total time:** ~3–4 minutes.

---

## Before you start

```bash
# Warm the daemon (do this off-camera, takes ~8s first time)
echo '{}' | phil "echo"

# Create a scratch repo for the commit/agent demos
rm -rf /tmp/phil-hero && mkdir /tmp/phil-hero && cd /tmp/phil-hero
git init -q && git commit --allow-empty -m "init" -q
printf 'fn main() {\n    println!("hello");\n}\n' > main.rs
git add main.rs
clear
```

---

## ACT 1 — Pipe anything through AI

**What to say:** "Phil is a single binary that turns your shell into an AI pipeline. Pipe anything in, describe what you want in plain English, get the answer on stdout."

```bash
echo '{"users":[{"name":"Alice","active":true},{"name":"Bob","active":false}]}' | phil "active user names, one per line"
```

**Then:** "It works on unstructured text too — incident logs, error messages, whatever."

```bash
echo "The deploy failed at 02:14 UTC due to OOM on node-3, auto-healed at 02:16" | phil @json
```

**Beat:** Let the JSON output land. "Structured output from a sentence. No jq. No regex."

---

## ACT 2 — Packs: reusable AI prompts

**What to say:** "Phil ships with 12 packs — think of them as reusable prompt recipes you call with @name."

```bash
phil pack ls
```

**Then:** "The one I use every day — @commit. Give it a diff, get a conventional commit."

```bash
git diff --cached | phil @commit
```

**Beat:** Let the commit message appear. "That's it. One pipe."

**Optional deeper cut:** "You can create your own packs in seconds."

```bash
phil pack gen "redact PII — replace emails, UUIDs, IPs with ***"
```

---

## ACT 3 — --do: AI generates and runs commands

**What to say:** "Sometimes you don't want to transform text — you want phil to do something. --do generates a shell command and asks before running it."

```bash
phil --do "list all rust files in /tmp/phil-hero"
```

**Then:** Type `y` to confirm.

**Beat:** "It shows you the command first. You approve it. No magic — just a faster way to remember syntax."

---

## ACT 4 — --agent: packs become tools

**What to say:** "This is the new one. --agent mode lets the model call packs as tools. It decides which ones to use, calls them, and combines the results."

```bash
git diff --cached | phil --agent "review this diff and write a conventional commit message"
```

**Beat:** Watch it call @review, then @commit. "It picked the right packs, called them in order, and gave me the final answer."

---

## ACT 5 — MCP: expose packs to other agents

**What to say:** "And if you use Claude, Copilot, or Cursor — phil packs become MCP tools with one command."

```bash
phil pack export | head -12
```

**Beat:** "That's an any2mcp manifest. Point your MCP client at it and every pack is callable."

---

## ACT 6 — Speed

**What to say:** "All of this runs on a local model — Phi-4-mini. The daemon keeps it warm. Watch the latency."

```bash
time echo "hello" | phil "reply in one word" 2>&1
```

```bash
time echo "hello" | phil "reply in one word" 2>&1
```

**Beat:** "~160ms. Fast enough to put in a pipe, a git hook, a CI step — anywhere."

---

## Closing

**What to say:** "One binary. No API keys required. Runs offline. Pipe anything through AI."

```bash
# Show the repo link
echo "github.com/alvaro-atlasai/phil"
```

---

## Cheat sheet if things go wrong

| Problem | Fix |
|---|---|
| Slow first response | Daemon wasn't warm — `echo '{}' \| phil "echo"` and wait 8s |
| `@commit` says "no diff" | You forgot `git add` — re-run the setup block |
| `--agent` loops too long | It runs max 5 rounds — let it finish, or Ctrl-C and retry |
| `--do` generates wrong cmd | Just type `n` to reject, try rephrasing |
