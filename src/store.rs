//! Local sync store: normalized records on disk plus a sync cursor.
//!
//! Lives under `~/.plaud/store/` by default (override with `PLAUD_STORE`).
//! Each recording is one `<id>.json` file; `sync-state.json` tracks the
//! high-water mark so `sync --since-last` only pulls what's new.

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};

use crate::model::Record;

const CURSOR_FILE: &str = "sync-state.json";

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Cursor {
    /// Highest recording `created_at` seen so far (ISO 8601, lexicographically
    /// comparable).
    #[serde(default)]
    pub last_created_at: Option<String>,
    /// When the last sync ran (ISO 8601 UTC).
    #[serde(default)]
    pub last_synced_at: Option<String>,
    #[serde(default)]
    pub record_count: usize,
}

/// Resolve the store directory, creating it if needed.
pub fn store_dir() -> Result<PathBuf> {
    let dir = match std::env::var_os("PLAUD_STORE") {
        Some(p) => PathBuf::from(p),
        None => {
            let base = BaseDirs::new().context("cannot resolve home directory")?;
            base.home_dir().join(".plaud").join("store")
        }
    };
    fs::create_dir_all(&dir).with_context(|| format!("creating store dir {}", dir.display()))?;
    Ok(dir)
}

/// Write one record to `<id>.json`.
pub fn save_record(record: &Record) -> Result<()> {
    let path = store_dir()?.join(format!("{}.json", record.id));
    let json = serde_json::to_vec_pretty(record)?;
    fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

/// True if a record for `id` already exists in the store.
pub fn has_record(id: &str) -> Result<bool> {
    Ok(store_dir()?.join(format!("{id}.json")).exists())
}

/// Load a single record by id, or `None` if it isn't in the store.
pub fn load_record(id: &str) -> Result<Option<Record>> {
    let path = store_dir()?.join(format!("{id}.json"));
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok(Some(serde_json::from_slice(&bytes)?))
}

/// Load all records from the store, sorted newest-first by `start_at`.
pub fn load_all() -> Result<Vec<Record>> {
    let dir = store_dir()?;
    let mut records = Vec::new();
    for entry in fs::read_dir(&dir).with_context(|| format!("reading {}", dir.display()))? {
        let path = entry?.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some(CURSOR_FILE) {
            continue;
        }
        let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
        match serde_json::from_slice::<Record>(&bytes) {
            Ok(rec) => records.push(rec),
            Err(_) => continue, // skip anything that isn't a record
        }
    }
    records.sort_by(|a, b| b.start_at.cmp(&a.start_at));
    Ok(records)
}

pub fn load_cursor() -> Result<Cursor> {
    let path = store_dir()?.join(CURSOR_FILE);
    if !path.exists() {
        return Ok(Cursor::default());
    }
    let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    Ok(serde_json::from_slice(&bytes).unwrap_or_default())
}

pub fn save_cursor(cursor: &Cursor) -> Result<()> {
    let path = store_dir()?.join(CURSOR_FILE);
    let json = serde_json::to_vec_pretty(cursor)?;
    fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::Record;

    fn record(id: &str) -> Record {
        Record {
            id: id.to_string(),
            name: format!("Recording {id}"),
            created_at: "2026-06-25T10:00:00".into(),
            start_at: "2026-06-25T09:00:00".into(),
            duration_ms: 1000,
            serial_number: "SN".into(),
            transcript: Vec::new(),
            notes: Vec::new(),
            action_items: Vec::new(),
            synced_at: "2026-06-25T10:00:00Z".into(),
        }
    }

    /// One serial round-trip test: it mutates the process-global `PLAUD_STORE`,
    /// so it must be the only test that touches the store.
    #[test]
    fn store_round_trip() {
        let dir = std::env::temp_dir().join(format!("plaud-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        // SAFETY: edition 2021, single test owns this env var.
        std::env::set_var("PLAUD_STORE", &dir);

        // Empty store.
        assert!(load_all().unwrap().is_empty());
        assert!(!has_record("a").unwrap());
        assert!(load_record("a").unwrap().is_none());
        assert!(load_cursor().unwrap().last_created_at.is_none());

        // Save two records, newest-first ordering by start_at.
        let mut older = record("old");
        older.start_at = "2026-06-01T09:00:00".into();
        save_record(&record("new")).unwrap();
        save_record(&older).unwrap();

        assert!(has_record("new").unwrap());
        assert_eq!(load_record("new").unwrap().unwrap().id, "new");
        let all = load_all().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].id, "new", "newest start_at sorts first");
        assert_eq!(all[1].id, "old");

        // Cursor round-trip.
        save_cursor(&Cursor {
            last_created_at: Some("2026-06-25T10:00:00".into()),
            last_synced_at: Some("2026-06-25T10:00:00Z".into()),
            record_count: 2,
        })
        .unwrap();
        let cur = load_cursor().unwrap();
        assert_eq!(cur.last_created_at.as_deref(), Some("2026-06-25T10:00:00"));
        assert_eq!(cur.record_count, 2);
        // Cursor file is not mistaken for a record.
        assert_eq!(load_all().unwrap().len(), 2);

        fs::remove_dir_all(&dir).unwrap();
    }
}
