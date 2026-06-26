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
    ///
    /// The server may bind the session to the old access token, so after a
    /// refresh we drop the session and re-run the handshake before replaying —
    /// except when the body *is* the initialize request, which would recurse.
    async fn post(&mut self, body: &Value) -> Result<reqwest::Response> {
        let resp = self.post_once(body).await?;
        if resp.status() == reqwest::StatusCode::UNAUTHORIZED {
            // refresh() already returns actionable errors; don't re-wrap.
            let refreshed = oauth::refresh(&self.http, &self.store).await?;
            config::save(&refreshed)?;
            self.store = refreshed;

            let is_initialize = body.get("method").and_then(|m| m.as_str()) == Some("initialize");
            if !is_initialize {
                // Re-establish a session under the new token before replaying.
                // Boxed: post -> ensure_initialized -> post is a recursion cycle.
                self.session_id = None;
                self.initialized = false;
                Box::pin(self.ensure_initialized()).await?;
            }
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
        let _ = parse_jsonrpc(resp, id).await?; // surface init errors

        // Acknowledge initialization (best-effort notification, no response).
        // Sent via post_once — the token was just validated, and routing it
        // through post() could re-enter the 401 refresh/re-init path.
        let note = json!({ "jsonrpc": "2.0", "method": "notifications/initialized" });
        let _ = self.post_once(&note).await?;

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
        let rpc = parse_jsonrpc(resp, id).await?;
        let result = rpc
            .get("result")
            .cloned()
            .ok_or_else(|| anyhow!("tool `{name}` returned no result"))?;
        extract_tool_payload(name, result)
    }
}

/// Read the JSON-RPC response matching `expected_id` from either a JSON body or
/// an SSE stream, ignoring any interleaved notifications, and surface any
/// protocol-level `error`.
async fn parse_jsonrpc(resp: reqwest::Response, expected_id: i64) -> Result<Value> {
    let content_type = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    let text = resp.text().await.context("reading MCP response body")?;

    let messages = if content_type.contains("text/event-stream") {
        parse_sse(&text)
    } else if text.trim().is_empty() {
        // e.g. 202 Accepted for a notification.
        return Ok(Value::Null);
    } else {
        let v: Value = serde_json::from_str(&text)
            .with_context(|| format!("parsing JSON-RPC body: {text}"))?;
        // A non-SSE body is a single message or a JSON-RPC batch array.
        match v {
            Value::Array(items) => items,
            other => vec![other],
        }
    };

    // Select the response whose id matches our request; notifications carry no
    // matching id and are skipped.
    let message = select_response(messages, expected_id)
        .ok_or_else(|| anyhow!("no JSON-RPC response matching request id {expected_id}"))?;

    if let Some(err) = message.get("error") {
        let msg = err
            .get("message")
            .and_then(|m| m.as_str())
            .unwrap_or("unknown error");
        bail!("MCP error: {msg}");
    }
    Ok(message)
}

/// Pick the JSON-RPC message whose `id` matches the request, skipping
/// notifications (which have no matching id).
fn select_response(messages: Vec<Value>, expected_id: i64) -> Option<Value> {
    let want = json!(expected_id);
    messages.into_iter().find(|m| m.get("id") == Some(&want))
}

/// Collect every JSON-RPC object from an SSE stream's `data:` lines.
fn parse_sse(text: &str) -> Vec<Value> {
    let mut out = Vec::new();
    let mut data = String::new();
    let flush = |data: &mut String, out: &mut Vec<Value>| {
        if !data.is_empty() {
            if let Ok(v) = serde_json::from_str::<Value>(data.trim()) {
                if v.get("jsonrpc").is_some() {
                    out.push(v);
                }
            }
            data.clear();
        }
    };
    for line in text.lines() {
        if let Some(rest) = line.strip_prefix("data:") {
            data.push_str(rest.trim_start());
            data.push('\n');
        } else if line.trim().is_empty() {
            flush(&mut data, &mut out);
        }
    }
    flush(&mut data, &mut out);
    out
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sse_collects_all_events() {
        let stream = "event: message\ndata: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\n\
                      event: message\ndata: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n";
        let msgs = parse_sse(stream);
        assert_eq!(msgs.len(), 2);
    }

    #[test]
    fn select_response_skips_notifications_and_matches_id() {
        let msgs = parse_sse(
            "data: {\"jsonrpc\":\"2.0\",\"method\":\"notifications/progress\"}\n\n\
             data: {\"jsonrpc\":\"2.0\",\"id\":7,\"result\":{\"ok\":true}}\n\n",
        );
        let picked = select_response(msgs, 7).unwrap();
        assert_eq!(picked.get("result").unwrap(), &json!({ "ok": true }));
    }

    #[test]
    fn select_response_none_when_id_absent() {
        let msgs = vec![json!({ "jsonrpc": "2.0", "id": 1, "result": {} })];
        assert!(select_response(msgs, 99).is_none());
    }

    #[test]
    fn extract_tool_payload_parses_text_json() {
        let result = json!({ "content": [{ "type": "text", "text": "{\"a\":1}" }] });
        assert_eq!(
            extract_tool_payload("t", result).unwrap(),
            json!({ "a": 1 })
        );
    }

    #[test]
    fn extract_tool_payload_surfaces_is_error() {
        let result = json!({ "isError": true, "content": [{ "type": "text", "text": "boom" }] });
        assert!(extract_tool_payload("t", result).is_err());
    }
}
