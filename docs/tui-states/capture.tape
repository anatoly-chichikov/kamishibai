Output live/capture.gif

Set Shell "bash"
Set FontSize 36
Set Width 2250
Set Height 1688
Set Padding 27
Set Theme { "name": "kamishibai", "black": "#0e0e10", "red": "#e6e3da", "green": "#e6e3da", "yellow": "#e6e3da", "blue": "#e6e3da", "purple": "#e6e3da", "cyan": "#e6e3da", "white": "#e6e3da", "brightBlack": "#5a5953", "brightRed": "#e6e3da", "brightGreen": "#e6e3da", "brightYellow": "#e6e3da", "brightBlue": "#e6e3da", "brightPurple": "#e6e3da", "brightCyan": "#e6e3da", "brightWhite": "#e6e3da", "background": "#0e0e10", "foreground": "#e6e3da", "selection": "#1c1c1f", "cursor": "#e6e3da" }
Set TypingSpeed 35ms
Set WaitTimeout 60s

Hide
Type "cd ../.. && rm -rf /tmp/kamishibai-vhs-data /tmp/kamishibai-vhs-home/Documents/Kamishibai /tmp/kamishibai-vhs-cache/cards /tmp/kamishibai-vhs-cache/understanding /tmp/kamishibai-vhs-cache/sessions && mkdir -p /tmp/kamishibai-vhs-home/Documents/Kamishibai && clear"
Enter
Type "unset NO_COLOR; export TERM=xterm-256color COLORTERM=truecolor KAMISHIBAI_DATA=/tmp/kamishibai-vhs-data KAMISHIBAI_CACHE=/tmp/kamishibai-vhs-cache KAMISHIBAI_OUTPUT=/tmp/kamishibai-vhs-home/Documents/Kamishibai"
Enter
Type "printenv GEMINI_API_KEY | ./target/release/kamishibai config --known en --key - >/dev/null"
Enter
Type "clear"
Enter
Type "./target/release/kamishibai"
Enter
Show

Wait+Screen /words you want to learn/
Sleep 400ms
Type "dépaysement"
Enter
Type "flâner"
Enter
Type "canard"
Enter
Type "chouette"
Enter
Type "râler"
Enter
Type "terroir"
Enter
Type "bof"
Sleep 700ms
Screenshot "live/01-your-words.png"

Ctrl+G

Wait+Screen /understanding your words/
Sleep 250ms
Screenshot "live/01b-busy.png"

Wait+Screen@2m /—/
Sleep 2s
Screenshot "live/02-what-i-understood.png"
Sleep 400ms

# smooth tick-tick navigation down to canard (row 3)
Down
Sleep 800ms
Down
Sleep 900ms
Screenshot "live/02a-nav.png"

# open canard senses, walk to the last one, toggle it on (the model picks the
# senses, so which figurative meaning lands there varies between runs)
Enter
Sleep 1500ms
Screenshot "live/03-senses.png"
Down
Sleep 700ms
Down
Sleep 700ms
Down
Sleep 700ms
Space
Sleep 2s
Screenshot "live/03b-senses-toggled.png"
Enter
Sleep 1500ms

# one more nav tick so the motion reads as deliberate
Down
Sleep 800ms

Ctrl+G

Wait+Screen@5m /(building|your cards)/
Sleep 6s
Screenshot "live/04-your-cards.png"

Wait+Screen@15m /(all done|make it)/
Sleep 1500ms
Screenshot "live/08-done.png"

# final beat: walk up from the last built card to chouette, raise it by exactly one CEFR step,
# capture the collapsed result for the gif, then reopen it for separate detail screenshots
Up
Sleep 650ms
Up
Sleep 650ms
Up
Sleep 750ms
Enter
Sleep 1400ms
Down
Sleep 650ms
Down
Sleep 650ms
Down
Sleep 900ms
Screenshot "live/09-card-adjusting.png"

Right
Sleep 1500ms
Screenshot "live/09a-level-raised.png"

Wait+Screen /1 pending/
Enter
Sleep 1600ms
Screenshot "live/09d-card-staged.png"

Ctrl+G
Wait+Screen@30s /drawing each card/
Sleep 800ms
Screenshot "live/09b-card-regenerating.png"

Wait+Screen@15m /all done/
Sleep 1500ms
Screenshot "live/09c-card-regenerated.png"
Enter
Sleep 900ms
Screenshot "live/09-card-open.png"

Ctrl+N
Sleep 180ms
Ctrl+N
Sleep 180ms
Ctrl+N
Sleep 180ms
Ctrl+N
Sleep 180ms
Ctrl+N
Sleep 180ms
Ctrl+N
Sleep 180ms
Ctrl+N
Sleep 1200ms
Screenshot "live/10-card-scroll-end.png"
Sleep 600ms

Ctrl+C
Sleep 250ms
Ctrl+C
