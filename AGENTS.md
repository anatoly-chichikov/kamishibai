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
- `kamishibai open [<id>]`: open the session in the interactive TUI (resumes from the cache)
- `kamishibai result [<id>]` / `ls` / `cancel [<id>]` / `rm [<id>] [--cache]` / `cache-path`
- `kamishibai regenerate [<id>] (--failed | --card T [--note "…"]) [--wait]`: re-roll committed cards — drop their cached artifacts and immediately regenerate + republish (runs a worker like `generate`); with `--note`, Gemini first rewrites the card from the instruction
- `kamishibai config [--known L] [--key K]`: save console defaults to preferences (no flags → show them) — `--known` (validated) so word sessions need no `--known`, and `--key` (verified through Gemini `models.list`; `-` reads it from stdin, empty clears it) so you need not export `GEMINI_API_KEY`; the key value is never printed back

There are exactly two output modes: **plain text** (default, for humans — line-oriented, not a parsing target) and **`--json`** (placed after the verb, for machines — exactly one JSON document on stdout: the success document, or the `{"ok":false,"error":{"code","exit","message","hint","retryable"}}` envelope on failure; `generate --wait --json` additionally streams NDJSON events on stderr). `agent-contract` is the text-only exception and refuses `--json`. There is no `-q` and no `result` path selectors — an agent uses `--json`. Exit codes, locking, and semantics are identical in both modes for invocations valid in both; `open` is interactive and also refuses `--json` before any session lookup. The full console contract lives in `llms.txt`. Plain output carries no bare capturable value — every single-session command opens with the header `your session <ID> · <KNOWN> → <LEARNING> · <phase>` and the id lives there; errors are one `kamishibai: <message>` line plus a next-step hint line on stderr. **Language codes are the app's canonical UPPERCASE form everywhere** — stored in config and `session.json`, minted into ids (`FR-…`), used in the cache layout (`cards/EN-FR`) and deck names (`FR_….apkg`), and emitted in plain and JSON; input is accepted in any case and normalised to uppercase, and the only lowercase code is the frozen `target_lang` on the Gemini wire (`src/gemini/client.rs`). Exit codes are centralized in `src/cli/error.rs` (`Refusal` carries the exit, optional hint, retryability, and optional session listing): `0` ok · `2` usage · `3` no such session · `4` not ready · `5` ambiguous · `1` other. The `<id>` positional is optional on every verb: an omitted id resolves to the only session, else the only unfinished one, else the command lists the newest five sessions and exits 5 (`session::resolve`). The background worker is the same binary re-invoked as the hidden `__run <id>`, detached into a new process group with its stdio redirected to `sessions/<id>/worker.log`. Concurrency is two flocks: the long-held liveness lock (`sessions/<id>/lock`, OS-released on death) decides who may generate — `status` derives `interrupted` from a recorded worker whose lock is free — and the short write lock makes every `session.json` change a serialized read-modify-write (`SessionStore::update`), so concurrent edits all apply. The worker writes only while the record still names it, which is how `cancel` and a finishing worker resolve their race. The TUI shares this same session model — it takes the liveness lock before generating and persists its live state to `session.json`, so `ls`/`status`/`open` see interactive runs too. The full agent-facing contract lives in `llms.txt` at the repo root. `--out` wins, `KAMISHIBAI_OUTPUT` is second, and new sessions otherwise resolve the platform Documents directory plus `Kamishibai`; resolved output is stored per session. For offline tests, `KAMISHIBAI_GEMINI_URL` overrides the Gemini base URL (point it at a 127.0.0.1 listener), `KAMISHIBAI_CACHE` overrides the exact cache root, `KAMISHIBAI_DATA` overrides the data home before `kamishibai/preferences.json` is appended, and `KAMISHIBAI_OUTPUT` overrides the exact output root.

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
- `src/cli/session`: the console (API) layer — `store` (the `session.json` record + serialized atomic `create`/`update` IO), `worker` (the managed background worker + the `__run` entrypoint, ownership-guarded writes), `liveness` (the two flocks + pid kill via rustix), `view` (the cache-derived status projection both renders share), `json` (the `Serialize` DTOs + the one emit seam), and one handler module per concern (`new`, `curate`, `generate`, `result`, `maintenance`) routed by `mod.rs`. This layer never links the TUI (`tests/separation.rs` enforces it): `open` hands the checked record to the `SessionOpener` port
- `src/cli/bridge.rs`: the TUI side of the session contract — projects between the live `App` and the persisted record, owns the `TuiSession` the shell claims and writes, and implements `SessionOpener` over `run_tui`

Within the card-workflow boundary, direct dependencies point inward: CLI delivery → concrete Gemini / production / publishing adapters → application ports and session domain values. `tests/separation.rs` rejects reverse imports and prevents workflow adapters from being composed outside `src/cli/wiring.rs`; legacy cache-backed session types are outside this narrower claim.

## Attempts

An artifact gets one plain try plus three retries on top of it — `ARTIFACT_ATTEMPT_CEILING` (4) attempts, which is also the durable picture-request series ceiling. **The first try is never numbered**: while it runs the step row just says `ai is working…`, and only a retry carries a number (`AttemptTally::retry`, `1..=retries`). Every spent attempt records **why** it was spent: `src/session/attempt.rs` pairs the `AttemptTally` with one `AttemptFault` per failure (`category` slug, user-facing `reason`, and the archived picture when the provider drew one). The production adapter supplies the renderer's real verdict; anything else — transport error, cache lease, exhausted request budget — is diagnosed by the engine from the error text under category `error`. Both surfaces number retries the same way, so at one moment the TUI row and the NDJSON line agree: `retry 2/3` means two tries are gone and the second retry is under way. The TUI step row also carries a muted `N rejected` note — plain text, not a control — and it **outlives the retries**: a `✓` row keeps showing what the artifact cost to reach, so a finished card still tells the story of its rejections. The expanded card shows the meta preview first and then, below a dashed rule, a `rejected attempts` block; each row names the try, whatever that try produced before being thrown away, and the gate that rejected it. Both stages leave something behind: a picture attempt archives the rejected frame, and a scene attempt archives the model reply it failed to decode (`RejectedReply` carries the body out of `src/gemini`, `attempt_archive::archived_reply` writes it as `scene-NNNN.json` when it parses as JSON and `scene-NNNN.txt` when it never was JSON). Both are muted underlined links that open with the system handler. A failure that never reached the model — transport, cache lease — archives nothing and leaves that column blank. Rejected frames are never deleted by a run — only `drop_artifacts` / `drop_incomplete_artifacts` clear them.

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
it are produced by two VHS tapes in `docs/tui-states/`:

- `capture.tape` runs the **live binary** (real Gemini) and writes the happy-path screenshots
  plus the raw `live/capture.gif`.
- `states.tape` drives the `examples/tui_states` **state-walker** (no Gemini) to write the
  synthetic edge-case / modal / Welcome screenshots that the live run cannot reach.

The README gif itself is then assembled deterministically by `encode.sh` from `timings.conf`
(the single source of truth for section windows/durations); it emits `timings.timeline.txt`
and splices the finale caption PNG (`live/caption.png`). Only `live/capture.gif` is committed; pass
`MAKE_HIRES=1` to `encode.sh` if you want a local-only hi-res `capture.hires.mp4` master. The
generation keyboard contract is `Ctrl+G`.

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
   rm -f states-throwaway.gif
   ```

   Writes `live/00-welcome.png`, `live/00b-welcome-env.png`,
   `live/03-change-something-modal.png`, `live/05-change-this-card-modal.png`,
   `live/06-your-cards-retrying.png`, and `live/07-your-cards-couldnt-finish.png` at 2x. The
   tape jumps to each state by **absolute index** (`Type "<n>"` then `Space`) and keeps a
   uniform 800 ms settle after each jump so VHS never captures a mid-repaint frame. Absolute
   jumps are immune to keystroke coalescing and to the stray Return the shell injects when it
   launches the binary — `Enter` in the walker only clears the queued digits. The two Welcome
   shots are the same `EnterKey` stage: `00-welcome.png` has no `GEMINI_API_KEY` (just the
   `submit` button), `00b-welcome-env.png` has it set (adds the focused `load from env` chip).

4. **Record the live-binary flow** (real Gemini run, ~2 minutes wall-clock with a warm
   cache, ~4 minutes cold):

   ```bash
   vhs capture.tape
   ```

   Writes `live/01-your-words.png`, `live/01b-busy.png`, `live/02-what-i-understood.png`,
   `live/02a-nav.png`, `live/03-senses.png`, `live/03b-senses-toggled.png`,
  `live/04-your-cards.png`, `live/08-done.png`, `live/09-card-open.png`,
  `live/10-card-scroll-end.png`, and a raw
   `live/capture.gif` that is roughly two minutes long.

5. **Stash the raw recording** before any post-processing — keep it around as `/tmp/raw.gif`
   so you can redo the slice/encode pass without re-running VHS. The raw is 1–5 min long and
   ~1 MB; the README payload is built on top of it.

   ```bash
   cp live/capture.gif /tmp/raw.gif
   ```

   Do NOT delete `/tmp/raw.gif` until you've reviewed the final gif and decided you don't
   need another iteration.

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
   | **indicator-wait** | spinner / progress bar; visually static minus the rotating indicator (Gemini text pass, generation queue) | compress aggressively. `fps = output_frames / source_duration`. Budget 1–2 s output total no matter how long the source is |
   | **transition** | a fast cross-fade between two states, < 1 s | usually skipped or rolled into the neighbouring section |

   For the standard kamishibai flow the typical mapping is:
   - `0s → first_busy`: A typing (workflow, 1.5 s output)
   - `first_busy → candidates_appear`: B busy understanding (indicator-wait, 1.2 s output)
   - candidates window: C `02-what-i-understood.png` static splice (read, 2.5 s output)
   - `building_starts → all_done`: D generation (indicator-wait, 1.5–2 s output)
   - `all_done → end`: E done (workflow, 1 s output)

   New states (e.g. an extra confirmation step, a style picker) will surface as additional
   transitions — slot them into a type by inspecting the cut frame, don't drop them.

9. **Propose the slice plan to the operator** — print a table with section type, source
   window, sample rate, and projected output duration **before** running ffmpeg. Get the
   green light, then encode. Sample sketch:

   ```
   Section          Type             Source           fps         Output
   A typing         workflow         1.0 → 2.5 s      25          1.5 s   (38 frames)
   B busy           indicator-wait   2.56 → 3.76 s    25          1.2 s   (30 frames)
   C candidates     read (splice)    static PNG       —           2.5 s   (62 frames)
   D generation    indicator-wait   11 → 405 s       0.114       1.8 s   (45 frames)
   E done           workflow         404.76 → 405.76  25          1.0 s   (25 frames)
   Total                                                          8.0 s   (200 frames)
   ```

10. **Encode** once the plan is approved:

    ```bash
    mkdir -p /tmp/seq && rm -f /tmp/seq/*.png
    i=1
    # Repeat per section: ffmpeg -ss <start> -t <dur> -i /tmp/raw.gif -vf "fps=<rate>" /tmp/x-%03d.png
    # then: for f in /tmp/x-*.png; do cp "$f" /tmp/seq/$(printf %04d $i).png; i=$((i+1)); done
    # For static splices: cp the chosen live/NN-…png N times into /tmp/seq/

    ffmpeg -y -framerate 25 -i /tmp/seq/%04d.png \
      -filter_complex "[0:v]palettegen=max_colors=64[p]" -map "[p]" /tmp/palette.png
    ffmpeg -y -framerate 25 -i /tmp/seq/%04d.png -i /tmp/palette.png \
      -filter_complex "[0:v][1:v]paletteuse" -loop 0 live/capture.gif

    rm -rf /tmp/seq /tmp/palette.png /tmp/cuts /tmp/transitions.txt
    ```

    Final gif is ~8–10 s, ~300–500 KB at 1152×864. `/tmp/raw.gif` stays on disk for the next
    iteration.

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
- **Never delete `/tmp/raw.gif` until you've decided you don't need another slice pass.**
  Re-recording costs a few minutes of Gemini wall-clock; re-slicing is seconds.
- **Don't try to "fix" an already-post-processed gif by duplicating its frames.** A
  derivative gif has already lost spinner sampling fidelity; you have to go back to the raw.

11. **Confirm** with `git status` that only the regenerated assets and intentional docs/code
    changes are staged. Once you're sure you don't need another slice pass,
    `rm /tmp/raw.gif`.

### Demo input

`capture.tape` runs with `my_language=en`, so the target language resolves to French and the
header reads **`EN → FR`**. The tape types seven French words on `YourWords`: `dépaysement`,
`flâner`, `canard`, `chouette`, `râler`, `terroir`, `bof` — a mix of untranslatable nouns, a
verb, and colloquialisms (`canard` doubles as "duck" and "newspaper hoax"); all yield strong
manga panels and interesting English glosses. The synthetic `examples/tui_states.rs` walker
mirrors this EN→FR flow with the first four of those words.

### Edge-case shots

The six PNGs that need modal interaction or environment/failure injection
(`00-welcome.png`, `00b-welcome-env.png`, `03-change-something-modal.png`,
`05-change-this-card-modal.png`, `06-your-cards-retrying.png`, `07-your-cards-couldnt-finish.png`)
are not produced by `capture.tape`. They are produced reproducibly by `states.tape` (step 3),
which drives `examples/tui_states.rs` through the same EN→FR flow at 2x. When the design
changes, edit the demo data in `examples/tui_states.rs` and re-run `vhs states.tape`. If you
add or reorder states in the vector, update the absolute indices in `states.tape` and in the
`pty_state_demo_switches_mouse_pointer_between_link_and_plain_cells` test (it jumps to the
`Your cards` and `Done` indices by number).
