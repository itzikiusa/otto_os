//! End-to-end over a real scripted git repo: `load_commits` → `compute`, plus
//! the HTTP route layer (`routes::handle`) against a mock host API. Env vars
//! are process-global, so everything runs in ONE test fn (no races).

use std::path::Path;
use std::process::Command;

use dora_metrics::config::Config;
use dora_metrics::{metrics, routes};
use serde_json::json;

const D: i64 = 86_400;

fn git(dir: &Path, args: &[&str], ts: i64) {
    let date = format!("@{ts} +0000");
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "-c",
            "user.email=t@example.com",
            "-c",
            "user.name=T",
            "-c",
            "commit.gpgsign=false",
        ])
        .args(args)
        .env("GIT_AUTHOR_DATE", &date)
        .env("GIT_COMMITTER_DATE", &date)
        .output()
        .expect("run git");
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn write(dir: &Path, name: &str, content: &str) {
    std::fs::write(dir.join(name), content).unwrap();
}

/// develop-based repo: feature merge → deploy → hotfix merge → deploy.
fn build_fixture(dir: &Path, now: i64) {
    std::fs::create_dir_all(dir).unwrap();
    git(dir, &["init", "-q", "-b", "develop"], now - 27 * D);
    write(dir, "a.txt", "0");
    git(dir, &["add", "."], now - 27 * D);
    git(dir, &["commit", "-q", "-m", "init"], now - 27 * D);

    git(dir, &["checkout", "-q", "-b", "feature/one"], now - 25 * D);
    write(dir, "a.txt", "1");
    git(dir, &["commit", "-aqm", "PROJ-1 add thing"], now - 25 * D);
    git(dir, &["checkout", "-q", "develop"], now - 24 * D);
    git(
        dir,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge branch 'feature/one'",
            "feature/one",
        ],
        now - 24 * D,
    );

    write(dir, "a.txt", "2");
    git(dir, &["commit", "-aqm", "chore: release v1"], now - 20 * D);
    git(dir, &["tag", "v1-deployed"], now - 20 * D);

    git(dir, &["checkout", "-q", "-b", "hotfix/fix"], now - 19 * D);
    write(dir, "a.txt", "3");
    git(dir, &["commit", "-aqm", "PROJ-2 urgent fix"], now - 19 * D);
    git(dir, &["checkout", "-q", "develop"], now - 18 * D);
    git(
        dir,
        &[
            "merge",
            "-q",
            "--no-ff",
            "-m",
            "Merge branch 'hotfix/fix'",
            "hotfix/fix",
        ],
        now - 18 * D,
    );

    write(dir, "a.txt", "4");
    git(dir, &["commit", "-aqm", "chore: release v2"], now - 16 * D);
    git(dir, &["tag", "v2-deployed"], now - 16 * D);
}

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn approx(a: f64, b: f64) {
    assert!((a - b).abs() < 1e-6, "{a} != {b}");
}

#[test]
fn fixture_repo_end_to_end() {
    let now = now_secs();
    let base = std::env::temp_dir().join(format!("dora-fixture-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let repo = base.join("repo");
    build_fixture(&repo, now);

    // -- engine level ------------------------------------------------------
    let cfg = Config::default();
    let commits = metrics::load_commits(repo.to_str().unwrap(), "fix", cfg.scan_depth);
    assert!(
        commits.len() >= 7,
        "expected scripted commits, got {}",
        commits.len()
    );
    let m = metrics::compute(&commits, 28, "fix", now, &cfg);

    approx(m["df_per_week"].as_f64().unwrap(), 0.5);
    // Leads: feature merge (now-24d) → v1 (now-20d) = 96h; hotfix merge
    // (now-18d) → v2 (now-16d) = 48h. Median = 72h.
    approx(m["lead_median_h"].as_f64().unwrap(), 72.0);
    // v1 failed (hotfix lands before v2); recovery v1→v2 = 4d = 96h.
    approx(m["cfr"].as_f64().unwrap(), 0.5);
    approx(m["mttr_h"].as_f64().unwrap(), 96.0);
    assert_eq!(m["unrecovered"], 0);
    assert_eq!(m["counts"]["feature"], 1);
    assert_eq!(m["counts"]["hotfix"], 1);
    assert_eq!(m["counts"]["release"], 0);
    assert_eq!(m["tiers"]["cfr"], "low");
    assert_eq!(m["tiers"]["overall"], "low");
    let weekly = m["weekly"].as_array().unwrap();
    let dep_sum: i64 = weekly.iter().map(|w| w["deploys"].as_i64().unwrap()).sum();
    let hot_sum: i64 = weekly.iter().map(|w| w["hotfix"].as_i64().unwrap()).sum();
    assert_eq!(dep_sum, 2);
    assert_eq!(hot_sum, 1);
    // CFR 50% → the critical suggestion fires.
    let sugg = m["suggestions"].as_array().unwrap();
    assert!(
        sugg.iter().any(|s| s["severity"] == "critical"),
        "expected critical suggestion: {sugg:?}"
    );

    // -- route level (env is process-global: single test fn) ----------------
    let data_dir = base.join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::env::set_var("OTTO_PLUGIN_DATA_DIR", &data_dir);

    // Mock host API serving /repos.
    let server = tiny_http::Server::http("127.0.0.1:0").unwrap();
    let port = server.server_addr().to_ip().unwrap().port();
    let repo_path = repo.to_str().unwrap().to_string();
    std::thread::spawn(move || {
        for req in server.incoming_requests() {
            let body = if req.url() == "/repos" {
                json!([{ "id": "r1", "name": "fix", "path": repo_path, "remote_url": null }])
            } else {
                json!({ "error": "unexpected" })
            };
            let data = serde_json::to_vec(&body).unwrap();
            let _ = req.respond(
                tiny_http::Response::from_data(data).with_header(
                    tiny_http::Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
                        .unwrap(),
                ),
            );
        }
    });
    std::env::set_var("OTTO_HOST_API", format!("http://127.0.0.1:{port}"));
    std::env::set_var("OTTO_PLUGIN_TOKEN", "test-token");

    let (code, health) = routes::handle("GET", "/health", "", "");
    assert_eq!((code, health["ok"].as_bool()), (200, Some(true)));

    // /metrics by name, by id, and aggregate.
    for needle in ["fix", "r1", "all"] {
        let (code, v) = routes::handle("GET", "/metrics", &format!("repo={needle}&days=28"), "");
        assert_eq!(code, 200, "repo={needle}: {v}");
        approx(v["df_per_week"].as_f64().unwrap(), 0.5);
    }
    let (code, v) = routes::handle("GET", "/metrics", "repo=all&days=28", "");
    assert_eq!(code, 200);
    assert_eq!(v["repo_name"], "All repos");

    let (code, _) = routes::handle("GET", "/metrics", "repo=nope&days=28", "");
    assert_eq!(code, 404);

    // Config round-trip + validation + effect on metrics.
    let (code, v) = routes::handle("GET", "/config", "", "");
    assert_eq!(code, 200);
    assert_eq!(v["deploy_tag_pattern"], "deploy");

    let (code, v) = routes::handle("PUT", "/config", "", r#"{"deploy_tag_pattern":""}"#);
    assert_eq!(code, 400, "{v}");
    let (code, v) = routes::handle("PUT", "/config", "", r#"{"scan_depth":1}"#);
    assert_eq!(code, 400, "{v}");
    let (code, v) = routes::handle("PUT", "/config", "", "not json");
    assert_eq!(code, 400, "{v}");

    let (code, v) = routes::handle("PUT", "/config", "", r#"{"deploy_tag_pattern":"prod-"}"#);
    assert_eq!(code, 200, "{v}");
    assert_eq!(v["deploy_tag_pattern"], "prod-");
    assert_eq!(v["scan_depth"], 5000, "defaults filled");

    // With pattern `prod-` the fixture has no deploys → null tiers + critical
    // no-deploys suggestion.
    let (code, v) = routes::handle("GET", "/metrics", "repo=fix&days=28", "");
    assert_eq!(code, 200);
    assert!(v["tiers"]["overall"].is_null());
    let sugg = v["suggestions"].as_array().unwrap();
    assert_eq!(sugg[0]["severity"], "critical");

    let (code, _) = routes::handle("GET", "/nope", "", "");
    assert_eq!(code, 404);

    let _ = std::fs::remove_dir_all(&base);
}
