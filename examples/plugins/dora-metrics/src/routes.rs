//! HTTP routing — pure `(method, path, query, body) -> (status, json)` so the
//! integration tests can drive it without a socket. Host-API calls (repos,
//! agent runs) go over `ureq` using the standard plugin env vars.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::config::Config;
use crate::metrics::{self, Commit};

fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_default()
}

fn data_dir() -> PathBuf {
    let d = env("OTTO_PLUGIN_DATA_DIR");
    if d.is_empty() {
        PathBuf::from(".")
    } else {
        PathBuf::from(d)
    }
}

fn host_get(path: &str) -> Result<Value, String> {
    ureq::get(&format!("{}{}", env("OTTO_HOST_API"), path))
        .set(
            "Authorization",
            &format!("Bearer {}", env("OTTO_PLUGIN_TOKEN")),
        )
        .call()
        .map_err(|e| e.to_string())?
        .into_json::<Value>()
        .map_err(|e| e.to_string())
}

fn host_post(path: &str, body: Value) -> Result<Value, String> {
    ureq::post(&format!("{}{}", env("OTTO_HOST_API"), path))
        .set(
            "Authorization",
            &format!("Bearer {}", env("OTTO_PLUGIN_TOKEN")),
        )
        .send_json(body)
        .map_err(|e| e.to_string())?
        .into_json::<Value>()
        .map_err(|e| e.to_string())
}

/// Registered repos as (id, name, path).
fn repos_list() -> Result<Vec<(String, String, String)>, String> {
    let repos = host_get("/repos")?;
    Ok(repos
        .as_array()
        .ok_or("repos not an array")?
        .iter()
        .map(|r| {
            let s = |k: &str| r.get(k).and_then(Value::as_str).unwrap_or("").to_string();
            (s("id"), s("name"), s("path"))
        })
        .collect())
}

/// Commits + display label for a repo selector (`all` = every registered repo).
fn gather_commits(needle: &str, cfg: &Config) -> Result<Option<(String, Vec<Commit>)>, String> {
    let repos = repos_list()?;
    if needle == "all" {
        let mut commits = vec![];
        for (_, name, path) in &repos {
            commits.extend(metrics::load_commits(path, name, cfg.scan_depth));
        }
        return Ok(Some(("All repos".into(), commits)));
    }
    for (id, name, path) in &repos {
        if id == needle || name == needle || path == needle {
            return Ok(Some((
                name.clone(),
                metrics::load_commits(path, name, cfg.scan_depth),
            )));
        }
    }
    Ok(None)
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn metrics_for(needle: &str, days: i64) -> Result<Option<Value>, String> {
    let cfg = Config::load(&data_dir());
    Ok(gather_commits(needle, &cfg)?
        .map(|(label, commits)| metrics::compute(&commits, days, &label, now_secs(), &cfg)))
}

fn qget(query: &str, key: &str) -> String {
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if k == key {
                return urldecode(v);
            }
        }
    }
    String::new()
}

fn urldecode(s: &str) -> String {
    let b = s.replace('+', " ");
    let bytes = b.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Ok(n) = u8::from_str_radix(&b[i + 1..i + 3], 16) {
                out.push(n);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Log the real error server-side; hand the client a generic 500 (never leak
/// details — information exposure).
fn internal(e: String) -> (u16, Value) {
    eprintln!("dora-metrics: {e}");
    (500, json!({ "error": "internal error" }))
}

pub fn handle(method: &str, path: &str, query: &str, body: &str) -> (u16, Value) {
    match (method, path) {
        ("GET", "/health") => (200, json!({ "ok": true })),
        ("GET", "/repos") => match host_get("/repos") {
            Ok(v) => (200, v),
            Err(e) => {
                eprintln!("dora-metrics: host /repos: {e}");
                (502, json!({ "error": "host api unavailable" }))
            }
        },
        ("GET", "/config") => match serde_json::to_value(Config::load(&data_dir())) {
            Ok(v) => (200, v),
            Err(e) => internal(e.to_string()),
        },
        ("PUT", "/config") => {
            let cfg: Config = match serde_json::from_str(body) {
                Ok(c) => c,
                Err(_) => return (400, json!({ "error": "invalid config json" })),
            };
            if let Err(e) = cfg.validate() {
                return (400, json!({ "error": e }));
            }
            match cfg.save(&data_dir()) {
                Ok(()) => (200, serde_json::to_value(cfg).unwrap_or(Value::Null)),
                Err(e) => internal(e),
            }
        }
        ("GET", "/metrics") => {
            let repo = qget(query, "repo");
            let days = qget(query, "days")
                .parse::<i64>()
                .unwrap_or(30)
                .clamp(1, 365);
            match metrics_for(&repo, days) {
                Ok(Some(v)) => (200, v),
                Ok(None) => (404, json!({ "error": "repo not registered" })),
                Err(e) => internal(e),
            }
        }
        ("POST", "/analyze") => {
            let b: Value = serde_json::from_str(body).unwrap_or(json!({}));
            let repo = b
                .get("repo")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let days = b
                .get("days")
                .and_then(Value::as_i64)
                .unwrap_or(30)
                .clamp(1, 365);
            let m = match metrics_for(&repo, days) {
                Ok(Some(v)) => v,
                Ok(None) => return (404, json!({ "error": "repo not registered" })),
                Err(e) => return internal(e),
            };
            let prompt = format!(
                "You are a delivery-performance analyst. Below are DORA metrics (JSON) \
                 for a repo over {days} days, including per-metric performance tiers and \
                 rule-based suggestions already shown to the user. Identify the top \
                 delivery bottlenecks, BUILD ON the existing suggestions without \
                 repeating them, and give concrete, prioritized recommendations. Be \
                 specific and concise.\n\n{}",
                serde_json::to_string_pretty(&m).unwrap_or_default()
            );
            match host_post("/agents/run", json!({ "prompt": prompt })) {
                Ok(r) => {
                    let summary = r
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string();
                    (200, json!({ "summary": summary, "metrics": m }))
                }
                Err(e) => {
                    eprintln!("dora-metrics: agents/run: {e}");
                    (502, json!({ "error": "agent run failed" }))
                }
            }
        }
        _ => (404, json!({ "error": "not found" })),
    }
}
