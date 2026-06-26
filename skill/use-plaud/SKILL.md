---
name: use-plaud
description: Drive the `plaud` CLI to work with your Plaud voice recordings — meetings, interviews, calls, voice memos. Use when someone wants to find, read, search, or export recordings, pull meeting action items, get a call transcript, or feed recording content into other systems (notes, knowledge base, CRM). Triggers include "what did we discuss in", "pull the transcript for", "find the call where", "search my recordings", "export the meeting notes", "action items from", "that Plaud recording", or any reference to Plaud / voice-recorder content.
---

# use-plaud — Driving the Plaud CLI

`plaud` is a Rust CLI that talks to Plaud's cloud via their MCP server. It's the
way to get at your voice recordings — transcripts, AI notes, action items,
audio — from the terminal. Repo and install instructions:
[github.com/leegonzales/plaud-cli](https://github.com/leegonzales/plaud-cli).

This skill assumes `plaud` is installed and on `PATH` (`cargo install --path .`
from the repo, or `cargo build --release`).

## First: is it authenticated?

Everything except `login`/`logout` needs a live session.

```sh
plaud whoami            # prints the account, or errors if not signed in
```

If it prints `session expired — run plaud login`, the user must run
`plaud login` themselves (it opens a browser for OAuth). You can't complete
that flow for them — ask them to run it, then continue.

## Core commands

| Goal | Command |
|------|---------|
| List recordings | `plaud list` (filters: `-q <kw>`, `--from/--to YYYY-MM-DD`, `--page`, `--page-size` ≥10) |
| Recording summary | `plaud get <id>` |
| AI notes (summary, minutes, action items) | `plaud note <id>` |
| Full transcript | `plaud transcript <id>` → `[mm:ss] Speaker: text` |
| Download audio | `plaud download <id> -o file.mp3` (only when Plaud has audio) |

## The local store: sync, search, export

`plaud` keeps a local store (`~/.plaud/store`, or `PLAUD_STORE`) so search is
fast/offline and export is cheap. **Search and export read the store**, so sync
first.

```sh
plaud sync                     # pull all recordings into the store
plaud sync --since-last        # only what's new since last sync (cursor-based)
plaud sync --since 2026-06-01  # only uploads on/after a date

plaud search "acme retainer"   # full-text over transcripts + notes + titles
plaud search "acme" --context 2  # ± N transcript segments around each hit

plaud export --dir ./out                  # all synced → Markdown (one file each)
plaud export <id> <id> --dir ./out        # specific recordings (live-fetches if unsynced)
plaud export --dir ./out --format json    # one normalized JSON per recording
```

`export --format md` writes `YYYY-MM-DD-<slug>.md` with YAML frontmatter +
action-item checklist + notes + transcript — drops straight into a Markdown
notes app or knowledge base (e.g. Obsidian).

## Scripting / piping (for agents)

Use these for programmatic work — never scrape the human tables.

```sh
plaud list --json | jq '.[].id'              # stable snake_case schema
plaud list --ndjson | jq -r .id              # one record per line (pipelines)
plaud note <id> --json | jq '.action_items'  # action items as a string[]
plaud transcript <id> --json | jq '.segments'
plaud get <id> --raw                          # unprocessed Plaud payload (debug)
plaud schema                                  # print the stable --json contract
```

Commands **exit non-zero on failure** — branch on exit codes in scripts.
Recording `id` is stable/immutable — safe to store in an external record.

## Common recipes

**"What did we discuss about X?"** — find the recording, then read it:
```sh
plaud sync --since-last >/dev/null
plaud search "X"                  # shows matching recordings + timestamps
plaud transcript <id>             # read the full thing
```

**Find every call mentioning a person or topic** (searches transcript bodies,
not just titles):
```sh
plaud sync --since-last >/dev/null
plaud search "<name or keyword>"
```

**Export meeting notes into a notes vault / knowledge base:**
```sh
plaud sync --since-last >/dev/null
plaud export --dir /path/to/your/notes/meetings/ --format md
```

**Action items from a meeting:**
```sh
plaud note <id> --json | jq -r '.action_items[]'
```

## Gotchas (verified against the live API)

- **Not every recording has a transcript or audio.** Older clips often return
  empty — that's Plaud's data, not a CLI bug. `transcript`/`download` say so.
- **Speaker labels are inconsistent.** Newer recordings carry `Speaker 1/2`;
  older ones have none. Don't assume labels are present.
- **`--page-size` must be ≥ 10** (Plaud server rule). The CLI surfaces the
  error cleanly.
- **`search`/`export` need a prior `sync`.** Empty store → "run `plaud sync`
  first".

## When NOT to use this

- Generating *new* recordings — `plaud` is read/export only.
- Editing notes or transcripts on Plaud's side — not supported.
- Anything requiring the Plaud mobile/desktop app's editing UI.
