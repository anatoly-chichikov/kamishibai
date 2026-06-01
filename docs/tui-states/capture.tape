Output live/capture.gif

Set Shell "bash"
Set FontSize 28
Set Width 2304
Set Height 1728
Set Padding 48
Set Theme { "name": "kamishibai", "black": "#0e0e10", "red": "#e6e3da", "green": "#e6e3da", "yellow": "#e6e3da", "blue": "#e6e3da", "purple": "#e6e3da", "cyan": "#e6e3da", "white": "#e6e3da", "brightBlack": "#5a5953", "brightRed": "#e6e3da", "brightGreen": "#e6e3da", "brightYellow": "#e6e3da", "brightBlue": "#e6e3da", "brightPurple": "#e6e3da", "brightCyan": "#e6e3da", "brightWhite": "#e6e3da", "background": "#0e0e10", "foreground": "#e6e3da", "selection": "#1c1c1f", "cursor": "#e6e3da" }
Set TypingSpeed 35ms
Set WaitTimeout 60s

Hide
Type "cd /Users/anatoly/Source/kamishibai && rm -rf kamishibai-out && clear"
Enter
Type "HOME=/tmp/khome ./target/release/kamishibai"
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

# smooth tick-tick navigation down to canard (row 3)
Down
Sleep 800ms
Down
Sleep 900ms
Screenshot "live/02a-nav.png"

# open canard senses, walk them, toggle the journalism "hoax" sense on
Enter
Sleep 1s
Screenshot "live/03-senses.png"
Down
Sleep 800ms
Space
Sleep 900ms
Down
Sleep 800ms
Screenshot "live/03b-senses-toggled.png"
Enter
Sleep 900ms

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

# final beat: step down to the canard "hoax / газетная утка" card (04) and linger
Down
Sleep 650ms
Down
Sleep 650ms
Down
Sleep 750ms
Enter
Sleep 3s
Screenshot "live/09-card-open.png"
Sleep 600ms

Ctrl+C
Sleep 250ms
Ctrl+C
