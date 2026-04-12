# kamishibai

Generate illustrated Anki decks from schema-driven vocabulary JSON.

## Development

Install dependencies with:

```bash
uv sync
```

Run the test suite with:

```bash
uv run pytest
```

Build distributions with:

```bash
uv build
```

## Running

The main flow assumes a directory that contains `kamishibai.json`:

```bash
uv run kamishibai
```

Pass an explicit file when needed:

```bash
uv run kamishibai path/to/my-words.json
```

Override the deck name:

```bash
uv run kamishibai --deck "Core Pack" path/to/my-words.json
```

Override output and cache locations:

```bash
uv run kamishibai --output ./output --cache ~/.cache/kamishibai path/to/my-words.json
```

Environment variables are also supported:

```bash
KAMISHIBAI_INPUT=path/to/my-words.json
KAMISHIBAI_OUTPUT=path/to/output
KAMISHIBAI_CACHE=path/to/cache
uv run kamishibai
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

`source.lang` drives source-side labels and report font selection. `target.lang` drives audio prompt language, scene prompt language, OCR configuration, cache naming, and default deck naming.

## Language Profiles

Language-specific behavior lives in declarative profiles in [`src/kamishibai/config.py`](src/kamishibai/config.py) and [`src/kamishibai/target.py`](src/kamishibai/target.py).

Each profile defines:

- display name for prompts
- OCR configuration
- cache directory names
- default deck naming
- report font family
- report labels

Adding a new language should only require adding one new profile entry.

## External Requirements

- `GEMINI_API_KEY` must be set
- `tesseract` must be installed
- required Tesseract language packs must be installed
- `fc-match` from fontconfig must be available for PDF font resolution
