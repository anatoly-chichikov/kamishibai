# Changelog

## 1.19.1

- Check explanation pronunciation claims and contrast reasons before polishing prose, omitting unsupported details and repeated reasons.

- Calibrate intentional omission of redundant construction advice only in the matching Russian-to-English data; prefer positive usage facts over invented incorrect alternatives and preserve meaning labels during editing.

- Accept equivalent quotation markers during explanation editing while still rejecting changed learning words, and allow repair to omit an unsupported optional nuance.

- Limit literal idiom translations to the words in the idiom, without appended scenes or origin stories.

- Keep the first explanation note within its complete sense label and exclude narrower synonyms from usage warnings.

- Present form changes through everyday example phrases and edit translations as natural reader-language sentences.

- Check each quoted combination against the sentence sense and remove register advice whose replacement does not fit its setting.

- Use positive nuance facts for fresh cards and identify altered learning phrases explicitly in repair feedback.

- Keep post-edit repairs in the editor role and omit rewritten examples from their input so a repair cannot revive a discarded sense.

- Make an ambiguous first sense explicit from its sentence and calibrate that qualification with separate pair data.

- Calibrate Russian-to-English explanations with complete examples restricted to that exact pair, without reusing evaluation cards or changing other readers' examples.

- Keep explanation facts and examples tied to the sentence's sense, avoid invented usage restrictions, and retain valid learning text when an editor corrupts it; reject foreign scripts in learning fragments and recognize short native words in echo checks.

- Inject explanation examples from the reader language; style checks for office wording, empty openers, grammar terms, and address forms currently apply to Russian readers, while other readers receive the shared structural checks.

- Preserve legacy note fields and GUIDs with an explicit explanation format, including imported JSON and rejected explanation attempts.
- Retain the metadata explanation without paid explanation requests above five meanings; accept natural single-character meanings and review non-Latin mistake reasons and quote ordering.
- Preserve native sentence punctuation and combining accents, correct Hebrew high quotes, and keep French quote spaces unbreakable in new explanations.

## 1.19.0

- Add reviewed, edited explanations with ordered meanings, reader-language quotation marks, short reasons for common mistakes, and omitted empty sections.
- Render the card's meaning brighter in Anki, terminal previews, and printable cards while keeping old note fields and identifiers unchanged.
- Expose the offline explanation judge and composer for embedding, and account for every bounded explanation request without refreshing existing cached cards.

## 1.18.0

- Add Latin in both language roles with localized examples, report labels, and Latin-script OCR.
- Request restored Classical Latin pronunciation through the existing language profile in metadata and speech prompts.
