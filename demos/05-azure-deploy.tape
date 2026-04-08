# Demo 5: Azure + PII redaction (WOW FACTOR: ★★★★★)
# Shows phil as both a cloud CLI copilot AND a PII filter
# Safe to record — all sensitive output gets piped through phil for redaction
#
# Pre-requisites:
#   az login
#   phil daemon running (warm)

Output demos/azure-deploy.gif
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

# Act 1: create resource group, redact output live
Type `az group create --name phil-demo --location eastus -o json | phil "redact all UUIDs with '***', output only the JSON"`
Enter
Sleep 8s

Type ""
Enter
Sleep 500ms

# Act 2: show resource group, redact again
Type `az group show --name phil-demo -o json | phil "redact all UUIDs and subscription paths with '***', keep JSON structure, output only JSON"`
Enter
Sleep 8s

Type ""
Enter
Sleep 500ms

# Act 3: clean up
Hide
Type `az group delete --name phil-demo --yes --no-wait 2>/dev/null`
Enter
Sleep 2s
Show
