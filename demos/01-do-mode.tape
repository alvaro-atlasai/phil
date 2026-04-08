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
Type "clear"
Enter
Sleep 500ms
Show

Type `cd /tmp && mkdir -p phil-demo && cd phil-demo`
Enter
Sleep 500ms

Type `phil --do "create a new rust project called hello-world"`
Enter
Sleep 5s

Type "y"
Enter
Sleep 3s

Type "ls hello-world/src/"
Enter
Sleep 1.5s

Type `phil --do "find all rust files in the current directory"`
Enter
Sleep 5s

Type "y"
Enter
Sleep 2s

Type "rm -rf /tmp/phil-demo"
Enter
Sleep 500ms
