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
