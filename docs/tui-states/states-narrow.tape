Output states-narrow-throwaway.gif

Set Shell "bash"
Set FontSize 36
Set Width 1200
Set Height 1688
Set Padding 27
Set Theme { "name": "kamishibai", "black": "#0e0e10", "red": "#e6e3da", "green": "#e6e3da", "yellow": "#e6e3da", "blue": "#e6e3da", "purple": "#e6e3da", "cyan": "#e6e3da", "white": "#e6e3da", "brightBlack": "#5a5953", "brightRed": "#e6e3da", "brightGreen": "#e6e3da", "brightYellow": "#e6e3da", "brightBlue": "#e6e3da", "brightPurple": "#e6e3da", "brightCyan": "#e6e3da", "brightWhite": "#e6e3da", "background": "#0e0e10", "foreground": "#e6e3da", "selection": "#1c1c1f", "cursor": "#e6e3da" }
Set TypingSpeed 35ms
Set WaitTimeout 60s

Hide
Type "cd ../.. && clear"
Enter
Type "unset NO_COLOR; export TERM=xterm-256color COLORTERM=truecolor"
Enter
Type "./target/release/examples/tui_states"
Enter
Show

# index 18 — S10: whole tags wrap onto later artifact rows; impossible summaries hide
Type "18"
Space
Sleep 800ms
Screenshot "live/20-s10-label-tags-narrow.png"

# index 27 — generation guidance wraps with the same mouse geometry at narrow width
Type "27"
Space
Sleep 800ms
Screenshot "live/29-batch-sentence-settings-narrow.png"
Sleep 250ms

Type "q"
Sleep 250ms
