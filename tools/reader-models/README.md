# Rebuilding the reader-language data

Reader sentences can be checked by script alone when the surviving letters
identify a single supported language. Latin, Cyrillic and Arabic require
frequency profiles. Japanese, Chinese and Korean share Han characters. The
byte n-gram classifier resolves ambiguous text, including Han-only sentences.
All data used by the CLI is embedded in its executable.

The following crates supply the published data. The checksums refer to the
unmodified crates.io archives.

| Source | Version | SHA-256 |
| --- | --- | --- |
| langidentify-models-lite-a | 0.1.0 | `e86f815105e9cb5636db95828c8460b7055fa6452940bc1de132cb3d24dfaeef` |
| langidentify-models-lite-b | 0.1.0 | `9f0f40f963126f5e66a6da9cc0ed4b7708178695c4d343b614dda0a7e6b6a34b` |
| langid-rs | 1.1.0 | `13737fdd7a7043346109f6481c4ffe0c85a26eb786b9228a46ee3ef202241322` |
| ablaut | 0.11.0 | `90fbd3b5f61d10d580f8fb2cfe635a31564c83983e5d775a894fd568e9e5232c` |

Extract the four verified archives under one directory, retaining the
`<name>-<version>` directories. An existing extracted Cargo registry works
too. From the repository root run

```sh
cargo run --release --locked --manifest-path tools/reader-models/Cargo.toml -- \
    "$READER_CRATE_SOURCES" .
```

The packer selects the shared-script languages, preserves the byte automaton
and its feature weights for those languages, and quantizes lite-model log
probabilities in steps of 1/256. The four gzip streams have fixed headers
and ordering.

The packer also reproduces the four lemma lists and the MIT-licensed
reverse analyzer in `crates/reader-verbs` from Ablaut's published source.
That crate calls the German, English, French and Spanish forward paradigms
directly. Their productive rules cover inflections of previously unseen verbs.
Its differential test compares the resulting analyses with upstream Ablaut.

Pass a temporary directory as the second argument to compare its generated
files with the tracked copies using `cmp`. [Attribution and licenses](../../NOTICE)

## Backlog

- Detect same-script languages outside the CLI catalog, including Marathi on Hindi fronts, Yiddish on Hebrew fronts and Danish on Swedish fronts, if these languages enter the generation context.
