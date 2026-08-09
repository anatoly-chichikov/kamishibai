# AGENTS.md

This file provides guidance to Codex when working in this repository.

## Project Overview

`kamishibai` is a Rust application that converts schema-driven vocabulary JSON into Anki decks with AI-generated audio and manga-style illustrations.

## Development Commands

Primary Rust workflow:

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Run the application:

```bash
cargo run --
```

## Required Environment

- a Gemini API key is required for any flow that calls Gemini — either via `GEMINI_API_KEY` (which wins) or a key previously saved through the Welcome screen; `GEMINI_API_KEY` need not be set when a saved key exists
- the first OCR-backed run downloads the required `PP-OCRv5` model files into the media cache

## Input Schema

Every entry must contain:

- `term`
- `meaning`
- `pronunciation`
- `transcription`
- `importance`
- `source.sentence`
- `source.lang`
- `source.highlight`
- `source.hint`
- `source.context`
- `target.sentence`
- `target.lang`

The input contract is strict. There are no optional entry fields.

## Sessions (non-interactive)

With no arguments `kamishibai` opens the interactive TUI; a bare JSON path opens the TUI on a prebuilt batch. Everything non-interactive is a **session** subcommand — a persistent, curatable unit of work an agent drives across invocations. A session moves through stages: understood → (curate) → generating → published (or **partial** when some cards fail but the deck still ships the rest, **failed** when no card survives).

- `kamishibai agent-contract`: print the version-matched `llms.txt` embedded in the installed binary; use this before any remote copy
- `kamishibai new (--word W [--word W…] | --words FILE | --build FILE) [--learning L] [--known L] [--senses primary|all] [--id NAME] [--generate]`: understand the words (exactly one input form; `--build` imports a cards JSON whose entries carry the pair, so it rejects `--known`/`--learning`/`--senses`) and create a session in the **understood** stage (`--learning` is autodetected from the words when omitted; `--known` is a one-off override that otherwise resolves from your saved preference and **refuses** when neither is set — save it once with `config`)
- `kamishibai select [<id>] --card T --sense 1,3` / `exclude [<id>] --card T` / `correct [<id>] --card T --note "…"`: curate the understanding before generating — pick senses, drop a card, or ask Gemini to add senses (each resets the session to understood)
- `kamishibai generate [<id>] [--wait]`: commit the curated plan and start a managed background worker that generates + publishes (`--wait` runs it in the foreground)
- `kamishibai status [<id>]`: stage + per-candidate senses (understood) or per-card progress (generating/published), read from the cache (no Gemini)
- `kamishibai adjust [<id>] --card T [--understanding U] [--register neutral|casual|formal|literary|archaic] [--kind statement|question|request|exclamation|dialogue] [--level a1|a2|b1|b2|c1|c2] [--restore register|level|kind|all] [--note "…"]`: stage an offline sentence-label/note patch for one committed card; at least one patch flag is required, omitted fields preserve an existing pending patch, `--restore` is repeatable or comma-delimited and restores labels only, and an explicitly empty note clears it
- `kamishibai open [<id>]`: open the session in the interactive TUI (resumes from the cache)
- `kamishibai result [<id>]` / `ls` / `cancel [<id>]` / `rm [<id>] [--cache]` / `cache-path`
- `kamishibai regenerate [<id>] (--failed | --pending | --card T [--note "…"]) [--wait]`: re-roll committed cards and republish (runs a worker like `generate`); `--pending` atomically activates every staged adjustment, while `--failed` resumes incomplete stages and `--card` targets one card, with an optional immediate Gemini rewrite note
- `kamishibai config [--known L] [--key K]`: save console defaults to preferences (no flags → show them) — `--known` (validated) so word sessions need no `--known`, and `--key` (verified through Gemini `models.list`; `-` reads it from stdin, empty clears it) so you need not export `GEMINI_API_KEY`; the key value is never printed back

There are exactly two output modes: **plain text** (default, for humans — line-oriented, not a parsing target) and **`--json`** (placed after the verb, for machines — exactly one JSON document on stdout: the success document, or the `{"ok":false,"error":{"code","exit","message","hint","retryable"}}` envelope on failure; `generate --wait --json` and `regenerate --wait --json` additionally stream NDJSON events on stderr). `agent-contract` is the text-only exception and refuses `--json`. There is no `-q` and no `result` path selectors — an agent uses `--json`. Exit codes, locking, and semantics are identical in both modes for invocations valid in both; `open` is interactive and also refuses `--json` before any session lookup. The full console contract lives in `llms.txt`. Plain output carries no bare capturable value — every single-session command opens with the header `your session <ID> · <KNOWN> → <LEARNING> · <phase>` and the id lives there; errors are one `kamishibai: <message>` line plus a next-step hint line on stderr. **Language codes are the app's canonical UPPERCASE form everywhere** — stored in config and `session.json`, minted into ids (`FR-…`), used in the cache layout (`cards/EN-FR`) and deck names (`FR_….apkg`), and emitted in plain and JSON; input is accepted in any case and normalised to uppercase, and the only lowercase code is the frozen `target_lang` on the Gemini wire (`src/gemini/client.rs`). Exit codes are centralized in `src/cli/error.rs` (`Refusal` carries the exit, optional hint, retryability, and optional session listing): `0` ok · `2` usage · `3` no such session · `4` not ready · `5` ambiguous · `1` other. The `<id>` positional is optional on every verb: an omitted id resolves to the only session, else the only unfinished one, else the command lists the newest five sessions and exits 5 (`session::resolve`). The background worker is the same binary re-invoked as the hidden `__run <id>`, detached into a new process group with its stdio redirected to `sessions/<id>/worker.log`. Concurrency is two flocks: the long-held liveness lock (`sessions/<id>/lock`, OS-released on death) decides who may generate — `status` derives `interrupted` from a recorded worker whose lock is free — and the short write lock makes every `session.json` change a serialized read-modify-write (`SessionStore::update`), so concurrent edits all apply. The worker writes only while the record still names it, which is how `cancel` and a finishing worker resolve their race. The TUI shares this same session model — it takes the liveness lock before generating and persists its live state to `session.json`, so `ls`/`status`/`open` see interactive runs too. The full agent-facing contract lives in `llms.txt` at the repo root. `--out` wins, `KAMISHIBAI_OUTPUT` is second, and new sessions otherwise resolve the platform Documents directory plus `Kamishibai`; resolved output is stored per session. For offline tests, `KAMISHIBAI_GEMINI_URL` overrides the Gemini base URL (point it at a 127.0.0.1 listener), `KAMISHIBAI_CACHE` overrides the exact cache root, `KAMISHIBAI_DATA` overrides the data home before `kamishibai/preferences.json` is appended, and `KAMISHIBAI_OUTPUT` overrides the exact output root.

Sentence tuning is a two-step persistent transaction in both delivery surfaces. `adjust` only patches the selected card's staged request and may be called repeatedly for several cards; it leaves the current cached metadata, artifacts, published paths, costs, and lifecycle phase untouched. `regenerate --pending` is the only headless command that activates the whole staged batch. `cards.pending` in session JSON counts staged rewrites, each card's `labels` is its current complete attribution, and `adjustment` carries `state` (`pending` or `active`), the possibly partial requested label selection, and the non-empty note when present. A partial-session pending run also resumes unrelated missing stages before the deck is republished. Ordinary `generate`, `regenerate --failed`, and `regenerate --card` refuse staged changes before any provider or destructive cache work.

## Architecture

The runtime is split into a few focused modules:

- `src/vocabulary`: validates the strict JSON document and exposes canonical entry types
- `src/languages`: keeps language profiles, naming, labels, and report font preferences
- `src/runtime`: resolves paths and renders progress and diagnosis output
- `src/application`: owns the UI-neutral ports for understanding, card production, study publishing, key validation, and cost attribution; `CardWorkflow` composes only the learner workflow (understand → produce → publish), while credential validation remains an independent delivery dependency
- `src/gemini`: owns the frozen direct REST contract plus the credential-access and cached-understanding adapters
- `src/generation/card_production`: implements metadata, sound, and visual production as focused Gemini adapters; its accounting, durable picture-request budget, scene-attempt cursor, and recovery policy remain independent of CLI sessions. A failed picture attempt reads its own verdict back from the attempt archive (`attempt_archive.rs`) and returns it as an `AttemptFault`, but only when the archive actually grew during that attempt — a failure that never reached the provider keeps its plain error instead of borrowing an older rejected frame
- `src/generation`: writes cached WAV audio, composes scenes, routes OCR, and validates manga output below the card-production adapter
- `src/publishing`: publishes the completed subset as one Anki deck plus printable PDF while holding visual leases in stable order
- `src/anki`: defines the language-neutral Anki note model and APKG writer
- `src/report`: builds the PDF report with layout, thumbnails, and font resolution
- `src/cli.rs`: parses arguments (clap, including the global `--json` flag) and routes to the interactive TUI or a `session` subcommand
- `src/cli/wiring.rs`: the sole composition root for interactive, console, and cost-attributed session variants of the Gemini-backed `CardWorkflow`; maintenance commands may address low-level cache invalidation directly but cannot compose workflow adapters
- `src/cli/console.rs`: drives the application workflow through the shared `produce` engine loop (meta → sound → scene → picture, then publish) and reports through the human / quiet / JSON `Reporter` port
- `src/cli/session`: the console (API) layer — `store` (the `session.json` record + serialized atomic `create`/`update` IO), `worker` (the managed background worker + the `__run` entrypoint, ownership-guarded writes), `liveness` (the two flocks + pid kill via rustix), `view` (the cache-derived status projection both renders share), `json` (the `Serialize` DTOs + the one emit seam), and one handler module per concern (`new`, `curate`, `adjust`, `generate`, `result`, `maintenance`) routed by `mod.rs`. This layer never links the TUI (`tests/separation.rs` enforces it): `open` hands the checked record to the `SessionOpener` port
- `src/cli/bridge.rs`: the TUI side of the session contract — projects between the live `App` and the persisted record, owns the `TuiSession` the shell claims and writes, and implements `SessionOpener` over `run_tui`

Within the card-workflow boundary, direct dependencies point inward: CLI delivery → concrete Gemini / production / publishing adapters → application ports and session domain values. `tests/separation.rs` rejects reverse imports and prevents workflow adapters from being composed outside `src/cli/wiring.rs`; legacy cache-backed session types are outside this narrower claim.

## Attempts

An artifact gets one plain try plus three retries on top of it — `ARTIFACT_ATTEMPT_CEILING` (4) attempts, which is also the durable picture-request series ceiling. `AttemptTally::retry` still numbers machine-facing retry events from `1..=retries`, but the TUI deliberately does not expose that number on an artifact row. Every active attempt, whether the first try or a retry, renders the same spinner plus `ai is working…`; an inactive retry renders only a dot, its artifact label, and any known artifact cost. A terminal row keeps its leading `✗`, says only `gave up`, and then shows any known cost. Every spent attempt records **why** it was spent: `src/session/attempt.rs` pairs the `AttemptTally` with one `AttemptFault` per failure (`category` slug, user-facing `reason`, and the archived picture when the provider drew one). The production adapter supplies the renderer's real verdict; anything else — transport error, cache lease, exhausted request budget — is diagnosed by the engine from the error text under category `error`. Retry history is summarized once on the card head as `  ↻N`, after the displayed total cost when one exists and omitted at zero. `N` sums `min(tally.done(), tally.retries())` across meta, audio, scene, and picture, so a terminal four-attempt artifact contributes `↻3`; unmetered and undiagnosed spent attempts still count. The expanded card shows the meta preview first and then, below a dashed rule, a `rejected attempts` block; each row names the try, whatever that try produced before being thrown away, and the gate that rejected it. Both stages leave something behind: a picture attempt archives the rejected frame, and a scene attempt archives the model reply it failed to decode (`RejectedReply` carries the body out of `src/gemini`, `attempt_archive::archived_reply` writes it as `scene-NNNN.json` when it parses as JSON and `scene-NNNN.txt` when it never was JSON). Both are muted underlined links that open with the system handler. A failure that never reached the model — transport, cache lease — archives nothing and leaves that column blank. Rejected frames are never deleted by a run — only `drop_artifacts` / `drop_incomplete_artifacts` clear them. The `YourCards` footer does not duplicate the terminal-card count.

## Cache layout

The cache (printed by `kamishibai cache-path`) groups one folder per card, keyed by a content hash of the card identity:

- `cards/<known>-<learning>/<key>/` holds `meta.json` and `audio.wav`; `visual/<revision>/` beneath it holds `scene.json` and `picture.jpg` for one visual-policy revision, plus `attempts/` where every image attempt is archived immutably as `attempt-NNNN.jpg` next to its `attempt-NNNN.json` verdict (`status`, `category`, `reason`), the scene and prompt it used, and the recall review; rejected scene replies land beside them as `scene-NNNN.json` / `scene-NNNN.txt`
- `understanding/<known>-<learning>/<key>.json` holds the understanding-pass result
- `sessions/<id>/` holds `session.json` (identity, phase, words, curated candidates, committed plan, worker pid, result) and `worker.log`
- `ocr-models/` holds the shared OCR model files

`CardCell` (`src/session/vault.rs`) owns this layout; deleting a card's folder forces just that card to regenerate. Visual revisions hash the production feature and scene-composer prompts, the composer schema, both layout/device registries, and the manga template together with the manual `LAYOUT_POLICY_VERSION`, so concurrent application versions never overwrite one another. Bump that version whenever a scene model/configuration, local scene specialization/validation rule, or renderer acceptance policy changes without changing an embedded asset. Anki media names are decoupled from disk filenames in `src/anki/deck.rs` so per-card role-named files stay unique inside the `.apkg`.

## Language Profiles

Language-specific behavior belongs only in `src/languages` profile declarations. A profile defines:

- Gemini prompt display name
- OCR configuration
- default deck naming
- user-facing report labels

If a new language is needed, add a new profile instead of editing the fixed runtime orchestration logic.

## Releasing

The version in `Cargo.toml` is the release trigger; nothing is tagged or published by hand. Merging a version bump into `main` does the rest: a green `Rust` CI run fires `.github/workflows/auto-release-tag.yml`, which tags `v<version>` and dispatches `release-artifacts.yml` — five platform archives (linux x86_64/aarch64, macos arm64/x86_64, windows) plus `SHA256SUMS.txt`, published as a GitHub Release with generated notes. `workflow_dispatch` on either workflow is the manual fallback, and `install.sh` always serves the latest release.

`Cargo.toml` `version` and the `Release:` header in `llms.txt` are one bidirectional contract and must change together in the same commit. Any `llms.txt` change requires an application version bump; any application version bump requires review/update of `llms.txt` with the exact matching `Release:`. The automated agent-contract test rejects a mismatch. Release archives contain both the binary and `llms.txt`, and `kamishibai agent-contract` must print that file byte-for-byte.

Homebrew is a separate, manual follow-up in the tap repository **`anatoly-chichikov/homebrew-tap`** (https://github.com/anatoly-chichikov/homebrew-tap — a local checkout normally sits beside this repository; search for a `homebrew-tap` directory locally before cloning). In the tap: bump the version and sha256 values in `Formula/kamishibai.rb` (hashes come from the release's `SHA256SUMS.txt`), open a PR, wait for the bottles to build on CI, then publish them with `gh workflow run publish.yml -f pull_request=<PR number>`.

## Recording the demo GIF and screenshots

`docs/tui-states/live/capture.gif` (linked from `README.md`) and the per-screen PNGs next to
it are produced by three VHS tapes in `docs/tui-states/`:

- `capture.tape` runs the **live binary** (real Gemini) and writes the happy-path screenshots
  plus the raw `live/capture.gif`.
- `states.tape` drives the `examples/tui_states` **state-walker** (no Gemini) to write the
  synthetic edge-case / modal / Welcome screenshots that the live run cannot reach.
- `states-narrow.tape` drives the same state-walker at 1200 px to write the intentionally
  narrow S10 sentence-label screenshot; VHS accepts geometry only at the top of a tape.

The README gif itself is then assembled deterministically by `encode.sh` from `timings.conf`
(the single source of truth for section windows, durations, and raw source); it emits
`timings.timeline.txt` and splices the finale caption PNG (`live/caption.png`). A window reads
`RAW` when its source is `main` and `ADJUST_RAW` when its source is `adjust`, so a supplementary
interaction recording can be cut together with the original without transcoding either raw.
Only `live/capture.gif` is committed; pass `MAKE_HIRES=1` to `encode.sh` if you want a local-only
hi-res `capture.hires.mp4` master. The generation keyboard contract is `Ctrl+G`.

### Why no manual chord patch is required

`Ctrl+G` is a simple control byte, which crossterm reads as the generation hotkey in raw
mode. The old temporary `Ctrl+S` recording chord is obsolete and must not be reintroduced.

### Procedure

From the repo root:

1. **Build the binaries** (release for the live run, release example for the Welcome shot):

   ```bash
   cargo build --release
   cargo build --release --example tui_states
   ```

2. **Confirm the release binary is current**. No recording-only key patch is needed:

   ```bash
   cargo build --release
   ```

3. **Record the synthetic state shots** (state-walker, no Gemini calls):

   ```bash
   cd docs/tui-states
   vhs states.tape
   vhs states-narrow.tape
   rm -f states-throwaway.gif states-narrow-throwaway.gif
   ```

   Writes the six environment/failure/retry shots (`live/00-welcome.png`,
   `live/00b-welcome-env.png`, `live/03-change-something-modal.png`,
   `live/06-your-cards-retrying.png`, `live/06b-your-cards-retry-stress.png`,
   `live/07-your-cards-couldnt-finish.png`) plus the
   twelve sentence-label S1–S12 PNGs from `live/11-s1-label-tags.png` through
   `live/22-s12-label-legacy-meta.png` and the five Esc lifecycle PNGs from
   `live/23-esc-words-clear.png` through `live/27-generation-partial.png`. All are 2x except S10, whose intentionally narrow
   frame comes from `states-narrow.tape` at 1200 px. Both synthetic tapes jump to each state
   by **absolute index** (`Type "<n>"` then `Space`) and keep a uniform 800 ms settle after
   each jump so VHS never captures a mid-repaint frame. Absolute jumps are immune to
   keystroke coalescing and to the stray Return the shell injects when it launches the
   binary — `Enter` in the walker only clears the queued digits. The two Welcome shots are
   the same `EnterKey` stage: `00-welcome.png` has no `GEMINI_API_KEY` (just the `submit`
   button), `00b-welcome-env.png` has it set (adds the focused `load from env` chip).

4. **Record the live-binary flow** (real Gemini run, roughly 5–7 minutes wall-clock because
   the tape starts with an empty cache and later regenerates one tuned card):

   ```bash
   vhs capture.tape
   ```

   Writes `live/01-your-words.png`, `live/01b-busy.png`, `live/02-what-i-understood.png`,
   `live/02a-nav.png`, `live/03-senses.png`, `live/03b-senses-toggled.png`,
   `live/04-your-cards.png`, `live/08-done.png`, `live/09-card-adjusting.png`,
   `live/09a-level-raised.png`, `live/09b-card-regenerating.png`,
   `live/09c-card-regenerated.png`, `live/09-card-open.png`,
   `live/10-card-scroll-end.png`, and the full raw `live/capture.gif`.

5. **Stash the raw recording** before any post-processing — keep it around as `/tmp/raw.gif`
   so you can redo the slice/encode pass without re-running VHS or Gemini. If an interaction
   is recorded separately, preserve complete takes as `/tmp/adjust-raw.gif` and
   `/tmp/nav-adjust-raw.gif`; windows in `timings.conf` can name `main`, `adjust`, or `nav`.
   The README payload is built on top of these raws.

   ```bash
   cp live/capture.gif /tmp/raw.gif
   ```

   Do NOT delete any raw until you've reviewed the final gif and decided you don't need another
   timing iteration.

6. **Detect scene transitions** automatically — never assume the time windows from a previous
   recording apply. Gemini latency varies wildly between runs (this session swung between
   2 min and 7 min wall-clock).

   ```bash
   ffmpeg -i /tmp/raw.gif -vf "select='gt(scene,0.005)',showinfo" -f null - 2>&1 \
     | awk '/pts_time/{gsub(/.*pts_time:/,"");print $1}' > /tmp/transitions.txt
   cat /tmp/transitions.txt
   ```

   The `0.005` threshold catches the major TUI transitions (TUI screens change in only a
   slice of cells per frame, so the default `0.3` returns 0 hits). The number of transitions
   is **not fixed** — it grows when new states are added to the flow. Don't hardcode an
   expected count.

7. **Dump a frame at every transition** and eyeball them to map each one to a screen state.

   ```bash
   mkdir -p /tmp/cuts && rm -f /tmp/cuts/*.png
   ffmpeg -y -ss 0 -i /tmp/raw.gif -frames:v 1 /tmp/cuts/cut-00.png
   i=1
   while read t; do
     ffmpeg -y -ss "$t" -i /tmp/raw.gif -frames:v 1 /tmp/cuts/cut-$(printf %02d $i)-t${t}.png
     i=$((i+1))
   done < /tmp/transitions.txt
   open /tmp/cuts
   ```

8. **Classify each section** between consecutive transitions:

   | Type | Signal | Sampling for the gif |
   | --- | --- | --- |
   | **workflow** | user-driven step or new content (typing, candidates land, Done lands) | `fps=25` on the section's natural window; preserve real-time animation |
   | **read** | a state that's only briefly visible in the recording but the viewer needs time to read (e.g. WhatIUnderstood gets click-through via `Ctrl+G` after ~1 s) | static splice from the matching `live/NN-…png` for 2–3 s — duplicate frames; do NOT use the raw window |
   | **indicator-wait** | spinner / progress bar; visually static minus the rotating indicator (Gemini text pass, generation queue) | take short real-time windows at 25 fps from meaningful milestones; never resample one long wait into a time-lapse |
   | **transition** | a fast cross-fade between two states, < 1 s | usually skipped or rolled into the neighbouring section |

   For the standard kamishibai flow the typical mapping is:
   - `0s → first_busy`: A typing (workflow, 1.5 s output)
   - `first_busy → candidates_appear`: B busy understanding (indicator-wait, 1.2 s output)
   - candidates window: C `02-what-i-understood.png` static splice (read, 1–3 s output)
   - `building_starts → all_done`: D 0.2 s real-time windows around each visible redraw,
     including retry ticks, until the fifth card fills the viewport; then one 0.6 s publish
     transition jumps to the completed batch
   - first `all_done`: E navigate to `chouette`, open the editor, focus level, and move `a2 → b1`
   - `1 pending → all_done`: F hold the struck sentence, press `Ctrl+G`, then show each
     one-card regeneration artifact for one consistent 0.6 s beat
   - final `all_done → end`: G hold the rewritten collapsed `b1` card; the gif does not
     reopen the editor after regeneration

   New states (e.g. an extra confirmation step, a style picker) will surface as additional
   transitions — slot them into a type by inspecting the cut frame, don't drop them.

9. **Propose the slice plan to the operator** — print a table with section type, source
   window, sample rate, and projected output duration **before** running ffmpeg. Get the
   green light, then encode. Sample sketch:

   ```
   Section             Type             Source                  fps     Output
   A typing            workflow         main 0.24 → 1.92 s      25      1.68 s
   B understand/review mixed            main + static PNG       25/—    2.40 s
   C senses            workflow         main event windows      25      5.00 s
   D first generation  indicator-wait   20 × 0.2 s + publish    25      4.60 s
   E navigate + raise  workflow         nav event windows       25/—    5.36 s
   F regenerate        indicator-wait   five 0.6 s windows      25      3.00 s
   G result            fade             collapsed result PNG    —       3.24 s
   Total                                                               25.28 s
   ```

10. **Encode** once the plan is approved:

    ```bash
    RAW=/tmp/raw.gif ADJUST_RAW=/tmp/adjust-raw.gif \
      NAV_RAW=/tmp/nav-adjust-raw.gif ./encode.sh
    ```

    `encode.sh` prints the exact final duration and writes every section boundary to
    `timings.timeline.txt`. All raw recordings stay on disk for the next iteration.

### Common pitfalls — read before recording

- **Never sample a spinner section below 25 fps.** Each source frame represents N × 40 ms
  of real rotation; if N > 1, the spinner appears N × faster in the output. Keep the source
  fps high (25) and shorten the window instead.
- **Never treat WhatIUnderstood (or any other click-through state) as a workflow section.**
  `Ctrl+G` fires immediately after candidates land, so the raw recording shows it for ~1.5 s.
  Use the screenshot as a static splice for 2–3 s so the glosses are readable.
- **Never carry over hardcoded section windows from a prior recording.** Gemini latency
  varies. Run scene-detect first.
- **Never count transitions in advance.** New states get added to the flow over time —
  scene-detect surfaces them automatically; classify by inspecting `cut-NN.png`, don't drop
  unknown sections.
- **Never delete `/tmp/raw.gif`, `/tmp/adjust-raw.gif`, or `/tmp/nav-adjust-raw.gif` until
  you've decided you don't need another slice pass.** Re-recording costs a few minutes of
  Gemini wall-clock; re-slicing is local and preserves the original takes.
- **Don't try to "fix" an already-post-processed gif by duplicating its frames.** A
  derivative gif has already lost spinner sampling fidelity; you have to go back to the raw.

11. **Confirm** with `git status` that only the regenerated assets and intentional docs/code
    changes are staged. Keep the raws when the operator wants room for later timing changes.

### Demo input

`capture.tape` runs with `my_language=en`, so the target language resolves to French and the
header reads **`EN → FR`**. The tape types seven French words on `YourWords`: `dépaysement`,
`flâner`, `canard`, `chouette`, `râler`, `terroir`, `bof` — a mix of untranslatable nouns, a
verb, and colloquialisms (`canard` doubles as "duck" and "newspaper hoax"); all yield strong
manga panels and interesting English glosses. The synthetic `examples/tui_states.rs` walker
mirrors this EN→FR flow with the first four of those words. After the first complete build,
the tape opens the simple `chouette` card, moves its level exactly one step from `a2` to `b1`,
waits until the footer proves there is exactly `1 pending`, and presses `Ctrl+G`; this keeps
the regeneration story scoped to one card. The README gif ends on the rewritten collapsed
card. The tape may continue afterward to capture the separate open-card screenshots.

### Synthetic and edge-case shots

The six environment/modal/failure/retry PNGs, twelve sentence-label scenarios, and five Esc
lifecycle PNGs listed in step 3 are not produced by `capture.tape`. They are produced reproducibly by `states.tape`
and `states-narrow.tape`, which drive `examples/tui_states.rs` through the same EN→FR flow
without Gemini. The sentence-label scenarios keep the established indices 0–10 intact: S1
is index 6, S2 replaces the removed per-card modal at index 7, S3–S9 are indices 11–17,
S10–S12 are indices 18–20, the retry stress gallery is index 21, and the Esc clear/back/stop/drain/partial
states are indices 22–26. When the design changes, edit the demo data in
`examples/tui_states.rs` and re-run both synthetic tapes. If you add or reorder states in
the vector, update the absolute indices in both tapes and in the
`pty_state_demo_switches_mouse_pointer_between_link_and_plain_cells` test (it jumps to
the `Your cards` and `Done` indices by number).

The level chips are the lowercase operational CEFR bands `a1`, `a2`, `b1`,
`b2`, `c1`, and `c2`. They classify only the language surrounding the target
term; the target term itself is exempt, and the estimate is not an official
proficiency assessment. Fresh cards first get the natural sentence required by
their approved understanding and only then receive a descriptive level; initial
generation never targets a band. A level becomes a rewrite constraint only
after the user explicitly changes it. Legacy `easy`, `takes practice`/`balanced`,
and `challenging`/`stretch` cache values reopen as `a2`, `b1`, and `b2`
respectively.

The artifact rows stay together in one left column: `meta`, `audio`, `scene`,
then `picture`. `meta` begins immediately after the last line of the card head's
target sentence, including when that sentence wraps. A collapsed card leaves
the `meta` row alone and renders no `sentence:` heading or separator glyph
anywhere. Register, phrase kind, and level appear as three consecutive tags in one
fixed column on the `audio` row, separated by spaces; `ai is working…`, ready,
cached, inactive retry, and recovered audio keep that same anchor. Active
audio always shows the same `ai is working…` text, and retry history lives on
the card head instead of adding volatile status beside the tags. At narrow
widths whole tags may continue at the same tag-column on the `scene` and
`picture` rows; if a wrapped tag or an audio tail would collide, the complete
inline summary is hidden and the card head remains the mouse entry into tuning.
Opening the card removes that compact summary and puts the already-open
editor below all four artifact rows, separated from `picture` by exactly one
blank row, before the expanded metadata and never to the artifacts' right. If
the focused editor block fits the viewport, opening it anchors the selected card
head at the top of the body; shorter viewports instead scroll only far enough to
keep the focused row visible.
Unchanged tags use a gray background; explicitly changed or previously pinned
tags use a white background with dark letters and no bold. An approximately
fulfilled pinned tag keeps that white treatment and adds an `≈` prefix.

The editor's three carousel questions are `how should it sound?`, `what kind of
phrase?`, and `what's the desired level?`. The note label is `one more thing`, and its
placeholder is `say what should change`. The active carousel question is white
and bold, and the selected chip has a white background. Every carousel is
permanently bracketed by the two-cell direction controls `< ` and ` >`; both
cells are clickable, focus that control's own row, and move one adjacent choice
without wrapping past either boundary. All three tracks use one render-time
width derived from the widest choice and the largest choice count across the axes,
so both chevrons share columns. Inside that fixed track the selected chip's
visual centre moves proportionally from the leading edge to the trailing edge
as its choice index increases. Every adjacent step transfers one hidden-choice
segment from the trailing rail to the leading rail. Segment widths differ by at most one cell,
spare cells go nearest the selected chip on each side, and every cell of a
segment belongs to the same clickable target. The nearest marker uses
`DIM2`, the next farther marker uses `RULE`, and every marker farther away uses
`HL`, saturating at `HL`. A legacy axis with no selected value shows `—` with
one two-cell marker on each side inside the shared track; both cells of either
marker are clickable.

Regeneration carries the complete current three-axis preset. Every unedited
axis must keep its current value exactly. Only an explicitly changed or already
pinned axis may differ from the requested value, and only when the result marks
that axis as approximate.

A successfully published live batch remains on `YourCards`; reopening that
published session uses `Done`. Both final views permanently show the muted
`[Esc] new cards` immediately before `[Ctrl+C] quit`. The first `Esc` arms a
one-second confirmation and changes its hint to the highest-priority `[Esc]
again`; the second starts a clean `YourWords` batch in the same process,
preserving preferences and output location while rotating the persistent
session identity and cost journal. Any other action or timeout disarms the
confirmation. Everywhere else `Esc` closes exactly one layer from inside out:
an error, a modal/editor/expanded sense list, then the current screen action.
On nonempty `YourWords`, double `Esc` clears the field; on collapsed
`WhatIUnderstood`, one `Esc` returns to the preserved words; during generation,
double `Esc` stops after the current request finishes and launches no next
request. A stop publishes the complete subset as `partial`, or, when no card is
complete, closes the old run as `cancelled`, rotates identity and cost scope,
and returns to the preserved review. While the current request drains the
header says `stopping…`. The same reset remains available after every card
terminally gives up and no package can be published, once the publication error
has been dismissed. `Ctrl+C` keeps an independent double-press quit confirmation.

Expanded metadata uses statement and noun labels: `the phrase`, `in your
language`, `a visual clue`, `word meaning`, `word pronunciation`, `phrase
pronunciation`, `worth learning`, and, when context exists, `the right context`.
