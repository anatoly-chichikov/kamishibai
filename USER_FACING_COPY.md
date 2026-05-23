# User-Facing Copy Inventory

Источник правды: `/Users/anatoly/Source/kamishibai/README.md`.

Ниже собраны актуальные user-facing строки после выравнивания с README.

## 1. CLI Help

Файл: `/Users/anatoly/Source/kamishibai/src/cli.rs`

```text
Turn a list of words into an illustrated Anki deck — sentences, native-speaker audio, manga-style art.

Usage: kamishibai [WORDS_JSON]

Arguments:
  [WORDS_JSON]  Optional path to a pre-built words JSON. If omitted, kamishibai walks you through the TUI.

Options:
  -h, --help     Print help
  -V, --version  Print version

With WORDS_JSON:
  Bring your own JSON with the required fields. kamishibai skips word entry,
  then uses its prompts to generate an Anki .apkg, a printable PDF,
  native-speaker audio, and manga-style illustrations.

WORDS_JSON format:
{
  "entries": [
    {
      "term": "lantern",
      "meaning": "a portable lamp",
      "pronunciation": "LAN-tern",
      "transcription": "/lantern/",
      "importance": 7,
      "source": {
        "sentence": "I carried a lantern through the dark hallway.",
        "lang": "en",
        "highlight": "lantern",
        "hint": "portable light",
        "context": "a simple everyday sentence"
      },
      "target": {
        "sentence": "Ich trug eine Laterne durch den dunklen Flur.",
        "lang": "de"
      }
    }
  ]
}

JSON rules:
  - entries must contain at least one item
  - all fields are required; unknown fields are rejected
  - text fields and lang values must be non-empty strings
  - importance must be an integer from 1 to 10
```

Короткий usage при лишних аргументах:

```text
usage: kamishibai [WORDS_JSON]   # optional; without it kamishibai opens the TUI
```

## 2. Cargo Metadata

Файл: `/Users/anatoly/Source/kamishibai/Cargo.toml`

```toml
description = "Turn a list of words into an illustrated Anki deck with native-speaker audio"
```

## 3. Homebrew Formula

Файл: `/Users/anatoly/Source/homebrew-tap/Formula/kamishibai.rb`

```ruby
desc "Turn a list of words into an illustrated Anki deck with native-speaker audio"
```

## 4. Tap README

Файл: `/Users/anatoly/Source/homebrew-tap/README.md`

```text
kamishibai turns a list of words you want to learn into an illustrated Anki deck plus a printable PDF. Each card has a sentence in your language, the same sentence in the target one, native-speaker audio, and an illustration.

Set GEMINI_API_KEY before running, or paste the key on the welcome screen.
```

## 5. Welcome Screen

Файл: `/Users/anatoly/Source/kamishibai/src/tui/screens/welcome.rs`

```text
kamishibai
set up two things
```

```text
kamishibai turns a list of words you want to learn into an anki deck plus a printable pdf. for each word it writes a natural example sentence, illustrates the scene as a manga panel, and reads it aloud in a natural, native-speaker voice.
```

## 6. Visible Workflow Copy

Файл: `/Users/anatoly/Source/kamishibai/src/tui/screens/modals.rs`

```text
your language
```

Файл: `/Users/anatoly/Source/kamishibai/src/tui/screens/your_words.rs`

```text
words you want to learn
each line becomes one anki card
```

Файл: `/Users/anatoly/Source/kamishibai/src/tui/app.rs`

```text
building your anki deck
rendering your printable pdf
```
