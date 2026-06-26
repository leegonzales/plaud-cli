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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A `get_transcript`-shaped payload: array of typed blocks, the verbatim
    /// transcript in the `transaction` block whose `data_content` is a
    /// JSON-encoded string of segments.
    fn transcript_payload() -> Value {
        let segments = json!([
            { "content": "Hello there.", "speaker": "Lee", "start_time": 0 },
            { "content": "General Kenobi.", "speaker": "", "start_time": 3500 },
            { "content": "An hour in.", "original_speaker": "Obi", "start_time": 3_600_000 },
        ]);
        json!([
            { "data_type": "transaction", "data_content": segments.to_string() },
            { "data_type": "outline", "data_content": "[]" },
        ])
    }

    /// A `get_note`-shaped payload: blocks whose `data_content` wraps markdown
    /// in `ai_content`.
    fn note_payload() -> Value {
        let summary = json!({ "ai_content": "## Action Items\n- Ship it\n- [ ] Review PR\n\n## Other\n- not an action" });
        let minutes = json!({ "ai_content": "Action Items\n* Call client\n- Ship it\n" });
        json!([
            { "data_type": "auto_sum_note", "data_tab_name": "Summary", "data_title": "Summary", "data_content": summary.to_string() },
            { "data_type": "sum_multi_note", "data_tab_name": "Meeting Minutes", "data_title": "Mtg", "data_content": minutes.to_string() },
        ])
    }

    fn file_payload() -> Value {
        json!({
            "id": "abc123",
            "name": "Client Call: Acme",
            "created_at": "2026-04-07T19:40:47",
            "start_at": "2026-04-07T14:33:16",
            "duration_ms": 1000,
            "duration": 1000,
            "serial_number": "SN-1",
            "source_list": transcript_payload(),
            "note_list": note_payload(),
        })
    }

    #[test]
    fn parse_data_content_decodes_inner_json() {
        let block = json!({ "data_content": "[{\"a\":1}]" });
        assert_eq!(parse_data_content(&block), Some(json!([{ "a": 1 }])));
    }

    #[test]
    fn parse_data_content_rejects_non_string_and_garbage() {
        assert_eq!(parse_data_content(&json!({ "data_content": 42 })), None);
        assert_eq!(
            parse_data_content(&json!({ "data_content": "not json" })),
            None
        );
        assert_eq!(parse_data_content(&json!({})), None);
    }

    #[test]
    fn segments_extracts_transaction_block_with_timestamps_and_speakers() {
        let segs = segments_from_transcript(&transcript_payload());
        assert_eq!(segs.len(), 3);
        assert_eq!(segs[0].text, "Hello there.");
        assert_eq!(segs[0].start_ms, Some(0));
        assert_eq!(segs[0].speaker.as_deref(), Some("Lee"));
        // Empty speaker string is dropped, not kept as "".
        assert_eq!(segs[1].speaker, None);
        // Falls back to original_speaker.
        assert_eq!(segs[2].speaker.as_deref(), Some("Obi"));
    }

    #[test]
    fn segments_empty_for_unrecognized_shape() {
        assert!(segments_from_transcript(&json!({ "nope": true })).is_empty());
        assert!(segments_from_transcript(&json!([])).is_empty());
    }

    #[test]
    fn notes_unwrap_ai_content() {
        let notes = notes_from_payload(&note_payload());
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].tab, "Summary");
        assert!(notes[0].markdown.contains("Ship it"));
    }

    #[test]
    fn action_items_extracted_deduped_across_notes() {
        let notes = notes_from_payload(&note_payload());
        let items = extract_action_items(&notes);
        // "Ship it", "Review PR" from summary; "Call client" from minutes.
        // "Ship it" appears in both -> deduped. "not an action" excluded.
        assert!(items.contains(&"Ship it".to_string()));
        assert!(items.contains(&"Review PR".to_string()));
        assert!(items.contains(&"Call client".to_string()));
        assert!(!items.contains(&"not an action".to_string()));
        assert_eq!(items.iter().filter(|i| *i == "Ship it").count(), 1);
    }

    #[test]
    fn action_items_checkbox_marker_stripped() {
        let notes = vec![Note {
            tab: "x".into(),
            title: "x".into(),
            note_type: "x".into(),
            markdown: "## Action Items\n- [ ] Do the thing\n- [x] Done thing".into(),
        }];
        let items = extract_action_items(&notes);
        assert_eq!(items, vec!["Do the thing", "Done thing"]);
    }

    #[test]
    fn action_items_next_steps_heading_recognized() {
        let notes = vec![Note {
            tab: "x".into(),
            title: "x".into(),
            note_type: "x".into(),
            markdown: "Next Steps\n- Follow up".into(),
        }];
        assert_eq!(extract_action_items(&notes), vec!["Follow up"]);
    }

    #[test]
    fn slug_is_filesystem_safe() {
        assert_eq!(slug("Client Call: Acme!"), "client-call-acme");
        assert_eq!(slug("  spaces  "), "spaces");
        assert_eq!(slug("a/b\\c"), "a-b-c");
    }

    #[test]
    fn slug_empty_and_symbol_only_become_untitled() {
        assert_eq!(slug(""), "untitled");
        assert_eq!(slug("!!! ???"), "untitled");
    }

    #[test]
    fn slug_truncates_without_trailing_dash() {
        let long = "word ".repeat(40);
        let s = slug(&long);
        assert!(s.len() <= 60);
        assert!(!s.ends_with('-'));
    }

    #[test]
    fn yaml_scalar_quotes_when_needed() {
        assert_eq!(yaml_scalar("plain"), "plain");
        assert_eq!(yaml_scalar("has: colon"), "\"has: colon\"");
        assert_eq!(yaml_scalar("- leading dash"), "\"- leading dash\"");
        assert_eq!(yaml_scalar("say \"hi\""), "\"say \\\"hi\\\"\"");
        assert_eq!(yaml_scalar(""), "\"\"");
    }

    #[test]
    fn fmt_mmss_handles_hours() {
        assert_eq!(fmt_mmss(0), "00:00");
        assert_eq!(fmt_mmss(3500), "00:03");
        assert_eq!(fmt_mmss(65_000), "01:05");
        assert_eq!(fmt_mmss(3_661_000), "1:01:01");
    }

    #[test]
    fn record_date_prefers_start_at() {
        let mut r = build_record_from_file(&file_payload(), "now".into());
        assert_eq!(r.date(), "2026-04-07");
        assert_eq!(r.file_stem(), "2026-04-07-client-call-acme");
        // Falls back to created_at when start_at is missing.
        r.start_at = String::new();
        assert_eq!(r.date(), "2026-04-07");
    }

    #[test]
    fn build_record_from_file_is_fully_normalized() {
        let r = build_record_from_file(&file_payload(), "2026-06-25T00:00:00Z".into());
        assert_eq!(r.id, "abc123");
        assert_eq!(r.name, "Client Call: Acme");
        assert_eq!(r.duration_ms, 1000);
        assert_eq!(r.transcript.len(), 3);
        assert_eq!(r.notes.len(), 2);
        assert!(!r.action_items.is_empty());
        assert_eq!(r.synced_at, "2026-06-25T00:00:00Z");
    }

    #[test]
    fn to_markdown_has_frontmatter_actions_and_transcript() {
        let r = build_record_from_file(&file_payload(), "now".into());
        let md = r.to_markdown();
        assert!(md.starts_with("---\n"));
        assert!(md.contains("id: \"abc123\""));
        // Colon in the title forces a quoted YAML scalar.
        assert!(md.contains("title: \"Client Call: Acme\""));
        assert!(md.contains("source: plaud"));
        assert!(md.contains("## Action Items"));
        assert!(md.contains("- [ ] Ship it"));
        assert!(md.contains("## Transcript"));
        assert!(md.contains("[00:00] Lee: Hello there."));
        // Empty-speaker segment renders without a speaker prefix.
        assert!(md.contains("[00:03] General Kenobi."));
    }

    #[test]
    fn missing_fields_default_cleanly() {
        let r = build_record_from_file(&json!({ "id": "x" }), "now".into());
        assert_eq!(r.id, "x");
        assert_eq!(r.name, "");
        assert_eq!(r.duration_ms, 0);
        assert!(r.transcript.is_empty());
        assert_eq!(r.date(), ""); // no dates at all
        assert_eq!(slug(&r.name), "untitled");
    }
}
