# Changelog

All notable changes to `plaud` are documented here. Format loosely follows
[Keep a Changelog](https://keepachangelog.com/); this project uses semantic
versioning.

## [0.3.0] — 2026-06-25

First GA-quality release: tested, hardened, and CI-gated.

### Added
- Local **sync store** (`~/.plaud/store`, override with `PLAUD_STORE`):
  `plaud sync` with `--since-last` (cursor), `--since <date>`, `--limit`, `--force`.
- `plaud search <kw>` — full-text over synced transcripts, notes, action items,
  and titles, with `[mm:ss]` timestamps and `--context N` windows.
- `plaud export` — one file per recording: `--format md` (YAML frontmatter +
  action-item checklist + notes + transcript, Obsidian/SecondBrain-ready) or
  `json`. Live-fetches and caches ids that aren't synced yet.
- Output modes on read commands: `--json` (stable snake_case schema),
  `--ndjson` (one element per line), `--raw` (unprocessed Plaud payload).
- `plaud schema` documents the stable `--json` contract.
- `action_items` surfaced as a typed `string[]` on `note --json`.
- 26 unit tests and a GitHub Actions CI pipeline (fmt, clippy `-D warnings`,
  test, release build).

### Fixed
- **Sync could lose recordings.** `--limit`/`--since-last` no longer advances
  the cursor past unsynced candidates; the `created_at` boundary is now an
  inclusive overlap window de-duped by the store, so same-second uploads split
  across runs are never skipped; hitting the page cap warns and holds the cursor.
- **MCP protocol.** Responses are now matched by request id (interleaved
  notifications are ignored), and a 401 refresh re-establishes the session
  under the new token before replaying.
- **Token security.** Tokens are written atomically at `0o600` (temp + rename);
  `~/.plaud` is locked to `0o700`.
- **Export.** YAML frontmatter escapes backslashes/control chars; same-date,
  same-title recordings get an id suffix instead of silently overwriting.
- **Errors.** An expired session reports `session expired — run \`plaud login\``
  instead of leaking raw server JSON. Partial syncs exit non-zero.
- `--version` now reports the correct version.

## [0.2.0]

- Sync-store core: `sync`, `search`, `export`, structured `--json`/`--ndjson`.

## [0.1.0]

- Initial thin MCP client: `login`, `whoami`, `list`, `get`, `note`,
  `transcript`, `download` over `https://mcp.plaud.ai/mcp`.
