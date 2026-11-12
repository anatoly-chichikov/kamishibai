# Anki history v1

`v1.zip` is produced by `tests/anki_history.rs` from a synthetic schema-18
collection. It has two selected home decks, two notes, two cards, three surviving
reviews (one Again), and three media names sharing two payloads. A filtered card
returns to its home deck and original due date. An unrelated deck and its media,
a deleted-card review, profile name and source paths never enter the file.

The fixed writer is `1.14.0`, `written_at` is `2023-11-21T20:53:20Z`, archive
entry timestamps are fixed, and every SQL/media/ZIP iteration is ordered.
The manifest matches the web server's strict `Manifest` with no extra fields.

SHA-256: `dd428a7b0fb2cddd47afa06dae50c0086054c87c540dcf6010fd3bafe2c688fa`

Verify two independent reads/exports against these checked-in bytes:

```sh
cargo test --test all anki_history::version_one_fixture_cannot_drift_between_two_exports
```

Regenerate only when intentionally updating the cross-repository fixture:

```sh
UPDATE_ANKI_HISTORY_FIXTURE=1 cargo test --test all anki_history::version_one_fixture_cannot_drift_between_two_exports
```

Coordinate any format change with the web reader before changing this fixture.
A changed Manifest requires a new artifact version, not silently changed v1 bytes.
