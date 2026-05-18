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

- `GEMINI_API_KEY` must be set before running the application
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

## Architecture

The runtime is split into a few focused modules:

- `src/vocabulary`: validates the strict JSON document and exposes canonical entry types
- `src/languages`: keeps language profiles, naming, labels, and report font preferences
- `src/runtime`: resolves paths and renders progress and diagnosis output
- `src/gemini`: talks to Gemini through the frozen direct REST contract
- `src/generation`: writes cached WAV audio, composes scenes, routes OCR, validates manga output, and orchestrates the fixed Gemini production pipeline
- `src/anki`: defines the language-neutral Anki note model and APKG writer
- `src/report`: builds the PDF report with layout, thumbnails, and font resolution
- `src/cli.rs`: orchestrates the end-to-end command-line flow

## Language Profiles

Language-specific behavior belongs only in `src/languages` profile declarations. A profile defines:

- Gemini prompt display name
- OCR configuration
- cache directory naming
- default deck naming
- report font family
- user-facing report labels

If a new language is needed, add a new profile instead of editing the fixed runtime orchestration logic.

## Recording the demo GIF and screenshots

`docs/tui-states/live/capture.gif` (linked from `README.md`) and the per-screen PNGs next to
it are produced by two VHS tapes in `docs/tui-states/`. The pipeline is manual end-to-end.
The generation keyboard contract is `Ctrl+G`.

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

3. **Record the Welcome shot** (state-walker, no Gemini calls):

   ```bash
   cd docs/tui-states
   vhs welcome.tape
   rm -f welcome-throwaway.gif
   ```

   Writes `live/00-welcome.png`.

4. **Record the live-binary flow** (real Gemini run, ~2 minutes wall-clock with a warm
   cache, ~4 minutes cold):

   ```bash
   vhs capture.tape
   ```

   Writes `live/01-your-words.png`, `live/01b-busy.png`, `live/02-what-i-understood.png`,
   `live/04-your-cards.png`, `live/04b-your-cards-mid.png`, `live/08-done.png`, and a raw
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

Tape types five English words on `YourWords` (`my_language=ru` is the user preference, so
the target language is non-Russian): `lantern`, `harbor`, `moonlight`, `bittersweet`,
`homesick`. Mix of concrete + emotional terms; all yield strong manga panels.

### Edge-case shots

The four PNGs that need failure injection or modal interaction
(`03-change-something-modal.png`, `05-change-this-card-modal.png`,
`06-your-cards-retrying.png`, `07-your-cards-couldnt-finish.png`) are intentionally not
produced by `capture.tape`. Re-snap them via `examples/tui_states.rs` when the design
changes enough to need fresh references.
