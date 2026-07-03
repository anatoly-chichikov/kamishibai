# Cards JSON

When kamishibai builds a deck from words, Gemini writes every card field for you. But the card format itself is plain JSON, and kamishibai accepts it from anywhere: a prompt to an LLM, a script over your reading history, your own hands. If you want to control every sentence that lands on a card, write this document and hand it over.

A batch built from JSON skips the writing pass entirely. Your text goes onto the cards verbatim; kamishibai adds only the two things JSON can't carry — native-speaker audio and the illustration, both generated from `target.sentence`.

## Feed it in

```bash
kamishibai cards.json                # review and build in the TUI
```

Or headless:

```bash
kamishibai new --build cards.json
kamishibai generate --wait
```

Finished decks round-trip: `kamishibai result --json` returns the cards in this same schema, so what comes out can be edited and fed back in.

## The document

```json
{
  "entries": [
    {
      "term": "flâner",
      "meaning": "to stroll, to wander aimlessly",
      "pronunciation": "flɑne",
      "transcription": "lə dimɑ̃ʃ ʒɛm flɑne lə lɔ̃ də la sɛn",
      "importance": 6,
      "source": {
        "sentence": "On Sundays I like to stroll along the Seine.",
        "lang": "en",
        "highlight": "stroll",
        "hint": "Not marcher toward somewhere, but drifting for the pleasure of it.",
        "context": "**Meaning.**\n- strolling: walking with no destination, for the pleasure of it.\n\n**Where you'll hear it.**\nLazy Sunday plans, travel writing, songs about Paris.\n\n**Where it's out of place.**\nWalking somewhere on purpose — that's marcher.\n\n**Subtlety.**\nThe flâneur — the person who does this — came into English from this verb."
      },
      "target": {
        "sentence": "Le dimanche, j'aime flâner le long de la Seine.",
        "lang": "fr"
      }
    }
  ]
}
```

`source` is the language you already know; `target` is the language you're learning. One document is one deck: every entry must carry the same language pair.

## How a card is assembled

The front asks, the back answers:

- **Front**: the illustration, `source.sentence` with `source.highlight` in bold, and `source.hint` beneath it.
- **Back**: the audio, `target.sentence`, then `term` with `pronunciation` and `meaning`, the `importance` score, and `source.context` as a usage note. Anki also shows `transcription` under the sentence.

## Fields

The contract is strict: every field is required, unknown fields are rejected, and every text value must be non-empty.

| Field | What it is |
| --- | --- |
| `term` | The word being learned, in the exact surface form you want taught — an inflected form stays inflected. Anki lowercases it on the card back; the PDF keeps the exact form. |
| `meaning` | A short gloss of the term in your language. A gloss, not a definition. |
| `pronunciation` | IPA of the term, without slashes or brackets. |
| `transcription` | IPA of the whole `target.sentence`, one space between words. |
| `importance` | Integer 1–10: how core the word is. Display only — it never affects scheduling. |
| `source.sentence` | The example sentence in your language: a faithful translation of `target.sentence`. This is the card front. |
| `source.lang` | Your language code (see below). |
| `source.highlight` | The exact substring of `source.sentence` that corresponds to the term. Bolded on the front, so it must appear in the sentence verbatim. |
| `source.hint` | One short line under the front sentence that points at the sense without giving the term away. |
| `source.context` | A usage note in markdown, revealed on the back: where the word lives, where it's out of place. |
| `target.sentence` | The example sentence in the learning language. The heart of the card: it lands on the back, the voice reads it, and the illustration is drawn from it. |
| `target.lang` | The learning language code. |

## House style

Validation checks structure, not taste. What makes the fields good is the style kamishibai's own generator follows — match it and hand-written cards sit next to generated ones without a seam:

- `target.sentence` is a natural sentence a native speaker would actually say, using the term in one specific sense. Remember it also drives the audio and the picture: a concrete, visual sentence makes a better card than an abstract one.
- `source.hint` is contrastive — "not X, but Y" — separating the term from its nearest neighbour in the learning language, naming that neighbour verbatim. Under a dozen words, and never mentioning the term or its translation.
- `source.context` is four short sections, each opened by a bold header on its own line, written in the source language — the equivalents of `**Meaning.**` (up to three `- ` bullets), `**Where you'll hear it.**`, `**Where it's out of place.**` (name the safer word to swap in), `**Subtlety.**` (one trap or false friend). The house style uses only `**bold**` and `- ` bullets, with blank lines between sections.

## Languages

Ten codes, accepted in any case and normalized to uppercase: `EN`, `ZH`, `ES`, `JA`, `FR`, `DE`, `RU`, `IT`, `PT`, `EL`.
