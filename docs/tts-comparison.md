# Gemini 3.8 speech and listening comparison

New speech synthesis uses `gemini-3.8-flash-tts`. Google announced the Flash and
Flash-Lite TTS models on September 23, 2026; the API release notes record GA on
September 22. Flash is the default here for its pronunciation and voice fidelity.
The comparison also includes `gemini-3.8-flash-lite-tts` and the previous
`gemini-3.1-flash-tts-preview`.

## Listen locally

```sh
cargo run --example tts_compare
```

Open `output/tts-compare/index.html` to compare four phrases in English, Russian,
French, and Greek. Each row uses the same text and the same prebuilt voice, Kore,
across all three models. Generation uses the application's Gemini client and WAV
writer. The example pins the voice at its transport boundary; normal card
generation retains the existing voice pool.

The run makes at most twelve speech requests, using `GEMINI_API_KEY` or the saved
application key. It produces local WAV files, a listening page, and a manifest
with model, text, voice, timing, and reported token usage. Completed samples are
reused on a repeat run. Use a different output directory for a fresh comparison:

```sh
cargo run --example tts_compare -- output/tts-compare-fresh
```

These calls can incur Gemini charges. The generated media stays in the ignored
`output/` directory. This is a small listening sample, not a statistical quality
benchmark. Compare pronunciation, natural pacing, question intonation, and whether
the clip speaks exactly the displayed sentence.

The September 25, 2026 live run completed all twelve requests with HTTP 200.
Every saved file decoded as nonempty 24 kHz mono PCM16, with durations from 4.60
to 6.16 seconds. The reported usage totals approximately $0.0233 at Standard
paid-tier rates. Browser playback was checked; these checks establish working
delivery and valid audio, not a subjective quality ranking.

A separate end-to-end session generated two EN → RU cards (`подъезд` and
`договариваться`) and reached `published`. Both speech cost records name
`gemini-3.8-flash-tts`. The Anki package contains two notes and two cards with
their two WAVs and two JPEGs; its SQLite integrity check passed. The one-page
PDF was rendered and visually checked for readable Cyrillic, IPA, and layout.

## API migration

Gemini 3.8 treats the text part as a verbatim transcript. Delivery instructions
therefore go into `speech_metadata.style` instead of being prepended to the text.
The request explicitly selects `AUDIO_L16` at 24 kHz so the existing PCM-to-WAV
writer adds exactly one WAV header. The new unary API otherwise defaults to an
already wrapped WAV. Legacy model overrides keep their previous request schema.

Existing cached card audio is retained. The new default applies when audio is
actually synthesized; the comparison uses its own cache and does not change
existing sessions or decks. Explicit Gemini profiles continue to separate their
cache identities by model and prompt policy.

Standard paid-tier estimates use the published promotional rates through
December 31, 2026, and the published regular rates from January 1, 2027 (UTC):

| Model | Input / output per million tokens through 2026 | Input / output from 2027 |
| --- | --- | --- |
| Gemini 3.8 Flash TTS | $0.50 / $9.00 | $1.00 / $18.00 |
| Gemini 3.8 Flash-Lite TTS | $0.50 / $6.00 | $1.00 / $12.00 |
| Gemini 3.1 Flash TTS Preview | $1.00 / $20.00 | $1.00 / $20.00 |

## Sources

- [Google announcement, September 23, 2026](https://blog.google/innovation-and-ai/models-and-research/gemini-models/gemini-3-8-text-to-speech/)
- [Gemini API release notes](https://ai.google.dev/gemini-api/docs/changelog)
- [Gemini 3.8 Flash TTS model and migration guide](https://ai.google.dev/gemini-api/docs/models/gemini-3.8-flash-tts)
- [GenerateContent speech schema and audio formats](https://ai.google.dev/gemini-api/docs/generate-content/speech-generation)
- [Gemini API pricing](https://ai.google.dev/gemini-api/docs/pricing)

Sources verified on September 25, 2026.
