//! Minimal Model Context Protocol client over the Streamable-HTTP transport.
//!
//! Only what this CLI needs: `initialize` (capturing the session id), the
//! `notifications/initialized` handshake, and `tools/call`. Responses may come
//! back as plain JSON or as a one-shot SSE stream — both are handled. A 401
//! triggers a token refresh and a single retry.

use anyhow::{anyhow, bail, Context, Result};
use serde_json::{json, Value};

use crate::config::{self, TokenStore};
use crate::oauth;

const PROTOCOL_VERSION: &str = "2025-06-18";

pub struct McpClient {
    http: reqwest::Client,
    store: TokenStore,
    session_id: Option<String>,
    next_id: i64,
    initialized: bool,
}

impl McpClient {
    pub fn new(http: reqwest::Client, store: TokenStore) -> Self {
        Self {
            http,
            store,
            session_id: None,
            next_id: 0,
            initialized: false,
        }
    }

    fn id(&mut self) -> i64 {
        self.next_id += 1;
        self.next_id
    }

    /// POST a JSON-RPC payload, refreshing the token once on 401.
    async fn post(&mut self, body: &Value) -> Result<reqwest::Response> {
        let resp = self.post_once(body).await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            // Access token likely expired — refresh, persist, retry once.
            let refreshed = oauth::refresh(&self.http, &self.store)
                .await
                .context("refreshing access token after 401")?;
            config::save(&refreshed)?;
            self.store = refreshed;
            let retry = self.post_once(body).await?;
            return Ok(retry);
        }
        Ok(resp)
    }

    async fn post_once(&self, body: &Value) -> Result<reqwest::Response> {
        let mut req = self
            .http
            .post(oauth::MCP_URL)
            .bearer_auth(&self.store.access_token)
            .header("Accept", "application/json, text/event-stream")
            .header("MCP-Protocol-Version", PROTOCOL_VERSION)
            .json(body);
        if let Some(sid) = &self.session_id {
            req = req.header("Mcp-Session-Id", sid);
        }
        req.send().await.context("posting to MCP endpoint")
    }

    /// Perform the initialize handshake if not already done.
    async fn ensure_initialized(&mut self) -> Result<()> {
        if self.initialized {
            return Ok(());
        }
        let id = self.id();
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "initialize",
            "params": {
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": {},
                "clientInfo": { "name": "plaud-cli", "version": env!("CARGO_PKG_VERSION") }
            }
        });

        let resp = self.post(&body).await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            bail!("not authorized — run `plaud login`");
        }
        if let Some(sid) = resp
            .headers()
            .get("Mcp-Session-Id")
            .and_then(|v| v.to_str().ok())
        {
            self.session_id = Some(sid.to_string());
        }
        let _ = parse_jsonrpc(resp).await?; // surface init errors

        // Acknowledge initialization (notification — no response body).
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        let _ = self.post(&note).await?;

        self.initialized = true;
        Ok(())
    }

    /// Call a tool and return its payload as JSON.
    pub async fn call_tool(&mut self, name: &str, arguments: Value) -> Result<Value> {
        self.ensure_initialized().await?;
        let id = self.id();
        let body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": "tools/call",
            "params": { "name": name, "arguments": arguments }
        });
        let resp = self.post(&body).await?;
        let status = resp.status();
        if !status.is_success() {
            let text = resp.text().await.unwrap_or_default();
            bail!("tool `{name}` failed (HTTP {status}): {text}");
        }
        let rpc = parse_jsonrpc(resp).await?;
        let result = rpc
            .get("result")
            .cloned()
            .ok_or_else(|| anyhow!("tool `{name}` returned no result"))?;
        extract_tool_payload(name, result)
    }
}

/// Read a JSON-RPC response from either a JSON body or an SSE stream, and
/// surface any protocol-level `error`.
async fn parse_jsonrpc(resp: reqwest::Response) -> Result<Value> {
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let text = resp.text().await.context("reading MCP response body")?;

    let message = if content_type.contains("text/event-stream") {
        parse_sse(&text).ok_or_else(|| anyhow!("no JSON-RPC message in SSE stream"))?
    } else if text.trim().is_empty() {
        // e.g. 202 Accepted for a notification.
        return Ok(Value::Null);
    } else {
        serde_json::from_str(&text).with_context(|| format!("parsing JSON-RPC body: {text}"))?
    };

    if let Some(err) = message.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        bail!("MCP error: {msg}");
    }
    Ok(message)
}

/// Pull the first JSON-RPC object out of an SSE stream's `data:` lines.
fn parse_sse(text: &str) -> Option<Value> {
    let mut data = String::new();
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            data.push_str(rest.trim_start());
            data.push('\n');
        } else if line.trim().is_empty() && !data.is_empty() {
            if let Ok(v) = serde_json::from_str::<Value>(data.trim()) {
                if v.get("jsonrpc").is_some() {
                    return Some(v);
                }
            }
            data.clear();
        }
    }
    if !data.is_empty() {
        if let Ok(v) = serde_json::from_str::<Value>(data.trim()) {
            if v.get("jsonrpc").is_some() {
                return Some(v);
            }
        }
    }
    None
}

/// Turn a `tools/call` result into usable JSON: prefer `structuredContent`,
/// else parse the concatenated text content (which is usually JSON), else
/// return the raw text.
fn extract_tool_payload(name: &str, result: Value) -> Result<Value> {
    let is_error = result
        .get("isError")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let text = collect_text(&result);

    if is_error {
        bail!(
            "tool `{name}` reported an error: {}",
            text.unwrap_or_default()
        );
    }

    if let Some(structured) = result.get("structuredContent") {
        if !structured.is_null() {
            return Ok(structured.clone());
        }
    }

    match text {
        Some(t) => Ok(serde_json::from_str(&t).unwrap_or(Value::String(t))),
        None => Ok(result),
    }
}

/// Concatenate the `text` fields of a result's `content` array.
fn collect_text(result: &Value) -> Option<String> {
    let content = result.get("content")?.as_array()?;
    let mut out = String::new();
    for item in content {
        if let Some(t) = item.get("text").and_then(|v| v.as_str()) {
            out.push_str(t);
        }
    }
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}
