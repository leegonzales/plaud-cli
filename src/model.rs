//! Normalized data model.
//!
//! Plaud's tool payloads are messy and nested (double-encoded `data_content`
//! blocks). This module flattens them into one stable, snake_case `Record`
//! that the store, search, and export paths all share. The field names here
//! are the documented contract (`plaud schema`) — treat them as stable.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A fully normalized recording: metadata + transcript + notes + action items.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Record {
    pub id: String,
    pub name: String,
    /// Upload time, ISO 8601 (Plaud `created_at`).
    pub created_at: String,
    /// Recording start time, ISO 8601 (Plaud `start_at`).
    pub start_at: String,
    pub duration_ms: u64,
    pub serial_number: String,
    pub transcript: Vec<Segment>,
    pub notes: Vec<Note>,
    pub action_items: Vec<String>,
    /// When this CLI last synced the record, ISO 8601 UTC.
    pub synced_at: String,
}

/// One transcript segment.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segment {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub start_ms: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speaker: Option<String>,
    pub text: String,
}

/// One AI note block (Summary, Meeting Minutes, ...).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Note {
    pub tab: String,
    pub title: String,
    #[serde(rename = "type")]
    pub note_type: String,
    pub markdown: String,
}

impl Record {
    /// Date portion (`YYYY-MM-DD`), preferring `start_at`.
    pub fn date(&self) -> &str {
        let pick = if self.start_at.len() >= 10 {
            &self.start_at
        } else {
            &self.created_at
        };
        pick.get(0..10).unwrap_or(pick)
    }

    /// `YYYY-MM-DD-<slug>` — the export filename stem.
    pub fn file_stem(&self) -> String {
        format!("{}-{}", self.date(), slug(&self.name))
    }

    /// Render as a single Markdown document with YAML frontmatter, suitable for
    /// Obsidian / SecondBrain ingestion.
    pub fn to_markdown(&self) -> String {
        let mut out = String::new();
        out.push_str("---\n");
        out.push_str(&format!("id: \"{}\"\n", self.id));
        out.push_str(&format!("title: {}\n", yaml_scalar(&self.name)));
        out.push_str(&format!("date: \"{}\"\n", self.date()));
        out.push_str(&format!("start_at: \"{}\"\n", self.start_at));
        out.push_str(&format!("created_at: \"{}\"\n", self.created_at));
        out.push_str(&format!("duration_ms: {}\n", self.duration_ms));
        out.push_str(&format!("serial_number: \"{}\"\n", self.serial_number));
        out.push_str("source: plaud\n");
        out.push_str("tags: [meeting, transcript]\n");
        out.push_str("---\n\n");

        out.push_str(&format!("# {}\n\n", self.name));

        if !self.action_items.is_empty() {
            out.push_str("## Action Items\n\n");
            for item in &self.action_items {
                out.push_str(&format!("- [ ] {item}\n"));
            }
            out.push('\n');
        }

        for note in &self.notes {
            let header = if note.tab.is_empty() {
                &note.title
            } else {
                &note.tab
            };
            let header = if header.is_empty() { "Notes" } else { header };
            out.push_str(&format!("## {header}\n\n{}\n\n", note.markdown.trim()));
        }

        out.push_str("## Transcript\n\n");
        for seg in &self.transcript {
            if let Some(ms) = seg.start_ms {
                out.push_str(&format!("[{}] ", fmt_mmss(ms)));
            }
            if let Some(sp) = &seg.speaker {
                out.push_str(&format!("{sp}: "));
            }
            out.push_str(&seg.text);
            out.push('\n');
        }
        out
    }
}

/// Quote a YAML scalar if it contains characters that would break a bare value.
fn yaml_scalar(s: &str) -> String {
    if s.is_empty()
        || s.contains([':', '"', '#', '\n', '\''])
        || s.starts_with(['[', '{', '-', '@'])
    {
        format!("\"{}\"", s.replace('"', "\\\""))
    } else {
        s.to_string()
    }
}

/// `mm:ss` / `h:mm:ss` from milliseconds.
pub fn fmt_mmss(ms: u64) -> String {
    let total = ms / 1000;
    let (h, m, s) = (total / 3600, (total % 3600) / 60, total % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m:02}:{s:02}")
    }
}

/// A Plaud `data_content` field is itself a JSON-encoded string; decode it.
pub fn parse_data_content(block: &Value) -> Option<Value> {
    let raw = block.get("data_content")?.as_str()?;
    serde_json::from_str(raw).ok()
}

fn block_of_type<'a>(blocks: &'a [Value], data_type: &str) -> Option<&'a Value> {
    blocks
        .iter()
        .find(|b| b.get("data_type").and_then(|v| v.as_str()) == Some(data_type))
}

fn nonempty_str(value: &Value, key: &str) -> Option<String> {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// Extract transcript segments from a `get_transcript` payload (the verbatim
/// transcript lives in the `transaction` block).
pub fn segments_from_transcript(payload: &Value) -> Vec<Segment> {
    let Some(blocks) = payload.as_array() else {
        return Vec::new();
    };
    let Some(Value::Array(segs)) =
        block_of_type(blocks, "transaction").and_then(parse_data_content)
    else {
        return Vec::new();
    };
    segs.iter()
        .filter_map(|seg| {
            let text = seg.get("content").and_then(|v| v.as_str())?.to_string();
            Some(Segment {
                start_ms: seg.get("start_time").and_then(|v| v.as_u64()),
                speaker: nonempty_str(seg, "speaker")
                    .or_else(|| nonempty_str(seg, "original_speaker")),
                text,
            })
        })
        .collect()
}

/// Extract note blocks from a `get_note` payload.
pub fn notes_from_payload(payload: &Value) -> Vec<Note> {
    let Some(blocks) = payload.as_array() else {
        return Vec::new();
    };
    blocks
        .iter()
        .filter_map(|block| {
            let markdown = parse_data_content(block)
                .and_then(|c| {
                    c.get("ai_content")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })
                .or_else(|| {
                    block
                        .get("data_content")
                        .and_then(|v| v.as_str())
                        .map(str::to_string)
                })?;
            Some(Note {
                tab: nonempty_str(block, "data_tab_name").unwrap_or_default(),
                title: nonempty_str(block, "data_title").unwrap_or_default(),
                note_type: nonempty_str(block, "data_type").unwrap_or_default(),
                markdown,
            })
        })
        .collect()
}

/// Pull action items out of note markdown. Heuristic: find an "Action Items"
/// heading and collect the bullet list beneath it, stopping at the next
/// heading.
pub fn extract_action_items(notes: &[Note]) -> Vec<String> {
    let mut items = Vec::new();
    for note in notes {
        let mut in_section = false;
        for raw in note.markdown.lines() {
            let line = raw.trim();
            let heading = line.trim_start_matches('#').trim();
            let is_heading = line.starts_with('#') || (!line.is_empty() && !is_bullet(line));

            if is_action_heading(heading) || is_action_heading(line) {
                in_section = true;
                continue;
            }
            if !in_section {
                continue;
            }
            if is_bullet(line) {
                let text = strip_bullet(line);
                if !text.is_empty() && !items.contains(&text) {
                    items.push(text);
                }
            } else if is_heading && !line.is_empty() {
                in_section = false; // next section
            }
        }
    }
    items
}

fn is_action_heading(s: &str) -> bool {
    let s = s.trim().to_ascii_lowercase();
    s == "action items" || s == "action item" || s == "action items:" || s == "next steps"
}

fn is_bullet(line: &str) -> bool {
    line.starts_with("- ")
        || line.starts_with("* ")
        || line.starts_with("• ")
        || line.starts_with("- [")
}

fn strip_bullet(line: &str) -> String {
    let t = line.trim_start_matches(['-', '*', '•', ' ']).trim_start();
    // strip leading "[ ] " / "[x] " checkbox marker
    let t = t
        .strip_prefix("[ ] ")
        .or_else(|| t.strip_prefix("[x] "))
        .unwrap_or(t);
    t.trim().to_string()
}

fn str_field(v: &Value, key: &str) -> String {
    v.get(key)
        .and_then(|x| x.as_str())
        .unwrap_or_default()
        .to_string()
}

/// Build a normalized record from a `get_file` payload, which already carries
/// both the transcript (`source_list`) and the notes (`note_list`).
pub fn build_record_from_file(file: &Value, synced_at: String) -> Record {
    let transcript = file
        .get("source_list")
        .map(segments_from_transcript)
        .unwrap_or_default();
    let notes = file
        .get("note_list")
        .map(notes_from_payload)
        .unwrap_or_default();
    let action_items = extract_action_items(&notes);
    Record {
        id: str_field(file, "id"),
        name: str_field(file, "name"),
        created_at: str_field(file, "created_at"),
        start_at: str_field(file, "start_at"),
        duration_ms: file.get("duration").and_then(|v| v.as_u64()).unwrap_or(0),
        serial_number: str_field(file, "serial_number"),
        transcript,
        notes,
        action_items,
        synced_at,
    }
}

/// A filesystem-safe, lowercase slug of a recording name (max ~60 chars).
pub fn slug(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    let mut last_dash = false;
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            last_dash = false;
        } else if !last_dash {
            out.push('-');
            last_dash = true;
        }
    }
    let slug = out.trim_matches('-').to_string();
    let slug: String = slug.chars().take(60).collect();
    let slug = slug.trim_end_matches('-').to_string();
    if slug.is_empty() {
        "untitled".to_string()
    } else {
        slug
    }
}
