//! OAuth 2.1 authorization-code + PKCE flow against the Plaud MCP server.
//!
//! Follows the MCP authorization spec: discover the protected-resource and
//! authorization-server metadata, dynamically register a public client, run
//! the PKCE browser flow via a loopback redirect, then exchange the code for
//! tokens. Refresh is a straight `refresh_token` grant.

use anyhow::{anyhow, bail, Context, Result};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use rand::RngCore;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

use crate::config::{self, TokenStore};

/// Resource we request access to (RFC 8707) and post tools against.
pub const MCP_URL: &str = "https://mcp.plaud.ai/mcp";
const PRM_URL: &str = "https://mcp.plaud.ai/.well-known/oauth-protected-resource/mcp";
const CLIENT_NAME: &str = "plaud-cli";
const CALLBACK_TIMEOUT_SECS: u64 = 300;

#[derive(Debug, Deserialize)]
struct ProtectedResourceMeta {
    authorization_servers: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AuthServerMeta {
    authorization_endpoint: String,
    token_endpoint: String,
    registration_endpoint: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RegistrationResponse {
    client_id: String,
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
    #[serde(default)]
    refresh_token: Option<String>,
    #[serde(default)]
    expires_in: Option<u64>,
}

/// Discover the authorization server and its endpoints.
async fn discover(http: &reqwest::Client) -> Result<AuthServerMeta> {
    let prm: ProtectedResourceMeta = http
        .get(PRM_URL)
        .send()
        .await
        .context("fetching protected-resource metadata")?
        .error_for_status()?
        .json()
        .await
        .context("parsing protected-resource metadata")?;

    let issuer = prm
        .authorization_servers
        .first()
        .ok_or_else(|| anyhow!("no authorization_servers in protected-resource metadata"))?
        .trim_end_matches('/')
        .to_string();

    let as_url = format!("{issuer}/.well-known/oauth-authorization-server");
    let meta: AuthServerMeta = http
        .get(&as_url)
        .send()
        .await
        .context("fetching authorization-server metadata")?
        .error_for_status()?
        .json()
        .await
        .context("parsing authorization-server metadata")?;
    Ok(meta)
}

/// Register a public (token_endpoint_auth_method=none) client for this
/// loopback redirect. DCR is cheap, so we register fresh each login to dodge
/// loopback port-mismatch issues.
async fn register(http: &reqwest::Client, endpoint: &str, redirect_uri: &str) -> Result<String> {
    let body = serde_json::json!({
        "client_name": CLIENT_NAME,
        "redirect_uris": [redirect_uri],
        "grant_types": ["authorization_code", "refresh_token"],
        "response_types": ["code"],
        "token_endpoint_auth_method": "none",
    });
    let resp: RegistrationResponse = http
        .post(endpoint)
        .json(&body)
        .send()
        .await
        .context("dynamic client registration request")?
        .error_for_status()
        .context("dynamic client registration rejected")?
        .json()
        .await
        .context("parsing registration response")?;
    Ok(resp.client_id)
}

struct Pkce {
    verifier: String,
    challenge: String,
}

fn pkce() -> Pkce {
    let mut raw = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw);
    let verifier = URL_SAFE_NO_PAD.encode(raw);
    let digest = Sha256::digest(verifier.as_bytes());
    let challenge = URL_SAFE_NO_PAD.encode(digest);
    Pkce {
        verifier,
        challenge,
    }
}

fn random_state() -> String {
    let mut raw = [0u8; 16];
    rand::thread_rng().fill_bytes(&mut raw);
    URL_SAFE_NO_PAD.encode(raw)
}

/// Run the full interactive login flow and return fresh tokens.
pub async fn login(http: &reqwest::Client) -> Result<TokenStore> {
    let meta = discover(http).await?;
    let reg_endpoint = meta
        .registration_endpoint
        .clone()
        .ok_or_else(|| anyhow!("server does not advertise a registration_endpoint"))?;

    // Bind the loopback catcher first so we register the exact redirect URI.
    let listener = TcpListener::bind("127.0.0.1:0")
        .await
        .context("binding loopback redirect listener")?;
    let port = listener.local_addr()?.port();
    let redirect_uri = format!("http://127.0.0.1:{port}/callback");

    let client_id = register(http, &reg_endpoint, &redirect_uri).await?;
    let pkce = pkce();
    let state = random_state();

    let mut auth_url = Url::parse(&meta.authorization_endpoint)?;
    auth_url
        .query_pairs_mut()
        .append_pair("response_type", "code")
        .append_pair("client_id", &client_id)
        .append_pair("redirect_uri", &redirect_uri)
        .append_pair("state", &state)
        .append_pair("code_challenge", &pkce.challenge)
        .append_pair("code_challenge_method", "S256")
        .append_pair("resource", MCP_URL);

    println!("Opening browser for Plaud sign-in...");
    println!("If it does not open, visit:\n  {auth_url}\n");
    let _ = open::that(auth_url.as_str());

    let code = wait_for_callback(listener, &state).await?;

    let token = exchange_code(
        http,
        &meta.token_endpoint,
        &client_id,
        &code,
        &redirect_uri,
        &pkce.verifier,
    )
    .await?;

    Ok(TokenStore {
        client_id,
        access_token: token.access_token,
        refresh_token: token.refresh_token,
        expires_at: token.expires_in.map(|s| config::now() + s),
    })
}

/// Block on the loopback listener until Plaud redirects back with a code.
async fn wait_for_callback(listener: TcpListener, expected_state: &str) -> Result<String> {
    let accept = async {
        loop {
            let (mut stream, _) = listener.accept().await?;
            let mut buf = vec![0u8; 8192];
            let n = stream.read(&mut buf).await?;
            let request = String::from_utf8_lossy(&buf[..n]);
            let target = request
                .lines()
                .next()
                .and_then(|line| line.split_whitespace().nth(1))
                .unwrap_or("/");

            if !target.starts_with("/callback") {
                respond(&mut stream, 404, "Not found").await?;
                continue;
            }

            let parsed = Url::parse(&format!("http://127.0.0.1{target}"))?;
            let mut code = None;
            let mut state = None;
            let mut err = None;
            for (k, v) in parsed.query_pairs() {
                match k.as_ref() {
                    "code" => code = Some(v.into_owned()),
                    "state" => state = Some(v.into_owned()),
                    "error" => err = Some(v.into_owned()),
                    _ => {}
                }
            }

            if let Some(e) = err {
                respond(
                    &mut stream,
                    400,
                    "Authorization failed. You can close this tab.",
                )
                .await?;
                bail!("authorization error from server: {e}");
            }
            if state.as_deref() != Some(expected_state) {
                respond(&mut stream, 400, "State mismatch. You can close this tab.").await?;
                bail!("state mismatch — possible CSRF, aborting");
            }
            match code {
                Some(c) => {
                    respond(
                        &mut stream,
                        200,
                        "Plaud sign-in complete. You can close this tab and return to the terminal.",
                    )
                    .await?;
                    return Ok(c);
                }
                None => {
                    respond(&mut stream, 400, "Missing code. You can close this tab.").await?;
                    bail!("callback missing authorization code");
                }
            }
        }
    };

    tokio::time::timeout(
        std::time::Duration::from_secs(CALLBACK_TIMEOUT_SECS),
        accept,
    )
    .await
    .map_err(|_| anyhow!("timed out waiting for browser sign-in"))?
}

async fn respond(stream: &mut tokio::net::TcpStream, status: u16, msg: &str) -> Result<()> {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        _ => "Not Found",
    };
    let body = format!(
        "<!doctype html><html><body style=\"font-family:system-ui;padding:3rem\">\
         <h2>{msg}</h2></body></html>"
    );
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

async fn exchange_code(
    http: &reqwest::Client,
    token_endpoint: &str,
    client_id: &str,
    code: &str,
    redirect_uri: &str,
    verifier: &str,
) -> Result<TokenResponse> {
    let resp = http
        .post(token_endpoint)
        .form(&[
            ("grant_type", "authorization_code"),
            ("code", code),
            ("redirect_uri", redirect_uri),
            ("client_id", client_id),
            ("code_verifier", verifier),
        ])
        .send()
        .await
        .context("token exchange request")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("token exchange failed ({status}): {body}");
    }
    resp.json().await.context("parsing token response")
}

/// Exchange a refresh token for a fresh access token.
pub async fn refresh(http: &reqwest::Client, store: &TokenStore) -> Result<TokenStore> {
    let refresh_token = store
        .refresh_token
        .as_ref()
        .ok_or_else(|| anyhow!("no refresh token stored — run `plaud login`"))?;
    let meta = discover(http).await?;

    let resp = http
        .post(&meta.token_endpoint)
        .form(&[
            ("grant_type", "refresh_token"),
            ("refresh_token", refresh_token.as_str()),
            ("client_id", store.client_id.as_str()),
        ])
        .send()
        .await
        .context("refresh request")?;
    if !resp.status().is_success() {
        let status = resp.status();
        let body = resp.text().await.unwrap_or_default();
        bail!("token refresh failed ({status}): {body}");
    }
    let token: TokenResponse = resp.json().await.context("parsing refresh response")?;

    Ok(TokenStore {
        client_id: store.client_id.clone(),
        access_token: token.access_token,
        // Some servers omit a new refresh token; keep the old one.
        refresh_token: token.refresh_token.or_else(|| store.refresh_token.clone()),
        expires_at: token.expires_in.map(|s| config::now() + s),
    })
}
