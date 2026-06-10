//! Local token storage.
//!
//! Tokens live under `~/.plaud/` to sit alongside the official client's
//! `tokens-mcp.json`, but in our own file (`cli-tokens.json`) so the two
//! never clobber each other's client registrations.

use std::fs;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

use anyhow::{Context, Result};
use directories::BaseDirs;
use serde::{Deserialize, Serialize};

const TOKEN_FILE: &str = "cli-tokens.json";

/// Persisted OAuth state: the dynamically-registered client plus the live
/// access/refresh tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenStore {
    pub client_id: String,
    pub access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub refresh_token: Option<String>,
    /// Unix seconds at which `access_token` expires (best-effort).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expires_at: Option<u64>,
}

impl TokenStore {
    /// True when the access token is within `skew` seconds of expiry.
    pub fn is_expired(&self, skew: u64) -> bool {
        match self.expires_at {
            Some(exp) => now() + skew >= exp,
            None => false,
        }
    }
}

/// `~/.plaud/` — created on demand.
fn config_dir() -> Result<PathBuf> {
    let base = BaseDirs::new().context("cannot resolve home directory")?;
    Ok(base.home_dir().join(".plaud"))
}

fn token_path() -> Result<PathBuf> {
    Ok(config_dir()?.join(TOKEN_FILE))
}

/// Load the token store, or `None` if the user has never logged in.
pub fn load() -> Result<Option<TokenStore>> {
    let path = token_path()?;
    if !path.exists() {
        return Ok(None);
    }
    let bytes = fs::read(&path).with_context(|| format!("reading {}", path.display()))?;
    let store =
        serde_json::from_slice(&bytes).with_context(|| format!("parsing {}", path.display()))?;
    Ok(Some(store))
}

/// Persist the token store with owner-only permissions.
pub fn save(store: &TokenStore) -> Result<()> {
    let dir = config_dir()?;
    fs::create_dir_all(&dir).with_context(|| format!("creating {}", dir.display()))?;
    let path = token_path()?;
    let json = serde_json::to_vec_pretty(store)?;
    fs::write(&path, json).with_context(|| format!("writing {}", path.display()))?;
    restrict_permissions(&path)?;
    Ok(())
}

/// Delete the stored tokens. Returns true if a file was removed.
pub fn clear() -> Result<bool> {
    let path = token_path()?;
    if path.exists() {
        fs::remove_file(&path).with_context(|| format!("removing {}", path.display()))?;
        Ok(true)
    } else {
        Ok(false)
    }
}

pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Current time as an RFC 3339 / ISO 8601 UTC string.
pub fn now_iso() -> String {
    use time::format_description::well_known::Rfc3339;
    time::OffsetDateTime::now_utc()
        .format(&Rfc3339)
        .unwrap_or_default()
}

#[cfg(unix)]
fn restrict_permissions(path: &std::path::Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let perms = fs::Permissions::from_mode(0o600);
    fs::set_permissions(path, perms).with_context(|| format!("chmod 600 {}", path.display()))?;
    Ok(())
}

#[cfg(not(unix))]
fn restrict_permissions(_path: &std::path::Path) -> Result<()> {
    Ok(())
}
