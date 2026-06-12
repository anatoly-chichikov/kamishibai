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

Got the JSON already? Pass it as an argument:

```bash
kamishibai path/to/words.json
```

## Console

For scripts and agents, kamishibai also runs headless as a curatable, asynchronous **session**: understand the words, curate which senses become cards, run a background worker that generates and publishes, poll for status, fetch the result.

```bash
# 1. understand the words and create a session — prints the id
id=$(kamishibai new --word "you're gonna get yours" --word bank --from ru --to en --out ./deck)

# 2. curate the understanding (optional)
kamishibai status "$id"                       # the candidates and their senses
kamishibai select "$id" --card bank --sense 2 # keep only the river-bank sense
kamishibai exclude "$id" --card bank          # …or drop the card entirely

# 3. generate + publish in the background, then poll
kamishibai generate "$id"
kamishibai status "$id"             # phase + per-card progress (no Gemini); -q for just the phase
kamishibai result "$id"            # the finished cards + deck.apkg / deck.pdf paths

# re-roll a committed card
kamishibai regenerate "$id" --failed
kamishibai regenerate "$id" --card bank --note "make the sentence shorter"
```

The output is plain text by default: stdout carries the one capturable value (id, or paths), everything else goes to stderr, so `id=$(kamishibai new --word bank)` just works. Every session verb also takes `--json` for exactly one JSON document per call — same exit codes, machine-readable error envelope. Sessions are idempotent and cache-backed: the cache holds one folder per card under `kamishibai cache-path`, so a re-run resumes from disk and only fills in what is missing. The TUI shares the same sessions — `kamishibai open "$id"` resumes one interactively. Agents should read [llms.txt](llms.txt) for the full session contract.

## Languages

Ten languages:

`en` English · `zh` Chinese · `es` Spanish · `ja` Japanese · `fr` French · `de` German · `ru` Russian · `it` Italian · `pt` Portuguese · `el` Greek

You pick your own language once — that's what the card labels are in. The language you're learning is detected from the words you type, so it changes from batch to batch: paste French words, you get a French deck; Japanese words, a Japanese one. All ten work either way, each with native-speaker voice.
