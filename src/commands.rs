//! Command handlers — one per CLI subcommand.

use anyhow::{anyhow, Context, Result};
use serde_json::{json, Map, Value};

use crate::cli::{Cli, Command, ListArgs};
use crate::config;
use crate::mcp::McpClient;
use crate::oauth;
use crate::output;

const USER_AGENT: &str = concat!("plaud-cli/", env!("CARGO_PKG_VERSION"));

pub async fn dispatch(cli: Cli) -> Result<()> {
    let http = reqwest::Client::builder()
        .user_agent(USER_AGENT)
        .build()
        .context("building HTTP client")?;
    let json_out = cli.json;

    match cli.command {
        Command::Login => login(&http).await,
        Command::Logout => logout(),
        Command::Whoami => whoami(&http, json_out).await,
        Command::List(args) => list(&http, args, json_out).await,
        Command::Get { id } => get(&http, &id, json_out).await,
        Command::Note { id } => note(&http, &id, json_out).await,
        Command::Transcript { id } => transcript(&http, &id, json_out).await,
        Command::Download { id, out } => download(&http, &id, out).await,
    }
}

/// Build an authenticated MCP client, refreshing the token if it's near expiry.
async fn client(http: &reqwest::Client) -> Result<McpClient> {
    let mut store = config::load()?.ok_or_else(|| anyhow!("not logged in — run `plaud login`"))?;
    if store.is_expired(60) && store.refresh_token.is_some() {
        store = oauth::refresh(http, &store)
            .await
            .context("refreshing access token")?;
        config::save(&store)?;
    }
    Ok(McpClient::new(http.clone(), store))
}

async fn login(http: &reqwest::Client) -> Result<()> {
    let store = oauth::login(http).await?;
    config::save(&store)?;
    // Confirm by reading the account back.
    let mut mcp = McpClient::new(http.clone(), store);
    match mcp.call_tool("get_current_user", json!({})).await {
        Ok(user) => {
            let who = user
                .get("email")
                .or_else(|| user.get("name"))
                .and_then(|v| v.as_str())
                .unwrap_or("(account)");
            println!("Logged in as {who}.");
        }
        Err(_) => println!("Logged in."),
    }
    Ok(())
}

fn logout() -> Result<()> {
    if config::clear()? {
        println!("Logged out — stored tokens removed.");
    } else {
        println!("Already logged out.");
    }
    Ok(())
}

async fn whoami(http: &reqwest::Client, json_out: bool) -> Result<()> {
    let mut mcp = client(http).await?;
    let user = mcp.call_tool("get_current_user", json!({})).await?;
    output::print_pretty(&user)?;
    let _ = json_out; // get_current_user is structured already; pretty == json
    Ok(())
}

async fn list(http: &reqwest::Client, args: ListArgs, json_out: bool) -> Result<()> {
    let mut arguments = Map::new();
    if let Some(q) = args.query {
        arguments.insert("query".into(), json!(q));
    }
    if let Some(f) = args.from {
        arguments.insert("date_from".into(), json!(f));
    }
    if let Some(t) = args.to {
        arguments.insert("date_to".into(), json!(t));
    }
    if let Some(p) = args.page {
        arguments.insert("page".into(), json!(p));
    }
    if let Some(ps) = args.page_size {
        arguments.insert("page_size".into(), json!(ps));
    }

    let mut mcp = client(http).await?;
    let payload = mcp
        .call_tool("list_files", Value::Object(arguments))
        .await?;
    if json_out {
        output::print_pretty(&payload)
    } else {
        output::render_list(&payload)
    }
}

async fn get(http: &reqwest::Client, id: &str, _json_out: bool) -> Result<()> {
    let mut mcp = client(http).await?;
    let payload = mcp.call_tool("get_file", json!({ "id": id })).await?;
    output::print_pretty(&payload)
}

async fn note(http: &reqwest::Client, id: &str, _json_out: bool) -> Result<()> {
    let mut mcp = client(http).await?;
    let payload = mcp.call_tool("get_note", json!({ "id": id })).await?;
    output::print_pretty(&payload)
}

async fn transcript(http: &reqwest::Client, id: &str, json_out: bool) -> Result<()> {
    let mut mcp = client(http).await?;
    let payload = mcp.call_tool("get_transcript", json!({ "id": id })).await?;
    if json_out {
        output::print_pretty(&payload)
    } else {
        output::render_transcript(&payload)
    }
}

async fn download(http: &reqwest::Client, id: &str, out: Option<std::path::PathBuf>) -> Result<()> {
    let mut mcp = client(http).await?;
    let file = mcp.call_tool("get_file", json!({ "id": id })).await?;
    let url = find_presigned_url(&file)
        .ok_or_else(|| anyhow!("no presigned_url in get_file response for {id}"))?;

    let path = out.unwrap_or_else(|| std::path::PathBuf::from(format!("{id}.mp3")));
    let resp = http
        .get(&url)
        .send()
        .await
        .context("downloading audio")?
        .error_for_status()
        .context("audio download rejected")?;
    let bytes = resp.bytes().await.context("reading audio bytes")?;
    std::fs::write(&path, &bytes).with_context(|| format!("writing {}", path.display()))?;
    println!("Saved {} ({} bytes).", path.display(), bytes.len());
    Ok(())
}

/// Locate a `presigned_url` anywhere shallow in the get_file payload.
fn find_presigned_url(value: &Value) -> Option<String> {
    if let Some(u) = value.get("presigned_url").and_then(|v| v.as_str()) {
        return Some(u.to_string());
    }
    // Some payloads nest it under the file object.
    for key in ["file", "data", "result"] {
        if let Some(nested) = value.get(key) {
            if let Some(u) = nested.get("presigned_url").and_then(|v| v.as_str()) {
                return Some(u.to_string());
            }
        }
    }
    None
}
