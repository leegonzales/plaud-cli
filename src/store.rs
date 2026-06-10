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
