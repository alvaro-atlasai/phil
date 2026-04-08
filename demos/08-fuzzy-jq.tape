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
Type "clear"
Enter
Sleep 500ms
Show

# Real API: weather — plain English question about nested JSON
Type `curl -s wttr.in/London?format=j1 | phil "current temperature and wind speed, one line"`
Enter
Sleep 5s

Type ""
Enter
Sleep 500ms

# Real API: GitHub — ask about repos without remembering jq syntax
Type `curl -s https://api.github.com/orgs/charmbracelet/repos?per_page=5 | phil "repo names and star counts, most stars first"`
Enter
Sleep 5s

Type ""
Enter
Sleep 500ms

# Real API: earthquake data — summarize a complex feed
Type `curl -s https://earthquake.usgs.gov/earthquakes/feed/v1.0/summary/significant_month.geojson | phil "list recent significant earthquakes: location, magnitude, date"`
Enter
Sleep 5s
