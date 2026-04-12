# kamishibai

Generate illustrated Anki decks from vocabulary JSON.

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

The main user flow should be one command from a directory that contains `kamishibai.json`:

```bash
uv run kamishibai
```

If your file has a different name, pass it explicitly:

```bash
uv run kamishibai path/to/my-words.json
```

Optional deck name override:

```bash
uv run kamishibai --deck "Greek Basics" path/to/my-words.json
```

Advanced path overrides:

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

## External Requirements

- `GEMINI_API_KEY` must be set
- `tesseract` must be installed
- required Tesseract language packs must be installed
- `fc-match` from fontconfig must be available for PDF font resolution
