Output states-throwaway.gif

Set Shell "bash"
Set FontSize 36
Set Width 2250
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

# index 4 — WhatIUnderstood with the closed batch sentence summary
Type "4"
Space
Sleep 800ms
Screenshot "live/02-what-i-understood.png"

# index 5 — ChangeSomething modal over WhatIUnderstood
Type "5"
Space
Sleep 800ms
Screenshot "live/03-change-something-modal.png"

# index 6 — S1: three collapsed tags continue inline after the audio status
Type "6"
Space
Sleep 800ms
Screenshot "live/11-s1-label-tags.png"

# index 7 — S2: editor opens below the complete artifact block
Type "7"
Space
Sleep 800ms
Screenshot "live/12-s2-label-editor.png"

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

# index 11 — S3: pending register remains visible in the below-artifacts editor
Type "11"
Space
Sleep 800ms
Screenshot "live/13-s3-label-pending-register.png"

# index 12 — S4: pending note continues below the editor axes
Type "12"
Space
Sleep 800ms
Screenshot "live/14-s4-label-pending-note.png"

# index 13 — S5: collapsed defaults restored automatically clear pending
Type "13"
Space
Sleep 800ms
Screenshot "live/15-s5-label-restored.png"

# index 14 — S6: two collapsed cards accumulate independently before regeneration
Type "14"
Space
Sleep 800ms
Screenshot "live/16-s6-label-multiple-pending.png"

# index 15 — S7: Ctrl+G activates the accumulated batch
Type "15"
Space
Sleep 800ms
Screenshot "live/17-s7-label-regenerating.png"

# index 16 — S8: pinned tags stay audio-anchored despite picture rejections
Type "16"
Space
Sleep 800ms
Screenshot "live/18-s8-label-regenerated.png"

# index 17 — S9: collapsed card shows an approximately fulfilled register pin
Type "17"
Space
Sleep 800ms
Screenshot "live/19-s9-label-approx.png"

# index 19 — S11: mouse-selected chip in the below-artifacts editor
Type "19"
Space
Sleep 800ms
Screenshot "live/21-s11-label-mouse-selection.png"

# index 20 — S12: legacy metadata opens a below-artifacts editor with no active chips
Type "20"
Space
Sleep 800ms
Screenshot "live/22-s12-label-legacy-meta.png"

# index 21 — active audio plus retry, recovery and terminal rows as a layout stress test
Type "21"
Space
Sleep 800ms
Screenshot "live/06b-your-cards-retry-stress.png"

# index 22 — first Esc on nonempty YourWords arms a guarded clear
Type "22"
Space
Sleep 800ms
Screenshot "live/23-esc-words-clear.png"

# index 23 — collapsed review advertises the harmless Esc back action
Type "23"
Space
Sleep 800ms
Screenshot "live/24-esc-review-back.png"

# index 24 — first Esc during generation arms a guarded stop
Type "24"
Space
Sleep 800ms
Screenshot "live/25-esc-generation-stop.png"

# index 25 — confirmed stop drains the current artifact
Type "25"
Space
Sleep 800ms
Screenshot "live/26-generation-stopping.png"

# index 26 — completed subset is published as a settled partial batch
Type "26"
Space
Sleep 800ms
Screenshot "live/27-generation-partial.png"

# index 27 — generation guidance retains b1 + questions in both inline carousels
Type "27"
Space
Sleep 800ms
Screenshot "live/28-batch-sentence-settings.png"
Sleep 250ms

Type "q"
Sleep 250ms
