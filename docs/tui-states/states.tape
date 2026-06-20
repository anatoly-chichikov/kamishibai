Output states-throwaway.gif

Set Shell "bash"
Set FontSize 48
Set Width 2250
Set Height 1688
Set Padding 36
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

# The walker uses ABSOLUTE navigation: `Type "<n>"` then `Space` jumps straight
# to state <n>, so a dropped or coalesced keystroke cannot accumulate across the
# run and the stray Return that launches the binary cannot contaminate the
# index. Each screenshot re-asserts its target index. Keep the uniform 800ms
# settle after the jump so VHS never captures a mid-repaint frame.

# index 0 — Welcome · key step with no GEMINI_API_KEY in env (submit only)
Type "0"
Space
Sleep 800ms
Screenshot "live/00-welcome.png"

# index 1 — Welcome · key step with GEMINI_API_KEY in env (load-from-env chip)
Type "1"
Space
Sleep 800ms
Screenshot "live/00b-welcome-env.png"

# index 5 — ChangeSomething modal over WhatIUnderstood
Type "5"
Space
Sleep 800ms
Screenshot "live/03-change-something-modal.png"

# index 7 — ChangeThisCard modal over YourCards
Type "7"
Space
Sleep 800ms
Screenshot "live/05-change-this-card-modal.png"

# index 8 — YourCards with inline retry indicators
Type "8"
Space
Sleep 800ms
Screenshot "live/06-your-cards-retrying.png"

# index 9 — YourCards couldn't finish (terminal failure)
Type "9"
Space
Sleep 800ms
Screenshot "live/07-your-cards-couldnt-finish.png"

Type "q"
Sleep 250ms
