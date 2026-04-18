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
| `YourWords`           | fullscreen  | `01-your-words.png`                                       |
| `WhatIUnderstood`     | fullscreen  | `02-what-i-understood.png`                                |
| `ChangeSomething`     | modal       | `03-change-something-modal.png` over `WhatIUnderstood`    |
| `YourCards`           | fullscreen  | `04-your-cards.png`, `06-your-cards-retrying.png`, `07-your-cards-couldnt-finish.png` |
| `ChangeThisCard`      | modal       | `05-change-this-card-modal.png` over `YourCards`          |
| `Done`                | fullscreen  | `08-done.png`                                             |

Retry, failure banner and recovery are inline within `YourCards` — not separate screens.

There is **no** standalone fullscreen language wizard. Language pair is rendered
as a compact header widget on every fullscreen screen. The widget is a missing
requirement relative to the PDF and must be added on top of every screenshot in
the same visual language.

## Language pair surface

| Screen            | What is shown                                                                                  |
| ----------------- | ---------------------------------------------------------------------------------------------- |
| `YourWords`       | Detected target (pending), persisted `my` language. `[L]` opens an inline `my` language pick. |
| `WhatIUnderstood` | Confirmed target, current `my`. `[L]` can flip `my`, `[T]` can override target if unsure.     |
| `YourCards`       | Frozen pair for the batch — read-only.                                                         |
| `Done`            | Pair remains visible next to the batch summary.                                                |

`target language` is resolved before `WhatIUnderstood`. `my language` is read
from `config/preferences.json` at batch start and defaults to `en`.

## Transitions

```
              [Enter]
    YourWords ─────────► resolving target ─► WhatIUnderstood
        ▲                                          │
        │                                          │
        │            [Esc] from WhatIUnderstood    │ [R]
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

    WhatIUnderstood ──[Enter]──► YourCards
    YourCards ──[R on card]──► ChangeThisCard ──[Enter]──► YourCards
    YourCards ──[R on card]──► ChangeThisCard ──[Esc]────► YourCards
    YourCards ──[all ready]──► Done
    YourCards ──[all attempts done]──► Done (if nothing fatal)
                                 │
                                 └─[has failed cards]─► Done (with failure summary)

    Done ──[N]──► YourWords (new batch, same my language)
    Done ──[Q]──► exit
```

## Keyboard contract (per state)

| State             | Keys                                                                                     |
| ----------------- | ---------------------------------------------------------------------------------------- |
| `YourWords`       | `⌘V/Ctrl+V` paste · `Enter` continue · `L` toggle my language                             |
| `WhatIUnderstood` | `↑↓` nav · `d` drop row · `R` change something · `Enter` make cards · `L` flip my · `T` override target |
| `ChangeSomething` | text area input · `Enter` send · `Esc` cancel                                            |
| `YourCards`       | `↑↓` nav · `Enter` expand/collapse · `R` change this card · `d` drop artifact · `F` regenerate failed |
| `ChangeThisCard`  | text area input · `Enter` send · `Esc` cancel                                            |
| `Done`            | `N` new batch · `Q` quit · file paths stay visible                                       |

## Event ownership

Events are divided between the app shell and individual screens:

- **Shell owns**: `Resize`, `Quit`, global modal dismissal (`Esc` unwind), timers that drive
  queue progress.
- **Screen owns**: every key in the table above, text-editing events for modals, list
  navigation, row expansion.
- **Session engine owns**: LLM response events (`UnderstandingReady`, `BulkCorrectionReady`,
  `CardCorrectionReady`, `ArtifactReady`, `ArtifactFailed`, `RetryStarted`, `RetryFailedTerminally`).

## Recovery semantics (MVP)

- Retry: each artifact (`scene`, `picture`, `sound`) retries up to 3 times. Between
  attempts the card row shows an inline retry indicator without blocking the queue.
- Terminal failure: after 3 attempts, the card stays in the queue but marked as failed.
- Recovery: the `Done` screen exposes `regenerate failed` when at least one card failed.
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
