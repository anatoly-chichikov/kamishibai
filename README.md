# kamishibai

<img src="docs/hero/hero.jpg" alt="kamishibai hero" width="40%" align="left">

You have a list of words from a language you're learning. **kamishibai** writes a sentence around each one — a context aimed at that exact word — and turns it into a card: the sentence in your language, the same sentence in the foreign one, native-speaker audio, and an illustration that makes the word stick. You memorize phrases in context, not flashcards.

Drop the deck into Anki. That's it.

Built for people who actually do the reps: kamishibai multiplies your effort, it doesn't replace it. The rest is discipline — sadly, that part doesn't ship in the .apkg.

> Example Deck (EN→FR): [**PDF**](docs/samples/fr-en.pdf) · [**Anki APKG**](docs/samples/fr-en.apkg)

<br clear="left">

## Three ways to drive it

- **[By hand](#run)** — the TUI walks you from raw words to a finished deck.
- **[Headless](#console)** — a console API for scripts and agents that build decks on the fly; the contract is [llms.txt](llms.txt).
- **[Your own JSON](#bring-your-own-json)** — you write every sentence, kamishibai adds the voice and the picture; the contract is [docs/cards-json.md](docs/cards-json.md).

## Why kamishibai

1. Spaced repetition turns recognition into recall — the gap between "I've seen this" and "I can use it".
2. Every sentence is written for its word — one sense, your language pair, nothing pulled from a textbook. Phrase by phrase, a language matrix forms in your head.
3. One word in, full card out in seconds — image, sentence, audio. No 20-minute manual workflow, no quality cut.
4. Natural emotional voice from Gemini. Real intonation, not robotic playback.
5. Designed to look good, especially for manga readers — learning shouldn't feel like a spreadsheet.
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

kamishibai asks once — your language and your Gemini key, on the welcome screen — and remembers both. If `GEMINI_API_KEY` is set, the welcome screen offers to load it.

The TUI walks you from raw words through review to finished cards, and writes the `.apkg` plus a printable PDF into `./kamishibai-out`.

![demo](docs/tui-states/live/capture.gif)

### What You Get

kamishibai writes:

- an `.apkg` deck for Anki ([example](docs/samples/fr-en.apkg))
- a printable `.pdf` review sheet ([example](docs/samples/fr-en.pdf))

The front asks: the illustration, the sentence in your language, a hint. The back answers: the same sentence in the learning language, read by a native voice, with the gloss, IPA, and a usage note.

Import the deck into Anki, and the cards look roughly like this on your phone:

<p align="center">
  <img src="docs/previews/anki-card-front.jpg" alt="Anki card front preview" width="260">
  <img src="docs/previews/anki-card-back.jpg" alt="Anki card back preview" width="260">
</p>

## Console

The same flow runs headless — kamishibai understands the words, builds the deck in the background, and writes it into the same `./kamishibai-out`. Sessions persist across invocations, so an agent can drive the whole flow:

```bash
kamishibai new --word flâner --word canard   # creates a session; the language is autodetected
kamishibai generate --wait                   # generate + publish, blocking until done
kamishibai result                            # the finished cards + the deck and PDF paths
```

Set your language and Gemini key once — through the TUI's welcome screen or `kamishibai config`. Building an agent? [llms.txt](llms.txt) is the full console contract.

## Bring Your Own JSON

You don't have to let Gemini write the cards. The card format is plain, strict JSON, and kamishibai takes it from anywhere: an LLM you prompt yourself, a script over your reading history, your own hands. Entries land on the cards verbatim — the writing pass is skipped — and kamishibai adds only what JSON can't carry: the audio and the illustration, both generated from the target sentence.

```bash
kamishibai cards.json                # review and build in the TUI
kamishibai new --build cards.json    # or headless, as usual
```

Decks round-trip, too: `kamishibai result --json` returns the finished cards in the same schema, ready to edit and feed back in. The per-field contract lives in [docs/cards-json.md](docs/cards-json.md).

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
