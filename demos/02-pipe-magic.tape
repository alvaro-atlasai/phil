# Demo 2: Pipe magic — phil as semantic jq/sed (WOW FACTOR: ★★★★☆)
# The core value prop: meaning, not structure

Output demos/pipe-magic.gif
Set FontSize 18
Set Width 1000
Set Height 600
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

Type `echo '{"users":[{"name":"Alice","active":true},{"name":"Bob","active":false}]}' | phil "names of active users"`
Enter
Sleep 4s

Type ""
Enter
Sleep 500ms

Type `echo "The server crashed at 3am due to OOM, restarted by watchdog at 3:02" | phil "timeline, one event per line"`
Enter
Sleep 4s

Type ""
Enter
Sleep 500ms

Type `echo 'name = "phil"' | phil "convert to JSON"`
Enter
Sleep 4s
