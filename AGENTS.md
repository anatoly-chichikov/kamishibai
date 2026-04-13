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

Reference-only Python workflow:

```bash
uv sync
uv run pytest
uv run python scripts/regenerate_rust_parity.py
```

## Required Environment

- `GEMINI_API_KEY` must be set before running the application
- the first OCR-backed run downloads the required `PP-OCRv5` model files into the media cache
- `fc-match` is only needed when regenerating archived Python parity artifacts

## Input Schema

Every entry must contain:

- `term`
- `source.sentence`
- `source.lang`
- `target.sentence`
- `target.lang`

Optional fields:

- `meaning`
- `pronunciation`
- `transcription`
- `importance`
- `source.highlight`
- `source.hint`
- `source.context`

Normalized entries always carry both `source_lang` and `target_lang`.

## Architecture

The runtime is split into a few focused modules:

- `src/input.rs`: validates the JSON document and normalizes entries
- `src/profile.rs`: immutable language profiles, naming, labels, and report fonts
- `src/paths.rs`: resolves input, output, and cache locations from args and env
- `src/gemini.rs`: talks to Gemini through the frozen direct REST contract
- `src/ocr.rs`: routes legacy OCR tokens to cached PaddleOCR bundles and downloads model files
- `src/audio.rs`: writes cached WAV audio
- `src/scene.rs`: translates scenes, runs OCR checks, and validates manga output
- `src/media.rs`: wires per-language services and orchestrates the batch pipeline
- `src/anki.rs`: defines the language-neutral Anki note model and APKG writer
- `src/report.rs`: builds the PDF report with profile-driven labels and fonts
- `src/progress.rs`: renders plain and rich progress output
- `src/diagnosis.rs`: renders plain and rich startup diagnostics
- `src/cli.rs`: orchestrates the end-to-end command-line flow

## Language Profiles

Language-specific behavior belongs only in `profile.rs` declarations. A profile defines:

- prompt display name
- OCR configuration
- cache directory naming
- default deck naming
- report font family
- user-facing report labels

If a new language is needed, add a new profile instead of editing runtime orchestration logic.

## Archived Python Reference

The old Python runtime lives in `python_reference/src/kamishibai`. It is kept only as a parity oracle and reference-fixture generator, not as a shipping entrypoint.
