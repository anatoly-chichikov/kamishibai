Output live/capture.gif

Set Shell "bash"
Set FontSize 14
Set Width 1400
Set Height 720
Set Padding 24
Set Theme "Builtin Solarized Dark"
Set TypingSpeed 5ms

Type "../../target/debug/examples/tui_states"
Enter
Sleep 1500ms

# State 0: Welcome
Screenshot "live/00-welcome.png"

Space
Sleep 600ms

# State 1: Your words
Screenshot "live/01-your-words.png"

Space
Sleep 600ms

# State 1b: Busy understanding
Screenshot "live/01b-busy.png"

Space
Sleep 600ms

# State 2: What I understood
Screenshot "live/02-what-i-understood.png"

Space
Sleep 600ms

# State 2b: Change something modal
Screenshot "live/03-change-something-modal.png"

Space
Sleep 600ms

# State 3: Your cards
Screenshot "live/04-your-cards.png"

Space
Sleep 600ms

# State 3b: Change this card modal
Screenshot "live/05-change-this-card-modal.png"

Space
Sleep 600ms

# State 3c: Your cards retrying
Screenshot "live/06-your-cards-retrying.png"

Space
Sleep 600ms

# State 3d: Your cards couldn't finish
Screenshot "live/07-your-cards-couldnt-finish.png"

Space
Sleep 1500ms

# State 4: Done
Screenshot "live/08-done.png"

Sleep 500ms
Type "q"
Sleep 500ms
