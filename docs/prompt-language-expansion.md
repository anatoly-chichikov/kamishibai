# Task: expand kamishibai from 11 to 21 supported languages

You are working in the kamishibai repository (Rust, Anki-deck generator with Gemini-backed
metadata, TTS audio, and manga illustrations). Read `AGENTS.md` and `llms.txt` first; they
are authoritative for architecture, session semantics, and the release contract.

## Mission

Add ten languages in one branch, in three tiers, each language usable in **both roles**
(`known` and `learning`):

- **Tier 1**: Korean `ko`
- **Tier 2**: Turkish `tr`, Polish `pl`, Ukrainian `uk`, Indonesian `id`
- **Tier 3**: Hindi `hi`, Arabic `ar`, Thai `th`, Hebrew `he`, Vietnamese `vi`

All three tiers ship together in this branch. Do not merge to `main` yourself: merging a
version bump triggers the release pipeline. Open one draft PR at the end and stop there.

## Ground facts (verified — do not re-derive, but do verify at the call sites)

1. **Profiles**: `src/languages/registry.rs` holds the canonical fixed-size array
   `[LanguageProfile; 11]` (`code`, `prompt` display name, `ocr` token string, `DeckNaming`,
   `UiLabels`). Order = global learning popularity and drives the Welcome chips, the
   `Cmd+L` picker, and the Gemini language list. Insert new languages by popularity:
   Korean belongs right after the Japanese/German cluster; Hindi/Arabic/Turkish before
   Greek/Dutch; Hebrew near the tail. Every fixed-size `11` (array types, `codes()`
   signature) grows to `21`.
2. **Prompt examples**: `assets/prompt_examples.json` needs one typed entry per language
   (`spacing`, `understanding`, `card`, `recall`, ~1.4 KB each). `src/languages/prompt_examples.rs`
   panics unless the JSON keys exactly cover `profile_codes()` and every entry passes its
   typed `valid()` policy — study that validator before writing examples. Examples must be
   native-quality sentences in the target language, mirroring the register and shape of the
   existing `fr`/`ja`/`zh` entries, not literal translations of them.
3. **Side effect to accept**: `recall_document()` serializes recall examples of **all**
   languages into the visual-policy hash, so this change mints a new visual revision for
   every existing card. Expected and acceptable; do not bump `LAYOUT_POLICY_VERSION` for
   this (an embedded asset changed, the hash moves by itself).
4. **OCR models**: `src/generation/manga/ocr_bundle.rs` downloads PP-OCRv5 bundles from
   `zibo-chen/rust-paddle-ocr` branch `next`. Upstream **has** rec models + charsets for:
   `korean`, `arabic`, `devanagari`, `th`, `ta`, `te`, `cyrillic`, `eslav`, `el`, `en`,
   `latin`, plus the multilingual default (`zh`/`ja`). Upstream has **no** Hebrew model,
   and the `latin` charset does not reliably cover Vietnamese tone stacks.
5. **Text gate routing** (this is the user's explicit instruction): a language whose script
   has a usable upstream OCR bundle keeps the OCR text gate — `ko` → new `Korean` bundle,
   `hi` → new `Devanagari` bundle, `th` → new `Th` bundle, `ar` → new `Arabic` bundle,
   `tr`/`pl`/`id` → existing `Latin`, `uk` → existing `Cyrillic` (token `ukr` is already
   routed). A language with **no usable OCR** (`he`, `vi`) must send the manga text check
   **directly to an LLM-as-a-judge** instead — never attempt OCR for them, and do not fall
   back to the English recognizer.
6. **LLM judge seam**: a Gemini vision judge already exists — `RecallJudge` /
   `GeminiRecall` in `src/generation/card_production/gemini_media.rs` (prompt + schema in
   `assets/picture_recall_judge_prompt.txt|schema.json`), and
   `src/generation/card_production/picture_recovery.rs` maps failure slugs
   `"ocr" | "recall_text"` to a recovery category. Model the new text judge on that
   pattern: a dedicated port, structured JSON output, Flash-tier model, verdicts that keep
   the existing attempt/fault taxonomy and recovery mapping working. Which gate a language
   uses must be declared in its language profile (per `AGENTS.md`: language-specific
   behavior lives only in `src/languages` profile declarations) — prefer a typed
   declaration over a magic token inside the `ocr` string.
7. **RTL and shaping — handle it, don't over-engineer it**: Arabic and Hebrew are RTL;
   Devanagari needs real shaping (matra reordering, ligatures); Thai stacks combining
   marks. The PDF card sheet (`src/report/`) currently does naive per-glyph LTR layout
   with a three-track font palette (Arial / Hiragino Sans / Arial Unicode MS fallback,
   glyph-presence dispatch via `carries()` in `src/report/font.rs`). Requirements:
   - RTL text must come out in the correct direction with correct contextual Arabic forms —
     isolated-form left-to-right garbage is not shippable. Adding mature crates
     (`rustybuzz` for shaping, `unicode-bidi`) to `src/report` is allowed and expected.
   - Extend the font fallback chain with macOS system fonts where Arial Unicode MS is not
     enough for a script; keep the glyph-presence dispatch approach.
   - Anki HTML cards: RTL languages need `dir="rtl"` on the relevant fields in the note
     model/templates (`src/anki/`), driven from the profile.
   - TUI: terminals own RTL rendering; just make sure widths/wrapping don't panic or
     misalign the layout on RTL and Thai strings.
   Aim for "correct and readable", not typographically perfect.
8. **TTS**: the wired `gemini-3.1-flash-tts-preview` supports 70+ languages including all
   ten (Hebrew/Thai/Vietnamese are Preview-stage per Google's list). The voice pool is a
   fixed 30-voice set with language auto-detection; verify pronunciation empirically in the
   validation phase rather than adding wire parameters. The Gemini wire contract in
   `src/gemini/client.rs` is frozen — the lowercase `target_lang` stays the only lowercase
   code, and prompts to Gemini stay in English.
9. **Contract & version**: `llms.txt` lists supported languages (Languages section) — it
   must gain the ten codes. `llms.txt` `Release:` header and `Cargo.toml` `version` are one
   bidirectional contract changed in the same commit (an automated test enforces it). Bump
   the minor version. Release archives and `agent-contract` are already wired.
10. **UI at 21 languages**: `catalog().codes()` feeds the TUI in `src/tui/app.rs:558`,
    `src/tui/pointer.rs:83`, `src/tui/transition.rs:414+`. Verify the Welcome chips and the
    language picker render sanely with 21 entries at narrow widths (the S10 screenshot
    width, 1200 px, is the established narrow reference). Do not re-record demo GIFs or
    screenshots; just keep the UI from breaking.
11. **Separation rules**: `tests/separation.rs` enforces module boundaries and the single
    composition root (`src/cli/wiring.rs`). Respect them.

## Per-language deliverables checklist

For each of the ten languages:

- [ ] `LanguageProfile` entry: code, Gemini display name, OCR/judge declaration,
      `DeckNaming` ("Korean Vocabulary", "ko", …), `UiLabels` translated natively
      (Translation / Context / Hint / Importance equivalents).
- [ ] `assets/prompt_examples.json` typed entry passing `valid()`.
- [ ] Text gate: OCR bundle routing or judge declaration per fact 5.
- [ ] Fonts verified for the script on the PDF sheet (front and back, both roles).
- [ ] Anki note renders correctly (direction attribute for `ar`/`he`).
- [ ] Both roles smoke-tested offline: `<lang>` as learning with EN known, and as known
      with EN learning (labels, deck name, cache path `cards/EN-<L>` / `cards/<L>-EN`).

## Validation phase — real generations, hard budget

Offline tests must never touch the real user config, cache, or output — inject
`KAMISHIBAI_DATA`, `KAMISHIBAI_CACHE`, `KAMISHIBAI_OUTPUT`, and `KAMISHIBAI_GEMINI_URL`
(stub on `127.0.0.1:0`); a past regression wiped the saved API key this way.

After all offline tests are green, validate against real Gemini. A saved API key or
`GEMINI_API_KEY` is expected to be present; if neither resolves, stop and report instead of
asking for a key. Never print the key.

**Hard cap: 300 real card generations total** (a "generation" = one word submitted to
`generate`/`regenerate`, i.e. one card attempted end-to-end; internal artifact retries do
not count separately). Suggested allocation — adjust as findings demand, never exceed the
cap:

| Slice | Cards |
| --- | --- |
| `ko` both roles (deep pass) | 40 |
| `tr`, `pl`, `uk`, `id` — 15 each | 60 |
| `hi`, `ar`, `th` — 30 each (shaping + OCR risk) | 90 |
| `he`, `vi` — 30 each (LLM-judge path) | 60 |
| Reserve for re-runs after fixes | 50 |

Pick 5-word batches per language that stress the script: diacritic stacks (`vi`), tone
marks (`th`), ligature-heavy words (`hi`, `ar`), mixed Hangul syllable shapes (`ko`),
plus one colloquialism per language. Drive everything through the headless session flow
(`kamishibai new … --generate --wait --json`, `status --json`, `result --json`) with
`KAMISHIBAI_OUTPUT` pointed at the report workspace. Keep a journal of every run: session
id, language pair, words, outcome per card, retries, cost, gate verdicts (OCR vs judge).
Stop generating the moment the budget is spent; report what remains unvalidated.

## Mandatory final report (in Russian)

Produce a self-contained report directory `validation-report/` in the repo root (add it to
`.gitignore` — WAV/JPG payloads must not be committed) with an `index.html` that works
offline from the local filesystem:

- **Per language × role**: the generated cards — picture, source/target sentences, labels,
  and an `<audio controls>` player for **every generated WAV** (this is non-negotiable: the
  operator will listen to the pronunciations), the text-gate verdict (OCR or LLM judge,
  with the verdict content), retries spent, cost.
- **PDF proof**: for each new script, embed a rendered card-sheet page image (front and
  back) so RTL/shaping quality is visible, including at least one `ar` and one `he` page.
- **Summary table**: language, role, cards attempted/published, average retries, total
  cost, gate type, verdict quality notes.
- **Findings**: per-language pronunciation quality notes (TTS), image-model script
  legibility, OCR vs judge disagreements, anything deferred.
- Report prose in Russian; keep code identifiers in English.

Final message must include: branch name, PR link, absolute path to
`validation-report/index.html` (as a `file://` link), total generations spent vs the 300
cap, and the list of anything left unvalidated.

## Done means

- One branch, one draft PR, not merged.
- 21 languages in the catalog; all offline tests green (`fmt`, `clippy -D warnings`,
  `test`).
- `Cargo.toml` + `llms.txt` bumped together; agent-contract test green.
- `he`/`vi` demonstrably use the LLM text judge (show it in the report journal).
- Validation journal shows ≤300 real generations.
- `validation-report/index.html` exists with listenable audio for every new language.
