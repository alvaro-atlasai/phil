# Demo 6: speed — daemon warmth (WOW FACTOR: ★★★☆☆)
# Shows the ~160ms response time that makes pipes viable

Output demos/speed.gif
Set FontSize 18
Set Width 1000
Set Height 450
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

Type `time echo "hello" | phil "reply in one word" 2>&1`
Enter
Sleep 4s

Type `time echo "hello" | phil "reply in one word" 2>&1`
Enter
Sleep 4s

Type `time echo "hello" | phil "reply in one word" 2>&1`
Enter
Sleep 4s
