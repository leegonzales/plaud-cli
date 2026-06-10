# plaud

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

```sh
plaud login                       # browser OAuth sign-in
plaud whoami                      # current account
plaud list                        # recent recordings (table)
plaud list -q standup --from 2026-06-01 --to 2026-06-10
plaud list --page 2 --page-size 50
plaud get <id>                    # full recording detail
plaud note <id>                   # AI summary / action items / key topics
plaud transcript <id>             # transcript with timestamps + speakers
plaud download <id> -o talk.mp3   # audio via 24h presigned URL
plaud logout                      # forget stored tokens

plaud --json list | jq '.[].id'   # raw JSON on any command
```

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
- `commands.rs` — one handler per subcommand
- `output.rs` — table / transcript rendering; `--json` passthrough
- `config.rs` — token storage

Unofficial; not affiliated with Plaud.
