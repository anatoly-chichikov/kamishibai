Output live/capture.gif

Set Shell "bash"
Set FontSize 14
Set Width 1152
Set Height 864
Set Padding 24
Set Theme { "name": "kamishibai", "black": "#0e0e10", "red": "#e6e3da", "green": "#e6e3da", "yellow": "#e6e3da", "blue": "#e6e3da", "purple": "#e6e3da", "cyan": "#e6e3da", "white": "#e6e3da", "brightBlack": "#5a5953", "brightRed": "#e6e3da", "brightGreen": "#e6e3da", "brightYellow": "#e6e3da", "brightBlue": "#e6e3da", "brightPurple": "#e6e3da", "brightCyan": "#e6e3da", "brightWhite": "#e6e3da", "background": "#0e0e10", "foreground": "#e6e3da", "selection": "#1c1c1f", "cursor": "#e6e3da" }
Set TypingSpeed 35ms
Set WaitTimeout 60s

Hide
Type "cd /Users/anatoly/Source/kamishibai && rm -rf kamishibai-out && clear"
Enter
Type "./target/release/kamishibai"
Enter
Show

Wait+Screen /words you want to learn/
Sleep 400ms
Type "lantern"
Enter
Type "ghosting"
Enter
Type "serendipity"
Enter
Type "cold feet"
Enter
Type "bark"
Enter
Type "spoiler"
Enter
Type "bittersweet"
Sleep 600ms
Screenshot "live/01-your-words.png"

Ctrl+G

Wait+Screen /understanding your words/
Sleep 250ms
Screenshot "live/01b-busy.png"

Wait+Screen@2m /—/
Sleep 15s
Screenshot "live/02-what-i-understood.png"

Down 4
Sleep 2s
Type "r"
Sleep 2s
Screenshot "live/03-change-something-modal.png"
Type "в значении кора дерева"
Sleep 1s
Enter

Sleep 1.5s
Screenshot "live/01c-busy-correction.png"

Sleep 12s
Screenshot "live/02b-what-i-understood-corrected.png"

Ctrl+G

Wait+Screen@5m /your cards/
Sleep 8s
Screenshot "live/04-your-cards.png"
Sleep 25s
Screenshot "live/04b-your-cards-mid.png"

Wait+Screen@15m /(all done|make it)/
Sleep 800ms
Screenshot "live/08-done.png"

Ctrl+C
Sleep 250ms
Ctrl+C
