# kamishibai

<img src="docs/hero/hero.jpg" alt="kamishibai hero" width="40%" align="left">

You have a list of words from a language you're learning. **kamishibai** turns them into a deck where every card has a sentence in your language, the same sentence in the foreign one, native-speaker audio, and an illustration that makes the word stick.

Drop the deck into Anki. That's it.

The rest is discipline — sadly, that part doesn't ship in the .apkg.

> Example EN -> FR: [**Printable PDF**](docs/samples/fr-en.pdf) · [**Anki deck (.apkg)**](docs/samples/fr-en.apkg)

<br clear="left">

## Why kamishibai

1. Spaced repetition turns recognition into recall — the gap between "I've seen this" and "I can use it".
2. One word in, full card out in seconds — image, sentence, audio. No 20-minute manual workflow, no quality cut.
3. Natural emotional voice from Gemini. Real intonation, not robotic playback.
4. Designed to look good, especially for manga readers — learning shouldn't feel like a spreadsheet.
5. Give it a word, get a sentence. You memorize phrases, not flashcards — and a language matrix forms in your head.
6. Bring your own Gemini key — your data, your control.

## Get it running

```bash
cargo run --release
```

If `GEMINI_API_KEY` is set in your environment, kamishibai picks it up and runs straight through. Otherwise it asks for the key on the welcome screen and remembers it for next time.

The TUI walks you from raw words through review to finished cards, and writes the `.apkg` plus a printable PDF into `./kamishibai-out`.

![demo](docs/tui-states/live/capture.gif)

Got the JSON already? Pass it as an argument:

```bash
cargo run --release -- path/to/words.json
```

## Languages

Pairs ship as profiles in [`src/languages`](src/languages): `en`, `zh`, `es`, `ja`, `fr`, `de`, `ru`, `it`, `pt`, `el`. Native side picks one of these for the UI labels in your language; target side picks one of these for the deck — Gemini generates the audio, so the practical ceiling on adding a new pair is whether Gemini TTS speaks it (it covers most of the world).
