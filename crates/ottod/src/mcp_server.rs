//! `ottod mcp-server` — the OUTWARD "Otto as an MCP server".
//!
//! An external agent launches this over stdio with an `OTTO_API_TOKEN` that is a
//! restricted `kind='mcp'` token (minted in the MCP Control Plane UI). It speaks
//! newline-delimited JSON-RPC 2.0 and exposes the `otto.*` tools. Every
//! `tools/call` is forwarded to `POST /api/v1/mcp/otto-tools/invoke`, which is the
//! ONLY route the restricted token may reach — the control plane governs (enabled?
//! allowlisted? dangerous→approval?), audits, and executes each call. `tools/list`
//! reflects the currently-enabled tool set from `GET /api/v1/mcp/otto-server`.
//!
//! **MCP profiles.** `--profile <id>[,<id>]` (or `OTTO_MCP_PROFILE`) attaches
//! this connection as one profile ("Otto · Database"): `tools/list` shows only
//! that profile's tools and a call outside it is refused naming the profile,
//! before it reaches the daemon (`otto_mcp::profiles`). Without it, every
//! enabled tool — the connection exactly as before.

use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

use otto_server::mcp_profiles::ToolFilter;

use crate::config::Config;

const PROTOCOL_VERSION: &str = "2024-11-05";
const CALL_TIMEOUT: Duration = Duration::from_secs(35);

struct Ctx {
    http: reqwest::Client,
    base: String,
    token: String,
    /// `--profile` / `OTTO_MCP_PROFILE` (comma-separated ids); `None` ⇒ all.
    profiles: Option<String>,
}

impl Ctx {
    async fn get_json(&self, path: &str) -> Result<Value, String> {
        let url = format!("{}/api/v1{}", self.base.trim_end_matches('/'), path);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;
        let status = resp.status();
        let body = resp.text().await.map_err(|e| format!("read body: {e}"))?;
        if !status.is_success() {
            return Err(format!(
                "daemon returned {status}: {}",
                body.chars().take(300).collect::<String>()
            ));
        }
        serde_json::from_str(&body).map_err(|e| format!("parse json: {e}"))
    }

    async fn post_json(&self, path: &str, body: &Value, within: Duration) -> Result<Value, String> {
        let url = format!("{}/api/v1{}", self.base.trim_end_matches('/'), path);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .timeout(within)
            .json(body)
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.map_err(|e| format!("read body: {e}"))?;
        if !status.is_success() {
            return Err(format!(
                "daemon returned {status}: {}",
                text.chars().take(300).collect::<String>()
            ));
        }
        serde_json::from_str(&text).map_err(|e| format!("parse json: {e}"))
    }

    /// The MCP `tools/list`: the otto.* specs filtered to the enabled set.
    async fn tools_list(&self) -> Value {
        // The set of currently-enabled tools (best-effort; on error list all),
        // from the light names-only read; a daemon older than this binary
        // lacks it, so fall back to the full status.
        let mut filter = None;
        let enabled: Option<Vec<String>> = match self.get_json(&self.enabled_path()).await {
            Ok(v) => {
                filter = self.filter_in(&v);
                v.get("enabled").and_then(Value::as_array).map(|names| {
                    names
                        .iter()
                        .filter_map(|n| n.as_str().map(str::to_string))
                        .collect()
                })
            }
            Err(_) => self.get_json("/mcp/otto-server").await.ok().and_then(|v| {
                v.get("tools").and_then(Value::as_array).map(|tools| {
                    tools
                        .iter()
                        .filter(|t| t.get("enabled").and_then(Value::as_bool).unwrap_or(false))
                        .filter_map(|t| t.get("name").and_then(Value::as_str).map(str::to_string))
                        .collect()
                })
            }),
        };
        let tools: Vec<Value> = otto_server::mcp_outward::otto_tool_specs_cached()
            .iter()
            .filter(|s| {
                let name = s.get("name").and_then(Value::as_str).unwrap_or("");
                enabled
                    .as_ref()
                    .map(|e| e.iter().any(|n| n == name))
                    .unwrap_or(true)
                    && filter.as_ref().is_none_or(|f: &ToolFilter| f.allows(name))
            })
            .map(|s| {
                json!({
                    "name": s["name"],
                    "description": s["description"],
                    "inputSchema": s["inputSchema"],
                })
            })
            .collect();
        json!({ "tools": tools })
    }

    /// The enable-list read, resolving this connection's profile when set.
    fn enabled_path(&self) -> String {
        match &self.profiles {
            Some(p) => format!("/mcp/otto-server/enabled?profiles={p}"),
            None => "/mcp/otto-server/enabled".to_string(),
        }
    }

    /// The resolved profile filter of an enable-list answer (only when this
    /// connection asked for one).
    fn filter_in(&self, v: &Value) -> Option<ToolFilter> {
        self.profiles.as_ref()?;
        serde_json::from_value(v.get("profile")?.clone()).ok()
    }

    /// The MCP `tools/call`: forward to the governed choke point — unless the
    /// connection's profile leaves the tool out.
    async fn tools_call(&self, name: &str, args: &Value) -> Value {
        if self.profiles.is_some() {
            if let Ok(v) = self.get_json(&self.enabled_path()).await {
                if let Some(f) = self.filter_in(&v).filter(|f| !f.allows(name)) {
                    return tool_result(&json!({ "error": f.denial(name) }), true);
                }
            }
        }
        let res = self
            .post_json(
                "/mcp/otto-tools/invoke",
                &json!({ "tool": name, "arguments": args }),
                invoke_timeout(name, args),
            )
            .await;
        match res {
            Ok(v) => {
                let executed = v.get("executed").and_then(Value::as_bool).unwrap_or(false);
                let decision = v.get("decision").and_then(Value::as_str).unwrap_or("");
                let is_error = decision == "denied"
                    || decision == "error"
                    || v.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                // Surface the whole governed envelope to the agent so it can see a
                // denial reason / pending-approval id / the tool content.
                let _ = executed;
                tool_result(&v, is_error)
            }
            Err(e) => tool_result(&json!({ "error": e }), true),
        }
    }
}

/// How long one `tools/call` may take: the daemon's own per-tool self-call
/// budget (up to 180 s for an automation) plus slack. The flat 35 s cut long
/// tools off while the daemon ran on — the agent saw a failure, retried, and
/// the tool ran twice.
fn invoke_timeout(name: &str, args: &Value) -> Duration {
    let short = name.strip_prefix("otto.").unwrap_or(name);
    otto_server::mcp_outward::call_timeout(short, args) + Duration::from_secs(15)
}

fn tool_result(value: &Value, is_error: bool) -> Value {
    let text = serde_json::to_string_pretty(value).unwrap_or_else(|_| value.to_string());
    json!({ "content": [ { "type": "text", "text": text } ], "isError": is_error })
}

fn rpc_ok(id: Value, result: Value) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "result": result })
}
fn rpc_err(id: Value, code: i64, message: impl Into<String>) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message.into() } })
}

async fn handle(ctx: &Ctx, msg: Value) -> Option<Value> {
    let method = msg.get("method").and_then(|v| v.as_str()).unwrap_or("");
    let id = msg.get("id").cloned();
    // A notification (no `id`) is never answered — and never EXECUTES
    // anything: an id-less `tools/call` used to run the governed tool with
    // nobody receiving its result, then reply with `"id": null`.
    id.as_ref()?;
    match method {
        "initialize" => Some(rpc_ok(
            id.unwrap_or(Value::Null),
            json!({
                "protocolVersion": PROTOCOL_VERSION,
                "capabilities": { "tools": {} },
                "serverInfo": { "name": "otto", "version": env!("CARGO_PKG_VERSION") }
            }),
        )),
        "notifications/initialized" | "initialized" => None,
        "ping" => Some(rpc_ok(id.unwrap_or(Value::Null), json!({}))),
        "tools/list" => Some(rpc_ok(id.unwrap_or(Value::Null), ctx.tools_list().await)),
        "tools/call" => {
            let id = id.unwrap_or(Value::Null);
            let params = msg.get("params").cloned().unwrap_or(Value::Null);
            let name = params
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("")
                .to_string();
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            Some(rpc_ok(id, ctx.tools_call(&name, &args).await))
        }
        _ => Some(rpc_err(
            id.unwrap_or(Value::Null),
            -32601,
            format!("method not found: {method}"),
        )),
    }
}

pub async fn run() -> Result<(), String> {
    let token = std::env::var("OTTO_API_TOKEN").unwrap_or_default();
    if token.is_empty() {
        return Err("OTTO_API_TOKEN not set (mint a restricted MCP token in the MCP Control Plane → Otto Server tab)".into());
    }
    let base = std::env::var("OTTO_MCP_BASE")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| format!("http://127.0.0.1:{}", Config::load().port));

    let http = reqwest::Client::builder()
        .timeout(CALL_TIMEOUT)
        .build()
        .map_err(|e| format!("build http client: {e}"))?;
    let profiles = connection_profiles(
        profile_arg(&std::env::args().collect::<Vec<_>>())
            .or_else(|| std::env::var("OTTO_MCP_PROFILE").ok()),
    )?;
    let ctx = Ctx {
        http,
        base,
        token,
        profiles,
    };

    let stdin = tokio::io::stdin();
    let mut reader = BufReader::new(stdin);
    let mut stdout = tokio::io::stdout();
    let mut line = String::new();
    loop {
        line.clear();
        let n = reader
            .read_line(&mut line)
            .await
            .map_err(|e| format!("read stdin: {e}"))?;
        if n == 0 {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(trimmed) {
            Ok(v) => v,
            Err(e) => {
                write_line(
                    &mut stdout,
                    &rpc_err(Value::Null, -32700, format!("parse error: {e}")),
                )
                .await?;
                continue;
            }
        };
        if let Some(resp) = handle(&ctx, msg).await {
            write_line(&mut stdout, &resp).await?;
        }
    }
    Ok(())
}

/// The connection's profile ids from a raw `--profile` / `OTTO_MCP_PROFILE`
/// value, parsed like the HTTP path (`otto_mcp::profiles::parse_ids`: each id
/// trimmed, blanks dropped), so `"database, issues"` names both profiles.
/// Fails CLOSED: an id with any character outside `[A-Za-z0-9_-]` is an error
/// naming it — the connection must never silently fall back to "no profile"
/// (every enabled tool) when the person asked for a narrowed one. An absent or
/// all-blank value means no profile.
fn connection_profiles(raw: Option<String>) -> Result<Option<String>, String> {
    let Some(raw) = raw else { return Ok(None) };
    let ids = otto_mcp::profiles::parse_ids(&raw);
    if let Some(bad) = ids.iter().find(|id| {
        !id.chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_".contains(c))
    }) {
        return Err(format!(
            "invalid MCP profile id {bad:?} in --profile / OTTO_MCP_PROFILE \
             (ids are letters, digits, '-' and '_', comma-separated)"
        ));
    }
    Ok((!ids.is_empty()).then(|| ids.join(",")))
}

/// `--profile <ids>` / `--profile=<ids>` in `args`.
fn profile_arg(args: &[String]) -> Option<String> {
    let mut it = args.iter();
    while let Some(a) = it.next() {
        if a == "--profile" {
            return it.next().cloned();
        }
        if let Some(p) = a.strip_prefix("--profile=") {
            return Some(p.to_string());
        }
    }
    None
}

async fn write_line(stdout: &mut tokio::io::Stdout, value: &Value) -> Result<(), String> {
    let mut buf = serde_json::to_vec(value).map_err(|e| format!("encode: {e}"))?;
    buf.push(b'\n');
    stdout
        .write_all(&buf)
        .await
        .map_err(|e| format!("write: {e}"))?;
    stdout.flush().await.map_err(|e| format!("flush: {e}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> Ctx {
        // Nothing listens here: a request that went out would fail, not hang.
        Ctx {
            http: reqwest::Client::new(),
            base: "http://127.0.0.1:9".into(),
            token: "t".into(),
            profiles: None,
        }
    }

    #[tokio::test]
    async fn notifications_are_never_answered_or_executed() {
        let call = json!({"jsonrpc": "2.0", "method": "tools/call",
            "params": {"name": "otto.list_repos", "arguments": {}}});
        assert_eq!(handle(&ctx(), call).await, None);
        let ping = json!({"jsonrpc": "2.0", "method": "ping"});
        assert_eq!(handle(&ctx(), ping).await, None);
        let req = json!({"jsonrpc": "2.0", "id": 7, "method": "ping"});
        assert_eq!(handle(&ctx(), req).await.unwrap()["id"], 7);
    }

    #[test]
    fn a_connection_profile_is_named_by_flag_and_resolved_by_the_daemon() {
        let a = |v: &[&str]| v.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(
            profile_arg(&a(&["ottod", "mcp-server", "--profile", "database"])).as_deref(),
            Some("database")
        );
        assert_eq!(
            profile_arg(&a(&["ottod", "mcp-server", "--profile=git,cloud"])).as_deref(),
            Some("git,cloud")
        );
        assert_eq!(profile_arg(&a(&["ottod", "mcp-server"])), None);
        let mut c = ctx();
        assert_eq!(c.enabled_path(), "/mcp/otto-server/enabled");
        let answer = json!({"profile": {"ids": ["database"], "label": "Database",
            "profiles": [{"id": "database", "name": "Database", "groups": ["database"]}]}});
        assert_eq!(c.filter_in(&answer), None, "no profile asked ⇒ no filter");
        c.profiles = Some("database".into());
        assert_eq!(
            c.enabled_path(),
            "/mcp/otto-server/enabled?profiles=database"
        );
        let f = c.filter_in(&answer).unwrap();
        assert!(f.allows("otto.query_db_readonly") && !f.allows("otto.create_pr"));
        assert!(f.denial("otto.create_pr").contains("\"Database\""));
    }

    #[test]
    fn a_profile_value_is_trimmed_per_id_and_a_bad_id_fails_closed() {
        let p = |v: &str| connection_profiles(Some(v.into()));
        assert_eq!(
            p("database, issues").unwrap().as_deref(),
            Some("database,issues")
        );
        assert_eq!(p(" git ,, cloud\n").unwrap().as_deref(), Some("git,cloud"));
        assert_eq!(p("  ").unwrap(), None, "blank ⇒ no profile asked");
        assert_eq!(connection_profiles(None).unwrap(), None);
        // A stray character must never widen the connection to every tool.
        let err = p("database, is.sues").unwrap_err();
        assert!(err.contains("\"is.sues\""), "{err}");
        assert!(p("data base").is_err());
    }

    #[test]
    fn long_tools_get_their_daemon_budget() {
        let args = json!({});
        assert!(invoke_timeout("otto.api_run_automation", &args) > Duration::from_secs(180));
        assert!(invoke_timeout("api_run_automation", &args) > Duration::from_secs(180));
        assert!(invoke_timeout("otto.list_repos", &args) >= Duration::from_secs(30));
    }
}
