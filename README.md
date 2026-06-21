# kamishibai

<img src="docs/hero/hero.jpg" alt="kamishibai hero" width="40%" align="left">

You have a list of words from a language you're learning. **kamishibai** turns them into a deck where every card has a sentence in your language, the same sentence in the foreign one, native-speaker audio, and an illustration that makes the word stick.

Drop the deck into Anki. That's it.

The rest is discipline — sadly, that part doesn't ship in the .apkg.

> Example Deck (EN→FR): [**PDF**](docs/samples/fr-en.pdf) · [**Anki APKG**](docs/samples/fr-en.apkg)

<br clear="left">

## Why kamishibai

1. Spaced repetition turns recognition into recall — the gap between "I've seen this" and "I can use it".
2. One word in, full card out in seconds — image, sentence, audio. No 20-minute manual workflow, no quality cut.
3. Natural emotional voice from Gemini. Real intonation, not robotic playback.
4. Designed to look good, especially for manga readers — learning shouldn't feel like a spreadsheet.
5. Give it a word, get a sentence. You memorize phrases, not flashcards — and a language matrix forms in your head.
6. Bring your own Gemini key — your data, your control.

## Install

Install with the shell installer:

```bash
curl -fsSL https://raw.githubusercontent.com/anatoly-chichikov/kamishibai/main/install.sh | sh
```

Or through Homebrew:

```bash
brew install anatoly-chichikov/tap/kamishibai
```

## Run

```bash
kamishibai
```

If `GEMINI_API_KEY` is set in your environment, kamishibai picks it up and runs straight through. Otherwise it asks for the key on the welcome screen and remembers it for next time.

The TUI walks you from raw words through review to finished cards, and writes the `.apkg` plus a printable PDF into `./kamishibai-out`.

![demo](docs/tui-states/live/capture.gif)

### What You Get

kamishibai writes:

- an `.apkg` deck for Anki ([example](docs/samples/fr-en.apkg))
- a printable `.pdf` review sheet ([example](docs/samples/fr-en.pdf))

Import the deck into Anki, and the cards look roughly like this on your phone:

<p align="center">
  <img src="docs/previews/anki-card-front.jpg" alt="Anki card front preview" width="260">
  &nbsp;&nbsp;&nbsp;&nbsp;
  <img src="docs/previews/anki-card-back.jpg" alt="Anki card back preview" width="260">
</p>

Got the JSON already? Pass it as an argument:

```bash
kamishibai path/to/words.json
```

## Console

Prefer the terminal? kamishibai also runs headless — it understands the words, builds the deck in the background, and writes it out:

```bash
kamishibai new --word flâner --word canard   # creates a session; the language is autodetected
kamishibai generate --wait                   # generate + publish, blocking until done
kamishibai result                            # the finished cards + deck.apkg / deck.pdf
```

Set your language and Gemini key once — through the TUI's welcome screen or `kamishibai config`. Building an agent? [llms.txt](llms.txt) is the full console contract.

## Languages

Ten languages:

- `en` English
- `zh` Chinese
- `es` Spanish
- `ja` Japanese
- `fr` French
- `de` German
- `ru` Russian
- `it` Italian
- `pt` Portuguese
- `el` Greek

Choose your language once. The app uses it for card labels, then detects what you're learning from each batch: paste French words for a French deck, Japanese words for a Japanese deck.
