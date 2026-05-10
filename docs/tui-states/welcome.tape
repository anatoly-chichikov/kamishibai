Output welcome-throwaway.gif

Set Shell "bash"
Set FontSize 14
Set Width 1152
Set Height 864
Set Padding 24
Set Theme { "name": "kamishibai", "black": "#0e0e10", "red": "#e6e3da", "green": "#e6e3da", "yellow": "#e6e3da", "blue": "#e6e3da", "purple": "#e6e3da", "cyan": "#e6e3da", "white": "#e6e3da", "brightBlack": "#5a5953", "brightRed": "#e6e3da", "brightGreen": "#e6e3da", "brightYellow": "#e6e3da", "brightBlue": "#e6e3da", "brightPurple": "#e6e3da", "brightCyan": "#e6e3da", "brightWhite": "#e6e3da", "background": "#0e0e10", "foreground": "#e6e3da", "selection": "#1c1c1f", "cursor": "#e6e3da" }
Set TypingSpeed 35ms

Hide
Type "cd /Users/anatoly/Source/kamishibai && clear"
Enter
Show

Type "./target/release/examples/tui_states"
Enter
Sleep 3s
Screenshot "live/00-welcome.png"
Sleep 300ms
Type "q"
