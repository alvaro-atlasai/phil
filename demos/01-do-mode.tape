# Demo 1: "The --do" demo (WOW FACTOR: ★★★★★)
# Shows phil as a copilot for one-liner tasks

Output demos/do-mode.gif
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
# Clean slate + warm daemon
Type `rm -rf /tmp/phil-demo && mkdir /tmp/phil-demo && cd /tmp/phil-demo`
Enter
Type `echo '{}' | phil "echo"`
Enter
Sleep 8s
Type "clear"
Enter
Sleep 500ms
Show

Type `phil --do "create a new rust project called hello-world"`
Enter
Sleep 5s

Type "y"
Enter
Sleep 3s

Type `phil --do "find all rust files in the current directory"`
Enter
Sleep 5s

Type "y"
Enter
Sleep 2s

Hide
Type "rm -rf /tmp/phil-demo"
Enter
Sleep 500ms
