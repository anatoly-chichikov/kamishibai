# Rust Parity Contract

Этот документ фиксирует Python baseline, на который должен опираться Rust rewrite из Linear project `Kamishibai Rust Rewrite Ready for Execution`.

## Baseline

- На `2026-04-12` Python-реализация проходит `uv run pytest` без пропусков: `176 passed in 5.64s`
- Reference source of truth: архивные Python модули в `python_reference/src/kamishibai/` и manifests в `tests/fixtures/reference/manifests/`
- Regeneration command: `uv run python scripts/regenerate_rust_parity.py`

## Input Contract

- Корень документа обязан быть JSON object с ключом `entries`
- Каждая валидная entry обязана содержать `term`, `source.sentence`, `source.lang`, `target.sentence`, `target.lang`
- Невалидные entries отфильтровываются, а полностью пустой валидный результат превращается в `ValueError`
- Normalized output shape зафиксирован и не сокращается:
  - `word`
  - `pronunciation`
  - `translation`
  - `example`
  - `source_lang`
  - `target_lang`
  - `sentence`
  - `highlight`
  - `hint`
  - `context`
  - `importance`
  - `transcription`
- `importance` приводится к строке
- Nullable text fields коалесцируются в пустую строку

## Language Profiles

- Поддерживаемые языки: `de`, `el`, `en`, `es`, `ru`, `zh`
- Fallback OCR: `eng`
- Полный profile oracle хранится в `tests/fixtures/reference/manifests/profiles.json`
- Зафиксированная асимметрия обязательна:
  - `Fonts.selected()` смотрит на `source_lang` и `target_lang`
  - `Labels.selected()` смотрит только на `source_lang`
- Chinese font special-case обязателен: `Hiragino Sans GB`

## Runtime Contract

- Packaged assets:
  - `audio_prompt.txt`
  - `scene_prompt.txt`
  - `manga_template.json`
- Gemini model IDs зафиксированы:
  - `gemini-3-flash-preview`
  - `gemini-3.1-flash-image-preview`
  - `gemini-2.5-flash-preview-tts`
  - `gemini-2.5-pro-preview-tts`
- TTS использует фиксированный voice pool, зафиксированный в `tests/fixtures/reference/manifests/runtime.json`
- Scene translation обязана:
  - strip markdown fences
  - parse raw JSON array
  - deep-copy template
  - merge generated panels into `manga_panel.panels`
  - заполнить `meta.title`, `meta.description`, `meta.target_lang`
  - clamp bounds и принудительно ставить `text_in_frame = none`
- Scene cache semantics, digest naming и progress behavior зафиксированы в `tests/fixtures/reference/manifests/cache.json`

## Path And CLI Contract

- Источники входного файла в порядке приоритета:
  - positional path
  - `KAMISHIBAI_INPUT`
  - `kamishibai.json` в current working directory
- Источники output directory:
  - `--output`
  - `KAMISHIBAI_OUTPUT`
  - `output` рядом с input file
- Источники cache directory:
  - `--cache`
  - `KAMISHIBAI_CACHE`
  - platform-specific cache home + `kamishibai`
- Platform fallbacks:
  - macOS cache: `~/Library/Caches`
  - macOS data: `~/Library/Application Support`
  - non-macOS cache: `XDG_CACHE_HOME` or `~/.cache`
  - non-macOS data: `XDG_DATA_HOME` or `~/.local/share`
- Exit codes:
  - success `0`
  - handled failure `1`
  - Ctrl+C `130`
- Plain and rich progress expectations зафиксированы в:
  - `tests/fixtures/reference/manifests/plain-cli.txt`
  - `tests/fixtures/reference/manifests/rich-progress.json`

## Artifact Contract

- Stable ID algorithm:
  - SHA-256 от имени
  - первые `8` hex digits
  - `mod 2^31`
- Anki note contract:
  - точные `11` field names
  - точный field order
  - текущие HTML templates без изменений
  - media references только относительные внутри note payload
  - attach API принимает абсолютные file paths
- APKG structural oracle хранится в `tests/fixtures/reference/manifests/apkg.json`
- PDF contract:
  - thumbnail size `150`
  - JPEG quality `60`
  - source-language labels
  - mixed-font rendering
  - page break threshold, эквивалентный Python baseline
- PDF layout and structural oracle хранится в `tests/fixtures/reference/manifests/report.json`

## External Dependencies

- Runtime:
  - `GEMINI_API_KEY`
  - `tesseract`
  - language packs для Tesseract
- Archived Python reference harness:
  - `fc-match`
- Rust rewrite version policy:
  - crate versions берутся как latest stable на `2026-04-12`
  - после выбора фиксируются одновременно в `Cargo.toml` и `Cargo.lock`
  - автоматические upgrades вне scope baseline

## Reference Fixtures

- `tests/fixtures/reference/inputs/` содержит:
  - single-target English
  - single-target Greek
  - single-target German
  - single-target Spanish
  - single-target Russian
  - single-target Chinese
  - mixed-target deck naming case
  - invalid document case
  - aggregate supported-languages case
- `tests/fixtures/reference/manifests/normalized/` содержит exact normalized outputs
- `tests/fixtures/reference/manifests/invalid-document.json` фиксирует failure shape
- `tests/fixtures/reference/manifests/baseline.json` фиксирует pytest baseline

## Rewrite Policy

- Rust parity tests должны быть behavior-first
- Python internal hooks не обязаны становиться частью Rust public API, если behavior закрыт tests и manifests
- Любое расхождение с manifests считается регрессией, пока оно не оформлено как отдельное архитектурное решение
