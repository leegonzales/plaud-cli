# plaud

[![CI](https://github.com/leegonzales/plaud-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/leegonzales/plaud-cli/actions/workflows/ci.yml)

A small Rust CLI for your [Plaud](https://www.plaud.ai/) recordings. It's a
thin [Model Context Protocol](https://modelcontextprotocol.io) client over the
official Plaud MCP server (`https://mcp.plaud.ai/mcp`) — the same capabilities
Plaud exposes to AI assistants, but as a terminal tool you can pipe into `jq`.

## Install

```sh
cargo install --path .
# or
cargo build --release   # -> target/release/plaud
```

## Usage

### Live reads

```sh
plaud login                       # browser OAuth sign-in
plaud whoami                      # current account
plaud list                        # recent recordings (table)
plaud list -q standup --from 2026-06-01 --to 2026-06-10
plaud list --page 2 --page-size 50          # page-size >= 10
plaud get <id>                    # recording summary
plaud note <id>                   # AI summary / action items / minutes
plaud transcript <id>             # transcript: [mm:ss] Speaker: text
plaud download <id> -o talk.mp3   # audio via 24h presigned URL
plaud logout                      # forget stored tokens
```

### Local store: sync, search, export

`plaud` keeps a local store of normalized recordings so search is fast and
offline, and export is cheap. The store lives at `~/.plaud/store/` (override
with `PLAUD_STORE`).

```sh
plaud sync                        # pull all recordings into the store
plaud sync --since-last           # only what's new since the last sync
plaud sync --since 2026-06-01     # only uploads on/after a date
plaud sync --limit 20             # cap work this run (newest first)

plaud search "acme retainer"      # full-text over transcripts + notes + titles
plaud search "acme" --context 2   # ± N transcript segments of context

plaud export --dir ./meetings                 # all synced -> Markdown files
plaud export <id> <id> --dir ./out            # specific recordings
plaud export --dir ./out --format json        # one JSON per recording
```

`export --format md` writes one `YYYY-MM-DD-<slug>.md` per recording, with YAML
frontmatter, an action-items checklist, the AI notes, and the transcript —
ready for Obsidian / SecondBrain. Exporting an id that isn't synced fetches it
live and caches it.

### Output modes (global flags)

```sh
plaud list --json | jq '.[].id'             # normalized JSON, stable schema
plaud list --ndjson | jq -r .id             # one record per line (pipelines)
plaud note <id> --json | jq '.action_items' # action items as a string[]
plaud transcript <id> --ndjson              # one segment per line
plaud get <id> --raw                        # unprocessed Plaud payload
plaud schema                                 # print the stable --json schema
```

`--json` emits a documented, stable, snake_case schema (see `plaud schema`).
`--ndjson` emits one element per line. `--raw` dumps the unprocessed Plaud
payload. Commands exit non-zero on failure.

## How auth works

`login` runs the standard MCP OAuth 2.1 flow against `mcp.plaud.ai`:

1. Discover the protected-resource and authorization-server metadata.
2. Dynamically register a public client (PKCE, `token_endpoint_auth_method=none`).
3. Open the browser to authorize; a localhost loopback catches the redirect.
4. Exchange the code (PKCE `S256`) for access + refresh tokens.

Tokens are cached at `~/.plaud/cli-tokens.json` (mode `0600`) and refreshed
automatically on a `401`. This file is separate from the official client's
`~/.plaud/tokens-mcp.json`, so the two never collide. `logout` deletes it.

## Tools mapped

| Command      | MCP tool           |
|--------------|--------------------|
| `whoami`     | `get_current_user` |
| `list`       | `list_files`       |
| `get`        | `get_file`         |
| `note`       | `get_note`         |
| `transcript` | `get_transcript`   |
| `download`   | `get_file` → `presigned_url` |

`download` only works when Plaud has a `presigned_url` for the recording;
some recordings (e.g. very short clips) don't carry one, and the command
reports that clearly. `transcript` and `note` decode Plaud's nested
`data_content` blocks — the verbatim transcript and the AI summary / meeting
minutes — and fall back to raw JSON for any block shape they don't recognize.

## Layout

- `oauth.rs` — discovery, DCR, PKCE, loopback redirect, token + refresh
- `mcp.rs` — Streamable-HTTP MCP client (initialize, session id, `tools/call`)
- `model.rs` — normalize Plaud's nested payloads into one stable `Record`
- `store.rs` — local sync store (records + cursor)
- `commands.rs` — one handler per subcommand
- `output.rs` — human-readable rendering (table, transcript, notes)
- `config.rs` — token storage

Unofficial; not affiliated with Plaud.
