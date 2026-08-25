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

Initial batch generation guidance is separate from that post-generation rewrite transaction. `SentenceBatchSettings` persists beside the reviewed candidates in `session.json`; its default is no level plus `best-fit` example formats, which leaves the metadata prompt and provider call count unchanged. An explicit level becomes one initial pinned axis on every card; `statements`, `questions`, and `dialogue` pin one exact format throughout the batch, while `mixed` deterministically allocates one statement, one question, and one dialogue per complete group of three. The pending per-card metadata request persists only until metadata succeeds; the permanent batch setting remains in both session and result JSON as provenance. TUI and console (`new --level … --types questions`) must allocate through the same policy; old `natural` and `varied` values remain read/CLI aliases only.

## Architecture

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

## Attempts

An artifact gets one plain try plus three retries on top of it — `ARTIFACT_ATTEMPT_CEILING` (4) attempts, which is also the durable picture-request series ceiling. Picture attempts are graded, not merely rejected: every judged attempt carries a weighted scorecard (`score` 0–100, `blocker`, per-category `penalties` for topology, non-leaking writing, and fidelity omissions) persisted in its `attempt-NNNN.json` verdict. The topology penalty is itself graded by distance from the registered layout (`min(40, 24 + 8 × deviation)`, where deviation counts missing or surplus panel regions plus collapsed panel centres), so topology failures order meaningfully instead of all scoring 60, and the verdict reason names the concrete mismatch (`found 2 panel regions for 3 planned panels`, `planned panels share one drawn region`) instead of a generic sentence. A text-gate finding whose transcription holds no alphanumeric run of at least two characters is recognizer noise and keeps only a trace weight of 2. The only blockers are a non-monochrome frame, answer leakage, and a fully borderless page — the recall judge classifies `page_frame` (`framed`/`bleed`/`breakout`/`torn`/`borderless`) and its `borderless` verdict blocks only when the perimeter is also mechanically inked, so a hallucinated verdict against a real white margin is ignored. A partially bled outer border is an accepted stylistic device and ships exactly as the model drew it; `BorderDetector::repaired` survives only as measurement normalisation so the topology matchers keep seeing the clean margin they are calibrated against. Retries are gated only by blockers and frame structure: an attempt is accepted whenever it is not blocked and `penalties.topology < 24` — every text, fidelity, and literal finding (numerals, pseudo-writing, diagrams, non-leaking writing, missing anchors, continuity) is score-only and never burns a paid attempt; it stays visible in the verdict, the TUI findings, and the NDJSON events, and it ranks the salvage choice. The recall judge's `page_frame` axis distinguishes intent from damage: `bleed` and `breakout` (content deliberately crossing a panel border, a splash spanning several panel areas) are accepted stylistic devices — a judged `breakout` even downgrades a mechanical topology mismatch to a score-only 8 — while `torn` (frame lines carrying smeared, broken, or garbage-filled generation artifacts) raises the topology penalty to the retry floor. Because cosmetic findings no longer stop the review chain, the dedicated fidelity and zoom inspections always complete on leak-free frames before acceptance. The final attempt of a series salvages the highest-scoring non-blocked archived frame (its verdict flips to `salvaged`) — with graded topology that choice picks the frame closest to the registered layout — and every terminal failure path funnels through that same salvage seam, including a judge or transport error on the final attempt and an exhausted durable picture-request budget (which also arms salvage by itself when the archive already holds a full series), so a card fails only when its archive is empty or every archived frame was blocked. Two consecutive semantic (`ocr`/`recall_text`) rejections recompose the scene, and the final scene attempt relaxes the image-facing text gate (`compose(..., lenient)`). `AttemptTally::retry` still numbers machine-facing retry events from `1..=retries`, but the TUI deliberately does not expose that number on an artifact row. Every active attempt, whether the first try or a retry, renders the same turning spinner and nothing else on its step row; an inactive retry renders only a dot, its row label, and any known cost. A terminal row is its leading `✗` plus any known cost, with no status word beside it. Every spent attempt records **why** it was spent: `src/session/attempt.rs` pairs the `AttemptTally` with one `AttemptFault` per failure (`category` slug, user-facing `reason`, the archived picture when the provider drew one, and the parsed `AttemptScorecard` when the archived verdict carries one). The production adapter supplies the renderer's real verdict; anything else — transport error, cache lease, exhausted request budget — is diagnosed by the engine from the error text under category `error`. Retry history is summarized once on the card head as `  ↻N`, after the displayed total cost when one exists and omitted at zero. `N` sums `min(tally.done(), tally.retries())` across meta, audio, scene, and picture, so a terminal four-attempt artifact contributes `↻3`; unmetered and undiagnosed spent attempts still count. The expanded card shows the meta preview first and then, below a dashed rule, a `rejected attempts` block; each attempt renders a short header — which try it was, the archived file link, and one verdict word (the quality score as `94/100`, `blocked` for a blocked frame, or the raw category of an unjudged failure) — followed by one indented `·` row per judge finding, in the judge's own grounded words: the machine score prefix is dropped and gate prefixes are shortened to `missing:` / `writing:`. An unjudged failure's only finding row is its reason sentence. The per-axis penalty numbers never render in the TUI — they stay in the archived verdict and the NDJSON events. Both stages leave something behind: a picture attempt archives the rejected frame, and a scene attempt archives the model reply it failed to decode (`RejectedReply` carries the body out of `src/gemini`, `attempt_archive::archived_reply` writes it as `scene-NNNN.json` when it parses as JSON and `scene-NNNN.txt` when it never was JSON). Both are muted underlined links that open with the system handler. A failure that never reached the model — transport, cache lease — archives nothing and leaves that column blank. Rejected frames are never deleted by a run — only `drop_artifacts` / `drop_incomplete_artifacts` clear them. How many cards never finished is said in exactly one place — the outcome strip's tag (see below) — and nowhere else: not on the footer, not beside the title, not on the card rows, which carry a bare `✗` and no status word.

`CardPhase` (`src/session/draft.rs`) is the one derived vocabulary for where a card stands: `Adjusted`, `Failed`, `Working`, `Ready`, checked in that order because a staged rewrite sits on top of intact artifacts and would otherwise report as ready. `App::card_census` folds the batch once and backs `cards_ready` / `cards_failed` / `cards_pending`, whose meanings are unchanged; `working` is the exclusive remainder and stays private to `src/tui/app.rs`, where it only answers whether any card is left unfinished — the question that decides whether `Ctrl+G` has anything to re-roll. The status line of a live batch names progress exactly once, as the ready count against the batch size, and then `<n> pending`, cost, and elapsed time — a second count of the cards still owed would only restate `ready` against the total while naming a concurrency the sequential engine never has. One word means one thing across title, hint, footer, and step glyph.

The viewport rides the engine: a new or restarted batch arms following (`cards_started` and `restart_regeneration` both do), `cards_running` then carries the selection onto the card being built, and `Shell::follow_running_card` scrolls to it once per loop pass beside the existing clamp. The ride aims at the **finished** card, not at the stub drawn so far: a followed card reserves its head plus all three artifact rows plus its trailing blank (`focused_card_range`), and the clamp in `body_scroll_to_selection` widens to that range when it reaches past the rows drawn today, so the card lands once and stays put while it fills in instead of being parked on the bottom line and shedding every row it grows. Any manual scroll or arrow breaks the ride; the per-frame clamp and the snap itself must never break it. An expanded **focused** card suppresses following rather than clearing it, so collapsing (or walking off) that card resumes the ride; a ride that lands on a parked expanded card pauses the same way until that card is collapsed. `↑↓` are the only keys that move the card cursor: a jump to the next unfinished card existed on `Tab`/`Shift+Tab` and was removed, because during a build the viewport already rides the engine and afterwards a broken card announces itself with the one bright `✗` in its block while `Ctrl+G` re-rolls every failure at once — nobody has to walk to them. `Tab` and `BackTab` therefore map to no event at all (`to_app` in `src/tui/input.rs`). Card numbers are absolute batch positions and never renumber.

One keyboard grammar covers the whole TUI: `↑↓` move focus vertically — through lists, editor rows, and straight through expanded blocks; `←→` first serve the focused horizontal control (carousels, text cursors, picker columns, the Welcome language strip), and where the focused row owns no such control they fall through to its disclosure — `→` opens what `Enter` opens, `←` closes what `Esc` closes; `Enter` opens the focused disclosure and confirm-closes it; `Space` marks or acts on the focused item; `Esc` closes one layer; `C` toggles the whole screen's disclosure on the review and cards screens (a plain Latin letter, like `D`/`S`): with anything open it collapses every expanded block — closing an open guidance editor or card editor with it — and with everything collapsed it expands every card (editors stay closed) or opens every multi-meaning sense list (single-sense and off-language rows stay closed). `C` fires from anywhere except a real text input — on cards it works from the editor's carousel rows, only the focused note row types a literal `c` — and its secondary-tier footer hint reads `[C] collapse` or `[C] expand` to match the direction it will take. **No hotkey depends on the active keyboard layout.** Hotkeys are named in English, and every key the application dispatches on is folded back to the Latin letter printed on that physical key by `latin_key` (`src/tui/input.rs`), which covers ЙЦУКЕН, Ukrainian, and Greek: `Ctrl`/`Cmd` combinations fold inside `to_app`, plain letters fold in `transition::promote`, and the scroll pair `Ctrl+N`/`Ctrl+P` folds in `src/cli/terminal.rs`. The fold is applied only where nothing is being typed — `WhatIUnderstood` and a `YourCards` whose note row is unfocused — so the words editor, the Welcome key field, every modal, and the focused rewrite note keep the exact codepoint the layout produced. Expanded blocks are pass-through views: leaving one does not close it, and walking onto one flows straight into its interior. On `WhatIUnderstood` several sense lists may stay open at once (`ReviewFocus` + `OpenSenseLists` in `src/tui/app.rs`), `↓` from a head enters its open list, `↓` from the `+ add more` row moves to the next word, and a `Space` toggle commits into the candidate immediately — there is no tentative selection, so Esc or `←` collapses the focused list — from its head or from inside it — without discarding anything, and `Ctrl+G` needs no commit step. Opening a list retitles its head instead of leaving it repeating one of the rows now listed underneath: the head carries `multiple meanings:` whenever there is more than one sense to pick from — the same heading a collapsed word with several chosen meanings already shows, so it appears exactly when the `X/Y` counter does — and carries nothing at all when a single sense sits below it. That text lives in one `head_gloss` (`src/tui/screens/what_i_understood.rs`) read by the renderer, the height counter, and both scroll-snap sites, and `screen_lines_match_the_counted_height` fails the moment they disagree. Inside the list a chosen sense reads at `Ink::Detail`, exactly like the same gloss on a collapsed row, and an unchosen one a rank below at `Ink::Aside`, so the screen steps from the word to what it will mean to what it will not. On `YourCards` expansion is per-card (`ExpandedCards`) and **opening a card never gives its editor the keyboard**: `Enter`, `→`, and `Space` show the whole block — step rows without tags, the four tune rows, meta preview, rejected attempts — and `Enter`, `←`, or `Esc` close it again, so a card can be opened and closed freely without landing in its adjustments. Open and tuning are one geometry; the only difference is focus (`tune_rows` in `src/tui/screens/your_cards.rs` answers with the live editor or with a copy seeded from the draft, and the renderer, the block height, and the hit-tester all read that one answer). Unlit, no question is white or bold, no chevron is bright, and the note owns no cursor. The one keyboard door into tuning is the walk: `↓` from an open tunable head lights the register row, arriving from below lands on the note row, and the walk saturates inside the last card's editor instead of cycling. Walking off a card with a live editor parks it — the rows stay, unlit — and walking back in lights them again. The footer names that door as `[↓] tune` while the focused card is open and tunable. The mouse needs no walk: clicking any tune control of the focused card hands it the keyboard and applies in the same gesture, clicking one on another open card selects that card and row first, and clicking a collapsed card's tags (or its head when the tags cannot fit) reveals the card with its editor already live. `Enter` inside the editor still closes editor and card together. An expanded card whose rewrite is running shows its previous meta with the phrase struck (`CardRewrite::previous`) instead of a bare `meta not generated yet` placeholder.
The status bar states where the screen stands on the left and what its keys do on the right, and a screen no longer decides that alone: `ScreenView` (`src/tui/screens/mod.rs`) answers with `status` spans and a `hints` list, and `common::render_screen` assembles the bar. That seam is where the overlay rule lives — while a busy spinner or a modal is up, `transit` swallows everything, so the bar drops the screen's hints and keeps only `[Ctrl+C] quit`, the one key the terminal loop consumes before `transit` sees it. **A hint is a promise the key will do something.** No hint may name a key that is inert in the state it is drawn in: `Ctrl+G` disappears over the card ceiling, on a running batch (where it would only queue a silent restart) and on a finished one with nothing staged (where it would merely republish); the disclosure hint reads the live open state so its arrow and its verb agree; the empty words box has no bright key at all, because typing has no key to name.

Two independent axes decide how a hint is drawn and how long it lives. `Tier` is brightness and means rank — `Primary` is `Ink::Subject`, `Secondary` is `Ink::Detail`, `Ghost` is `Ink::Aside`, never underlined, since underline means clickable and footer hints are not. Weight is spent on one thing only: an **armed** confirmation (`FooterHint::armed`, used by `[Esc] again` and `[Ctrl+C] again`) is bright and **bold**, which is the ordinary rule rather than an exception to it — for the length of the confirmation window that key is what the keyboard owns, nothing else can answer until it fires or times out, and the bar has to say so loudly enough to stop a hand already moving. No other hint carries weight. Two kinds of key wear `Primary`: the spine of the walk (`Ctrl+G`), and the **door** into whatever the cursor stands on — `[Enter/→] open`, `[↑] guidance`, `[↓] tune` — because stepping inside is how a row is acted on at all. `FooterHint::door` mints those, and its closing half (`[Enter/←] close`) drops back to `Secondary`, since closing only undoes what the door did. `Keep` (`src/tui/screens/common.rs`) is the drop order on a narrow bar, where whole hints are shed — never clipped — lowest rank first and rightmost on a tie: `Optional` (conventional keys, discovery, `[Esc] back`) < `Useful` (screen actions, doors included) < `Exit` < `Main` < `Confirm`. `Exit` belongs to `[Ctrl+C] quit` alone, so nothing can tie with the way out and lose the tie-break to it. Read as one sentence: the exit outlives every action a screen offers and yields only to that screen's primary, or to a confirmation already armed.

The bar is ordered by that same grammar, left to right, on every screen: the spine first, then the doors of the focused row, then the screen's own actions (`[D] drop`, the `[C]` sweep), then the conventional keys (`[↑↓] nav`, `[Ctrl+L] languages`), then `Esc`, then `[Ctrl+C] quit` last. The sweep is one offer seen from its two ends and reads `Secondary` in both directions, so it sits between the doors it operates and the keys nobody needs told, and outlives them when the bar narrows.

One key means one word everywhere. `Ctrl+G` is the spine of the walk and names what the user gets — `understand`, `generate`, `regenerate`. Every destructive `Esc` takes two beats and names the consequence on the first: `[Esc] clear`, `[Esc] stop`, `[Esc] new cards`, each followed by a bright bold `[Esc] again` once armed — the same treatment `[Ctrl+C] again` takes, so an armed key looks the same whichever one holds it — and because each of those names a loss, they are ranked to survive the conventional keys beside them. `[Esc] back` is the one Escape that breaks nothing: it walks back to words that are still there, so it is drawn as a ghost, placed immediately before the way out, and is the first hint a narrowing bar sheds. A disclosure says `open` or `close`, matching the `→`/`←` printed beside it; `nav` is vertical movement and `pick` is horizontal, on every screen and inside both modals; both modal action rows read primary-first like the bar. Key brackets carry no inner space (`[↑↓]`, `[←→]`).


## Batch limits

One batch is bounded at both ends of the pipeline, and the two limits are separate because one word yields one card per selected sense (`MAX_SENSES` is 6) while `--build` imports cards with no words at all. `MAX_INTAKE_WORDS` (60, `src/session/candidate.rs`) caps the vocabulary lines entering one understanding pass: intake is the only Gemini call carrying the whole batch, it is neither streamed nor retried, and a polysemous list bills roughly 570 tokens per word, so beyond this a single request stops fitting the 300-second transport timeout. `MAX_PLAN_CARDS` (80, `src/session/draft.rs`) caps the cards one batch commits: generation is sequential single-flight at roughly 48 seconds and $0.19 per card, so 80 keeps the slowest realistic run under two hours and under twenty dollars. `INTAKE_CHUNK_WORDS` (20, `src/session/cache.rs`) is how many lines one intake request carries, and `INTAKE_MAX_OUTPUT_TOKENS` (16384, `src/gemini/client.rs`) is that request's output ceiling — deliberately low enough that reaching it takes well under the timeout, so a truncation becomes a named refusal instead of a hang. The ceiling lives on a dedicated `intake_text` method rather than the shared `text_metered`, whose request bytes are frozen by `tests/gemini.rs`.

`CachedUnderstanding::understand` refuses an oversized batch as its first statement, before script detection and before the cache is touched, so every caller is covered including the language-pair re-read door on `WhatIUnderstood`. Repeated lines are asked about once and the single answer fans back out to every row that shares the line. Each chunk is written to the cache as soon as it decodes, and a short reply re-asks only that chunk — never the whole batch, which is what the previous count-mismatch path did. Both limits bite only on a fresh commit: `ensure_plan` early-returns on a non-empty plan, so a session committed before the ceilings existed still generates. Intake spend is still not journaled (`understand` drops its `CostRecord`) and now spans several requests, at roughly 2% of a batch's cost.

## Cache layout

The cache (printed by `kamishibai cache-path`) groups one folder per card, keyed by a content hash of the card identity:

- `cards/<known>-<learning>/<key>/` holds `meta.json` and `audio.wav`; `visual/<revision>/` beneath it holds `scene.json` and `picture.jpg` for one visual-policy revision, plus `attempts/` where every image attempt is archived immutably as `attempt-NNNN.jpg` next to its `attempt-NNNN.json` verdict (`status`, `category`, `reason`, plus `score`, `blocker`, and `penalties` when the attempt was judged), the scene and prompt it used, the literal-text verdict when that gate ran (`attempt-NNNN.text.json`), and the merged review when the picture reached the later gates (`attempt-NNNN.recall.json`, with independent answer-leakage, scene-fidelity, literal-policy, and page-frame verdicts plus explicit dedicated-fidelity and zoom inspection proof); rejected scene replies land beside them as `scene-NNNN.json` / `scene-NNNN.txt`
- `understanding/<known>-<learning>/<key>.json` holds the understanding-pass result; entries are written per intake chunk, so a batch that fails part-way keeps everything the earlier chunks produced and a rerun asks only for the rest
- `sessions/<id>/` holds `session.json` (identity, phase, words, curated candidates, committed plan, worker pid, result) and `worker.log`
- `ocr-models/` holds the shared OCR model files

`CardCell` (`src/session/vault.rs`) owns this layout; deleting a card's folder forces just that card to regenerate. Visual revisions hash the production feature and scene-composer prompts, the composer schema, all four judge prompt/schema pairs (literal text, full recall, dedicated fidelity, and scale-aware literal zoom), the all-language recall examples, both layout/device registries, and the manga template together with the manual `LAYOUT_POLICY_VERSION`, so concurrent application versions never overwrite one another. Bump that version whenever a scene model/configuration, local scene specialization/validation rule, or renderer acceptance policy changes without changing an embedded asset. Anki media names are decoupled from disk filenames in `src/anki/deck.rs` so per-card role-named files stay unique inside the `.apkg`.

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

## Recording the demo GIF and screenshots

`docs/tui-states/live/capture.gif` (linked from `README.md`) and the per-screen PNGs next to
it are produced by three VHS tapes in `docs/tui-states/`:

- `capture.tape` runs the **live binary** (real Gemini) and writes the happy-path screenshots
  plus the raw `live/capture.gif`.
- `states.tape` drives the `examples/tui_states` **state-walker** (no Gemini) to write the
  review, batch-settings, edge-case, modal, and Welcome screenshots reproducibly.
- `states-narrow.tape` drives the same state-walker at 1200 px to write the intentionally
  narrow S10 sentence-label and batch-settings screenshots; VHS accepts geometry only at the top of a tape.

The README gif itself is then assembled deterministically by `encode.sh` from `timings.conf`
(the single source of truth for section windows, durations, and raw source); it emits
`timings.timeline.txt` and splices the finale caption PNG (`live/caption.png`). A window reads
`RAW` when its source is `main` and `ADJUST_RAW` when its source is `adjust`, so a supplementary
interaction recording can be cut together with the original without transcoding either raw.
Only `live/capture.gif` is committed; pass `MAKE_HIRES=1` to `encode.sh` if you want a local-only
hi-res `capture.hires.mp4` master. The generation keyboard contract is `Ctrl+G`.

### Why no manual chord patch is required

`Ctrl+G` is a simple control byte, which crossterm reads as the generation hotkey in raw
mode. The old temporary `Ctrl+S` recording chord is obsolete and must not be reintroduced.

### Procedure

From the repo root:

1. **Build the binaries** (release for the live run, release example for the Welcome shot):

   ```bash
   cargo build --release
   cargo build --release --example tui_states
   ```

2. **Confirm the release binary is current**. No recording-only key patch is needed:

   ```bash
   cargo build --release
   ```

3. **Record the synthetic state shots** (state-walker, no Gemini calls):

   ```bash
   cd docs/tui-states
   vhs states.tape
   vhs states-narrow.tape
   rm -f states-throwaway.gif states-narrow-throwaway.gif
   ```

   Writes the review and six environment/failure/retry shots (`live/02-what-i-understood.png`,
   `live/00-welcome.png`,
   `live/00b-welcome-env.png`, `live/03-change-something-modal.png`,
   `live/06-your-cards-retrying.png`, `live/06b-your-cards-retry-stress.png`,
   `live/07-your-cards-couldnt-finish.png`) plus the
   twelve sentence-label S1–S12 PNGs from `live/11-s1-label-tags.png` through
   `live/22-s12-label-legacy-meta.png`, the five Esc lifecycle PNGs from
   `live/23-esc-words-clear.png` through `live/27-generation-partial.png`, plus
   the open batch-settings pair `live/28-batch-sentence-settings.png` and
   `live/29-batch-sentence-settings-narrow.png`, plus the two language-pair shots
   `live/30-plausible-alternates.png` and `live/31-language-pair-modal.png`, plus
   the Welcome language grid `live/32-welcome-language-grid.png`, plus the
   parked multi-expanded cards shot `live/33-your-cards-parked.png`, plus the
   open sense list `live/34-open-sense-list.png`.
   All are 2x except S10 and the narrow
   batch-settings frame, which come from `states-narrow.tape` at 1200 px. Both synthetic tapes jump to each state
   by **absolute index** (`Type "<n>"` then `Space`) and keep a uniform 800 ms settle after
   each jump so VHS never captures a mid-repaint frame. Absolute jumps are immune to
   keystroke coalescing and to the stray Return the shell injects when it launches the
   binary — `Enter` in the walker only clears the queued digits. The two Welcome shots are
   the same `EnterKey` stage: `00-welcome.png` has no `GEMINI_API_KEY` (just the `submit`
   button), `00b-welcome-env.png` has it set (adds the focused `load from env` chip); both show the language step already
   answered and collapsed to its one value, while `00c-welcome-language-grid` at
   index 30 is that step still open.

4. **Record the live-binary flow** (real Gemini run, roughly 5–7 minutes wall-clock because
   the tape starts with an empty cache and later regenerates one tuned card):

   ```bash
   vhs capture.tape
   ```

   Writes `live/01-your-words.png`, `live/01b-busy.png`, `live/02-what-i-understood.png`,
   `live/02a-nav.png`, `live/03-senses.png`, `live/03b-senses-toggled.png`,
   `live/04-your-cards.png`, `live/08-done.png`, `live/09-card-adjusting.png`,
   `live/09a-level-raised.png`, `live/09d-card-staged.png`,
   `live/09b-card-regenerating.png`,
   `live/09c-card-regenerated.png`, `live/09-card-open.png`,
   `live/10-card-scroll-end.png`, and the full raw `live/capture.gif`.

5. **Stash the raw recording** before any post-processing — keep it around as `/tmp/raw.gif`
   so you can redo the slice/encode pass without re-running VHS or Gemini. If an interaction
   is recorded separately, preserve complete takes as `/tmp/adjust-raw.gif` and
   `/tmp/nav-adjust-raw.gif`; windows in `timings.conf` can name `main`, `adjust`, or `nav`.
   The README payload is built on top of these raws.

   ```bash
   cp live/capture.gif /tmp/raw.gif
   ```

   Do NOT delete any raw until you've reviewed the final gif and decided you don't need another
   timing iteration.

6. **Detect scene transitions** automatically — never assume the time windows from a previous
   recording apply. Gemini latency varies wildly between runs (this session swung between
   2 min and 7 min wall-clock).

   ```bash
   ffmpeg -i /tmp/raw.gif -vf "select='gt(scene,0.005)',showinfo" -f null - 2>&1 \
     | awk '/pts_time/{gsub(/.*pts_time:/,"");print $1}' > /tmp/transitions.txt
   cat /tmp/transitions.txt
   ```

   The `0.005` threshold catches the major TUI transitions (TUI screens change in only a
   slice of cells per frame, so the default `0.3` returns 0 hits). The number of transitions
   is **not fixed** — it grows when new states are added to the flow. Don't hardcode an
   expected count.

7. **Dump a frame at every transition** and eyeball them to map each one to a screen state.

   ```bash
   mkdir -p /tmp/cuts && rm -f /tmp/cuts/*.png
   ffmpeg -y -ss 0 -i /tmp/raw.gif -frames:v 1 /tmp/cuts/cut-00.png
   i=1
   while read t; do
     ffmpeg -y -ss "$t" -i /tmp/raw.gif -frames:v 1 /tmp/cuts/cut-$(printf %02d $i)-t${t}.png
     i=$((i+1))
   done < /tmp/transitions.txt
   open /tmp/cuts
   ```

8. **Classify each section** between consecutive transitions:

   | Type | Signal | Sampling for the gif |
   | --- | --- | --- |
   | **workflow** | user-driven step or new content (typing, candidates land, Done lands) | `fps=25` on the section's natural window; preserve real-time animation |
   | **read** | a state that's only briefly visible in the recording but the viewer needs time to read (e.g. WhatIUnderstood gets click-through via `Ctrl+G` after ~1 s) | static splice from the matching `live/NN-…png` for 2–3 s — duplicate frames; do NOT use the raw window |
   | **indicator-wait** | spinner / progress bar; visually static minus the rotating indicator (Gemini text pass, generation queue) | take short real-time windows at 25 fps from meaningful milestones; never resample one long wait into a time-lapse |
   | **transition** | a fast cross-fade between two states, < 1 s | usually skipped or rolled into the neighbouring section |

   For the standard kamishibai flow the typical mapping is:
   - `0s → first_busy`: A typing (workflow, 1.5 s output)
   - `first_busy → candidates_appear`: B busy understanding (indicator-wait, 1.2 s output)
   - candidates window: C `02-what-i-understood.png` static splice (read, 1–3 s output)
   - `building_starts → all_done`: D 0.2 s real-time windows around each visible redraw,
     including retry ticks, until the fifth card fills the viewport; then one 0.6 s publish
     transition jumps to the completed batch
   - first `all_done`: E navigate to `chouette`, open the editor, focus level, and move `a2 → b1`
   - `1 pending → all_done`: F hold the struck sentence, press `Ctrl+G`, then show each
     one-card regeneration artifact for one consistent 0.6 s beat
   - final `all_done → end`: G hold the rewritten collapsed `b1` card; the gif does not
     reopen the editor after regeneration

   New states (e.g. an extra confirmation step, a style picker) will surface as additional
   transitions — slot them into a type by inspecting the cut frame, don't drop them.

9. **Propose the slice plan to the operator** — print a table with section type, source
   window, sample rate, and projected output duration **before** running ffmpeg. Get the
   green light, then encode. Sample sketch:

   ```
   Section             Type             Source                  fps     Output
   A typing            workflow         main 0.24 → 1.92 s      25      1.68 s
   B understand/review mixed            main + static PNG       25/—    2.40 s
   C senses            workflow         main event windows      25      5.00 s
   D first generation  indicator-wait   20 × 0.2 s + publish    25      4.60 s
   E navigate + raise  workflow         nav event windows       25/—    5.36 s
   F regenerate        indicator-wait   five 0.6 s windows      25      3.00 s
   G result            fade             collapsed result PNG    —       3.24 s
   Total                                                               25.28 s
   ```

10. **Encode** once the plan is approved:

    ```bash
    RAW=/tmp/raw.gif ADJUST_RAW=/tmp/adjust-raw.gif \
      NAV_RAW=/tmp/nav-adjust-raw.gif ./encode.sh
    ```

    `encode.sh` prints the exact final duration and writes every section boundary to
    `timings.timeline.txt`. All raw recordings stay on disk for the next iteration.

### Common pitfalls — read before recording

- **Never sample a spinner section below 25 fps.** Each source frame represents N × 40 ms
  of real rotation; if N > 1, the spinner appears N × faster in the output. Keep the source
  fps high (25) and shorten the window instead.
- **Never treat WhatIUnderstood (or any other click-through state) as a workflow section.**
  `Ctrl+G` fires immediately after candidates land, so the raw recording shows it for ~1.5 s.
  Use the screenshot as a static splice for 2–3 s so the glosses are readable.
- **Never carry over hardcoded section windows from a prior recording.** Gemini latency
  varies. Run scene-detect first.
- **Never count transitions in advance.** New states get added to the flow over time —
  scene-detect surfaces them automatically; classify by inspecting `cut-NN.png`, don't drop
  unknown sections.
- **Never delete `/tmp/raw.gif`, `/tmp/adjust-raw.gif`, or `/tmp/nav-adjust-raw.gif` until
  you've decided you don't need another slice pass.** Re-recording costs a few minutes of
  Gemini wall-clock; re-slicing is local and preserves the original takes.
- **Don't try to "fix" an already-post-processed gif by duplicating its frames.** A
  derivative gif has already lost spinner sampling fidelity; you have to go back to the raw.

11. **Confirm** with `git status` that only the regenerated assets and intentional docs/code
    changes are staged. Keep the raws when the operator wants room for later timing changes.

### Demo input

`capture.tape` runs with `my_language=en`, so the target language resolves to French and the
header reads **`EN → FR`**. The tape types seven French words on `YourWords`: `dépaysement`,
`flâner`, `canard`, `chouette`, `râler`, `terroir`, `bof` — a mix of untranslatable nouns, a
verb, and colloquialisms (`canard` doubles as "duck" and "newspaper hoax"); all yield strong
manga panels and interesting English glosses. The synthetic `examples/tui_states.rs` walker
mirrors this EN→FR flow with the first four of those words. After the first complete build,
the tape walks up from the last built card to the simple `chouette` card, moves its level
exactly one step, waits until the footer proves there is exactly `1 pending`, closes the card
so the staged rewrite reads as one collapsed row (`09d-card-staged.png`), and presses
`Ctrl+G`; this keeps the regeneration story scoped to one card. The README gif ends on the rewritten collapsed
card. The tape may continue afterward to capture the separate open-card screenshots.

### Synthetic and edge-case shots

The review, six environment/modal/failure/retry PNGs, twelve sentence-label scenarios,
five Esc lifecycle PNGs, two batch-settings PNGs, two language-pair PNGs, the
Welcome language grid, and the parked multi-expanded cards view listed in step 3 are produced
reproducibly by `states.tape` and `states-narrow.tape`, which drive
`examples/tui_states.rs` through the same EN→FR flow without Gemini. The sentence-label
scenarios keep the established indices 0–10 intact: S1 is index 6, S2 replaces the removed
per-card modal at index 7, S3–S9 are indices 11–17, S10–S12 are indices 18–20, the retry
stress gallery is index 21, the Esc clear/back/stop/drain/partial states are indices 22–26,
the open generation-guidance editor is index 27, the `also plausible` alternates row is
index 28, the language-pair modal is index 29, the Welcome language grid is index 30,
and the parked multi-expanded cards view is index 31.
When the design changes, edit the demo data in
`examples/tui_states.rs` and re-run both synthetic tapes. If you add or reorder states in
the vector, update the absolute indices in both tapes and in the
`pty_state_demo_switches_mouse_pointer_between_link_and_plain_cells` test (it jumps to
the `Your cards` and `Done` indices by number).

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

One grammar of colour covers the whole TUI, and each of its three channels
carries exactly one meaning (`src/tui/palette.rs`). **Ink is rank inside the
row, never state**: `Ink::Subject` (`FG`) is what you came for — a built term,
a chosen value, a destination file, a broken artifact; `Ink::Detail` (`DIM`) is
whatever explains it — glosses, sentences, questions, labels; `Ink::Aside`
(`DIM2`) is the bookkeeping beside it — indices, costs, timings, separators,
inactive markers; `rule()` draws structure. **Background is focus and nothing
else**: the row under the cursor takes `HL` (`#26262a`, raised so it carries the
signal alone) and keeps the same inks as its neighbours. **Weight is focus too**:
`Modifier::BOLD` marks whatever the keyboard owns right now and nothing else, so
every letter the cursor covers is bold and no letter outside it is. The two
channels come out of one function (`Ink::on`), which is why a row cannot take the
band without taking the weight; a row that owns the keyboard while drawing no
band — the lit question of a card or guidance editor — asks for the weight alone
through `Ink::lit`. One consequence follows: an **underline means
clickable** and takes its brightness from its rank, so the same affordance reads
at three depths without three link colours. The label chips are the deliberate
exception to "ink is rank" — the three sentence-label values read as one
attribution strip, so both states are blocks and only their brightness changes.
The header is the deliberate exception to "weight is focus": its inverted title
block and its bold bright language chip are chrome standing outside the row
grammar, so weight means focus everywhere between them and never on them.

A card head that holds all four artifacts turns its term `FG`; its
target sentence stays `DIM` until the learner asks for that card to be rewritten,
and the sentence that comes back reads `Ink::Subject` — the same principle that
whitens a changed label chip, so the one card you touched stands out from the
ones you did not. A batch-wide level or type is pinned onto every card at
generation and deliberately does not count: `CardMeta::rewritten` is set only on
the per-card correction path (`CardCorrectionResponse::into_revision`), not on
the generation path every batch travels. While any artifact is still owed —
including a terminally failed card, which never holds all four — the term drops
to `Ink::Aside`, the same rank as the number beside it, and a card with a staged
rewrite keeps that quiet term beside its struck sentence. The glyph, the number, the `→` and the trailing cost are
`Ink::Aside`, which makes the head the same row as a `WhatIUnderstood`
candidate: index, term, separator, explanation. Brightness therefore means
built and weight means focused: a built card is bright wherever it sits, and
the selected head — built or not — is the one that goes bold, along with every
other letter its `HL` band covers. Every card state renders the same **step block**
beginning immediately after the last line of the head's target sentence,
including when that sentence wraps: up to three old-style rows — `scene`
(the written material, i.e. the meta slot), `voice` (audio), `manga` (the whole
visual phase) — each a state glyph, a five-letter label, and its own
incremental cost (`StepRow` in `src/tui/screens/your_cards.rs`). Every state
starts in one shared value column right after the eight-cell label column.
A row appears only once its work started.
A ready row shows a quiet `Ink::Aside` `✓` and an `Ink::Detail` label,
underlined when it clicks open: `scene` the card's cache cell (the parent of
`meta.json`) with the system handler, `voice` the audio file, `manga` the
rendered page; a label without a recorded target renders plain and does not
click. The one artifact that terminally gave up is the exception and the only
`Ink::Subject` span a card block ever draws — its `✗` and its label both — so a
healthy screen holds no bright spot outside its built terms and a broken card
announces itself without a word of status. A ready row states either
its incremental cost or, when it cost nothing because the artifact came back
from the cache, `cached` — never both, and never an empty value column for a
cache hit. The active row is its spinner and its label alone — no words, no
value; a terminal failure is a bare `✗` plus any known cost, with no `gave up`
to push the money out of the shared column; a discarded artifact `⊘ discarded`; an inactive
retry (or a ready scene whose picture is still owed) `·` plus any known cost.
Scene and picture work share the one `manga` row. File names and file sizes
are gone everywhere. Costs are incremental per row: `scene`
folds the meta and scene-composition spend, `voice` and `manga` carry only
their own artifact, retries included, and the head keeps the card total. No
`sentence:` heading or separator glyph is drawn anywhere. Register, phrase
kind, and level appear as three consecutive tags at one fixed column three
cells past the value column, on the `voice` row — **and only while that card is
collapsed**: an open block carries the same three values as its own tune rows,
so repeating them beside the steps would only say it twice. At
narrow widths whole tags may
continue at that same column on the `manga` row, and when the complete atomic
sequence cannot fit the whole summary hides while
the rows remain and the card head stays the mouse entry into tuning. A staged
rewrite parks the step rows one rank quieter (`Ink::Aside`) while the staged
tags remain visible; the expanded preview, which reads a rank higher, parks at
`DIM`. Retry
history lives on the card head instead of adding volatile status beside the
tags.
The tune rows sit
below the step rows, separated from `manga` by exactly one
blank row, before the expanded metadata and never to the rows' right. If
the live editor block fits the viewport, lighting it anchors the selected card
head at the top of the body; shorter viewports instead scroll only far enough to
keep the focused row visible.
Unchanged actual tags use a gray `DIM` background with dark letters; explicitly
changed or exactly fulfilled pinned tags use a white background with dark
letters and no bold. If
a pinned target could not be fulfilled exactly, the generated actual value stays
gray and is followed by muted `· aimed for` plus the requested value in a white
tag. The actual value remains the attribution of what was generated; the white
value remains the target for a later regeneration. Legacy cached approximation
records that predate separate actual-value storage show only muted `aimed for`
plus the requested white tag and never invent an actual value.

The editor's three carousel questions are `how should it sound?`, `what kind of
phrase?`, and `what's the desired level?`. The note label is `one more thing`, and its
placeholder is `say what should change`. The active carousel question is white
and bold (`Ink::lit`) against `Ink::Aside` for the rest, its two chevrons take
the same weight, and the selected chip has a white background — the editor is
the one place focus is drawn without the cursor band, so weight carries it
there. Every carousel is
permanently bracketed by the two-cell direction controls `< ` and ` >`; both
cells are clickable, focus that control's own row, and move one adjacent choice
without wrapping past either boundary. All three tracks use one render-time
width derived from the widest choice and the largest choice count across the axes,
so both chevrons share columns. Inside that fixed track the selected chip's
visual centre moves proportionally from the leading edge to the trailing edge
as its choice index increases. Every adjacent step transfers one hidden-choice
segment from the trailing rail to the leading rail. Segment widths differ by at most one cell,
spare cells go nearest the selected chip on each side, and every cell of a
segment belongs to the same clickable target. The nearest marker uses
`DIM2`, the next farther marker uses `RULE`, and every marker farther away is the
page background — the rail fades out rather than stopping, and it deliberately
does not borrow the cursor highlight, so it cannot brighten when a row does. A legacy axis with no selected value shows `—` with
one two-cell marker on each side inside the shared track; both cells of either
marker are clickable.

Regeneration carries the complete current three-axis preset. Every unedited
axis must keep its current requested value exactly. Only an explicitly changed
or already pinned axis may differ from its requested target, and only when the
result names that axis in `approx`; the generated value remains the actual
attribution and the requested target remains visible separately.

A settled batch opens its body with the **outcome strip** (`src/tui/screens/banner.rs`) — one block reading left to right as what you got and what you lost. On the left, one row per produced artifact: `FOLDER`, `APKG`, `PDF`, each an underlined `Ink::Subject` label over its path in `Ink::Detail`, behind the `│` gutter bar. On the right, level with the first row and pushed to the body's right edge so it lines up with the header's language chip, a bright ` N unfinished ` tag drawn with `sentence_labels::tag_style(true)` in **bold** — the one bold span outside the row the keyboard owns, because this block is the single sentence a settled screen must not let the eye slide past — the same white block that marks a changed sentence label — the same white block that marks a changed sentence label, so the only exception it opens is that weight. The tag is the **only** place the loss is stated; the dim tagline beside the title falls silent when it appears, and the footer never carried the count. `banner::losses` answers how many from the durable published tally when there is one and from the live census otherwise, and every reader — the tag, the tagline, the `Ctrl+G` gate on `Done` — asks it, so they cannot disagree. The strip shows itself whenever it has either half to report, which is what keeps a run where *every* card failed — and so published no deck at all — from losing its outcome along with its cards. A dashed rule closes the block off from the cards below, with no blank row on either side of it: the last content line, the rule, then the first card — the same way the status rule sits between the disclaimer above it and the footer below it without a spare row for either. That rule is chrome, not body: `ScreenView::body_rule` answers with the body-relative row and `common::render_screen` paints `dashed_rule` across the **full terminal width**, past the gutter the body rectangle stops short of, in the same `RULE` colour and the same column phase as the rule above the footer, so the two borders read as one vocabulary. `banner::height` is the single source of the strip's row count and `banner::rule_row` derives the rule's row from it; the scroll viewport, the scroll clamp, and the card hit-tester all subtract the height, so the block can grow without any of them being told.

A successfully published live batch remains on `YourCards`; reopening that
published session uses `Done`. Both final views permanently show the muted
`[Esc] new cards` immediately before `[Ctrl+C] quit`. The first `Esc` arms a
one-second confirmation and changes its hint to the highest-priority `[Esc]
again`; the second starts a clean `YourWords` batch in the same process,
preserving preferences and output location while rotating the persistent
session identity and cost journal. Any other action or timeout disarms the
confirmation. Everywhere else `Esc` closes exactly one layer from inside out:
an error, a modal, then on `YourCards` the editor first and the card's
expansion second (two presses peel an open editor down to a collapsed card),
on `WhatIUnderstood` the focused open sense list, then the current screen
action. Open blocks the focus is not on do not intercept `Esc` — `C` sweeps
them all at once.
On nonempty `YourWords`, a quiet `[Esc] clear` precedes the double-`Esc` clear;
on collapsed `WhatIUnderstood`, one `Esc` returns to the preserved words without
arming that clear, so it takes two fresh presses there; during generation, a
quiet `[Esc] stop` precedes the double-`Esc` the same way, and that double
`Esc` stops after the current request finishes and launches no next request. A
stop publishes the complete subset as `partial`, or, when no card is
complete, closes the old run as `cancelled`, rotates identity and cost scope,
and starts clean `YourWords`. While the current request drains the
header says `stopping…`. The same reset remains available after every card
terminally gives up and no package can be published, once the publication error
has been dismissed. `Ctrl+C` keeps an independent double-press quit confirmation.

Expanded metadata uses statement and noun labels: `the phrase`, `in your
language`, `a visual clue`, `word meaning`, `word pronunciation`, `phrase
pronunciation`, `worth learning`, and, when context exists, `the right context`.
