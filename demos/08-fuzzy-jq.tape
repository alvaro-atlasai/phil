# Demo 8: Fuzzy jq — ask questions about JSON
# Nobody remembers jq syntax. Phil understands the question.

Output demos/fuzzy-jq.gif
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
# Warm the daemon before recording
Type `echo '{}' | phil "echo"`
Enter
Sleep 8s
Type "clear"
Enter
Sleep 500ms
Show

# Real API: weather — plain English question about JSON
Type `curl -s 'https://api.open-meteo.com/v1/forecast?latitude=51.5&longitude=-0.1&current_weather=true' | phil "current temperature and wind speed in London, one line"`
Enter
Sleep 5s

Type ""
Enter
Sleep 500ms

# Real API: ISS position — ask a plain English question
Type `curl -s 'http://api.open-notify.org/iss-now.json' | phil "where is the ISS right now? one sentence"`
Enter
Sleep 5s

Type ""
Enter
Sleep 500ms

# Real API: random users — extract and format
Type `curl -s 'https://randomuser.me/api/?results=3&inc=name,email&noinfo' | phil "list names and emails, one per line"`
Enter
Sleep 5s
