# Kamishibai TUI State Map

This is the locked-in source of truth for the word-first TUI flow.
All future work references this map instead of re-deriving transitions.

## UI Stack (frozen)

- TUI framework: **ratatui**
- Terminal backend, input handling, terminal control: **crossterm**
- No alternative UI stack is allowed.

## Screens and overlays

| Id                | Kind                                                                              | Reference shot                  |
| ----------------- | --------------------------------------------------------------------------------- | ------------------------------- |
| `Welcome`         | fullscreen (two stages: pick language → enter key)                                | `00-welcome.png` (no env key) · `00b-welcome-env.png` (`GEMINI_API_KEY` set) |
| `YourWords`       | fullscreen                                                                        | `01-your-words.png`             |
| `WhatIUnderstood` | fullscreen                                                                        | `02-what-i-understood.png`      |
| `ChangeSomething` | modal over `WhatIUnderstood`, opened from the `+ add more` row in the sense picker | `03-change-something-modal.png` |
| `YourCards`       | fullscreen                                                                        | `04-your-cards.png`             |
| `ChangeThisCard`  | modal over `YourCards`, opened with `R`                                           | `05-change-this-card-modal.png` |
| `Done`            | fullscreen                                                                        | `08-done.png`                   |
| `PickMyLanguage`  | modal over `Welcome` / `YourWords` / `WhatIUnderstood`, opened with `Ctrl+L`      | header chip (no standalone shot) |
| Busy              | one universal blocking overlay on any screen                                      | `01b-busy.png`                  |

Source of truth for these names (`src/tui/screen.rs`, `src/tui/app.rs`):
`Screen` = {`Welcome`, `YourWords`, `WhatIUnderstood`, `YourCards`, `Done`};
`ModalKind` = {`ChangeSomething`, `ChangeThisCard`, `PickMyLanguage`};
`BusyKind` = {`Understanding`, `BulkCorrection`, `CardCorrection`, `CheckingKey`,
`PublishingDeck`, `PublishingReport`}. There is no separate `BulkCorrectionBusy` screen —
bulk correction is just `BusyKind::BulkCorrection` drawn by the universal busy overlay.

Retry, failure banner and recovery are inline within `YourCards` — not separate screens.

The edge-case PNGs (`00-welcome.png`, `00b-welcome-env.png`,
`03-change-something-modal.png`, `05-change-this-card-modal.png`,
`06-your-cards-retrying.png`, `07-your-cards-couldnt-finish.png`) require modal
setup or environment/failure injection that the live-binary `capture.tape` does
not exercise. They are produced reproducibly by `states.tape`, which drives
`examples/tui_states.rs` (no Gemini) through the same EN→FR French flow at 2x.
Re-snap them with `vhs states.tape`. The two Welcome shots are the same `EnterKey`
stage with the only difference being `GEMINI_API_KEY`: absent it shows just the
`submit` button (`00-welcome.png`), present it adds the focused `load from env`
chip (`00b-welcome-env.png`).

`states.tape` navigates the walker by **absolute index** (`Type "<n>"` then
`Space` jumps straight to state `<n>`); each screenshot re-asserts its target, so
a dropped or coalesced keystroke cannot accumulate across the run and the stray
Return the shell injects when it launches the binary cannot drift or contaminate
the index.

Every background phase uses one universal blocking overlay on top of the current
screen: first understanding, bulk correction, per-card correction, the Welcome
key check, and the two publish steps (deck then report). While a busy overlay is
up it owns keyboard input — every non-redraw key is swallowed until the
background request finishes (`transit` short-circuits when `app.busy()` is set).
An error overlay behaves the same way but clears on any key.

`Welcome` is the explicit setup gate, with two stages. **Pick language**: `←/→`
(or `Ctrl+L`) cycle `my language`, `Enter` confirms it and advances. **Enter key**:
type or paste a Gemini key; `←/→` move focus between the submit button and a
`load from env` action (the env action only joins the cycle when `GEMINI_API_KEY`
is set); `Enter` on a filled field runs a live validity check and only on success
persists the key and moves to `YourWords`; `Esc` steps back to the language stage.
`GEMINI_API_KEY` may prefill the key step, but it must never skip the language choice.

After setup, the language pair is rendered as a compact header chip on every
steady-state fullscreen screen, reading `my → target` (e.g. `EN → FR`).

## Language pair surface

| Screen            | What is shown                                                                                  |
| ----------------- | ---------------------------------------------------------------------------------------------- |
| `Welcome`         | Unlocked setup language. `←/→` (or `Ctrl+L`) cycles `my language`; `Enter` confirms it.        |
| `YourWords`       | Detected target (pending), persisted `my`. `Ctrl+L` opens the language picker.                 |
| `WhatIUnderstood` | Confirmed target, current `my`. `Ctrl+L` opens the language picker (re-runs understanding).    |
| `YourCards`       | Frozen pair for the batch — read-only.                                                         |
| `Done`            | Pair remains visible next to the batch summary.                                                |

The only way to change `my language` mid-flow is the `Ctrl+L` picker modal
(`PickMyLanguage`), available on `Welcome`, `YourWords`, and `WhatIUnderstood`.
There is no `[L]` flip key and no `[T]` target-cycle key — both were removed.

`target language` is resolved before `WhatIUnderstood`. `my language` is read
from `config/preferences.json` only after the stored value has been explicitly
confirmed by the user; otherwise startup falls back to `en` while showing
`Welcome`.

## Candidate contract

`WordCandidate` is intentionally small: it carries the target `term`, an ordered
list of support-language sense sentences, the selected sense indexes, and an
`ok` inclusion flag. The first Gemini pass still folds part of speech,
inflection, register, typo correction, ambiguity, and exclusion reason into
those sentences instead of maintaining a parallel taxonomy.

`WhatIUnderstood` renders one row per candidate: included selected senses
proceed to card generation as separate cards, while `ok=false` rows stay visible
with a struck-through term so the user can see what was rejected and why.

## Transitions

```
    YourWords ──[Ctrl+G, blob not blank]──► (Understanding busy) ──► WhatIUnderstood

    WhatIUnderstood
        ├─ [Enter]/[→] on a row ──► sense picker opens
        │       ├─ [Space] toggle sense · [↑↓]/[j][k] move · [Enter]/[←] done (collapse)
        │       └─ [Enter] on the "+ add more" row ──► ChangeSomething (bulk modal)
        │                                                  ├─ [Enter] send ──► (BulkCorrection busy) ──► WhatIUnderstood
        │                                                  └─ [Esc] cancel ──► WhatIUnderstood
        ├─ [D] drop selected row ──► last row dropped returns to YourWords (blob cleared)
        ├─ [Ctrl+L] ──► PickMyLanguage modal ──► re-runs understanding
        └─ [Ctrl+G, ≥1 ok row] ──► (StartGeneration) ──► YourCards

    YourCards
        ├─ [↑↓]/[←→] nav · [Enter] expand/collapse
        ├─ [R] ──► ChangeThisCard modal ──► [Enter] send (CardCorrection) | [Esc] cancel ──► YourCards
        ├─ [Ctrl+G] ──► regenerate the selected card
        └─ queue drained ──► (StartPublish: PublishingDeck ──► PublishingReport busy) ──► Done

    Done
        ├─ [Ctrl+G, if failed cards > 0] ──► YourCards (regenerate failed)
        └─ [Ctrl+C] ──► exit
```

Note: there is no `Esc`-to-go-back from `WhatIUnderstood`, and `R` is a no-op there
(the bulk modal is reached only through the `+ add more` row). The only path back to
`YourWords` is dropping the last remaining candidate. Publishing the deck/PDF is automatic
once the generation queue drains — it is not a key the user presses.

## Keyboard contract (per state)

| State                       | Keys (footer leads with the bright primary)                                                                 |
| --------------------------- | ----------------------------------------------------------------------------------------------------------- |
| `Welcome` · pick language   | `←/→` cycle `my language` · `Enter` next · `Ctrl+C` quit                                                     |
| `Welcome` · enter key       | type/`Cmd+V` paste key · `←/→` move focus (submit ↔ load-from-env, env only) · `Enter` submit · `Esc` back   |
| `YourWords`                 | type/paste one item per line · `Enter` newline · `←/→/↑/↓` move cursor · `Ctrl+G` continue · `Ctrl+L` language |
| `WhatIUnderstood` (list)    | `↑↓`/`j`/`k` nav · `Enter`/`→` pick meanings · `D` drop row · `Ctrl+G` make cards · `Ctrl+L` language        |
| `WhatIUnderstood` (picker)  | `Space` toggle sense · `↑↓`/`j`/`k` move · `Enter`/`←` done · `Enter` on `+ add more` opens ChangeSomething  |
| `ChangeSomething`           | text input · `Enter` send · `Esc` cancel                                                                    |
| `YourCards`                 | `↑↓`/`←→` nav · `Enter` expand/collapse · `R` change this card · `Ctrl+G` regenerate selected card           |
| `ChangeThisCard`            | text input · `Enter` send · `Esc` cancel                                                                    |
| `PickMyLanguage`            | `←/→`/`↑↓` move · `Enter` confirm · `Esc` cancel                                                             |
| `Done`                      | `Ctrl+G` regenerate failed (only when failures) · `Ctrl+C` quit · file paths stay visible                   |

`Ctrl+C` quits from every screen (a second press confirms when a `quit_pending` prompt shows).

## Event ownership

Events are divided between the app shell and individual screens:

- **Shell owns**: terminal `Resize`, pumping the session-engine channel, and the final quit.
- **Transition owns**: every key in the table above, modal dismissal (`Esc` → `Cancel`),
  text editing in modals, list navigation, row expansion.
- **Session engine emits** (fed back into the transition as `AppEvent`s):
  `UnderstandingReady`, `BulkCorrectionReady`, `CardCorrectionReady`, `RetryStarted`,
  `RetryExhausted`, `BatchReady`, `BatchDone { failed }`.

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
- Recovery via `Ctrl+G`: on `YourCards` it regenerates the currently selected card
  (`RegenerateCurrent`); on `Done` it regenerates every failed card, but only when
  `cards_failed > 0` (`RegenerateFailed`). Building the `.apkg` and `.pdf` is not a
  user action — it runs automatically once the queue drains (`StartPublish`).
- There is no `Retry` fullscreen and no separate `Failure` fullscreen.

## App shell and ratatui mapping

- Root widget: a single full-terminal area split vertically into
  `header · body · AI disclaimer · dashed divider · footer`. The lowercase
  `ai may be wrong, please verify results` reminder sits right-aligned in a
  fixed `DIM2` row immediately above the divider, so screen content, inputs,
  and overlays cannot scroll or paint over it.
- `header` always renders the language pair chip as `my → target` (`pair.label()` is
  `"{support} → {target}"`, e.g. `EN → FR`).
- `body` renders the active screen. Modals are rendered last by drawing into a
  centered rectangle over `body` using `Clear + Block::bordered()`.
- `footer` renders the active screen's keyboard hints as a tiered, width-aware
  status bar: the primary action leads in bright ink, secondary actions follow,
  and conventional keys (navigation, quit) are dimmed. When the row is too narrow
  the dim hints are shed first — the primary action and quit never clip.
- The crossterm event loop reads `KeyEvent`, `ResizeEvent`, and the session-engine
  channel, then dispatches through the transition function below.

## Pure transition function

The state machine is the pure function `transit(app, event) -> (App, Side)` in
`src/tui/transition.rs`: no IO, no Gemini calls. It returns the next `App` plus a
`Side` effect the shell runs outside the function (`RunUnderstanding`,
`RunBulkCorrection`, `RunCardCorrection`, `StartGeneration`, `RegenerateCurrent`,
`RegenerateFailed`, `StartPublish`, `ValidateKey`, `LoadEnvKey`,
`PersistMyLanguage…`, `ExitApp`). Tests drive it with fabricated events to verify
the full path `YourWords → WhatIUnderstood → YourCards → Done` without touching the
network.

This map documents the state machine only. Widget rendering lives in
`src/tui/screens/`; the real Gemini passes and the artifact pipeline live in
`src/gemini` and `src/generation`. Both are fully implemented — not scaffold.
