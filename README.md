# kamishibai

You have a list of words you want to remember. **kamishibai** turns them into a deck where every card has a sentence in your language, the same sentence in the language you're learning, native-speaker audio, and an illustration that makes the word stick.

Drop the deck into Anki. That's it.

<p align="center">
  <img src="docs/hero/hero.jpg" alt="kamishibai hero" width="360">
</p>

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

kamishibai works for any source/target pair declared in [`src/languages`](src/languages). Adding a new pair is a profile, not a code branch.
