# Demo 7: The full flow — use phil → pack it → reuse forever
# Story: "See something useful? Pack it."

Output demos/pack-flow.gif
Set FontSize 18
Set Width 1100
Set Height 600
Set Theme "Catppuccin Mocha"
Set TypingSpeed 35ms
Set Shell zsh
Set Padding 20

Hide
Type `export PATH="$HOME/.local/bin:$PATH"`
Enter
Type "clear"
Enter
Sleep 500ms
Show

# Act 1: ad-hoc use — phil redacts PII inline
Type `echo '{"user":"alice@corp.com","id":"a1b2c3d4-e5f6-7890-abcd-ef1234567890","ip":"10.0.1.42"}' | phil "redact PII, output only JSON"`
Enter
Sleep 5s

Type ""
Enter
Sleep 1s

# Act 2: that was useful — let's pack it
Type `phil pack gen "redact PII — replace UUIDs, emails, IPs with ***. Keep format. Output only redacted text."`
Enter
Sleep 8s

Type ""
Enter
Sleep 1s

# Act 3: now reuse it forever with @redact
Type `echo '{"token":"ghp_abc123secret","tenant":"f9e8d7c6-b5a4-3210-fedc-ba0987654321"}' | phil @pii-redactor`
Enter
Sleep 5s
