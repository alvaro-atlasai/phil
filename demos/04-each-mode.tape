# Demo 4: --each mode — semantic sed (WOW FACTOR: ★★★★☆)
# Unique to phil — no other tool does line-by-line semantic processing

Output demos/each-mode.gif
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

Type `printf "bonjour\nhola\nguten tag\nciao" | phil --each "what language? one word"`
Enter
Sleep 10s

Type ""
Enter
Sleep 500ms

Type `printf "WARN disk 92%%\nINFO request ok\nERROR segfault\nINFO heartbeat" | phil --each "classify: CRITICAL/WARN/OK — one word"`
Enter
Sleep 10s
