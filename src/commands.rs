//! Command handlers — one per CLI subcommand.

use std::fs;
use std::path::PathBuf;

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Map, Value};

use crate::cli::{
    Cli, Command, ExportArgs, ExportFormat, ListArgs, OutputMode, SearchArgs, SyncArgs,
};
use crate::config;
use crate::mcp::McpClient;
use crate::model::{self, Record};
use crate::oauth;
use crate::output;
use crate::store;

const USER_AGENT: &str = concat!("plaud-cli/", env!("CARGO_PKG_VERSION"));
const SYNC_PAGE_SIZE: u32 = 100;
const SYNC_PAGE_CAP: u32 = 200;

pub async fn dispatch(cli: Cli) -> Result<()> {
    let http = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .context("building HTTP client")?;
    let mode = cli.output_mode();

    match cli.command {
        Command::Login => login(&http).await,
        Command::Logout => logout(),
        Command::Whoami => whoami(&http).await,
        Command::List(args) => list(&http, args, mode).await,
        Command::Get { id } => get(&http, &id, mode).await,
        Command::Note { id } => note(&http, &id, mode).await,
        Command::Transcript { id } => transcript(&http, &id, mode).await,
        Command::Download { id, out } => download(&http, &id, out).await,
        Command::Sync(args) => sync(&http, args).await,
        Command::Search(args) => search(args, mode),
        Command::Export(args) => export(&http, args).await,
        Command::Schema => {
            schema();
            Ok(())
        }
    }
}

/// Build an authenticated MCP client, refreshing the token if it's near expiry.
async fn client(http: &reqwest::Client) -> Result<McpClient> {
    let mut store = config::load()?.ok_or_else(|| anyhow!("not logged in — run `plaud login`"))?;
    if store.is_expired(60) && store.refresh_token.is_some() {
        // oauth::refresh already returns actionable errors (e.g. "session
        // expired — run `plaud login`"); don't bury them under more context.
        store = oauth::refresh(http, &store).await?;
        config::save(&store)?;
    }
    Ok(McpClient::new(http.clone(), store))
}

// ---- auth ----------------------------------------------------------------

async fn login(http: &reqwest::Client) -> Result<()> {
    let store = oauth::login(http).await?;
    config::save(&store)?;
    let mut mcp = McpClient::new(http.clone(), store);
    match mcp.call_tool("get_current_user", json!({})).await {
        Ok(user) => {
            let who = user
                .get("email")
                .or_else(|| user.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("(account)");
            println!("Logged in as {who}.");
        }
        Err(_) => println!("Logged in."),
    }
    Ok(())
}

fn logout() -> Result<()> {
    if config::clear()? {
        println!("Logged out — stored tokens removed.");
    } else {
        println!("Already logged out.");
    }
    Ok(())
}

async fn whoami(http: &reqwest::Client) -> Result<()> {
    let mut mcp = client(http).await?;
    let user = mcp.call_tool("get_current_user", json!({})).await?;
    output::print_pretty(&user)
}

// ---- live reads ----------------------------------------------------------

async fn list(http: &reqwest::Client, args: ListArgs, mode: OutputMode) -> Result<()> {
    let mut mcp = client(http).await?;
    let payload = mcp.call_tool("list_files", list_arguments(&args)).await?;
    match mode {
        OutputMode::Raw => output::print_pretty(&payload),
        OutputMode::Human => output::render_list(&payload),
        OutputMode::Json => {
            let items: Vec<Value> = data_items(&payload).iter().map(|m| summary(m)).collect();
            output::print_pretty(&Value::Array(items))
        }
        OutputMode::Ndjson => {
            for item in data_items(&payload) {
                print_line(&summary(item))?;
            }
            Ok(())
        }
    }
}

async fn get(http: &reqwest::Client, id: &str, mode: OutputMode) -> Result<()> {
    let mut mcp = client(http).await?;
    let payload = mcp.call_tool("get_file", json!({ "file_id": id })).await?;
    match mode {
        OutputMode::Raw => output::print_pretty(&payload),
        OutputMode::Human => output::render_file(&payload),
        OutputMode::Json | OutputMode::Ndjson => output::print_pretty(&file_summary(&payload)),
    }
}

async fn note(http: &reqwest::Client, id: &str, mode: OutputMode) -> Result<()> {
    let mut mcp = client(http).await?;
    let payload = mcp.call_tool("get_note", json!({ "file_id": id })).await?;
    match mode {
        OutputMode::Raw => output::print_pretty(&payload),
        OutputMode::Human => output::render_note(&payload),
        OutputMode::Json => {
            let notes = model::notes_from_payload(&payload);
            let actions = model::extract_action_items(&notes);
            output::print_pretty(&json!({
                "id": id,
                "action_items": actions,
                "notes": notes,
            }))
        }
        OutputMode::Ndjson => {
            for n in model::notes_from_payload(&payload) {
                print_line(&serde_json::to_value(n)?)?;
            }
            Ok(())
        }
    }
}

async fn transcript(http: &reqwest::Client, id: &str, mode: OutputMode) -> Result<()> {
    let mut mcp = client(http).await?;
    let payload = mcp
        .call_tool("get_transcript", json!({ "file_id": id }))
        .await?;
    match mode {
        OutputMode::Raw => output::print_pretty(&payload),
        OutputMode::Human => output::render_transcript(&payload),
        OutputMode::Json => {
            let segments = model::segments_from_transcript(&payload);
            output::print_pretty(&json!({ "id": id, "segments": segments }))
        }
        OutputMode::Ndjson => {
            for seg in model::segments_from_transcript(&payload) {
                print_line(&serde_json::to_value(seg)?)?;
            }
            Ok(())
        }
    }
}

async fn download(http: &reqwest::Client, id: &str, out: Option<PathBuf>) -> Result<()> {
    let mut mcp = client(http).await?;
    let file = mcp.call_tool("get_file", json!({ "file_id": id })).await?;
    let url = file
        .get("presigned_url")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow!("no audio available for {id} (Plaud has no presigned_url)"))?;

    let path = out.unwrap_or_else(|| PathBuf::from(format!("{id}.mp3")));
    let resp = http
        .get(url)
        .send()
        .await
        .context("downloading audio")?
        .error_for_status()
        .context("audio download rejected")?;
    let bytes = resp.bytes().await.context("reading audio bytes")?;
    fs::write(&path, &bytes).with_context(|| format!("writing {}", path.display()))?;
    println!("Saved {} ({} bytes).", path.display(), bytes.len());
    Ok(())
}

// ---- sync ----------------------------------------------------------------

async fn sync(http: &reqwest::Client, args: SyncArgs) -> Result<()> {
    let mut mcp = client(http).await?;
    let cursor = store::load_cursor()?;
    let threshold: Option<String> = if args.since_last {
        cursor.last_created_at.clone()
    } else {
        args.since.clone()
    };

    // Enumerate candidate recordings across all pages.
    let mut candidates: Vec<Value> = Vec::new();
    let mut page = 1u32;
    loop {
        let payload = mcp
            .call_tool(
                "list_files",
                json!({ "page": page, "page_size": SYNC_PAGE_SIZE }),
            )
            .await?;
        let items = data_items(&payload);
        let count = items.len();
        for item in &items {
            let created = item
                .get("created_at")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if let Some(th) = &threshold {
                if created <= th.as_str() {
                    continue;
                }
            }
            candidates.push((*item).clone());
        }
        if count < SYNC_PAGE_SIZE as usize || page >= SYNC_PAGE_CAP {
            break;
        }
        page += 1;
    }

    let mut max_created = cursor.last_created_at.clone();
    for item in &candidates {
        let created = item
            .get("created_at")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if max_created.as_deref().is_none_or(|m| created > m) {
            max_created = Some(created.to_string());
        }
    }

    let mut synced = 0usize;
    let mut skipped = 0usize;
    let mut failed: Vec<String> = Vec::new();
    for item in &candidates {
        if args.limit.is_some_and(|lim| synced >= lim) {
            break;
        }
        let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
        if id.is_empty() {
            continue;
        }
        if !args.force && store::has_record(id)? {
            skipped += 1;
            continue;
        }
        // One bad recording must not abort the whole batch.
        match fetch_and_store(&mut mcp, id).await {
            Ok(record) => {
                synced += 1;
                println!("  + {} {}", record.date(), record.name);
            }
            Err(e) => {
                let name = item.get("name").and_then(|v| v.as_str()).unwrap_or(id);
                eprintln!("  ! skipped {id} ({name}): {e:#}");
                failed.push(id.to_string());
            }
        }
    }

    let total = store::load_all()?.len();
    // Only advance the cursor on a clean run. Synced records are saved
    // immediately and `has_record` skips them, so re-running after a failure is
    // cheap and idempotent — but the cursor must not leapfrog a failed record.
    let cursor_created = if failed.is_empty() {
        max_created
    } else {
        cursor.last_created_at.clone()
    };
    store::save_cursor(&store::Cursor {
        last_created_at: cursor_created,
        last_synced_at: Some(config::now_iso()),
        record_count: total,
    })?;

    let dir = store::store_dir()?;
    println!(
        "Synced {synced} new, {skipped} already present{}. Store: {total} records at {}.",
        if failed.is_empty() {
            String::new()
        } else {
            format!(", {} failed (re-run to retry)", failed.len())
        },
        dir.display()
    );
    if !failed.is_empty() {
        // Non-zero exit so scripts/cron can detect a partial sync.
        bail!("{} recording(s) failed to sync", failed.len());
    }
    Ok(())
}

/// Fetch one recording's full payload and persist it to the store.
async fn fetch_and_store(mcp: &mut McpClient, id: &str) -> Result<Record> {
    let file = mcp.call_tool("get_file", json!({ "file_id": id })).await?;
    let record = model::build_record_from_file(&file, config::now_iso());
    store::save_record(&record)?;
    Ok(record)
}

// ---- search --------------------------------------------------------------

fn search(args: SearchArgs, mode: OutputMode) -> Result<()> {
    let records = store::load_all()?;
    if records.is_empty() {
        bail!("local store is empty — run `plaud sync` first");
    }
    let needle = args.query.to_lowercase();
    let mut hits: Vec<Value> = Vec::new();

    for record in &records {
        let mut matches: Vec<Value> = Vec::new();

        if record.name.to_lowercase().contains(&needle) {
            matches.push(json!({ "kind": "title", "text": record.name }));
        }
        for item in &record.action_items {
            if item.to_lowercase().contains(&needle) {
                matches.push(json!({ "kind": "action", "text": item }));
            }
        }
        for n in &record.notes {
            for line in n.markdown.lines() {
                if line.to_lowercase().contains(&needle) {
                    matches.push(json!({ "kind": "note", "text": line.trim() }));
                }
            }
        }
        for (i, seg) in record.transcript.iter().enumerate() {
            if seg.text.to_lowercase().contains(&needle) {
                let lo = i.saturating_sub(args.context);
                let hi = (i + args.context + 1).min(record.transcript.len());
                let context: Vec<Value> = record.transcript[lo..hi]
                    .iter()
                    .map(|s| {
                        json!({
                            "timestamp_ms": s.start_ms,
                            "speaker": s.speaker,
                            "text": s.text,
                        })
                    })
                    .collect();
                matches.push(json!({ "kind": "transcript", "segments": context }));
            }
        }

        if !matches.is_empty() {
            hits.push(json!({
                "id": record.id,
                "name": record.name,
                "date": record.date(),
                "matches": matches,
            }));
        }
    }

    match mode {
        OutputMode::Json => output::print_pretty(&Value::Array(hits)),
        OutputMode::Ndjson => {
            for hit in &hits {
                print_line(hit)?;
            }
            Ok(())
        }
        _ => render_search_human(&hits),
    }
}

fn render_search_human(hits: &[Value]) -> Result<()> {
    if hits.is_empty() {
        println!("No matches.");
        return Ok(());
    }
    for hit in hits {
        let date = hit.get("date").and_then(|v| v.as_str()).unwrap_or("");
        let name = hit.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let id = hit.get("id").and_then(|v| v.as_str()).unwrap_or("");
        println!("\n=== {date}  {name}  ({id}) ===");
        for m in hit
            .get("matches")
            .and_then(|v| v.as_array())
            .into_iter()
            .flatten()
        {
            let kind = m.get("kind").and_then(|v| v.as_str()).unwrap_or("");
            match kind {
                "transcript" => {
                    for seg in m
                        .get("segments")
                        .and_then(|v| v.as_array())
                        .into_iter()
                        .flatten()
                    {
                        let ts = seg
                            .get("timestamp_ms")
                            .and_then(|v| v.as_u64())
                            .map(model::fmt_mmss)
                            .unwrap_or_default();
                        let sp = seg.get("speaker").and_then(|v| v.as_str()).unwrap_or("");
                        let text = seg.get("text").and_then(|v| v.as_str()).unwrap_or("");
                        let prefix = if sp.is_empty() {
                            String::new()
                        } else {
                            format!("{sp}: ")
                        };
                        println!("  [{ts}] {prefix}{text}");
                    }
                }
                other => {
                    let text = m.get("text").and_then(|v| v.as_str()).unwrap_or("");
                    println!("  ({other}) {text}");
                }
            }
        }
    }
    Ok(())
}

// ---- export --------------------------------------------------------------

async fn export(http: &reqwest::Client, args: ExportArgs) -> Result<()> {
    fs::create_dir_all(&args.dir).with_context(|| format!("creating {}", args.dir.display()))?;

    let records = if args.ids.is_empty() {
        store::load_all()?
    } else {
        let mut out = Vec::new();
        let mut mcp: Option<McpClient> = None;
        for id in &args.ids {
            if let Some(rec) = store::load_record(id)? {
                out.push(rec);
                continue;
            }
            // Not synced yet — fetch live and cache it.
            if mcp.is_none() {
                mcp = Some(client(http).await?);
            }
            let file = mcp
                .as_mut()
                .unwrap()
                .call_tool("get_file", json!({ "file_id": id }))
                .await?;
            let rec = model::build_record_from_file(&file, config::now_iso());
            store::save_record(&rec)?;
            out.push(rec);
        }
        out
    };

    if records.is_empty() {
        bail!("nothing to export — run `plaud sync` or pass recording ids");
    }

    let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
    for record in &records {
        let (ext, content) = match args.format {
            ExportFormat::Md => ("md", record.to_markdown()),
            ExportFormat::Json => ("json", serde_json::to_string_pretty(record)?),
        };
        // Disambiguate same-date/same-title collisions so no record is lost.
        let mut stem = record.file_stem();
        if !used.insert(stem.clone()) {
            let suffix: String = record.id.chars().take(8).collect();
            stem = format!("{stem}-{suffix}");
            used.insert(stem.clone());
        }
        let path = args.dir.join(format!("{stem}.{ext}"));
        fs::write(&path, content).with_context(|| format!("writing {}", path.display()))?;
        println!("  wrote {}", path.display());
    }
    println!(
        "Exported {} record(s) to {}.",
        records.len(),
        args.dir.display()
    );
    Ok(())
}

// ---- schema --------------------------------------------------------------

fn schema() {
    println!(
        r#"plaud --json schema (stable, snake_case)

list:        [{{ id, name, created_at, start_at, duration_ms, serial_number }}]
get:         {{ id, name, created_at, start_at, duration_ms, serial_number,
              transcript_blocks, note_blocks, audio_url }}
transcript:  {{ id, segments: [{{ start_ms?, speaker?, text }}] }}
note:        {{ id, action_items: [string], notes: [{{ tab, title, type, markdown }}] }}

store record (sync / export --format json):
  {{ id, name, created_at, start_at, duration_ms, serial_number,
     transcript: [{{ start_ms?, speaker?, text }}],
     notes: [{{ tab, title, type, markdown }}],
     action_items: [string],
     synced_at }}

--ndjson emits one element per line (per recording, segment, or note).
--raw emits the unprocessed Plaud tool payload."#
    );
}

// ---- helpers -------------------------------------------------------------

fn list_arguments(args: &ListArgs) -> Value {
    let mut m = Map::new();
    if let Some(q) = &args.query {
        m.insert("query".into(), json!(q));
    }
    if let Some(f) = &args.from {
        m.insert("date_from".into(), json!(f));
    }
    if let Some(t) = &args.to {
        m.insert("date_to".into(), json!(t));
    }
    if let Some(p) = args.page {
        m.insert("page".into(), json!(p));
    }
    if let Some(ps) = args.page_size {
        m.insert("page_size".into(), json!(ps));
    }
    Value::Object(m)
}

/// The recording objects inside a `list_files` payload (`data` array).
fn data_items(payload: &Value) -> Vec<&Value> {
    payload
        .get("data")
        .and_then(|v| v.as_array())
        .or_else(|| payload.as_array())
        .map(|a| a.iter().collect())
        .unwrap_or_default()
}

/// Normalize a list item into the stable summary shape.
fn summary(meta: &Value) -> Value {
    json!({
        "id": meta.get("id").and_then(|v| v.as_str()).unwrap_or_default(),
        "name": meta.get("name").and_then(|v| v.as_str()).unwrap_or_default(),
        "created_at": meta.get("created_at").and_then(|v| v.as_str()).unwrap_or_default(),
        "start_at": meta.get("start_at").and_then(|v| v.as_str()).unwrap_or_default(),
        "duration_ms": meta.get("duration").and_then(|v| v.as_u64()).unwrap_or(0),
        "serial_number": meta.get("serial_number").and_then(|v| v.as_str()).unwrap_or_default(),
    })
}

/// Normalize a `get_file` payload into a summary with block counts + audio URL.
fn file_summary(payload: &Value) -> Value {
    let n_src = payload
        .get("source_list")
        .and_then(|v| v.as_array())
        .map_or(0, |a| a.len());
    let n_note = payload
        .get("note_list")
        .and_then(|v| v.as_array())
        .map_or(0, |a| a.len());
    json!({
        "id": payload.get("id").and_then(|v| v.as_str()).unwrap_or_default(),
        "name": payload.get("name").and_then(|v| v.as_str()).unwrap_or_default(),
        "created_at": payload.get("created_at").and_then(|v| v.as_str()).unwrap_or_default(),
        "start_at": payload.get("start_at").and_then(|v| v.as_str()).unwrap_or_default(),
        "duration_ms": payload.get("duration").and_then(|v| v.as_u64()).unwrap_or(0),
        "serial_number": payload.get("serial_number").and_then(|v| v.as_str()).unwrap_or_default(),
        "transcript_blocks": n_src,
        "note_blocks": n_note,
        "audio_url": payload.get("presigned_url").cloned().unwrap_or(Value::Null),
    })
}

fn print_line(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string(value)?);
    Ok(())
}
