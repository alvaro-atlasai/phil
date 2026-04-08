# Demo 3: @commit pack — every developer needs this (WOW FACTOR: ★★★★☆)
# Uses a synthetic diff to avoid leaking real file contents

Output demos/commit-pack.gif
Set FontSize 18
Set Width 1000
Set Height 500
Set Theme "Catppuccin Mocha"
Set TypingSpeed 40ms
Set Shell zsh
Set Padding 20

Hide
Type `export PATH="$HOME/.local/bin:$PATH"`
Enter
Type "clear"
Enter
Sleep 500ms
Show

Type `printf 'diff --git a/src/auth.rs b/src/auth.rs\n--- a/src/auth.rs\n+++ b/src/auth.rs\n@@ -10,6 +10,12 @@\n fn login(user: &str) {\n+    let token = generate_jwt(user);\n+    set_cookie("session", &token);\n+    log::info!("user {} logged in", user);\n }' | phil @commit`
Enter
Sleep 4s
