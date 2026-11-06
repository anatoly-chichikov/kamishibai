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
Show

# Each capture launches one prepared scene by stable ID, waits for the AI reminder every
# scene draws, repaints with Ctrl+L, and holds the scene on screen around the shot,
# because VHS writes a Screenshot from recorded frames that lag behind the tape.

# cards.narrow — S10: whole tags wrap onto later artifact rows; impossible summaries hide
Type "./target/release/examples/tui_states --state cards.narrow"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/20-s10-label-tags-narrow.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# review.guidance — generation guidance wraps with the same mouse geometry at narrow width
Type "./target/release/examples/tui_states --state review.guidance"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/29-batch-sentence-settings-narrow.png"
Sleep 2500ms
Type "q"
Sleep 500ms
