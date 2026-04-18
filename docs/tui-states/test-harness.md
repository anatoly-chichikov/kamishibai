# Kamishibai TUI Test Harness (locked)

## Locked choices

| Layer                              | Pick                                             | Reason |
| ---------------------------------- | ------------------------------------------------ | ------ |
| Deterministic render tests         | `ratatui::backend::TestBackend` + `insta`        | Native to the chosen UI stack, yields byte-accurate buffers that diff cleanly in snapshot review, zero real-terminal flakiness. |
| Keyboard-driven flow tests         | `ratatui::backend::TestBackend` + handcrafted `crossterm::event::KeyEvent` | Uses the actual input types the production code will route, stays in-process, deterministic. |
| PTY-level end-to-end flow tests    | `expectrl`                                       | Still maintained, runs on Unix CI, sufficient for one smoke test that validates the binary boots, accepts keystrokes, and exits cleanly. |
| Mocked LLM responses               | Plain trait impls / in-test fakes (no network)   | No separate mock library needed — all LLM surfaces are trait-bounded, tests provide fake implementations. |

Rejected: `termwright`. It targets a richer editor experience we do not need and
its upstream moves slower than `expectrl`. Revisit only if `expectrl` regresses.

## File layout

- `tests/snapshot.rs` — deterministic render snapshots per locked-in screen.
- `tests/keyboard.rs` — KeyEvent → transit → render assertion for the
  `YourWords -> WhatIUnderstood` path.
- `tests/state_machine.rs` — render-free transition coverage.
- `tests/pty.rs` — one PTY smoke test over the `tui_skeleton` example.
- `tests/config.rs`, `tests/session.rs`, `tests/language_flow.rs` — foundation
  language/preference tests.
- `examples/tui_skeleton.rs` — minimal stdin/stdout skeleton the PTY test
  drives through `expectrl`. Emits plain-text screen markers so that the PTY
  harness does not have to decode ANSI escape sequences; the real ratatui
  render is covered by `tests/snapshot.rs`.

## Conventions

- Snapshots live under `tests/snapshots/`. Review with `cargo insta review`.
  Update in CI with `INSTA_UPDATE=always` only when a render change is
  intentional and reviewed.
- All tests must avoid network calls. `GEMINI_API_KEY` must not be read.
- Fake `TargetDetection` / LLM pass implementations live inside each test file
  to keep fixtures inline, per the testing principles in CLAUDE.md.
- Tests must use the `TestBackend::new(80, 12)` default size unless a screen
  needs more rows — in that case increase height only, keep width at 80.
- PTY tests are Unix-only (`#![cfg(unix)]`). Windows regression is out of
  scope for this project.

## Why there is a separate plain-text skeleton

`expectrl` operates on raw PTY bytes. ratatui writes optimised ANSI escape
sequences that may interleave cursor moves between the characters of a word,
which makes substring matching fragile. The render path is already covered by
`tests/snapshot.rs` with byte-accurate `TestBackend` output. The PTY test only
needs to prove that the shell boots, the keyboard contract lands, and the
process exits cleanly — a plain-text marker skeleton is the most reliable way
to assert that without second-guessing ratatui's optimiser.
