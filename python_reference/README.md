# Python Reference

Этот каталог хранит архивный Python runtime, который использовался как oracle во время Rust cutover.

- `python_reference/src/kamishibai` содержит исходный Python implementation
- root `tests/fixtures/reference` содержит frozen parity fixtures
- `uv run pytest` и `uv run python scripts/regenerate_rust_parity.py` нужны только для parity maintenance
- shipping entrypoint репозитория теперь только Rust binary через `cargo run --`
