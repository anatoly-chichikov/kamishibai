# AGENTS.md

This file provides guidance to Codex when working in this repository.

## Project Overview

`kamishibai` is a Python 3.9+ application that converts schema-driven vocabulary JSON into Anki decks with AI-generated audio and manga-style illustrations.

## Development Commands

Install dependencies:

```bash
uv sync
```

Run tests:

```bash
uv run pytest
```

Run the application:

```bash
uv run kamishibai
```

## Required Environment

- `GEMINI_API_KEY` must be set before running the application
- `tesseract` and the required language packs must be installed
- `fc-match` must be available for PDF font resolution

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

- `src/kamishibai/input.py`: validates the JSON document and normalizes entries
- `src/kamishibai/target.py`: immutable language-profile objects
- `src/kamishibai/config.py`: declarative language registry, deck naming, labels, and font selection
- `src/kamishibai/runtime.py`: builds audio, illustration, OCR, and Gemini clients from profiles
- `src/kamishibai/media.py`: runs the media pipeline and assembles processed entries
- `src/kamishibai/anki.py`: defines the language-neutral Anki note model
- `src/kamishibai/report.py`: builds the PDF report with profile-driven labels and fonts
- `src/kamishibai/cli.py`: orchestrates the end-to-end command-line flow

## Language Profiles

Language-specific behavior belongs only in `config.py` profile declarations. A profile defines:

- prompt display name
- OCR configuration
- cache directory naming
- default deck naming
- report font family
- user-facing report labels

If a new language is needed, add a new profile instead of editing runtime orchestration logic.
