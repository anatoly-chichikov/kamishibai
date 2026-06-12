# The kamishibai console

Everything non-interactive in kamishibai is a **session**: a persistent, curatable unit of
work you drive across separate invocations. A session moves through
`understood → (curate) → generating → published`, ending `partial` when some cards failed
but the deck shipped the rest, `failed` when no card survived, `interrupted` when its
worker died, or `cancelled` when you stopped it. This page is the human reference for all
thirteen verbs, with real example outputs; agents should read [llms.txt](../llms.txt), the
machine-facing contract.

## Conventions

- **stdout carries the one capturable value** — a session id, a path, or a requested block
  of text. Everything else (previews, progress, hints, errors) goes to **stderr**, so
  `id=$(kamishibai new --word bank)` just works.
- `-q`/`--quiet` reduces a command to its bare capturable value and silences the narration.
- `--json` (after the verb) prints exactly one JSON document per invocation instead; it is
  mutually exclusive with `-q` and the `result` path selectors. Schema in
  [llms.txt](../llms.txt).
- Exit codes are the script signal:

  | code | meaning |
  | ---- | ------- |
  | 0 | ok |
  | 2 | usage — change the invocation |
  | 3 | no such session |
  | 4 | not ready yet — generate or wait first |
  | 5 | ambiguous — an omitted id matched several sessions |
  | 1 | any other error |

### Omitting the session id

The `<id>` is optional everywhere. When you leave it out, the command resolves the session
once, at invocation:

1. exactly one session exists → it is used, even a finished one;
2. otherwise, exactly one session is **unfinished** (`understood`/`generating`/`interrupted`)
   → it is used;
3. otherwise nothing runs: the command prints the newest five sessions (newest first, plus
   `…and N more — kamishibai ls` when more exist) and exits `5`.

With no sessions at all it exits `3`. Whenever resolution picks a session for you, one
stderr line says so: `using session <id>`. The cascade prefers the session you are
*working on*, not the one that is done — with a published deck and a fresh understood
session, `result` resolves to the fresh one and exits `4`; pass the id to read the older
deck.

## A session, start to finish

```console
$ kamishibai new --word canard --from en --to fr   # stderr: the understood senses
canard
  card  zool.   a duck, the water bird.
  skip          a deliberately false story planted in the press.
session fr-20260612_192903_8x2k · senses=primary · generate: kamishibai generate
fr-20260612_192903_8x2k                            # ← stdout: the id (capturable)

$ kamishibai select --card canard --sense 2        # curate: keep the hoax sense
$ kamishibai generate                              # detached worker starts
$ kamishibai status -q                             # poll at a relaxed interval
generating
$ kamishibai status -q
published
$ kamishibai result --deck
using session fr-20260612_192903_8x2k
/Users/you/kamishibai-out/fr_2026-06-12_192919.apkg
```

## Commands

### kamishibai new

`kamishibai new (--word WORD … | --words FILE | --build FILE) [--to LANG] [--from LANG]
[--senses primary|all] [--out DIR] [--id NAME] [--generate] [-q | --json]`

Understands the words (one Gemini pass) and creates a session in the **understood** stage.
Exactly one input form: repeated `--word` flags, `--words FILE` (one word per line, `-` for
stdin), or `--build FILE` (a strict cards JSON, skips understanding; its entries carry the
language pair). The understood-senses preview lands on stderr, the bare id on stdout:

```console
$ kamishibai new --word "you're gonna get yours" --word bank --from ru --to en
understanding 2 word(s) · ru → en
you're gonna get yours
  card  разг.   Идиом. «ты получишь по заслугам», обещание неизбежного наказания.
bank
  card  фин.    Сущ. «банк», организация для хранения денег.
  skip          Сущ. «берег» реки, озера, любого природного водоёма.
session en-20260612_101502_0cfy · senses=primary · generate: kamishibai generate
en-20260612_101502_0cfy
```

### kamishibai status

`kamishibai status [<id>] [-q | --json]` — the session's stage, read from the cache (no
Gemini). Before a plan is committed it lists the curatable candidates (`*` marks a selected
sense); after, the per-card artifact progress. `-q` prints only the phase word
(`understood`/`generating`/`published`/`partial`/`failed`/`interrupted`/`cancelled`).

```console
$ kamishibai status
using session fr-demo
session  fr-demo
pair     en → fr
senses   primary
phase    understood
words    1 understood · 1 card(s) selected
word  canard   card
  *  1          a duck
out      /Users/you/kamishibai-out
```

While generating, each card shows which of its four artifacts already exist:

```console
$ kamishibai status
session  en-20260612_101502_0cfy
pair     ru → en
phase    generating
worker   pid 48213 alive
cards    2 total · 1 ready · 0 failed
card  you're gonna get yours   meta:ok sound:ok scene:ok picture:ok   ready
card  bank                     meta:ok sound:ok scene:-- picture:--   building
out      /Users/you/kamishibai-out
```

### kamishibai select / exclude / correct

`kamishibai select [<id>] --card TERM --sense 1,3` — pick which 1-based senses become
cards (re-including the word). `kamishibai exclude [<id>] --card TERM` — drop a word from
the plan while keeping it visible in the understanding. `kamishibai correct [<id>] --card
TERM --note "…"` — ask Gemini to add senses from an instruction. Each resets the session
to **understood** and prints the id again:

```console
$ kamishibai select --card bank --sense 2
using session en-20260612_101502_0cfy
selected sense(s) 2 of 'bank'
en-20260612_101502_0cfy
```

### kamishibai generate

`kamishibai generate [<id>] [--wait] [-q | --json]` — commits the curated plan (derived
from the selected senses) and starts a managed background worker that generates the
missing artifacts and publishes; prints the id and returns at once. Re-running is
idempotent and cache-backed: a killed run resumes where it stopped.

```console
$ kamishibai generate
using session fr-demo
started session fr-demo (background)
poll: kamishibai status fr-demo
fr-demo
```

`--wait` runs the worker in the foreground, streaming stable progress tokens on stderr and
printing the three paths (deck, pdf, dir) on stdout:

```console
$ kamishibai generate --wait
generating 2 card(s)…
  cache  you're gonna get yours · meta
  ok     bank · meta
  retry  bank · picture (1/3)
  ok     bank · picture
building deck and report…
done: 2 card(s) published
/Users/you/kamishibai-out/en_2026-06-12.apkg
/Users/you/kamishibai-out/en_2026-06-12.pdf
/Users/you/kamishibai-out
```

### kamishibai open

`kamishibai open [<id>]` — reopen the session in the interactive TUI, resuming from the
cache. Refused while a worker is generating it.

### kamishibai result

`kamishibai result [<id>] [--deck | --pdf | --dir | -q | --json]` — the finished cards and
published paths, once the session is **published** or **partial** (exit `4` otherwise).
The selectors print exactly one path each; `-q` prints the three paths.

```console
$ kamishibai result
using session fr-demo
session  fr-demo
pair     en → fr
deck     /Users/you/kamishibai-out/fr_2026-06-12_192919.apkg
pdf      /Users/you/kamishibai-out/fr_2026-06-12_192919.pdf
dir      /Users/you/kamishibai-out
cards    1 in deck

card 1/1  canard   importance 5
  meaning  a duck
  say      ka.naʁ
  fr       Le canard a nagé dans l'étang.
  en       The «duck» swam across the pond.
  hint     a water bird
```

### kamishibai regenerate

`kamishibai regenerate [<id>] (--failed | --card TERM [--note "…"]) [--json]` — drop
cached artifacts so the next `generate` rebuilds them: every unfinished card with
`--failed`, or one card by `--card`. With `--note`, Gemini first rewrites the card from
the instruction:

```console
$ kamishibai regenerate --card bank --note "make the sentence shorter"
using session en-20260612_101502_0cfy
rewrote card 'bank'; generate the session to rebuild it
en-20260612_101502_0cfy
```

### kamishibai cancel

`kamishibai cancel [<id>] [--json]` — stop the running worker. Always exits `0` with a
correct final phase, even when it races the worker finishing; an already-finished session
stays finished.

```console
$ kamishibai cancel
using session fr-demo
cancelled session fr-demo
```

### kamishibai ls

`kamishibai ls [-q | --json]` — every session, one line each: id · pair · phase ·
progress (`-- / N` is a curation count before a plan is committed, `ready/total` after).
`-q` prints only the ids.

```console
$ kamishibai ls
fr-demo   en → fr  published   1/1
fr-next   en → fr  understood  -- / 1
```

### kamishibai rm

`kamishibai rm [<id>] [--cache] [--json]` — delete the session; `--cache` also removes its
cached card folders, forcing a future rebuild from scratch.

```console
$ kamishibai rm fr-demo
removed session fr-demo
```

### kamishibai cache-path

`kamishibai cache-path [--json]` — print the cache directory (override with
`KAMISHIBAI_CACHE`).

```console
$ kamishibai cache-path
/Users/you/Library/Caches/kamishibai
```

## When something refuses

Every error is one `kamishibai: …` line on stderr, and the exit code tells a script what
to do next:

```console
$ kamishibai status ghost
kamishibai: no session 'ghost'                       # exit 3 — fix the id
$ kamishibai result
using session fr-next
kamishibai: session 'fr-next' not ready (phase understood)   # exit 4 — generate first
$ kamishibai generate
fr-third  en → fr  understood  -- / 1                # exit 5 — several sessions matched:
fr-next   en → fr  understood  -- / 1                #   the newest five, newest first
fr-demo   en → fr  published   1/1
kamishibai: 3 sessions; pass an id (kamishibai ls)
$ kamishibai select --card bank --sense 9
kamishibai: sense 9 out of range (1..=2) for 'bank'  # exit 2 — change the invocation
```

## See also

- [llms.txt](../llms.txt) — the agent-facing contract: JSON schemas, exit-code table,
  concurrency promises.
- [README](../README.md) — install and the interactive TUI.
