# Changelog

## 1.19.1

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
