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
    println!("{c_id:<26}  {c_dur:>9}  {c_created:<19}  {c_name}");
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
        println!("{id:<26}  {duration:>9}  {created:<19}  {name}");
    }
    Ok(())
}

/// Render a transcript as `[mm:ss] Speaker: text` lines, falling back to
/// pretty JSON when the segment shape isn't recognized.
pub fn render_transcript(value: &Value) -> Result<()> {
    let segments = value.as_array().or_else(|| {
        ["source_list", "segments", "transcript", "data"]
            .iter()
            .find_map(|k| value.get(*k).and_then(|v| v.as_array()))
    });
    let Some(segments) = segments else {
        return print_pretty(value);
    };
    if segments.is_empty() {
        println!("(empty transcript)");
        return Ok(());
    }

    let mut rendered = false;
    for seg in segments {
        let text = field_str(seg, &["text", "content", "transcript"]);
        let Some(text) = text else { continue };
        let speaker = field_str(seg, &["speaker", "speaker_name", "role"]);
        let start_ms = seg
            .get("start")
            .or_else(|| seg.get("start_time"))
            .or_else(|| seg.get("begin"))
            .and_then(|v| v.as_u64());

        let mut line = String::new();
        if let Some(ms) = start_ms {
            line.push_str(&format!("[{}] ", fmt_timestamp_ms(ms)));
        }
        if let Some(sp) = speaker {
            line.push_str(&format!("{sp}: "));
        }
        line.push_str(&text);
        println!("{line}");
        rendered = true;
    }

    if !rendered {
        return print_pretty(value);
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
