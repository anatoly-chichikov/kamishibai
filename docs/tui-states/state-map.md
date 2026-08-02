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
| Retry stress      | synthetic `YourCards` gallery with active, inactive, recovered, and terminal attempts | `06b-your-cards-retry-stress.png` |
| Sentence labels   | collapsed three-tag summary starts inline on `audio` and wraps onto `scene` / `picture`; expanded question-led editor sits below every artifact | `11-s1-label-tags.png` through `22-s12-label-legacy-meta.png` |
| `Done`            | fullscreen                                                                        | `08-done.png`                   |
| `PickMyLanguage`  | modal over `Welcome` / `YourWords` / `WhatIUnderstood`, opened with `Ctrl+L`      | header chip (no standalone shot) |
| Busy              | one universal blocking overlay on any screen                                      | `01b-busy.png`                  |

Source of truth for these names (`src/tui/screen.rs`, `src/tui/app.rs`):
`Screen` = {`Welcome`, `YourWords`, `WhatIUnderstood`, `YourCards`, `Done`};
`ModalKind` = {`ChangeSomething`, `PickMyLanguage`};
`BusyKind` = {`Understanding`, `BulkCorrection`, `CheckingKey`,
`PublishingDeck`, `PublishingReport`}. There is no separate `BulkCorrectionBusy` screen —
bulk correction is just `BusyKind::BulkCorrection` drawn by the universal busy overlay.

Sentence-label editing, retry, failure banner and recovery are inline within
`YourCards` — not separate screens or modals.

The synthetic PNGs (`00-welcome.png`, `00b-welcome-env.png`,
`03-change-something-modal.png`, `06-your-cards-retrying.png`,
`06b-your-cards-retry-stress.png`, `07-your-cards-couldnt-finish.png`, and the S1–S12 sentence-label set from
`11-s1-label-tags.png` through `22-s12-label-legacy-meta.png`) require modal,
editor, cache, width, or failure injection that the live-binary `capture.tape`
does not exercise. They are produced reproducibly by `states.tape` plus the
1200 px `states-narrow.tape` for S10; both drive `examples/tui_states.rs` (no
Gemini) through the same EN→FR French flow at 2x. Re-snap them with
`vhs states.tape` and `vhs states-narrow.tape`. The two Welcome shots are the
same `EnterKey` stage with the only difference being `GEMINI_API_KEY`: absent it
shows just the `submit` button (`00-welcome.png`), present it adds the focused
`load from env` chip (`00b-welcome-env.png`).

Both synthetic tapes navigate the walker by **absolute index** (`Type "<n>"`
then `Space` jumps straight to state `<n>`); each screenshot re-asserts its
target, so a dropped or coalesced keystroke cannot accumulate across the run and
the stray Return the shell injects when it launches the binary cannot drift or
contaminate the index.

The retry stress gallery is index 21. Its six cards preserve valid pipeline
order while showing the identical active-attempt copy (`ai is working…`),
inactive retry rows with only their dot, artifact, and known cost, a recovered
artifact, and a terminal `gave up` row together. Their card heads carry the
complete retry summary as `↻1`, `↻2`, or `↻3` after the total cost.

Every blocking text phase uses one universal overlay on top of the current
screen: first understanding, bulk correction, the Welcome key check, and the two
publish steps (deck then report). Sentence-label edits stay pending on the
current card until `Ctrl+G` activates every pending card as one batch; only then
are those cards' artifacts cleared and sent through the ordinary generation
steps. While a busy overlay is up it
owns keyboard input — every non-redraw key is swallowed until the background
request finishes (`transit` short-circuits when `app.busy()` is set). An error
overlay behaves the same way but clears on any key.

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

## Sentence-label surface

Fresh generated metadata may attribute the sentence by register, type, and an
operational CEFR band. The lowercase choices are `a1`, `a2`, `b1`, `b2`, `c1`,
and `c2`. They classify only the language surrounding the target term; the
target term itself is exempt, and the estimate is not an official proficiency
assessment. A new card first gets the natural sentence required by its approved
understanding and only then receives a descriptive level; initial generation
never targets a band. The level becomes a rewrite constraint only after the
user explicitly changes it. Every card head keeps `term → target sentence`.
The artifacts begin immediately after the last line of that head, including
when the target sentence wraps, and remain an uninterrupted left column in
`meta`, `audio`, `scene`, `picture` order, including their size and final `$…`
or `cached` indicators. A collapsed card leaves the `meta` row unadorned and
draws no `sentence:` heading or separator glyph anywhere. The three labels
start together in one fixed column after the stable `audio` core:

```text
meta     … cached
audio    … cached   formal statement b1
audio    $.0021     formal statement b1
scene    … cached
picture  … cached
```

The actual register, sentence-type, and CEFR values replace the three
example values. Each value remains a separate tag with dark `BG` letters.
Unchanged tags use the gray `DIM` background — the same color used for the
compact target sentence's foreground — while explicitly changed or previously
pinned tags use a white background without bold. An approximately fulfilled
pinned tag keeps the white background and adds an `≈` prefix. Adjacent tags have
one ordinary-background space between them. At narrow widths wrapping occurs
only between whole chips and may use the same tag-column on the `scene` and
`picture` rows. `ai is working…`, ready, cached, inactive retry, and recovered
audio all keep the same tag column. Retry history appears once in the card head
instead of beside the tags; when the complete row or a wrapped chip would
collide, the complete inline summary is hidden. There is no
vertical rail, rule, or filled backing behind the labels.
If the complete atomic set cannot fit even that way, the card head remains the
mouse entry into tuning. There is no grammar axis or grammar row.

`Enter`, `→`, `Space`, or a tag click immediately expands the focused card and
opens the inline editor on `how should it sound?`. The head remains `term →
target sentence` and all four artifact rows stay together above it. The
collapsed inline summary disappears; exactly one blank row separates `picture`
from the editor, which renders below the complete artifact block, before the
expanded metadata and never beside the artifacts. Its three carousel questions
are `how should it sound?`, `what kind of phrase?`, and `what's the desired level?`.
The following note row is labelled `one more thing` and uses the single-line
`TextField` with the placeholder `say what should change`.

The active question is white and bold. The selected chip has a white background.
Every carousel is permanently bracketed by the two-cell direction controls
`< ` and ` >`; both cells are clickable, focus that control's own row, and move
one adjacent choice without wrapping past either boundary. All three tracks use
one render-time width derived from the widest choice and the largest choice
count across the axes, so both chevrons share columns. Within that fixed track, the
selected chip's visual centre advances proportionally from the leading edge to
the trailing edge as its choice index increases. The remaining rail is divided
into one marker segment per hidden choice; every adjacent step transfers exactly one
segment from the trailing side to the leading side. Segment widths differ by at
most one cell, spare cells sit nearest the selected chip on each side, and every
cell of a segment belongs to the same clickable target. The nearest marker uses
`DIM2`, the next farther marker uses `RULE`, and every marker farther away uses
`HL`, saturating at `HL`. On a legacy
axis with no selected value, `—` is flanked by one two-cell marker on each side
inside the same shared track; both cells of either marker are clickable. Legacy
metadata renders no collapsed inline summary but remains tunable through this
same below-artifacts editor. The collapsed footer advertises only `[Enter/→]
tune`; `Space` remains an unadvertised keyboard alias.

The expanded metadata that follows the editor uses statement and noun labels
rather than questions: `the phrase` for the target sentence, `in your language`
for the highlighted source sentence, `a visual clue` for the hint, `word
meaning` for meaning, `word pronunciation` for word pronunciation, `phrase
pronunciation` for transcription, `worth learning` for importance, and `the
right context` for non-empty context.

Every chip or note edit is pending immediately: the old target sentence is
struck through and its current metadata and artifact rows are muted. While the
editor is open, its white selected chips show the staged choices below the
artifacts; after it closes, the staged choices return inline on the `audio` row as
summary tags, gray for unchanged values and white without bold for changed or
pinned values. Regeneration carries this complete current preset: every unedited
axis must keep its value exactly, while only an explicitly changed or already
pinned axis may differ from the requested value and then must carry `≈` as an
approximate result. Returning all chips to their generated defaults and leaving
only a blank note removes pending automatically. `Enter` is inert while the
editor is open. `Esc` closes the editor and collapses the card while retaining
pending; `Ctrl+G` closes it and regenerates every pending card together. There
is no per-card modal and `R` has no `YourCards` action.

| Scenario | Synthetic reference |
| -------- | ------------------- |
| S1 · three collapsed sentence tags continue inline after the `audio` status | `11-s1-label-tags.png` |
| S2 · editor opens below the complete artifact block | `12-s2-label-editor.png` |
| S3 · pending sound choice remains visible in the below-artifacts editor | `13-s3-label-pending-register.png` |
| S4 · pending `one more thing` note continues below the choice rows | `14-s4-label-pending-note.png` |
| S5 · collapsed generated defaults restored, no pending | `15-s5-label-restored.png` |
| S6 · two collapsed cards accumulated as pending | `16-s6-label-multiple-pending.png` |
| S7 · pending batch regenerating | `17-s7-label-regenerating.png` |
| S8 · regenerated pinned value stays audio-anchored beside a recovered picture whose head shows `↻2` | `18-s8-label-regenerated.png` |
| S9 · collapsed approximate pinned value | `19-s9-label-approx.png` |
| S10 · whole-tag wrapping onto `scene` / `picture`, with impossible summaries hidden atomically | `20-s10-label-tags-narrow.png` |
| S11 · post-click `request` selection between both direction chevrons | `21-s11-label-mouse-selection.png` |
| S12 · legacy below-artifacts editor with a marker on each side of `—` | `22-s12-label-legacy-meta.png` |

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
        ├─ [↑↓] nav
        ├─ [Enter]/[→]/[Space]/[click tag] ──► expand + live editor on `how should it sound?`
        │       ├─ [←→] pick · [↑↓] row · type under `one more thing`
        │       ├─ every edit ──► pending now; defaults + blank note ──► no pending
        │       ├─ [Enter] ──► no action
        │       └─ [Esc] ──► close + collapse while retaining pending
        ├─ [Ctrl+G, pending > 0] ──► regenerate all pending cards in one batch
        ├─ [Ctrl+G, pending = 0] ──► existing retry/rebuild fallback
        └─ queue drained and pending = 0 ──► (StartPublish: deck ──► report busy) ──► Done

    Done
        ├─ [Ctrl+G, if failed cards > 0] ──► YourCards (regenerate failed)
        └─ [Ctrl+C] ──► exit
```

Note: there is no `Esc`-to-go-back from `WhatIUnderstood`, and `R` has no action
on `WhatIUnderstood` or `YourCards` (the bulk modal is reached only through the `+ add more` row). The only path back to
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
| `YourCards`                 | `Ctrl+G` regenerate pending batch/fallback · `Enter`/`→` tune · `↑↓` nav (`Space` is an unadvertised alias) |
| `YourCards` label editor    | `Ctrl+G` regenerate pending batch · `←→` pick · `↑↓` row · text editing under `one more thing` · `Esc` close (`Enter` inert) |
| `PickMyLanguage`            | `←/→`/`↑↓` move · `Enter` confirm · `Esc` cancel                                                             |
| `Done`                      | `Ctrl+G` regenerate failed (only when failures) · `Ctrl+C` quit · file paths stay visible                   |

`Ctrl+C` quits from every screen (a second press confirms when a `quit_pending` prompt shows).

## Event ownership

Events are divided between the app shell and individual screens:

- **Shell owns**: terminal `Resize`, pumping the session-engine channel, and the final quit.
- **Transition owns**: every key in the table above, modal dismissal (`Esc` → `Cancel`),
  live sentence-label staging, text editing, list navigation, row expansion.
- **Session engine emits** (fed back into the transition as `AppEvent`s):
  `UnderstandingReady`, `BulkCorrectionReady`, `RetryStarted`, `RetryExhausted`,
  `BatchReady`, `BatchDone { failed }`.

`YourWords` input is line-delimited. Plain `Enter` appends a new line to the raw
blob. Commas are literal text, not separators. The continue command must be a
distinct chord from text entry; the contract label is `Ctrl+G`.

The first pass is a Gemini Flash understanding request. It chooses one global
target language for the batch, returns candidate rows with part-of-speech/form
comments, and keeps off-language rows visible as `skip` rows. `skip` rows are
not forwarded to card generation.

## Recovery semantics (MVP)

- Retry: each artifact (`meta`, `scene`, `picture`, `sound`) gets one plain try plus up
  to 3 retries. Every active attempt uses the same spinner and `ai is working…`
  text. An inactive retry row keeps only its dot, artifact label, and known cost;
  the card head summarizes spent retries once as `↻N` after its total cost.
- Rejected attempts: expanding the card (Enter/→) reveals, below the card body and
  behind a dashed rule, a `rejected attempts` block:
  one row per failure, naming the gate (`border`, `topology`, `recall_text`, …) and its
  reason. A row links to whatever its own try left behind — the archived frame for a
  picture, the rejected model reply for a scene — and it opens with the system
  handler. A try that never reached the model leaves that column blank.
- Terminal failure: after the plain try and all 3 retries, the card stays in the queue;
  its artifact row keeps a leading `✗`, says `gave up`, and shows any known cost.
  The head contributes at most `↻3` for that artifact, and the footer does not
  duplicate the terminal count.
- Recovery via `Ctrl+G`: on `YourCards`, `RegenerateCards` activates every
  pending rewrite together; with no pending rewrite it preserves the existing
  failed/incomplete/rebuild fallback. On `Done` it regenerates every failed card,
  but only when `cards_failed > 0` (`RegenerateFailed`). Building the `.apkg` and
  `.pdf` is not a separate user action — it runs automatically once the queue
  drains with no pending cards (`StartPublish`).
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
`RunBulkCorrection`, `StartGeneration`, `RegenerateCards`, `RegenerateFailed`,
`StartPublish`, `ValidateKey`, `LoadEnvKey`,
`PersistMyLanguage…`, `ExitApp`). Tests drive it with fabricated events to verify
the full path `YourWords → WhatIUnderstood → YourCards → Done` without touching the
network.

This map documents the state machine only. Widget rendering lives in
`src/tui/screens/`; the real Gemini passes and the artifact pipeline live in
`src/gemini` and `src/generation`. Both are fully implemented — not scaffold.
