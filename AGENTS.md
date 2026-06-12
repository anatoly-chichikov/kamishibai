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

- `kamishibai new (--word W [--word W…] | --words FILE | --build FILE) [--to L] [--from L] [--senses primary|all] [--id NAME] [--generate]`: understand the words (exactly one input form; `--build` imports a cards JSON whose entries carry the pair, so it rejects `--from`/`--to`/`--senses`) and create a session in the **understood** stage (`--to` is autodetected from the words when omitted)
- `kamishibai select <id> --card T --sense 1,3` / `exclude <id> --card T` / `correct <id> --card T --note "…"`: curate the understanding before generating — pick senses, drop a card, or ask Gemini to add senses (each resets the session to understood)
- `kamishibai generate <id> [--wait]`: commit the curated plan and start a managed background worker that generates + publishes (`--wait` runs it in the foreground)
- `kamishibai status <id> [-q]`: stage + per-candidate senses (understood) or per-card progress (generating/published), read from the cache (no Gemini); `-q` prints just the phase word
- `kamishibai open <id>`: open the session in the interactive TUI (resumes from the cache)
- `kamishibai result <id> [-q | --deck | --pdf | --dir]` / `ls [-q]` / `cancel <id>` / `rm <id> [--cache]` / `cache-path`
- `kamishibai regenerate <id> (--failed | --card T [--note "…"])`: re-roll committed cards — with `--note`, Gemini first rewrites the card from the instruction

Output is **plain text only — never JSON.** stdout carries the one capturable value (a session id, a path), so `id=$(kamishibai new --word bank)` works; everything else (the understood-senses preview, progress, errors) goes to stderr. Exit codes are centralized in `src/cli/error.rs` (`Refusal`): `0` ok · `2` usage · `3` no such session · `4` not ready · `1` other. The background worker is the same binary re-invoked as the hidden `__run <id>`, detached into a new process group with its stdio redirected to `sessions/<id>/worker.log`. Concurrency is two flocks: the long-held liveness lock (`sessions/<id>/lock`, OS-released on death) decides who may generate — `status` derives `interrupted` from a recorded worker whose lock is free — and the short write lock makes every `session.json` change a serialized read-modify-write (`SessionStore::update`), so concurrent edits all apply. The worker writes only while the record still names it, which is how `cancel` and a finishing worker resolve their race. The TUI shares this same session model — it takes the liveness lock before generating and persists its live state to `session.json`, so `ls`/`status`/`open` see interactive runs too. The full agent-facing contract lives in `llms.txt` at the repo root. For offline tests, `KAMISHIBAI_GEMINI_URL` overrides the Gemini base URL (point it at a 127.0.0.1 listener) and `KAMISHIBAI_CACHE` overrides the cache root.

## Architecture

The runtime is split into a few focused modules:

- `src/vocabulary`: validates the strict JSON document and exposes canonical entry types
- `src/languages`: keeps language profiles, naming, labels, and report font preferences
- `src/runtime`: resolves paths and renders progress and diagnosis output
- `src/gemini`: talks to Gemini through the frozen direct REST contract
- `src/generation`: writes cached WAV audio, composes scenes, routes OCR, validates manga output, and orchestrates the fixed Gemini production pipeline
- `src/anki`: defines the language-neutral Anki note model and APKG writer
- `src/report`: builds the PDF report with layout, thumbnails, and font resolution
- `src/cli.rs`: parses arguments (clap) and routes to the interactive TUI or a `session` subcommand
- `src/cli/console.rs`: generation primitives shared by the session worker — the `produce` engine loop (meta → sound → scene → picture, then publish) and the `Reporter` port (human / quiet)
- `src/cli/session`: the session model — `store` (the `session.json` record + serialized atomic `create`/`update` IO), `bridge` (projects between the live TUI `App` and the persisted record, plus the `TuiSession` the shell claims and writes), `worker` (the managed background worker + the `__run` entrypoint, ownership-guarded writes), `liveness` (the two flocks + pid kill via rustix), `view` (cache-derived status projection), and one handler module per concern (`new`, `curate`, `generate`, `result`, `maintenance`) routed by `mod.rs`

## Cache layout

The cache (printed by `kamishibai cache-path`) groups one folder per card, keyed by a content hash of the card identity:

- `cards/<from>-<to>/<key>/` holds `meta.json`, `scene.json`, `voice.wav`, and `illustration.jpg` for one card
- `understanding/<from>-<to>/<key>.json` holds the understanding-pass result
- `sessions/<id>/` holds `session.json` (identity, phase, words, curated candidates, committed plan, worker pid, result) and `worker.log`
- `ocr-models/` holds the shared OCR model files

`CardCell` (`src/session/vault.rs`) owns this layout; deleting a card's folder forces just that card to regenerate. Anki media names are decoupled from disk filenames in `src/anki/deck.rs` so per-card role-named files stay unique inside the `.apkg`.

## Language Profiles

Language-specific behavior belongs only in `src/languages` profile declarations. A profile defines:

- Gemini prompt display name
- OCR configuration
- default deck naming
- user-facing report labels

If a new language is needed, add a new profile instead of editing the fixed runtime orchestration logic.

## Recording the demo GIF and screenshots

`docs/tui-states/live/capture.gif` (linked from `README.md`) and the per-screen PNGs next to
it are produced by two VHS tapes in `docs/tui-states/`:

- `capture.tape` runs the **live binary** (real Gemini) and writes the happy-path screenshots
  plus the raw `live/capture.gif`.
- `states.tape` drives the `examples/tui_states` **state-walker** (no Gemini) to write the
  synthetic edge-case / modal / Welcome screenshots that the live run cannot reach.

The README gif itself is then assembled deterministically by `encode.sh` from `timings.conf`
(the single source of truth for section windows/durations); it emits `timings.timeline.txt`
and splices the caption label PNGs (`live/cap-type.png`, `live/cap-pick.png`,
`live/cap-generate.png`, `live/caption.png`). Only `live/capture.gif` is committed; pass
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
   `live/04-your-cards.png`, `live/08-done.png`, `live/09-card-open.png`, and a raw
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
