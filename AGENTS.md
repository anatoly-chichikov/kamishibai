# AGENTS.md

This file provides guidance to Codex when working in this repository.

## Project Overview

`kamishibai` is a Rust application that converts schema-driven vocabulary JSON into Anki decks with AI-generated audio and manga-style illustrations.

## Development Commands

Primary Rust workflow:

```bash
cargo fmt
cargo clippy --all-targets --all-features -- -D warnings
cargo test
```

Run the application:

```bash
cargo run --
```

## Required Environment

- a Gemini API key is required for any flow that calls Gemini — either via `GEMINI_API_KEY` (which wins) or a key previously saved through the Welcome screen; `GEMINI_API_KEY` need not be set when a saved key exists
- the first OCR-backed run downloads the required `PP-OCRv5` model files into the media cache

## Input Schema

Every entry must contain:

- `term`
- `meaning`
- `pronunciation`
- `transcription`
- `importance`
- `source.sentence`
- `source.lang`
- `source.highlight`
- `source.hint`
- `source.context`
- `target.sentence`
- `target.lang`

The input contract is strict. There are no optional entry fields.

## Sessions (non-interactive)

With no arguments `kamishibai` opens the interactive TUI; a bare JSON path opens the TUI on a prebuilt batch. Everything non-interactive is a **session** subcommand — a persistent, curatable unit of work an agent drives across invocations. A session moves through stages: understood → (curate) → generating → published (or **partial** when some cards fail but the deck still ships the rest, **failed** when no card survives).

- `kamishibai agent-contract`: print the version-matched `llms.txt` embedded in the installed binary; use this before any remote copy
- `kamishibai new (--word W [--word W…] | --words FILE | --build FILE) [--learning L] [--known L] [--senses primary|all] [--level a1|a2|b1|b2|c1|c2] [--types best-fit|statements|questions|dialogue|mixed] [--id NAME] [--generate [--wait]]`: understand the words (exactly one input form; `--build` imports a cards JSON whose entries carry the pair, so it rejects `--known`/`--learning`/`--senses`) and create a session in the **understood** stage (`--learning` is autodetected from the words when omitted; `--known` is a one-off override that otherwise resolves from your saved preference and **refuses** when neither is set — save it once with `config`; `--level` pins one initial surrounding-language band, non-`best-fit` types pin an exact format or deterministic mix, and `--wait` requires `--generate`)
- `kamishibai select [<id>] --card T --sense 1,3` / `exclude [<id>] --card T` / `correct [<id>] --card T --note "…"`: curate the understanding before generating — pick senses, drop a card, or ask Gemini to add senses (each resets the session to understood)
- `kamishibai generate [<id>] [--wait]`: commit the curated plan and start a managed background worker that generates + publishes (`--wait` runs it in the foreground)
- `kamishibai status [<id>]`: stage + per-candidate senses (understood) or per-card progress (generating/published), read from the cache (no Gemini)
- `kamishibai adjust [<id>] --card T [--understanding U] [--register neutral|casual|formal|literary|archaic] [--kind statement|question|request|exclamation|dialogue] [--level a1|a2|b1|b2|c1|c2] [--restore register|level|kind|all] [--note "…"]`: stage an offline sentence-label/note patch for one committed card; at least one patch flag is required, omitted fields preserve an existing pending patch, `--restore` is repeatable or comma-delimited and restores labels only, and an explicitly empty note clears it
- `kamishibai open [<id>]`: open the session in the interactive TUI (resumes from the cache)
- `kamishibai result [<id>]` / `ls` / `cancel [<id>]` / `rm [<id>] [--cache]` / `cache-path`
- `kamishibai regenerate [<id>] (--failed | --pending | --card T [--note "…"]) [--wait]`: re-roll committed cards and republish (runs a worker like `generate`); `--pending` atomically activates every staged adjustment, while `--failed` resumes incomplete stages and `--card` targets one card, with an optional immediate Gemini rewrite note
- `kamishibai config [--known L] [--key K]`: save console defaults to preferences (no flags → show them) — `--known` (validated) so word sessions need no `--known`, and `--key` (verified through Gemini `models.list`; `-` reads it from stdin, empty clears it) so you need not export `GEMINI_API_KEY`; the key value is never printed back

There are exactly two output modes: **plain text** (default, for humans — line-oriented, not a parsing target) and **`--json`** (placed after the verb, for machines — exactly one JSON document on stdout: the success document, or the `{"ok":false,"error":{"code","exit","message","hint","retryable"}}` envelope on failure; `generate --wait --json` and `regenerate --wait --json` additionally stream NDJSON events on stderr). `agent-contract` is the text-only exception and refuses `--json`. There is no `-q` and no `result` path selectors — an agent uses `--json`. Exit codes, locking, and semantics are identical in both modes for invocations valid in both; `open` is interactive and also refuses `--json` before any session lookup. The full console contract lives in `llms.txt`. Plain output carries no bare capturable value — every single-session command opens with the header `your session <ID> · <KNOWN> → <LEARNING> · <phase>` and the id lives there; errors are one `kamishibai: <message>` line plus a next-step hint line on stderr. **Language codes are the app's canonical UPPERCASE form everywhere** — stored in config and `session.json`, minted into ids (`FR-…`), used in the cache layout (`cards/EN-FR`) and deck names (`FR_….apkg`), and emitted in plain and JSON; input is accepted in any case and normalised to uppercase, and the only lowercase code is the frozen `target_lang` on the Gemini wire (`src/gemini/client.rs`). Exit codes are centralized in `src/cli/error.rs` (`Refusal` carries the exit, optional hint, retryability, and optional session listing): `0` ok · `2` usage · `3` no such session · `4` not ready · `5` ambiguous · `1` other. The `<id>` positional is optional on every verb: an omitted id resolves to the only session, else the only unfinished one, else the command lists the newest five sessions and exits 5 (`session::resolve`). The background worker is the same binary re-invoked as the hidden `__run <id>`, detached into a new process group with its stdio redirected to `sessions/<id>/worker.log`. Concurrency is two flocks: the long-held liveness lock (`sessions/<id>/lock`, OS-released on death) decides who may generate — `status` derives `interrupted` from a recorded worker whose lock is free — and the short write lock makes every `session.json` change a serialized read-modify-write (`SessionStore::update`), so concurrent edits all apply. The worker writes only while the record still names it, which is how `cancel` and a finishing worker resolve their race. The TUI shares this same session model — it takes the liveness lock before generating and persists its live state to `session.json`, so `ls`/`status`/`open` see interactive runs too. The full agent-facing contract lives in `llms.txt` at the repo root. `--out` wins, `KAMISHIBAI_OUTPUT` is second, and new sessions otherwise resolve the platform Documents directory plus `Kamishibai`; resolved output is stored per session. For offline tests, `KAMISHIBAI_GEMINI_URL` overrides the Gemini base URL (point it at a 127.0.0.1 listener), `KAMISHIBAI_CACHE` overrides the exact cache root, `KAMISHIBAI_DATA` overrides the data home before `kamishibai/preferences.json` is appended, and `KAMISHIBAI_OUTPUT` overrides the exact output root.

Sentence tuning is a two-step persistent transaction in both delivery surfaces. `adjust` only patches the selected card's staged request and may be called repeatedly for several cards; it leaves the current cached metadata, artifacts, published paths, costs, and lifecycle phase untouched. `regenerate --pending` is the only headless command that activates the whole staged batch. `cards.pending` in session JSON counts staged rewrites, each card's `labels` is its current complete attribution, and `adjustment` carries `state` (`pending` or `active`), the possibly partial requested label selection, and the non-empty note when present. A partial-session pending run also resumes unrelated missing stages before the deck is republished. Ordinary `generate`, `regenerate --failed`, and `regenerate --card` refuse staged changes before any provider or destructive cache work.

Regeneration carries the complete current three-axis preset. Every unedited
axis must keep its current requested value exactly. Only an explicitly changed
or already pinned axis may differ from its requested target, and only when the
result names that axis in `approx`; the generated value remains the actual
attribution and the requested target remains visible separately.

Initial batch generation guidance is separate from that post-generation rewrite transaction. `SentenceBatchSettings` persists beside the reviewed candidates in `session.json` as per-session provenance; its default is no level plus `best-fit` example formats, which leaves the metadata prompt and provider call count unchanged. Every actual TUI choice on `WhatIUnderstood` is also atomically remembered in `preferences.json` under the canonical learning-language code. Fresh understanding loads that target's override, returning both axes to `best-fit` removes it, and a resumed session keeps its own recorded settings. An explicit level becomes one initial pinned axis on every card; `statements`, `questions`, and `dialogue` pin one exact format throughout the batch, while `mixed` deterministically allocates one statement, one question, and one dialogue per complete group of three. The pending per-card metadata request persists only until metadata succeeds; the permanent batch setting remains in both session and result JSON as provenance. TUI and console (`new --level … --types questions`) must allocate through the same policy; old `natural` and `varied` values remain read/CLI aliases only.

The level chips are the lowercase operational CEFR bands `a1`, `a2`, `b1`,
`b2`, `c1`, and `c2`. They classify only the language surrounding the target
term; the target term itself is exempt, and the estimate is not an official
proficiency assessment. With the TUI's `best fit` level (stored as no
level), fresh cards first get the natural sentence required by their approved
understanding and only then receive a descriptive level; that default initial
generation does not target a band. An explicit batch-level choice is the
initial-generation exception and constrains every draft. A later per-card level
change becomes a rewrite constraint. Legacy `easy`, `takes practice`/`balanced`,
and `challenging`/`stretch` cache values reopen as `a2`, `b1`, and `b2`
respectively.

## Architecture

Understanding accepts input in either half of the pair with a selected or automatically detected learning language. `WordCandidate::term` retains the original review expression; `Sense::translation` optionally carries its natural learning-language equivalent. Different senses can resolve to different terms. `CardDraft::from_candidate` uses the selected learning expression and keeps only reviewed senses for that expression, with translation provenance stripped from the finalized card glossary. Candidate records preserve translations in both the understanding cache and `session.json`; the review TUI, plain console, and JSON expose them. Corrections to translated rows must return a translation for every new sense. Autodetection uses foreign-language words anywhere in the full batch as destination evidence, even when known-language lines are the majority. Every chunk receives the full batch as detection context, and automatic cache identities include that context; explicit targets retain per-entry cache reuse. Known-language-only input works with a selected or previously resolved target. Without one, `LearningLanguageRequired` opens the TUI language picker with a plain question and keeps the words intact; choosing a target resumes understanding. Returning from review to edit the same batch retains its resolved target. The default choice is the first, context-matched or common everyday meaning. The understanding cache policy must change when this intake contract changes.

The missing-destination question uses `PickTranslationLanguage`; its presentation and transitions are recorded in [the TUI catalog](docs/tui-states/catalog.ron). `Preferences::with_learning_language` records explicit choices in either TUI language picker in `learning_languages` as canonical codes plus `last_selected_at` Unix seconds. History is shared across sessions and survives other preference updates. The TUI includes ordered history in `RawInputBatch`; with no foreign vocabulary anywhere in the full batch, Gemini may prepare the latest eligible destination and mark `LearningGuess::requires_confirmation`. `PickTranslationLanguage` then overlays the prepared review with that language highlighted and its saved generation guidance loaded. Enter on the prepared target pins it without another request; a different target clears the proposed candidates and reruns understanding, and Escape clears the proposal and returns to input. Proposed candidates are not persisted as a session before confirmation. Any foreign vocabulary keeps ordinary detection in charge. Detect cache identities include history and preserve the confirmation flag; explicit targets retain per-entry reuse. The ordinary pair picker remains available for editing both languages.

The runtime is split into a few focused modules:

- `src/vocabulary`: validates the strict JSON document and exposes canonical entry types
- `src/languages`: keeps language profiles, naming, labels, and report font preferences
- `src/runtime`: resolves paths and renders progress and diagnosis output
- `src/application`: owns the UI-neutral ports for understanding, card production, study publishing, key validation, and cost attribution; `CardWorkflow` composes only the learner workflow (understand → produce → publish), while credential validation remains an independent delivery dependency
- `src/gemini`: owns the frozen direct REST contract plus the credential-access and cached-understanding adapters
- `src/generation/card_production`: implements metadata, sound, and visual production as focused Gemini adapters; its accounting, durable picture-request budget, scene-attempt cursor, and recovery policy remain independent of CLI sessions. A failed picture attempt reads its own verdict back from the attempt archive (`attempt_archive.rs`) and returns it as an `AttemptFault`, but only when the archive actually grew during that attempt — a failure that never reached the provider keeps its plain error instead of borrowing an older rejected frame
- `src/generation`: writes cached WAV audio, composes scenes, routes OCR, and validates manga output below the card-production adapter
- `src/publishing`: publishes the completed subset as one Anki deck plus printable PDF while holding visual leases in stable order
- `src/anki`: defines the language-neutral Anki note model and APKG writer
- `src/report`: builds the PDF report with layout, thumbnails, and font resolution
- `src/cli.rs`: parses arguments (clap, including the global `--json` flag) and routes to the interactive TUI or a `session` subcommand
- `src/cli/wiring.rs`: the sole composition root for interactive, console, and cost-attributed session variants of the Gemini-backed `CardWorkflow`; maintenance commands may address low-level cache invalidation directly but cannot compose workflow adapters
- `src/cli/console.rs`: drives the application workflow through the shared `produce` engine loop (meta → sound → scene → picture, then publish) and reports through the human / quiet / JSON `Reporter` port
- `src/cli/session`: the console (API) layer — `store` (the `session.json` record + serialized atomic `create`/`update` IO), `worker` (the managed background worker + the `__run` entrypoint, ownership-guarded writes), `liveness` (the two flocks + pid kill via rustix), `view` (the cache-derived status projection both renders share), `json` (the `Serialize` DTOs + the one emit seam), and one handler module per concern (`new`, `curate`, `adjust`, `generate`, `result`, `maintenance`) routed by `mod.rs`. This layer never links the TUI (`tests/separation.rs` enforces it): `open` hands the checked record to the `SessionOpener` port
- `src/cli/bridge.rs`: the TUI side of the session contract — projects between the live `App` and the persisted record, owns the `TuiSession` the shell claims and writes, and implements `SessionOpener` over `run_tui`

Within the card-workflow boundary, direct dependencies point inward: CLI delivery → concrete Gemini / production / publishing adapters → application ports and session domain values. `tests/separation.rs` rejects reverse imports and prevents workflow adapters from being composed outside `src/cli/wiring.rs`; legacy cache-backed session types are outside this narrower claim.

## Picture style

The built-in drawing request (`MangaInk`, compiled by `src/generation/manga/image_prompt.rs`) opens with one style paragraph that describes only what is visible on the page: thin tapered pen lines, three flat values (open white, solid black shapes, hard-edged dot screentone), simplified manga faces, and a pure white background. The same paragraph names what must stay out — gradients, soft shading, blur, glow, sculpted volume, photorealism, tint, and any lettering. It never names a creator, studio, franchise, or character and never attributes the drawing to an outside artistic identity; `style_layer_cannot_borrow_a_creator_studio_or_franchise_name` enforces that. Scene `lighting` and camera `depth_plan` are deliberately not compiled into the request: lighting wording brings back glow and gradients, and lens-depth wording brings back blurred backgrounds, which together made earlier pages read as rendered volume instead of ink. The paragraph asks for artwork "inked digitally" rather than a printed or scanned page, because the model answers "printed" and "scanned" with a photograph of a physical page. A shorter style paragraph was tried and lets hatching and volume return, so its length is intentional. The request is bounded at 160–330 words; a longer scene is recompiled by omitting repeated purpose clauses and shortening camera instructions while retaining complete panel descriptions; it refuses rather than cutting an action to meet the ceiling. Changing this wording changes what is drawn, so it needs a `LAYOUT_POLICY_VERSION` bump.

## Attempts

An artifact gets one plain try plus three retries on top of it — `ARTIFACT_ATTEMPT_CEILING` (4) attempts, which is also the durable picture-request series ceiling. Picture attempts are graded, not merely rejected: every judged attempt carries a weighted scorecard (`score` 0–100, `blocker`, per-category `penalties` for topology, non-leaking writing, and fidelity omissions) persisted in its `attempt-NNNN.json` verdict. The topology penalty is itself graded by distance from the registered layout (`min(40, 24 + 8 × deviation)`, where deviation counts missing or surplus panel regions plus collapsed panel centres), so topology failures order meaningfully instead of all scoring 60, and the verdict reason names the concrete mismatch (`found 2 panel regions for 3 planned panels`, `planned panels share one drawn region`) instead of a generic sentence. A text-gate finding whose transcription holds no alphanumeric run of at least two characters is recognizer noise and keeps only a trace weight of 2. The only blockers are a non-monochrome frame, answer leakage, and a fully borderless page — the recall judge classifies `page_frame` (`framed`/`bleed`/`breakout`/`torn`/`borderless`) and its `borderless` verdict blocks only when the perimeter is also mechanically inked, so a hallucinated verdict against a real white margin is ignored. A partially bled outer border is an accepted stylistic device and ships exactly as the model drew it; `BorderDetector::repaired` survives only as measurement normalisation so the topology matchers keep seeing the clean margin they are calibrated against. Retries are gated only by blockers and frame structure: an attempt is accepted whenever it is not blocked and `penalties.topology < 24` — every text, fidelity, and literal finding (numerals, pseudo-writing, diagrams, non-leaking writing, missing anchors, continuity) is score-only and never burns a paid attempt; it stays visible in the verdict, the TUI findings, and the NDJSON events, and it ranks the salvage choice. The recall judge's `page_frame` axis distinguishes intent from damage: `bleed` and `breakout` (content deliberately crossing a panel border, a splash spanning several panel areas) are accepted stylistic devices — a judged `breakout` even downgrades a mechanical topology mismatch to a score-only 8 — while `torn` (frame lines carrying smeared, broken, or garbage-filled generation artifacts) raises the topology penalty to the retry floor. Because cosmetic findings no longer stop the review chain, the dedicated fidelity and zoom inspections always complete on leak-free frames before acceptance. The final attempt of a series salvages the highest-scoring non-blocked archived frame (its verdict flips to `salvaged`) — with graded topology that choice picks the frame closest to the registered layout — and every terminal failure path funnels through that same salvage seam, including a judge or transport error on the final attempt and an exhausted durable picture-request budget (which also arms salvage by itself when the archive already holds a full series), so a card fails only when its archive is empty or every archived frame was blocked. Two consecutive semantic (`ocr`/`recall_text`) rejections recompose the scene, and the final scene attempt relaxes the image-facing text gate (`compose(..., lenient)`). `AttemptTally::retry` numbers machine-facing retry events from `1..=retries`. Every spent attempt records **why** it was spent: `src/session/attempt.rs` pairs the `AttemptTally` with one `AttemptFault` per failure (`category` slug, user-facing `reason`, the archived picture when the provider drew one, and the parsed `AttemptScorecard` when the archived verdict carries one). The production adapter supplies the renderer's real verdict; anything else — transport error, cache lease, exhausted request budget — is diagnosed by the engine from the error text under category `error`. Both stages leave something behind: a picture attempt archives the rejected frame, and a scene attempt archives the model reply it failed to decode (`RejectedReply` carries the body out of `src/gemini`, `attempt_archive::archived_reply` writes it as `scene-NNNN.json` when it parses as JSON and `scene-NNNN.txt` when it never was JSON). A failure that never reached the model — transport, cache lease — archives nothing. Rejected frames are never deleted by a run — only `drop_artifacts` / `drop_incomplete_artifacts` clear them.

`CardPhase` (`src/session/draft.rs`) is the one derived vocabulary for where a card stands: `Adjusted`, `Failed`, `Working`, `Ready`, checked in that order because a staged rewrite sits on top of intact artifacts and would otherwise report as ready.

## Batch limits

One batch is bounded at both ends of the pipeline, and the two limits are separate because one word yields one card per selected sense (`MAX_SENSES` is 6) while `--build` imports cards with no words at all. `MAX_INTAKE_WORDS` (60, `src/session/candidate.rs`) caps the vocabulary lines entering one understanding pass: intake is the only Gemini call carrying the whole batch, it is neither streamed nor retried, and a polysemous list bills roughly 570 tokens per word, so beyond this a single request stops fitting the 300-second transport timeout. `MAX_PLAN_CARDS` (80, `src/session/draft.rs`) caps the cards one batch commits: generation is sequential single-flight at roughly 48 seconds and $0.19 per card, so 80 keeps the slowest realistic run under two hours and under twenty dollars. `INTAKE_CHUNK_WORDS` (20, `src/session/cache.rs`) is how many lines one intake request carries, and `INTAKE_MAX_OUTPUT_TOKENS` (16384, `src/gemini/client.rs`) is that request's output ceiling — deliberately low enough that reaching it takes well under the timeout, so a truncation becomes a named refusal instead of a hang. The ceiling lives on a dedicated `intake_text` method rather than the shared `text_metered`, whose request bytes are frozen by `tests/gemini.rs`.

`CachedUnderstanding::understand` refuses an oversized batch as its first statement, before script detection and before the cache is touched, so every caller is covered including the language-pair re-read door on `WhatIUnderstood`. Repeated lines are asked about once and the single answer fans back out to every row that shares the line. Each chunk is written to the cache as soon as it decodes, and a short reply re-asks only that chunk — never the whole batch, which is what the previous count-mismatch path did. Both limits bite only on a fresh commit: `ensure_plan` early-returns on a non-empty plan, so a session committed before the ceilings existed still generates. Intake spend is still not journaled (`understand` drops its `CostRecord`) and now spans several requests, at roughly 2% of a batch's cost.

## Cache layout

The cache (printed by `kamishibai cache-path`) groups one folder per card, keyed by a content hash of the card identity:

- `cards/<known>-<learning>/<key>/` holds `meta.json` and `audio.wav`; `visual/<revision>/` beneath it holds `scene.json` and `picture.jpg` for one visual-policy revision, plus `attempts/` where every image attempt is archived immutably as `attempt-NNNN.jpg` next to its `attempt-NNNN.json` verdict (`status`, `category`, `reason`, plus `score`, `blocker`, and `penalties` when the attempt was judged), the scene and prompt it used, the literal-text verdict when that gate ran (`attempt-NNNN.text.json`), and the merged review when the picture reached the later gates (`attempt-NNNN.recall.json`, with independent answer-leakage, scene-fidelity, literal-policy, and page-frame verdicts plus explicit dedicated-fidelity and zoom inspection proof); rejected scene replies land beside them as `scene-NNNN.json` / `scene-NNNN.txt`
- `understanding/<known>-<learning>/<key>.json` holds the understanding-pass result; entries are written per intake chunk, so a batch that fails part-way keeps everything the earlier chunks produced and a rerun asks only for the rest
- `sessions/<id>/` holds `session.json` (identity, phase, words, curated candidates, committed plan, worker pid, result) and `worker.log`
- `ocr-models/` holds the shared OCR model files

`CardCell` (`src/session/vault.rs`) owns this layout; deleting a card's folder forces just that card to regenerate. Visual revisions hash the production feature and scene-composer prompts, the composer schema, all four judge prompt/schema pairs (literal text, full recall, dedicated fidelity, and scale-aware literal zoom), the all-language recall examples, both layout/device registries, and the manga template together with the manual `LAYOUT_POLICY_VERSION`, so concurrent application versions never overwrite one another. Bump that version whenever a scene model/configuration, local scene specialization/validation rule, renderer acceptance policy, or the picture request wording changes without changing an embedded asset. Anki media names are decoupled from disk filenames in `src/anki/deck.rs` so per-card role-named files stay unique inside the `.apkg`.

## Language Profiles

Language-specific behavior belongs only in `src/languages` profile declarations. A profile defines:

- Gemini prompt display name
- typed literal-text gate (`TextGate::Ocr` with an `OcrModel`, or `TextGate::LlmJudge`)
- text direction (`TextDirection::Ltr` or `TextDirection::Rtl`)
- default deck naming
- user-facing report labels

If a new language is needed, add a new profile instead of editing the fixed runtime orchestration logic.

## Releasing

The version in `Cargo.toml` is the release trigger; nothing is tagged or published by hand. Merging a version bump into `main` does the rest: a green `Rust` CI run fires `.github/workflows/auto-release-tag.yml`, which tags `v<version>` and dispatches `release-artifacts.yml` — five platform archives (linux x86_64/aarch64, macos arm64/x86_64, windows) plus `SHA256SUMS.txt`, published as a GitHub Release with generated notes. `workflow_dispatch` on either workflow is the manual fallback, and `install.sh` always serves the latest release.

`Cargo.toml` `version` and the `Release:` header in `llms.txt` are one bidirectional contract and must change together in the same commit. Any `llms.txt` change requires an application version bump; any application version bump requires review/update of `llms.txt` with the exact matching `Release:`. The automated agent-contract test rejects a mismatch. Release archives contain both the binary and `llms.txt`, and `kamishibai agent-contract` must print that file byte-for-byte.

Homebrew is a separate, manual follow-up in the tap repository **`anatoly-chichikov/homebrew-tap`** (https://github.com/anatoly-chichikov/homebrew-tap — a local checkout normally sits beside this repository; search for a `homebrew-tap` directory locally before cloning). In the tap: bump the version and sha256 values in `Formula/kamishibai.rb` (hashes come from the release's `SHA256SUMS.txt`), open a PR, wait for the bottles to build on CI, then publish them with `gh workflow run publish.yml -f pull_request=<PR number>`.

## TUI changes

Interface knowledge lives in typed RON beside the code, not in Markdown. [docs/tui-states/catalog.ron](docs/tui-states/catalog.ron) maps the interface and [docs/tui-states/recording.ron](docs/tui-states/recording.ron) holds the VHS capture procedure. Their schemas are `tests/common/tui_catalog.rs` and `tests/common/tui_recording.rs`, and `cargo test --test all -- ui_catalog:: ui_recording::` checks their links against the code, the header and footer of every prepared scene against the real renderer, and each capture against the tape that wrote it; it never compares a capture's pixels with today's renderer.

The catalog reads top-down: rules, screens, states, scenarios. A rule is a cross-screen law whose `scope` is `Everywhere`, whole screens or named states. A screen's ID is the prefix of every state on it: `welcome`, `words`, `review`, `cards`, `done`. A state holds its surface, chrome, expectations and actions, the grounds it rests on and its captures; a fixture has the same shape but is a prepared scene that only checks layout, so no learner action leads into it; a scenario is a route through states. Every entry opens with one head line, `Kind(head: Head(id: "…", intent: "…"),`, whose intent says what the entry gives the learner and what it protects. Build on that intent: change it only when a request changes what the learner gets, and record code that contradicts an expectation as a `Gap` instead of rewriting either.

Navigate with grep from the repository root:

```sh
grep -n -A1 'Rule(head' docs/tui-states/catalog.ron                  # rules with intent and scope
grep -n 'Screen(head' docs/tui-states/catalog.ron                    # screens; the ID prefixes their states
grep -n 'State(head: Head(id: "cards\.' docs/tui-states/catalog.ron  # one screen's states with intents
grep -n 'Fixture(head' docs/tui-states/catalog.ron                   # layout fixtures, never learner states
grep -n 'Scenario(head' docs/tui-states/catalog.ron                  # scenarios with intents
grep -n '"cards.pending"' docs/tui-states/catalog.ron                # an entry, moves into it, routes and scopes
grep -n 'src/tui/screens/your_cards.rs' docs/tui-states/catalog.ron  # from code to the entries citing it
```

Complete each TUI feature in the same task:

1. Read the affected rules, screen, states and scenarios first. A new situation gets a state ID on its screen's prefix, transition edges, a scenario where a route changes, and a place in the scope of every rule that governs it.
2. Update chrome, expectations, actions and grounds, and the prepared scene in `examples/tui_states.rs`; every prepared scene's header and footer are compared with the real renderer.
3. Recapture the affected scenes with the synthetic tapes as `recording.ron` describes, inspect every rewritten PNG at each affected width, and update the capture's revision and note. An absent capture stays `Missing` with a reason; an older capture is never presented as current evidence.
4. Run `cargo test --test all -- ui_catalog:: ui_recording::`, the relevant behavior tests, `cargo fmt`, and `cargo clippy --all-targets --all-features -- -D warnings`.
5. Report the IDs updated, the checks run, the captures inspected, and any remaining gaps or missing evidence.

Keep interface behavior out of this file and out of new Markdown; when a concept needs a new property, extend the RON schema instead.
