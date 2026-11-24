//! "Draft from the diff" on a one-file change (hotfix 0.5.1): the commit
//! card's draft must come back in seconds, never wait on an interactive
//! agent TUI, and never write into the repository. The `claude` here is a
//! stand-in with the real CLI's two faces: `-p` speaks stream-json and
//! answers at once; the TUI stops on a start-up dialog (Claude Code 2.1.331's
//! "New MCP server found in this project") and never takes a prompt — what
//! the owner's draft hit for 60 s before it failed with a 502.
use otto_server::ServerCtx;
use otto_state::{DbPool, NewRepo};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::time::{Duration, Instant};

async fn mem_pool() -> DbPool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool.into()
}

/// The stand-in `claude`: headless answers `reply` as two text deltas and
/// logs its argv + cwd to `<bin>/claude.log`; the TUI shows the dialog.
fn fake_claude(bin: &std::path::Path, headless: &str) -> String {
    std::fs::create_dir_all(bin).unwrap();
    let path = bin.join("claude");
    let log = bin.join("claude.log");
    let script = format!(
        r#"#!/bin/sh
printf '%s\n' "cwd=$(pwd)" "$@" > '{log}'
for a in "$@"; do
  if [ "$a" = "-p" ] || [ "$a" = "--print" ]; then
    read -r _prompt
{headless}
  fi
done
printf 'New MCP server found in this project: otto\n\n> Use this MCP server\n  Continue without using this MCP server\n\nEnter to confirm\n'
exec cat > /dev/null
"#,
        log = log.display(),
    );
    std::fs::write(&path, script).unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path.to_string_lossy().into_owned()
}

const ANSWER: &str = r#"    printf '%s\n' '{"type":"system","subtype":"init","model":"claude-haiku"}'
    printf '%s\n' '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"feat(app): greet the world"}}}'
    printf '%s\n' '{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"\n\n- say hello world"}}}'
    printf '%s\n' '{"type":"result","subtype":"success","is_error":false,"result":"feat(app): greet the world\n\n- say hello world"}'
    exit 0"#;

struct Fixture {
    _tmp: tempfile::TempDir,
    _server: tokio::task::JoinHandle<()>,
    endpoint: String,
    token: String,
    repo_dir: std::path::PathBuf,
    bin: std::path::PathBuf,
}

/// A real repository with one edited file (what the owner drafted from),
/// served by a real router whose `claude` provider is the stand-in.
async fn fixture(headless: &str) -> Fixture {
    let tmp = tempfile::tempdir().unwrap();
    let pool = mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let bin = tmp.path().join("bin");
    // No stand-in for the one by-hand run against the real CLI.
    if !headless.is_empty() {
        let claude = fake_claude(&bin, headless);
        ctx.manager
            .providers()
            .reload(Some(&serde_json::json!({ "claude": { "cmd": claude } })));
    }
    let owner = otto_state::UsersRepo::new(pool.clone())
        .create("owner", "unused", "Owner", true)
        .await
        .unwrap();
    let token = otto_rbac::AuthRepo::new(pool.clone())
        .issue(&owner.id)
        .await
        .unwrap();
    let repo_dir = tmp.path().join("work");
    std::fs::create_dir_all(&repo_dir).unwrap();
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(&repo_dir)
            .args(args)
            .output()
            .unwrap()
            .status
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    git(&["config", "user.email", "t@otto.local"]);
    git(&["config", "user.name", "T"]);
    git(&["config", "commit.gpgsign", "false"]);
    std::fs::write(
        repo_dir.join("app.go"),
        "package app\n\nconst Greeting = \"hello\"\n",
    )
    .unwrap();
    git(&["add", "."]);
    git(&["commit", "-q", "-m", "init"]);
    std::fs::write(
        repo_dir.join("app.go"),
        "package app\n\nconst Greeting = \"hello world\"\n",
    )
    .unwrap();
    let ws = ctx
        .workspaces
        .create("Draft", repo_dir.to_str().unwrap(), &owner.id)
        .await
        .unwrap();
    let repo = ctx
        .git_store
        .create_repo(NewRepo {
            workspace_id: ws.id.clone(),
            name: "work".into(),
            path: repo_dir.to_string_lossy().into_owned(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();
    let (api, root) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx.clone(), api, root);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!(
        "http://{}/api/v1/repos/{}/draft-commit-message",
        listener.local_addr().unwrap(),
        repo.id
    );
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    Fixture {
        _tmp: tmp,
        _server: server,
        endpoint,
        token,
        repo_dir,
        bin,
    }
}

/// The owner's report: a one-file draft "takes forever". It must take
/// seconds (here, with a CLI that answers at once, well under 1.5 s), run
/// headless with no MCP servers, tools or project files, and leave the
/// repository exactly as it was.
#[tokio::test]
async fn one_file_commit_draft_is_headless_and_fast() {
    let f = fixture(ANSWER).await;
    let started = Instant::now();
    let resp = reqwest::Client::new()
        .post(&f.endpoint)
        .bearer_auth(&f.token)
        .json(&serde_json::json!({}))
        .timeout(Duration::from_secs(90))
        .send()
        .await
        .unwrap();
    let elapsed = started.elapsed();
    let status = resp.status();
    let body: serde_json::Value = resp.json().await.unwrap_or_default();
    assert!(status.is_success(), "{status} after {elapsed:?}: {body}");
    assert_eq!(
        body["message"],
        "feat(app): greet the world\n\n- say hello world"
    );
    assert!(
        elapsed < Duration::from_millis(1500),
        "the draft took {elapsed:?}"
    );
    let log = std::fs::read_to_string(f.bin.join("claude.log")).unwrap();
    let argv: Vec<&str> = log.lines().collect();
    for flag in [
        "-p",
        "--strict-mcp-config",
        "--tools=",
        "--setting-sources=",
    ] {
        assert!(argv.contains(&flag), "{flag} missing from {argv:?}");
    }
    assert!(
        argv.windows(2).any(|w| w == ["--model", "haiku"]),
        "{argv:?}"
    );
    let repo = std::fs::canonicalize(&f.repo_dir).unwrap();
    assert!(
        !argv[0].ends_with(&*repo.to_string_lossy()),
        "ran inside the repository: {}",
        argv[0]
    );
    assert!(!f.repo_dir.join(".mcp.json").exists(), "wrote .mcp.json");
    assert!(!f.repo_dir.join(".claude").exists(), "wrote .claude/");
}

/// R1-061: the route took no body before `stream` arrived — a caller that
/// still posts none (no Content-Type) gets the JSON draft, not a 415.
#[tokio::test]
async fn a_commit_draft_with_no_body_still_answers() {
    let f = fixture(ANSWER).await;
    let resp = reqwest::Client::new()
        .post(&f.endpoint)
        .bearer_auth(&f.token)
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
    let body: serde_json::Value = resp.json().await.unwrap();
    assert_eq!(
        body["message"],
        "feat(app): greet the world\n\n- say hello world"
    );
}

/// The card's live preview: with `stream`, the route answers NDJSON — the
/// text as the model writes it (subject first, then body), then the draft.
#[tokio::test]
async fn the_commit_draft_streams_subject_first_then_body() {
    let f = fixture(ANSWER).await;
    let resp = reqwest::Client::new()
        .post(&f.endpoint)
        .bearer_auth(&f.token)
        .json(&serde_json::json!({ "stream": true }))
        .send()
        .await
        .unwrap();
    assert!(resp.status().is_success(), "{}", resp.status());
    assert_eq!(resp.headers()["content-type"], "application/x-ndjson");
    let text = resp.text().await.unwrap();
    let lines: Vec<serde_json::Value> = text
        .lines()
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(
        lines,
        [
            serde_json::json!({ "delta": "feat(app): greet the world" }),
            serde_json::json!({ "delta": "\n\n- say hello world" }),
            serde_json::json!({ "done": {
                "message": "feat(app): greet the world\n\n- say hello world",
                "from_staged": false,
                "session_id": null,
            } }),
        ]
    );
}

/// The stream is not buffered: a piece the model wrote reaches the client
/// while the rest is still being written.
#[tokio::test]
async fn the_streamed_draft_arrives_piece_by_piece() {
    let f = fixture(&ANSWER.replacen("'\n    printf", "'\n    sleep 1\n    printf", 2)).await;
    let started = Instant::now();
    let mut resp = reqwest::Client::new()
        .post(&f.endpoint)
        .bearer_auth(&f.token)
        .json(&serde_json::json!({ "stream": true }))
        .send()
        .await
        .unwrap();
    let (mut first, mut body) = (None, String::new());
    while let Some(chunk) = resp.chunk().await.unwrap() {
        body.push_str(&String::from_utf8_lossy(&chunk));
        if body.contains("greet the world") {
            first.get_or_insert_with(|| started.elapsed());
        }
    }
    let (first, done) = (first.unwrap(), started.elapsed());
    assert!(body.contains("\"done\""), "{body}");
    assert!(
        done - first > Duration::from_millis(800),
        "first {first:?}, done {done:?}"
    );
}

/// A CLI that cannot draft (signed out) fails at once with its own words —
/// as a 502 for the JSON caller, as an `error` line for the stream.
#[tokio::test]
async fn a_failed_draft_says_why_at_once() {
    let f = fixture(
        r#"    printf '%s\n' '{"type":"result","subtype":"success","is_error":true,"result":"Not logged in · Please run /login"}'
    exit 0"#,
    )
    .await;
    let http = reqwest::Client::new();
    let started = Instant::now();
    let resp = http
        .post(&f.endpoint)
        .bearer_auth(&f.token)
        .json(&serde_json::json!({}))
        .send()
        .await
        .unwrap();
    assert_eq!(resp.status(), 502);
    let body: serde_json::Value = resp.json().await.unwrap();
    assert!(
        body["message"].as_str().unwrap().contains("Not logged in"),
        "{body}"
    );
    let resp = http
        .post(&f.endpoint)
        .bearer_auth(&f.token)
        .json(&serde_json::json!({ "stream": true }))
        .send()
        .await
        .unwrap();
    let text = resp.text().await.unwrap();
    assert_eq!(
        serde_json::from_str::<serde_json::Value>(text.trim()).unwrap(),
        serde_json::json!({ "error": "Claude: Not logged in · Please run /login" })
    );
    assert!(started.elapsed() < Duration::from_secs(5));
}

/// The same one-file draft against the REAL `claude` (one haiku prompt, on
/// the signed-in account) — run by hand, never in CI:
/// `cargo nextest run -p otto-server --test it --run-ignored only -E 'test(/real_claude/)' --no-capture`
#[tokio::test]
#[ignore = "spends one real haiku prompt"]
async fn real_claude_drafts_a_one_file_change_within_five_seconds() {
    let f = fixture("").await;
    let started = Instant::now();
    let resp = reqwest::Client::new()
        .post(&f.endpoint)
        .bearer_auth(&f.token)
        .json(&serde_json::json!({ "stream": true }))
        .send()
        .await
        .unwrap();
    let mut first = None;
    let mut body = String::new();
    let mut resp = resp;
    while let Some(chunk) = resp.chunk().await.unwrap() {
        first.get_or_insert_with(|| started.elapsed());
        body.push_str(&String::from_utf8_lossy(&chunk));
    }
    let elapsed = started.elapsed();
    eprintln!("real claude: first line {first:?}, done {elapsed:?}\n{body}");
    assert!(body.contains("\"done\""), "{body}");
    assert!(elapsed < Duration::from_secs(5), "took {elapsed:?}");
}
