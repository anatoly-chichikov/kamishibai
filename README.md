# kamishibai

![hero](docs/hero/hero.jpg)

You have a list of words you want to remember. kamishibai turns them into a deck where every card has a sentence in your language, the same sentence in the language you're learning, native-speaker audio, and an illustration that makes the word stick.

Drop the deck into Anki. That's it.

## Get it running

```bash
cargo run --release
```

kamishibai will ask for your Gemini API key on the welcome screen and remember it. If you'd rather not see that step, set `GEMINI_API_KEY` in your environment first.

The TUI walks you from raw words through review to finished cards, and writes the `.apkg` plus a printable PDF into `./kamishibai-out`.

![demo](docs/tui-states/live/capture.gif)

Got the JSON already? Pass it as an argument:

```bash
cargo run --release -- path/to/words.json
```

## Languages

kamishibai works for any source/target pair declared in [`src/languages`](src/languages). Adding a new pair is a profile, not a code branch.
