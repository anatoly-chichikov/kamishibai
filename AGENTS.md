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
it are produced by two VHS tapes in `docs/tui-states/`. The pipeline is manual end-to-end —
do NOT add automation that lands the recording-only chord patch (described below) in
production code. The keyboard contract on `Submit` stays exactly `Shift+Enter`.

### Why a manual chord patch is required

VHS cannot synthesize the kitty-format CSI u byte sequence that crossterm needs to recognize
`Shift+Enter`: VHS sends the ESC byte and the bracket bytes through the pty with delays that
exceed crossterm's escape-sequence timeout, so the parser falls back to plain Enter. The
recording therefore needs a temporary `Ctrl+S` → `Submit` chord, gated behind an env flag so
it cannot escape locally.

### Procedure

From the repo root:

1. **Build the binaries** (release for the live run, release example for the Welcome shot):

   ```bash
   cargo build --release
   cargo build --release --example tui_states
   ```

2. **Apply the recording-only chord patch** to `src/tui/input.rs`. Locate the
   `KeyCode::Char(symbol) if key.modifiers.contains(KeyModifiers::CONTROL)` arm and add the
   `'s'` case beneath the existing `'c'` and `'l'` cases:

   ```rust
   's' if std::env::var_os("KAMISHIBAI_RECORDING_HOTKEYS").is_some() => {
       Some(AppEvent::Submit)
   }
   ```

   The chord only fires when the env var is set, so the patch never affects production
   users. Re-build:

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

5. **Post-process the gif** into the variable-speed README payload. The raw recording is
   90 % static spinner frames; the README version compresses those and preserves the
   typing animation:

   ```bash
   cp live/capture.gif /tmp/capture-original.gif
   mkdir -p /tmp/seq && rm -f /tmp/seq/*.png
   i=1
   # A: typing 0–1.7s at 25 fps (full animation)
   ffmpeg -y -ss 0 -t 1.7 -i /tmp/capture-original.gif -vf "fps=25" /tmp/a-%03d.png
   for f in /tmp/a-*.png; do cp "$f" /tmp/seq/$(printf %04d $i).png; i=$((i+1)); done
   rm /tmp/a-*.png
   # B: busy understanding 2–6s at 12.5 fps (spinner stays animated, half the frames)
   ffmpeg -y -ss 2 -t 4 -i /tmp/capture-original.gif -vf "fps=12.5" /tmp/b-%03d.png
   for f in /tmp/b-*.png; do cp "$f" /tmp/seq/$(printf %04d $i).png; i=$((i+1)); done
   rm /tmp/b-*.png
   # C: WhatIUnderstood static splice for 1.2s (30 frames)
   ffmpeg -y -i live/02-what-i-understood.png -pix_fmt rgba /tmp/what-rgba.png
   for k in $(seq 1 30); do cp /tmp/what-rgba.png /tmp/seq/$(printf %04d $i).png; i=$((i+1)); done
   # D: generation 6.5–121s sampled at 0.66 fps (~75 frames in 3s of output)
   ffmpeg -y -ss 6.5 -t 114 -i /tmp/capture-original.gif -vf "fps=0.66" /tmp/d-%03d.png
   for f in /tmp/d-*.png; do cp "$f" /tmp/seq/$(printf %04d $i).png; i=$((i+1)); done
   rm /tmp/d-*.png
   # E: done state 121–122s at 25 fps (full speed, shows panel cleanly)
   ffmpeg -y -ss 121 -t 1 -i /tmp/capture-original.gif -vf "fps=25" /tmp/e-%03d.png
   for f in /tmp/e-*.png; do cp "$f" /tmp/seq/$(printf %04d $i).png; i=$((i+1)); done
   rm /tmp/e-*.png
   # Encode with palette for size
   ffmpeg -y -framerate 25 -i /tmp/seq/%04d.png -filter_complex "[0:v]palettegen=max_colors=64[p]" -map "[p]" /tmp/palette.png
   ffmpeg -y -framerate 25 -i /tmp/seq/%04d.png -i /tmp/palette.png -filter_complex "[0:v][1:v]paletteuse" -loop 0 live/capture.gif
   rm -rf /tmp/seq /tmp/palette.png /tmp/what-rgba.png /tmp/capture-original.gif
   ```

   Final gif is ~9 s, ~400 KB at 1152×864.

6. **Revert the chord patch** before staging anything:

   ```bash
   git restore src/tui/input.rs
   ```

7. **Confirm** with `git diff src/tui/input.rs` (must be empty) and `git status` (only the
   regenerated assets should be staged).

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
