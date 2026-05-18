# Kamishibai TUI State Map

This is the locked-in source of truth for the word-first TUI flow.
All future work references this map instead of re-deriving transitions.

## UI Stack (frozen)

- TUI framework: **ratatui**
- Terminal backend, input handling, terminal control: **crossterm**
- No alternative UI stack is allowed.

## Screens and overlays

| Id                    | Kind        | PDF reference                                             |
| --------------------- | ----------- | --------------------------------------------------------- |
| `Welcome`             | fullscreen  | first-run setup; no live reference yet                    |
| `YourWords`           | fullscreen  | `01-your-words.png`                                       |
| `WhatIUnderstood`     | fullscreen  | `02-what-i-understood.png`, `02b-what-i-understood-corrected.png` |
| `ChangeSomething`     | modal       | `03-change-something-modal.png` over `WhatIUnderstood`    |
| `BulkCorrectionBusy`  | overlay     | `01c-busy-correction.png` over `WhatIUnderstood`          |
| `YourCards`           | fullscreen  | `04-your-cards.png`, `04b-your-cards-mid.png`             |
| `ChangeThisCard`      | modal       | over `YourCards` — reference shot pending                 |
| `Done`                | fullscreen  | `08-done.png`                                             |

Retry, failure banner and recovery are inline within `YourCards` — not separate screens.

The remaining edge-case PNGs (`05-change-this-card-modal.png`,
`06-your-cards-retrying.png`, `07-your-cards-couldnt-finish.png`) are intentionally absent
from `live/`. They require per-card modal setup or failure injection during recording, which
the live-binary `capture.tape` does not exercise. Re-snap them via `examples/tui_states.rs`
when those particular states need fresh references.

Text-only Gemini passes use one universal blocking overlay on top of the current
screen: first understanding, bulk correction, and per-card correction. The
overlay owns keyboard input until the background request finishes.

`Welcome` is the explicit setup gate. It appears until the user has confirmed
`my language` and a Gemini key is available from the environment, saved
preferences, or paste. `GEMINI_API_KEY` may prefill the key step, but it must
never skip the first-run language choice.

After setup, the language pair is rendered as a compact header widget on every
steady-state fullscreen screen in the same visual language.

## Language pair surface

| Screen            | What is shown                                                                                  |
| ----------------- | ---------------------------------------------------------------------------------------------- |
| `Welcome`         | Unlocked setup language. `←/→` or `Ctrl+L` cycles `my language`; `Enter` confirms it.          |
| `YourWords`       | Detected target (pending), persisted `my` language. `[Ctrl+L]` flips `my` language.          |
| `WhatIUnderstood` | Confirmed target, current `my`. `[L]` can flip `my`, `[T]` cycles target if unsure.           |
| `YourCards`       | Frozen pair for the batch — read-only.                                                         |
| `Done`            | Pair remains visible next to the batch summary.                                                |

`target language` is resolved before `WhatIUnderstood`. `my language` is read
from `config/preferences.json` only after the stored value has been explicitly
confirmed by the user; otherwise startup falls back to `en` while showing
`Welcome`.

## Candidate contract

`WordCandidate` is intentionally small: it carries the target `term`, one
support-language `understanding` sentence, and an `ok` inclusion flag. The first
Gemini pass folds part of speech, inflection, selected sense, register, typo
correction, ambiguity, and exclusion reason into that sentence instead of
maintaining a parallel taxonomy.

`WhatIUnderstood` renders one row per candidate: included rows proceed to card
generation, while `ok=false` rows stay visible with a struck-through term so the
user can see what was rejected and why.

## Transitions

```
              [Ctrl+G]
    YourWords ─────────► resolving target ─► WhatIUnderstood
        ▲                                          │
        │                                          │
        │            [Esc] from WhatIUnderstood    │ [Enter/R]
        └──────────────────────────────────────────┤
                                                   ▼
                                           ChangeSomething
                                             (bulk modal)
                                                   │
                                         [Enter]   │   [Esc]
                                             ┌─────┴─────┐
                                             │           │
                                             ▼           ▼
                                       bulk retry   WhatIUnderstood
                                             │
                                             ▼
                                      WhatIUnderstood

    WhatIUnderstood ──[Ctrl+G]──► YourCards
    YourCards ──[R on card]──► ChangeThisCard ──[Enter]──► YourCards
    YourCards ──[R on card]──► ChangeThisCard ──[Esc]────► YourCards
    YourCards ──[Ctrl+G]──► regenerate current card state / rebuild publish
    YourCards ──[all ready]──► Done
    YourCards ──[all attempts done]──► Done (if nothing fatal)
                                 │
                                 └─[has failed cards]─► Done (with failure summary)

    Done ──[Ctrl+G if failures]──► YourCards
    Done ──[Ctrl+C]──► exit
```

## Keyboard contract (per state)

| State             | Keys                                                                                     |
| ----------------- | ---------------------------------------------------------------------------------------- |
| `YourWords`       | type/paste one item per line · `Enter` newline · `Ctrl+G` continue · `Ctrl+L` toggle my language |
| `WhatIUnderstood` | `↑↓` nav · `d` drop row · `Enter` / `R` refine row · `Ctrl+G` make cards · `L` flip my · `T` cycle target |
| `ChangeSomething` | text area input · `Enter` send · `Esc` cancel                                            |
| `YourCards`       | `↑↓` nav · `Enter` expand/collapse · `R` / `r` change this card · `Ctrl+G` regenerate state/rebuild publish |
| `ChangeThisCard`  | text area input · `Enter` send · `Esc` cancel                                            |
| `Done`            | `Ctrl+G` regenerate failed · `Ctrl+C` quit · file paths stay visible                      |

## Event ownership

Events are divided between the app shell and individual screens:

- **Shell owns**: `Resize`, `Quit`, global modal dismissal (`Esc` unwind), timers that drive
  queue progress.
- **Screen owns**: every key in the table above, text-editing events for modals, list
  navigation, row expansion.
- **Session engine owns**: LLM response events (`UnderstandingReady`, `BulkCorrectionReady`,
  `CardCorrectionReady`, `ArtifactReady`, `ArtifactFailed`, `RetryStarted`, `RetryFailedTerminally`).

`YourWords` input is line-delimited. Plain `Enter` appends a new line to the raw
blob. Commas are literal text, not separators. The continue command must be a
distinct chord from text entry; the contract label is `Ctrl+G`.

The first pass is a Gemini Flash understanding request. It chooses one global
target language for the batch, returns candidate rows with part-of-speech/form
comments, and keeps off-language rows visible as `skip` rows. `skip` rows are
not forwarded to card generation.

## Recovery semantics (MVP)

- Retry: each artifact (`scene`, `picture`, `sound`) retries up to 3 times. Between
  attempts the card row shows an inline retry indicator without blocking the queue.
- Terminal failure: after 3 attempts, the card stays in the queue but marked as failed.
- Recovery: the final card views expose `Ctrl+G`: on `YourCards` it regenerates failed work or rebuilds APKG/PDF from ready cards; on `Done` it regenerates failed cards when failures exist.
- There is no `Retry` fullscreen and no separate `Failure` fullscreen.

## App shell and ratatui mapping

- Root widget: a single full-terminal area split vertically into
  `header · body · footer` using `Layout::default().constraints([Length(1), Min(1), Length(1)])`.
- `header` always renders the language pair widget (target → my).
- `body` renders the active screen. Modals are rendered last by drawing into a
  centered rectangle over `body` using `Clear + Block::bordered()`.
- `footer` renders the keyboard hints for the active screen.
- The crossterm event loop reads `KeyEvent`, `ResizeEvent`, and the session-engine
  channel, then dispatches through the transition function below.

## Pure transition function

```
transition(state, event) -> (state, side_effects)
```

It is pure: no IO, no Gemini calls. Tests drive it with fabricated events to
verify the full screen path `YourWords -> WhatIUnderstood -> YourCards -> Done`
without touching the network.

Out of scope for this map:

- Final widget rendering.
- Real Gemini integration.
- Implementation of individual screens beyond the skeleton scaffold that proves
  transitions.
