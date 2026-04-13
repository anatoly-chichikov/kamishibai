# kamishibai

Generate illustrated Anki decks from schema-driven vocabulary JSON.

## Development

The canonical runtime is Rust. The default developer flow is:

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

## Running

The main flow assumes a directory that contains `kamishibai.json`:

```bash
cargo run --
```

Pass an explicit file when needed:

```bash
cargo run -- path/to/my-words.json
```

Override the deck name:

```bash
cargo run -- --deck "Core Pack" path/to/my-words.json
```

Override output and cache locations:

```bash
cargo run -- --output ./output --cache ~/.cache/kamishibai path/to/my-words.json
```

Environment variables are also supported:

```bash
KAMISHIBAI_INPUT=path/to/my-words.json
KAMISHIBAI_OUTPUT=path/to/output
KAMISHIBAI_CACHE=path/to/cache
cargo run --
```

## Input Contract

Every entry must contain:

- `term`
- `source.sentence`
- `source.lang`
- `target.sentence`
- `target.lang`

Optional fields are `meaning`, `pronunciation`, `transcription`, `importance`, `source.highlight`, `source.hint`, and `source.context`.

Example:

```json
{
  "entries": [
    {
      "term": "cat",
      "meaning": "кошка",
      "pronunciation": "kæt",
      "transcription": "kat",
      "importance": 7,
      "source": {
        "sentence": "Кошка сидит на столе",
        "lang": "ru",
        "highlight": "Кошка",
        "hint": "домашнее животное",
        "context": "нейтральный стиль"
      },
      "target": {
        "sentence": "The cat sits on the table",
        "lang": "en"
      }
    }
  ]
}
```

`source.lang` drives report labels. `target.lang` drives audio prompts, scene prompts, OCR configuration, cache naming, and default deck naming.

## Architecture

The shipping runtime lives in Rust modules under `src/`:

- `src/cli.rs`
- `src/input.rs`
- `src/profile.rs`
- `src/paths.rs`
- `src/assets.rs`
- `src/gemini.rs`
- `src/cache.rs`
- `src/ocr.rs`
- `src/audio.rs`
- `src/scene.rs`
- `src/media.rs`
- `src/anki.rs`
- `src/report.rs`
- `src/progress.rs`
- `src/diagnosis.rs`

Language-specific behavior belongs only in [`src/profile.rs`](src/profile.rs). Add new languages there instead of branching runtime orchestration.

## Archived Python Reference

The old Python runtime is archived in [`python_reference/src/kamishibai`](python_reference/src/kamishibai). It is kept only as a migration oracle for parity fixtures and reference regeneration.

Reference-only Python commands still use `uv`:

```bash
uv sync
uv run pytest
uv run python scripts/regenerate_rust_parity.py
```

## External Requirements

- `GEMINI_API_KEY` must be set
- the first OCR-backed run downloads the required `PP-OCRv5` model files into the media cache
- `fc-match` is only needed when regenerating archived Python parity artifacts
