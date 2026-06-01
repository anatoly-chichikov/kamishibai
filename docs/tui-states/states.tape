Output states-throwaway.gif

Set Shell "bash"
Set FontSize 28
Set Width 2304
Set Height 1728
Set Padding 48
Set Theme { "name": "kamishibai", "black": "#0e0e10", "red": "#e6e3da", "green": "#e6e3da", "yellow": "#e6e3da", "blue": "#e6e3da", "purple": "#e6e3da", "cyan": "#e6e3da", "white": "#e6e3da", "brightBlack": "#5a5953", "brightRed": "#e6e3da", "brightGreen": "#e6e3da", "brightYellow": "#e6e3da", "brightBlue": "#e6e3da", "brightPurple": "#e6e3da", "brightCyan": "#e6e3da", "brightWhite": "#e6e3da", "background": "#0e0e10", "foreground": "#e6e3da", "selection": "#1c1c1f", "cursor": "#e6e3da" }
Set TypingSpeed 35ms
Set WaitTimeout 60s

Hide
Type "cd /Users/anatoly/Source/kamishibai && clear"
Enter
Type "./target/release/examples/tui_states"
Enter
Show

# The walker advances one state per keypress. Use generous, uniform 800ms sleeps
# between keys: short/uneven sleeps coalesce keystrokes and the index drifts.

# index 0 — Welcome (API-key setup gate)
Sleep 1500ms
Screenshot "live/00-welcome.png"

# index 4 — ChangeSomething modal over WhatIUnderstood (4 keypresses from index 0)
Enter
Sleep 800ms
Enter
Sleep 800ms
Enter
Sleep 800ms
Enter
Sleep 800ms
Screenshot "live/03-change-something-modal.png"

# index 6 — ChangeThisCard modal over YourCards
Enter
Sleep 800ms
Enter
Sleep 800ms
Screenshot "live/05-change-this-card-modal.png"

# index 7 — YourCards with inline retry indicators
Enter
Sleep 800ms
Screenshot "live/06-your-cards-retrying.png"

# index 8 — YourCards couldn't finish (terminal failure)
Enter
Sleep 800ms
Screenshot "live/07-your-cards-couldnt-finish.png"

Type "q"
Sleep 250ms
