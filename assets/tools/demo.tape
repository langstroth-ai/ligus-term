# The README GIF: lt --demo starts a workflow run, its stages finish, and
# the sign-off gate is approved. The demo backend only (in memory), with a
# throwaway data dir and the file secret backend. From the repo root:
#
#   cargo build --locked && vhs assets/tools/demo.tape
#   gifsicle -O3 --lossy=40 -b assets/lt-demo.gif   # optional, smaller
#
# Colours: the brand's dark tile behind lt's dark theme.

Output assets/lt-demo.gif

Set Shell "bash"
Set FontFamily "Menlo"
Set FontSize 13
Set Width 1000
Set Height 560
Set Padding 14
Set Framerate 12
Set TypingSpeed 90ms
Set Theme { "name": "ligus", "background": "#1a1206", "foreground": "#f4f1ea", "cursor": "#f0c25a", "selection": "#3a2812", "black": "#1a1206", "red": "#ef6b5b", "green": "#8cc86b", "yellow": "#f0c25a", "blue": "#7fb4e0", "magenta": "#b79ce8", "cyan": "#7fb4e0", "white": "#f4f1ea", "brightBlack": "#8a8073", "brightRed": "#ef6b5b", "brightGreen": "#8cc86b", "brightYellow": "#f0c25a", "brightBlue": "#7fb4e0", "brightMagenta": "#b79ce8", "brightCyan": "#7fb4e0", "brightWhite": "#ffffff" }

Hide
Type "export LT_DATA_DIR=$(mktemp -d) LIGUS_SECRET_BACKEND=file PATH=$PWD/target/debug:$PATH PS1='$ '; clear" Enter
Sleep 300ms
Show

Type "lt --demo --theme dark" Sleep 400ms Enter
Sleep 1.5s
# Workflows, then start a run of "Implement, review, ship".
Type "f" Sleep 1.5s
Type "S" Sleep 800ms
Type "export the wash counts" Sleep 600ms
Enter
# The stages run on the demo's agents, then the gate waits.
Sleep 3.5s
# Approve the sign-off gate.
Type "A"
Sleep 2.5s
Type "q"
Sleep 500ms
Hide
Type "rm -rf $LT_DATA_DIR" Enter
