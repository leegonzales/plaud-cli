# plaud

[![CI](https://github.com/leegonzales/plaud-cli/actions/workflows/ci.yml/badge.svg)](https://github.com/leegonzales/plaud-cli/actions/workflows/ci.yml)
[![Platform: macOS](https://img.shields.io/badge/platform-macOS-lightgrey.svg)](#requirements)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

Your [Plaud](https://www.plaud.ai/) voice recordings, from the terminal.

`plaud` is a small, fast Rust CLI for the recordings on your Plaud account —
meetings, interviews, calls, voice memos. List them, read transcripts and AI
notes, pull action items, search across everything, and export to Markdown or
JSON. It's a thin [Model Context Protocol](https://modelcontextprotocol.io)
client over Plaud's official MCP server, so it offers the same capabilities
Plaud exposes to AI assistants — but as a Unix tool you can pipe into `jq`.

```console
$ plaud search "pricing"
=== 2026-06-12  Acme renewal call  (a1b2c3…) ===
  [14:02] Lee: …so the retainer pricing we discussed was twelve…
  (action) Send Acme the updated pricing sheet by Friday

$ plaud export --dir ~/notes/meetings --format md
  wrote ~/notes/meetings/2026-06-12-acme-renewal-call.md
```

---

## Requirements

- **macOS only** (Apple Silicon or Intel). Prebuilt binaries and the install
  script support macOS exclusively.
- A **Plaud account** with recordings (you sign in through your browser).
- For the from-source path only: a **Rust toolchain** (≥ 1.82).

> **Other platforms:** the code is portable Rust and will likely build and run
> on Linux from source (`cargo install --git …`), but Linux is **untested and
> unsupported**. Windows is not supported.

## Install

### Quick install (recommended)

```sh
curl -fsSL https://raw.githubusercontent.com/leegonzales/plaud-cli/main/install.sh | sh
```

This downloads the prebuilt binary for your Mac from the
[latest release](https://github.com/leegonzales/plaud-cli/releases/latest),
installs it to `~/.local/bin`, and prints PATH guidance. Override the location
with `PLAUD_INSTALL_DIR=/usr/local/bin`. If no prebuilt binary matches, it falls
back to building from source with `cargo`.

### With cargo

```sh
cargo install --git https://github.com/leegonzales/plaud-cli
```

### With cargo-binstall

```sh
cargo binstall --git https://github.com/leegonzales/plaud-cli plaud
```

### From source

```sh
git clone https://github.com/leegonzales/plaud-cli
cd plaud-cli
cargo install --path .          # -> ~/.cargo/bin/plaud
# or: cargo build --release     # -> target/release/plaud
```

### PATH

The quick installer puts `plaud` in `~/.local/bin`; `cargo install` uses
`~/.cargo/bin`. Make sure the relevant directory is on your `PATH`:

```sh
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.zshrc && exec zsh
```

## Quick start

```sh
plaud login          # opens your browser to sign in (one time)
plaud list           # see your recent recordings
plaud sync           # pull everything into a local store
plaud search "topic" # full-text search across transcripts and notes
```

## Usage

### Reading recordings

```sh
plaud whoami                      # current account
plaud list                        # recent recordings (table)
plaud list -q standup --from 2026-06-01 --to 2026-06-10
plaud list --page 2 --page-size 50          # page-size >= 10
plaud get <id>                    # recording summary
plaud note <id>                   # AI summary / action items / minutes
plaud transcript <id>             # transcript: [mm:ss] Speaker: text
plaud download <id> -o talk.mp3   # audio via a 24h presigned URL
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
ready for a Markdown notes app such as Obsidian. Exporting an id that isn't
synced fetches it live and caches it.

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
payload. Commands exit non-zero on failure, so they compose safely in scripts.

## Configuration

| What | Where | Notes |
|------|-------|-------|
| Tokens | `~/.plaud/cli-tokens.json` | Mode `0600`; written atomically. Removed by `plaud logout`. |
| Sync store | `~/.plaud/store/` | Override with `PLAUD_STORE`. One JSON per recording + a sync cursor. |
| Install dir | `~/.local/bin` | Override the installer with `PLAUD_INSTALL_DIR`. |

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

`transcript` and `note` decode Plaud's nested `data_content` blocks — the
verbatim transcript and the AI summary / meeting minutes — and fall back to raw
JSON for any block shape they don't recognize.

## Troubleshooting

- **`session expired — run plaud login`** — your refresh token is no longer
  valid; run `plaud login` again.
- **`plaud: command not found`** — the install dir isn't on your `PATH`. See
  [PATH](#path).
- **`download` says no audio** — Plaud doesn't keep a `presigned_url` for every
  recording (short clips especially). That's expected, not a bug.
- **Empty transcript / notes** — not every recording is transcribed on Plaud's
  side; older clips may have none. `transcript`/`note` report this clearly.
- **`page_size: Input should be greater than or equal to 10`** — Plaud requires
  `--page-size` ≥ 10.
- **`search`/`export` say the store is empty** — run `plaud sync` first; they
  read the local store.

## Agent skill

[`skill/use-plaud/SKILL.md`](skill/use-plaud/SKILL.md) is a ready-made
[Claude Code](https://claude.com/claude-code) skill that teaches an AI agent how
to drive this CLI — the commands, the stable `--json`/`--ndjson` contract, the
sync→search→export flow, and common recipes. Drop it into your agent's skills
directory (e.g. `~/.claude/skills/use-plaud/`) to let an assistant work your
recordings for you. It's generic — no personal data, no assumptions about your
setup.

## Development

```sh
cargo test                                  # unit tests
cargo fmt --all -- --check                  # formatting
cargo clippy --all-targets -- -D warnings   # lints (zero-warning bar)
cargo build --release
```

CI runs all of the above on every push and PR; tagged releases build macOS
binaries automatically. The codebase is small and organized one concern per
module:

- `oauth.rs` — discovery, DCR, PKCE, loopback redirect, token + refresh
- `mcp.rs` — Streamable-HTTP MCP client (initialize, session id, `tools/call`)
- `model.rs` — normalize Plaud's nested payloads into one stable `Record`
- `store.rs` — local sync store (records + cursor)
- `commands.rs` — one handler per subcommand
- `output.rs` — human-readable rendering (table, transcript, notes)
- `config.rs` — token storage

Contributions welcome — please keep `cargo fmt`/`clippy`/`test` green.

## Disclaimer

This is an **unofficial** project, not affiliated with, endorsed by, or
supported by Plaud. It talks to the same MCP endpoint Plaud documents for AI
assistants (`https://mcp.plaud.ai/mcp`), using your own account via the
standard OAuth flow — it stores no credentials beyond the tokens that flow
returns, kept locally at `~/.plaud/cli-tokens.json` (mode `0600`). Use it with
your own recordings and at your own risk. Endpoint or payload changes on
Plaud's side may break it.

## License

[MIT](LICENSE) © Lee Gonzales
