//! Rendering helpers. `--json` always emits the raw tool payload (jq-friendly);
//! otherwise we pretty-print, with a compact table for recording listings.

use anyhow::Result;
use serde_json::Value;

/// Pretty-print any JSON value to stdout.
pub fn print_pretty(value: &Value) -> Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}

/// Render a `list_files` payload as a table, or fall back to pretty JSON.
pub fn render_list(value: &Value) -> Result<()> {
    let Some(items) = find_array(value) else {
        return print_pretty(value);
    };
    if items.is_empty() {
        println!("No recordings found.");
        return Ok(());
    }

    let (c_id, c_dur, c_created, c_name) = ("ID", "DURATION", "CREATED", "NAME");
    println!("{c_id:<32}  {c_dur:>9}  {c_created:<19}  {c_name}");
    for item in items {
        let id = field_str(item, &["id", "file_id", "uuid"]).unwrap_or_else(|| "?".into());
        let name = field_str(item, &["name", "title", "filename"]).unwrap_or_default();
        let created =
            field_str(item, &["created_at", "start_at", "create_time"]).unwrap_or_default();
        let duration = item
            .get("duration")
            .and_then(|v| v.as_u64())
            .map(fmt_duration_ms)
            .unwrap_or_else(|| "-".into());
        println!("{id:<32}  {duration:>9}  {created:<19}  {name}");
    }
    Ok(())
}

/// A Plaud `data_content` field is itself a JSON-encoded string; decode it.
fn parse_data_content(block: &Value) -> Option<Value> {
    let raw = block.get("data_content")?.as_str()?;
    serde_json::from_str(raw).ok()
}

/// Find the first `data_*` block of a given `data_type`.
fn block_of_type<'a>(blocks: &'a [Value], data_type: &str) -> Option<&'a Value> {
    blocks
        .iter()
        .find(|b| b.get("data_type").and_then(|v| v.as_str()) == Some(data_type))
}

/// Render a transcript as `[mm:ss] Speaker: text` lines.
///
/// `get_transcript` returns an array of typed blocks; the verbatim transcript
/// lives in the `transaction` block, whose `data_content` is a JSON string of
/// segments (`content`, `speaker`, `start_time` ms). Falls back to pretty JSON
/// if that shape isn't present.
pub fn render_transcript(value: &Value) -> Result<()> {
    let Some(blocks) = value.as_array() else {
        return print_pretty(value);
    };
    let segments = block_of_type(blocks, "transaction").and_then(parse_data_content);
    let Some(Value::Array(segments)) = segments else {
        return print_pretty(value);
    };
    if segments.is_empty() {
        println!("(empty transcript)");
        return Ok(());
    }

    for seg in &segments {
        let Some(text) = seg.get("content").and_then(|v| v.as_str()) else {
            continue;
        };
        let mut line = String::new();
        if let Some(ms) = seg.get("start_time").and_then(|v| v.as_u64()) {
            line.push_str(&format!("[{}] ", fmt_timestamp_ms(ms)));
        }
        if let Some(sp) = field_str(seg, &["speaker", "original_speaker"]) {
            if !sp.is_empty() {
                line.push_str(&format!("{sp}: "));
            }
        }
        line.push_str(text);
        println!("{line}");
    }
    Ok(())
}

/// Render notes as Markdown sections. `get_note` returns an array of note
/// blocks; each block's `data_content` is a JSON string wrapping the Markdown
/// in `ai_content`. Falls back to pretty JSON if unrecognized.
pub fn render_note(value: &Value) -> Result<()> {
    let Some(blocks) = value.as_array() else {
        return print_pretty(value);
    };
    if blocks.is_empty() {
        println!("(no notes)");
        return Ok(());
    }

    let mut rendered = false;
    for (i, block) in blocks.iter().enumerate() {
        let header =
            field_str(block, &["data_tab_name", "data_title"]).unwrap_or_else(|| "Note".into());
        let body = parse_data_content(block).and_then(|c| {
            c.get("ai_content")
                .and_then(|v| v.as_str())
                .map(str::to_string)
        });
        let Some(body) = body else { continue };

        if i > 0 {
            println!();
        }
        println!("## {header}\n");
        println!("{body}");
        rendered = true;
    }

    if !rendered {
        return print_pretty(value);
    }
    Ok(())
}

/// Render a `get_file` payload as a compact summary.
pub fn render_file(value: &Value) -> Result<()> {
    if let Some(name) = field_str(value, &["name"]) {
        println!("{name}");
    }
    if let Some(id) = field_str(value, &["id"]) {
        println!("  id:        {id}");
    }
    if let Some(ms) = value.get("duration").and_then(|v| v.as_u64()) {
        println!("  duration:  {}", fmt_duration_ms(ms));
    }
    if let Some(v) = field_str(value, &["start_at"]) {
        println!("  recorded:  {v}");
    }
    if let Some(v) = field_str(value, &["created_at"]) {
        println!("  uploaded:  {v}");
    }
    if let Some(v) = field_str(value, &["serial_number"]) {
        println!("  device:    {v}");
    }
    let n_src = value
        .get("source_list")
        .and_then(|v| v.as_array())
        .map_or(0, |a| a.len());
    let n_note = value
        .get("note_list")
        .and_then(|v| v.as_array())
        .map_or(0, |a| a.len());
    println!("  transcript: {n_src} block(s)");
    println!("  notes:      {n_note} block(s)");
    match value.get("presigned_url").and_then(|v| v.as_str()) {
        Some(u) => println!("  audio:      {u}"),
        None => println!("  audio:      (not available)"),
    }
    Ok(())
}

/// Format milliseconds as `mm:ss` (or `h:mm:ss`).
fn fmt_timestamp_ms(ms: u64) -> String {
    let total = ms / 1000;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// Format milliseconds as `1h02m03s` / `4m12s` / `9s`.
pub fn fmt_duration_ms(ms: u64) -> String {
    let total = ms / 1000;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}h{m:02}m{s:02}s")
    } else if m > 0 {
        format!("{m}m{s:02}s")
    } else {
        format!("{s}s")
    }
}

/// Find the list of records inside a payload: a bare array, or a common
/// wrapper key (`files`, `data`, `items`, `list`, `results`).
fn find_array(value: &Value) -> Option<&Vec<Value>> {
    if let Some(arr) = value.as_array() {
        return Some(arr);
    }
    for key in ["files", "data", "items", "list", "results", "records"] {
        if let Some(arr) = value.get(key).and_then(|v| v.as_array()) {
            return Some(arr);
        }
    }
    None
}

fn field_str(item: &Value, keys: &[&str]) -> Option<String> {
    for key in keys {
        match item.get(*key) {
            Some(Value::String(s)) => return Some(s.clone()),
            Some(Value::Number(n)) => return Some(n.to_string()),
            _ => {}
        }
    }
    None
}
