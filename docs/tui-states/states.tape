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
Show

# Each capture launches one prepared scene by stable ID, waits for the AI reminder every
# scene draws, repaints with Ctrl+L, and holds the scene on screen around the shot,
# because VHS writes a Screenshot from recorded frames that lag behind the tape.

# welcome.key — Welcome · key step with no GEMINI_API_KEY in env (submit only)
Type "./target/release/examples/tui_states --state welcome.key"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/00-welcome.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# welcome.key-env — Welcome · key step with GEMINI_API_KEY in env (load-from-env chip)
Type "./target/release/examples/tui_states --state welcome.key-env"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/00b-welcome-env.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# review.closed — WhatIUnderstood with the closed batch sentence summary
Type "./target/release/examples/tui_states --state review.closed"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/36-word-meanings.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# review.correction — ChangeSomething modal over WhatIUnderstood
Type "./target/release/examples/tui_states --state review.correction"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/03-change-something-modal.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.closed — S1: three collapsed tags continue inline after the audio status
Type "./target/release/examples/tui_states --state cards.closed"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/11-s1-label-tags.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.tuning — S2: editor opens below the complete artifact block
Type "./target/release/examples/tui_states --state cards.tuning"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/12-s2-label-editor.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.retrying — YourCards with inline retry indicators
Type "./target/release/examples/tui_states --state cards.retrying"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/06-your-cards-retrying.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.failed — YourCards couldn't finish (terminal failure)
Type "./target/release/examples/tui_states --state cards.failed"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/07-your-cards-couldnt-finish.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.pending-register — S3: pending register remains visible in the below-artifacts editor
Type "./target/release/examples/tui_states --state cards.pending-register"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/13-s3-label-pending-register.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.pending-note — S4: pending note continues below the editor axes
Type "./target/release/examples/tui_states --state cards.pending-note"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/14-s4-label-pending-note.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.restored — S5: collapsed defaults restored automatically clear pending
Type "./target/release/examples/tui_states --state cards.restored"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/15-s5-label-restored.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.multiple-pending — S6: two collapsed cards accumulate independently before regeneration
Type "./target/release/examples/tui_states --state cards.multiple-pending"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/16-s6-label-multiple-pending.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.rewriting — S7: Ctrl+G activates the accumulated batch
Type "./target/release/examples/tui_states --state cards.rewriting"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/17-s7-label-regenerating.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.rewritten — S8: pinned tags stay audio-anchored despite picture rejections
Type "./target/release/examples/tui_states --state cards.rewritten"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/18-s8-label-regenerated.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.approximate — S9: collapsed card keeps the actual register beside its requested target
Type "./target/release/examples/tui_states --state cards.approximate"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/19-s9-label-approx.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.mouse-tuning — S11: mouse-selected chip in the below-artifacts editor
Type "./target/release/examples/tui_states --state cards.mouse-tuning"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/21-s11-label-mouse-selection.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.legacy-tuning — S12: legacy metadata opens a below-artifacts editor with no active chips
Type "./target/release/examples/tui_states --state cards.legacy-tuning"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/22-s12-label-legacy-meta.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.retry-stress — active audio plus retry, recovery and terminal rows as a layout stress test
Type "./target/release/examples/tui_states --state cards.retry-stress"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/06b-your-cards-retry-stress.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# words.clear-armed — first Esc on nonempty YourWords arms a guarded clear
Type "./target/release/examples/tui_states --state words.clear-armed"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/23-esc-words-clear.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# words.returned — review back preserves words with a fresh unarmed Esc clear
Type "./target/release/examples/tui_states --state words.returned"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/24-esc-review-back.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.stop-armed — first Esc during generation arms a guarded stop
Type "./target/release/examples/tui_states --state cards.stop-armed"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/25-esc-generation-stop.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.stopping — confirmed stop drains the current artifact
Type "./target/release/examples/tui_states --state cards.stopping"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/26-generation-stopping.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.partial — completed subset is published as a settled partial batch
Type "./target/release/examples/tui_states --state cards.partial"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/27-generation-partial.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# review.guidance — generation guidance retains b1 + questions in both inline carousels
Type "./target/release/examples/tui_states --state review.guidance"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/28-batch-sentence-settings.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# review.alternates — the pass reports languages the same words would equally have read as
Type "./target/release/examples/tui_states --state review.alternates"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/30-plausible-alternates.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# review.languages — the pair modal, opened on the learning half from the header chip
Type "./target/release/examples/tui_states --state review.languages"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/31-language-pair-modal.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# welcome.language — Welcome · the language step, laid out as a grid across the width
Type "./target/release/examples/tui_states --state welcome.language"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/32-welcome-language-grid.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# cards.parked — two parked cards stay expanded while the walk stands on a third
Type "./target/release/examples/tui_states --state cards.parked"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/33-your-cards-parked.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# review.senses — an open sense list: the head names its meanings instead of
# repeating one, and the rows step down from chosen to unchosen
Type "./target/release/examples/tui_states --state review.senses"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/34-open-sense-list.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# review.translation-preview — prepared meanings await confirmation of the destination
Type "./target/release/examples/tui_states --state review.translation-preview"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/35-translation-preview.png"
Sleep 2500ms
Type "q"
Sleep 500ms

# done.published — a reopened published session settles on the final cards view
Type "./target/release/examples/tui_states --state done.published"
Enter
Wait+Screen /please verify results/
Ctrl+L
Sleep 2500ms
Screenshot "live/37-done-published.png"
Sleep 2500ms
Type "q"
Sleep 500ms
