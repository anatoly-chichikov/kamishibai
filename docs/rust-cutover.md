# Rust Cutover

Этот документ фиксирует итог cutover после переписывания `kamishibai` на Rust.

## Preserved

- входной JSON contract и full normalized entry shape
- поддерживаемые языки `de`, `el`, `en`, `es`, `ru`, `zh`
- fallback OCR `eng`
- packaged assets `audio_prompt.txt`, `scene_prompt.txt`, `manga_template.json`
- Gemini model IDs, TTS voice pool, `RESOURCE_EXHAUSTED` fallback
- cache digest naming, scene JSON persistence, blocked-image diagnostics
- stable IDs, 11 Anki fields, HTML newline-to-`<br>` conversion, APKG media semantics
- PDF labels, font-selection asymmetry, thumbnail compression, pagination behavior
- CLI exit codes `0`, `1`, `130`
- plain and rich progress and diagnosis output contracts

## Eliminated

- Python CLI как shipping entrypoint
- `uv run kamishibai` как основной способ запуска приложения
- dual-runtime documentation в root README и AGENTS
- зависимость на внешнюю утилиту `fc-match` для Rust runtime
- Python package path в корневом `src/`

## Archive

- архивный Python oracle находится в `python_reference/src/kamishibai`
- parity manifests и offline fixtures остаются в `tests/fixtures/reference`
- regeneration script остаётся `scripts/regenerate_rust_parity.py`
- Python harness больше не является canonical runtime и используется только для parity maintenance
