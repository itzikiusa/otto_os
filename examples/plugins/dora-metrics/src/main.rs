//! dora-metrics — Otto runtime plugin (Rust sidecar).
//!
//! Otto spawns this with: OTTO_PLUGIN_PORT (bind here), OTTO_PLUGIN_TOKEN +
//! OTTO_HOST_API (call back for repos/agents), OTTO_PLUGIN_DATA_DIR. Otto
//! reverse-proxies /api/v1/plugins/dora-metrics/* to these routes. DORA signal:
//! deploy = a `*deployed*` tag (case-insensitive); merges into develop classified
//! by source branch (hotfix/release/feature). MTTR omitted.

use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};
use tiny_http::{Header, Method, Response, Server};

fn env(k: &str) -> String {
    std::env::var(k).unwrap_or_default()
}

// ---- host API + git -------------------------------------------------------

fn host_get(path: &str) -> Result<Value, String> {
    ureq::get(&format!("{}{}", env("OTTO_HOST_API"), path))
        .set("Authorization", &format!("Bearer {}", env("OTTO_PLUGIN_TOKEN")))
        .call()
        .map_err(|e| e.to_string())?
        .into_json::<Value>()
        .map_err(|e| e.to_string())
}

fn host_post(path: &str, body: Value) -> Result<Value, String> {
    ureq::post(&format!("{}{}", env("OTTO_HOST_API"), path))
        .set("Authorization", &format!("Bearer {}", env("OTTO_PLUGIN_TOKEN")))
        .send_json(body)
        .map_err(|e| e.to_string())?
        .into_json::<Value>()
        .map_err(|e| e.to_string())
}

fn git(repo: &str, args: &[&str]) -> String {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        .unwrap_or_default()
}

// ---- DORA computation -----------------------------------------------------

struct Commit {
    ts: i64,
    subject: String,
    parents: usize,
    refs: Vec<String>,
}

fn load_commits(repo: &str) -> Vec<Commit> {
    let out = git(repo, &["log", "--all", "-n", "2000", "--pretty=%ct\x1f%P\x1f%s\x1f%D"]);
    out.lines()
        .filter_map(|line| {
            let f: Vec<&str> = line.split('\x1f').collect();
            if f.len() < 4 {
                return None;
            }
            let ts = f[0].trim().parse::<i64>().ok()?;
            let parents = f[1].split_whitespace().count();
            let refs = if f[3].is_empty() {
                vec![]
            } else {
                f[3].split(", ").map(|s| s.to_string()).collect()
            };
            Some(Commit { ts, subject: f[2].to_string(), parents, refs })
        })
        .collect()
}

fn classify(subject: &str) -> Option<&'static str> {
    let s = subject.to_lowercase();
    if s.contains("hotfix/") {
        Some("hotfix")
    } else if s.contains("release/") {
        Some("release")
    } else if s.contains("feature/") {
        Some("feature")
    } else {
        None
    }
}

fn deploy_tag(refs: &[String]) -> Option<String> {
    for r in refs {
        if let Some(tag) = r.trim().strip_prefix("tag: ") {
            if tag.to_lowercase().contains("deployed") {
                return Some(tag.to_string());
            }
        }
    }
    None
}

fn mean(v: &[f64]) -> Value {
    if v.is_empty() {
        Value::Null
    } else {
        json!(v.iter().sum::<f64>() / v.len() as f64)
    }
}

fn avg_gap(targets: &[i64], preceding: &[i64]) -> Value {
    let mut g = vec![];
    for &t in targets {
        if let Some(&p) = preceding.iter().filter(|&&p| p <= t).max() {
            g.push((t - p) as f64 / 3600.0);
        }
    }
    mean(&g)
}

fn compute(commits: &[Commit], days: i64, repo_name: &str, now: i64) -> Value {
    let cutoff = now - days.max(1) * 86400;
    let mut deploys: Vec<(i64, String)> = vec![];
    let mut merges: Vec<(i64, &'static str, String)> = vec![];
    for c in commits {
        if c.ts < cutoff {
            continue;
        }
        if let Some(tag) = deploy_tag(&c.refs) {
            deploys.push((c.ts, tag));
        }
        if c.parents >= 2 {
            if let Some(k) = classify(&c.subject) {
                merges.push((c.ts, k, c.subject.clone()));
            }
        }
    }
    deploys.sort_by_key(|d| d.0);
    merges.sort_by_key(|m| m.0);

    let (mut feat, mut rel, mut hot) = (0u32, 0u32, 0u32);
    for m in &merges {
        match m.1 {
            "feature" => feat += 1,
            "release" => rel += 1,
            "hotfix" => hot += 1,
            _ => {}
        }
    }

    let mut lead: Vec<f64> = vec![];
    let mut failing = 0u32;
    let mut prev: Option<i64> = None;
    for d in &deploys {
        let included: Vec<&(i64, &str, String)> = merges
            .iter()
            .filter(|m| m.0 <= d.0 && prev.map_or(true, |p| m.0 > p))
            .collect();
        if let Some(oldest) = included.iter().map(|m| m.0).min() {
            lead.push((d.0 - oldest) as f64 / 3600.0);
        }
        if included.iter().any(|m| m.1 == "hotfix") {
            failing += 1;
        }
        prev = Some(d.0);
    }
    let cfr = if deploys.is_empty() {
        0.0
    } else {
        failing as f64 / deploys.len() as f64
    };
    let freq = deploys.len() as f64 / (days.max(1) as f64 / 7.0);

    let feat_dates: Vec<i64> = merges.iter().filter(|m| m.1 == "feature").map(|m| m.0).collect();
    let rel_dates: Vec<i64> = merges.iter().filter(|m| m.1 == "release").map(|m| m.0).collect();
    let dep_dates: Vec<i64> = deploys.iter().map(|d| d.0).collect();

    let mut recent = merges.clone();
    recent.sort_by(|a, b| b.0.cmp(&a.0));
    recent.truncate(50);

    json!({
        "repo_name": repo_name,
        "window_days": days,
        "deployment_frequency_per_week": freq,
        "lead_time_hours": mean(&lead),
        "change_failure_rate": cfr,
        "avg_feature_to_release_hours": avg_gap(&rel_dates, &feat_dates),
        "avg_release_to_deploy_hours": avg_gap(&dep_dates, &rel_dates),
        "counts": { "feature": feat, "release": rel, "hotfix": hot },
        "deployments": deploys.iter().map(|d| json!({"ts": d.0, "tag": d.1})).collect::<Vec<_>>(),
        "recent_merges": recent.iter().map(|m| json!({"ts": m.0, "kind": m.1, "subject": m.2})).collect::<Vec<_>>(),
    })
}

// ---- routing --------------------------------------------------------------

fn resolve(needle: &str) -> Result<(String, String), String> {
    let repos = host_get("/repos")?;
    for r in repos.as_array().ok_or("repos not an array")? {
        let id = r.get("id").and_then(|x| x.as_str()).unwrap_or("");
        let name = r.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let p = r.get("path").and_then(|x| x.as_str()).unwrap_or("");
        if id == needle || name == needle || p == needle {
            return Ok((name.to_string(), p.to_string()));
        }
    }
    Err(format!("repo '{needle}' not registered"))
}

fn now_secs() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

fn metrics(needle: &str, days: i64) -> Result<Value, String> {
    let (name, path) = resolve(needle)?;
    Ok(compute(&load_commits(&path), days, &name, now_secs()))
}

fn analyze(needle: &str, days: i64) -> Result<Value, String> {
    let m = metrics(needle, days)?;
    let prompt = format!(
        "You are a delivery-performance analyst. Given these DORA metrics (JSON) for a repo \
         over {days} days, identify the top delivery BOTTLENECKS and give concrete, \
         prioritized recommendations. Be specific and concise.\n\n{}",
        serde_json::to_string_pretty(&m).unwrap_or_default()
    );
    let r = host_post("/agents/run", json!({ "prompt": prompt }))?;
    let summary = r.get("text").and_then(|x| x.as_str()).unwrap_or("").to_string();
    Ok(json!({ "summary": summary, "metrics": m }))
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

fn handle(method: &Method, path: &str, query: &str, body: &str) -> (u16, Value) {
    match (method, path) {
        (Method::Get, "/health") => (200, json!({ "ok": true })),
        (Method::Get, "/repos") => match host_get("/repos") {
            Ok(v) => (200, v),
            Err(e) => (502, json!({ "error": e })),
        },
        (Method::Get, "/metrics") => {
            let repo = qget(query, "repo");
            let days = qget(query, "days").parse::<i64>().unwrap_or(30);
            match metrics(&repo, days) {
                Ok(v) => (200, v),
                Err(e) => (500, json!({ "error": e })),
            }
        }
        (Method::Post, "/analyze") => {
            let b: Value = serde_json::from_str(body).unwrap_or(json!({}));
            let repo = b.get("repo").and_then(|x| x.as_str()).unwrap_or("").to_string();
            let days = b.get("days").and_then(|x| x.as_i64()).unwrap_or(30);
            match analyze(&repo, days) {
                Ok(v) => (200, v),
                Err(e) => (500, json!({ "error": e })),
            }
        }
        _ => (404, json!({ "error": "not found" })),
    }
}

fn main() {
    let port: u16 = env("OTTO_PLUGIN_PORT").parse().unwrap_or(0);
    let server = Server::http(format!("127.0.0.1:{port}")).expect("bind plugin port");
    eprintln!("dora-metrics sidecar on :{port}");
    for mut req in server.incoming_requests() {
        let method = req.method().clone();
        let raw = req.url().to_string();
        let (path, query) = match raw.split_once('?') {
            Some((p, q)) => (p.to_string(), q.to_string()),
            None => (raw.clone(), String::new()),
        };
        let mut body = String::new();
        if method == Method::Post {
            let _ = req.as_reader().read_to_string(&mut body);
        }
        let (code, val) = handle(&method, &path, &query, &body);
        let data = serde_json::to_vec(&val).unwrap_or_default();
        let resp = Response::from_data(data).with_status_code(code).with_header(
            Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..]).unwrap(),
        );
        let _ = req.respond(resp);
    }
}
