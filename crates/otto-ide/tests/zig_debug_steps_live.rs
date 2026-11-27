//! Live Zig Debug All Tests (rc.6 A4): a `build.zig` project with three test
//! suites (lib, exe, extra), compiled once through Otto's no-exec build-runner
//! hook and debugged one suite per step with the real Zig and LLDB, through the
//! IDE HTTP routes.
//!
//! Run by hand: `OTTO_LIVE_ZIG=<dir with zig 0.16> cargo test -p otto-ide --test
//! zig_debug_steps_live -- --ignored --nocapture` (needs Xcode's `lldb-dap`).

#![allow(clippy::disallowed_methods)] // fixture copies in a live test

mod common;

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use axum::http::Method;
use axum::Router;
use common::{build_test_app, send_json};
use serde_json::{json, Value};

struct Live {
    app: Router,
    root: PathBuf,
    zig: String,
    env: Value,
    _tmp: tempfile::TempDir,
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            copy_dir(&p, &to.join(e.file_name()));
        } else {
            std::fs::copy(&p, to.join(e.file_name())).unwrap();
        }
    }
}

async fn live() -> Option<Live> {
    let Ok(zig) = std::env::var("OTTO_LIVE_ZIG") else {
        eprintln!("skip: set OTTO_LIVE_ZIG");
        return None;
    };
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("suites");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/zig-multi-suites"),
        &root,
    );
    let (_ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("state.db")).await;
    let env = json!({
        "ZIG_GLOBAL_CACHE_DIR": tmp.path().join("zig-global").to_string_lossy(),
        "ZIG_LOCAL_CACHE_DIR": tmp.path().join("zig-local").to_string_lossy(),
    });
    Some(Live {
        app,
        root,
        zig,
        env,
        _tmp: tmp,
    })
}

impl Live {
    fn path(&self, rel: &str) -> String {
        self.root.join(rel).to_string_lossy().into_owned()
    }

    fn bp(&self, rel: &str, line: u32) -> Value {
        json!({ "path": self.path(rel), "line": line, "enabled": true })
    }

    async fn start(&self, mut launch: Value, bps: Vec<Value>, exceptions: bool) -> String {
        launch["env"] = self.env.clone();
        launch["cwd"] = json!(self.root.to_string_lossy());
        launch["sdk_path"] = json!(self.zig);
        let body = json!({
            "target": { "kind": "local", "root": self.root.to_string_lossy() },
            "language": "zig",
            "launch": launch,
            "breakpoints": bps,
            "break_on_exceptions": exceptions,
        });
        let (status, resp) =
            send_json(&self.app, Method::POST, "/ide/debug/sessions", Some(body)).await;
        assert!(status.is_success(), "start: {status} {resp}");
        resp["session"]["id"].as_str().unwrap().to_string()
    }

    async fn snap(&self, id: &str) -> Value {
        let (_, resp) = send_json(
            &self.app,
            Method::GET,
            &format!("/ide/debug/sessions/{id}"),
            None,
        )
        .await;
        resp["session"].clone()
    }

    async fn command(&self, id: &str, action: &str) -> Value {
        let (status, resp) = send_json(
            &self.app,
            Method::POST,
            &format!("/ide/debug/sessions/{id}/command"),
            Some(json!({ "action": action })),
        )
        .await;
        assert!(status.is_success(), "{action}: {status} {resp}");
        resp["session"].clone()
    }

    async fn set_bps(&self, id: &str, bps: Vec<Value>) {
        let (status, resp) = send_json(
            &self.app,
            Method::PUT,
            &format!("/ide/debug/sessions/{id}/breakpoints"),
            Some(json!({ "breakpoints": bps })),
        )
        .await;
        assert!(status.is_success(), "breakpoints: {status} {resp}");
    }

    async fn next_stop(&self, id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(400);
        loop {
            let s = self.snap(id).await;
            if matches!(s["state"].as_str(), Some("paused" | "terminated" | "error")) {
                return s;
            }
            assert!(Instant::now() < deadline, "no stop: {}", brief(&s));
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// The fixture's test binaries still running (they live in this run's Zig cache).
    fn test_processes(&self) -> Vec<String> {
        let out = std::process::Command::new("ps")
            .args(["-axo", "pid=,command="])
            .output()
            .unwrap();
        let cache = self._tmp.path().to_string_lossy().into_owned();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| l.contains(&cache) && l.contains("/o/") && !l.contains("lldb"))
            .map(str::to_string)
            .collect()
    }
}

fn brief(s: &Value) -> String {
    format!(
        "state={} step={} paused={}:{} reason={} error={} steps={}",
        s["state"],
        s["current_step"],
        file_of(s),
        s["paused_line"],
        s["stopped_reason"],
        s["error"],
        s["steps"]
    )
}

fn file_of(s: &Value) -> String {
    s["paused_file"]
        .as_str()
        .unwrap_or("")
        .rsplit('/')
        .next()
        .unwrap_or("")
        .to_string()
}

fn console(s: &Value) -> String {
    s["output"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|o| o["text"].as_str())
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[ignore = "live: needs Zig 0.16 (OTTO_LIVE_ZIG) and lldb-dap"]
async fn zig_debug_all_tests_runs_one_step_per_suite() {
    let Some(l) = live().await else { return };
    let all = json!({ "program": l.root.to_string_lossy(), "is_test": true });

    // Z1 + Z2 + Z8: three steps in emission order (src/main.zig's suite last), a
    // breakpoint in root.zig stops in the lib and the exe suite, results per step.
    let id = l
        .start(all.clone(), vec![l.bp("src/root.zig", 4)], false)
        .await;
    let mut stops: Vec<(u64, String)> = Vec::new();
    let end = loop {
        let s = l.next_stop(&id).await;
        if s["state"] != "paused" {
            break s;
        }
        stops.push((
            s["current_step"].as_u64().unwrap_or(99),
            format!("{}:{}", file_of(&s), s["paused_line"]),
        ));
        l.command(&id, "continue").await;
    };
    let text = console(&end);
    eprintln!("Z1 stops (step, where): {stops:?}");
    eprintln!("Z1 end: {}", brief(&end));
    for line in text.lines().filter(|l| {
        l.starts_with("── Step")
            || l.contains(" steps ·")
            || l.contains("passed")
            || l.contains("FAIL")
            || l.contains("retried")
    }) {
        eprintln!("console: {line}");
    }
    assert_eq!(end["state"], "terminated", "{}", brief(&end));
    assert_eq!(end["title"], "Debug all tests · 3 test suites");
    let labels: Vec<&str> = end["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["label"].as_str().unwrap())
        .collect();
    assert_eq!(
        labels,
        [
            "test (src/root.zig)",
            "test (src/extra.zig)",
            "test (src/main.zig)"
        ]
    );
    assert!(stops.contains(&(0, "root.zig:4".into())), "{stops:?}");
    assert!(
        stops.contains(&(2, "root.zig:4".into())),
        "the exe suite calls add too: {stops:?}"
    );
    let st = |i: usize| {
        (
            end["steps"][i]["state"].as_str().unwrap().to_string(),
            end["steps"][i]["summary"]
                .as_str()
                .unwrap_or("")
                .to_string(),
        )
    };
    assert_eq!(st(0), ("passed".into(), "2 passed".into()));
    assert_eq!(st(1), ("failed".into(), "1 of 3 failed".into()));
    assert!(
        end["steps"][1]["failed_tests"]
            .to_string()
            .contains("extra fails"),
        "{}",
        end["steps"][1]
    );
    assert_eq!(st(2), ("passed".into(), "1 passed".into()));
    assert!(
        !text.contains("retried with macOS 15.0"),
        "Z8: the default Zig links without the target workaround"
    );

    // Z3: a breakpoint added in extra.zig while paused in step 1 stops in step 2 (the extra suite).
    let id = l
        .start(all.clone(), vec![l.bp("src/root.zig", 4)], false)
        .await;
    let s = l.next_stop(&id).await;
    assert_eq!(s["current_step"].as_u64(), Some(0), "{}", brief(&s));
    l.set_bps(&id, vec![l.bp("src/extra.zig", 10)]).await;
    l.command(&id, "continue").await;
    let s = l.next_stop(&id).await;
    eprintln!("Z3: {}", brief(&s));
    assert_eq!(
        (
            s["current_step"].as_u64(),
            file_of(&s),
            s["paused_line"].as_u64()
        ),
        (Some(1), "extra.zig".into(), Some(10)),
        "{}",
        brief(&s)
    );

    // Z5: Skip during step 2 starts step 3; Stop ends the run; no test binary is left.
    l.command(&id, "skip_step").await;
    let s = l.next_stop(&id).await;
    eprintln!("Z5 after skip: {}", brief(&s));
    assert_eq!(s["steps"][1]["state"], "skipped");
    if s["state"] == "paused" {
        assert_eq!(s["current_step"].as_u64(), Some(2));
        let stopped = l.command(&id, "stop").await;
        assert_eq!(stopped["state"], "terminated");
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(
        l.test_processes().is_empty(),
        "left running: {:?}",
        l.test_processes()
    );

    // Z4: with -Dpanic=true, the extra suite stops on the Zig panic exception breakpoint.
    let mut panicking = all.clone();
    panicking["build_flags"] = json!(["-Dpanic=true"]);
    let id = l.start(panicking, vec![], true).await;
    let s = l.next_stop(&id).await;
    eprintln!("Z4: {}", brief(&s));
    assert_eq!(s["state"], "paused", "{}", brief(&s));
    assert_eq!(s["current_step"].as_u64(), Some(1), "{}", brief(&s));
    let frames = s["stack_frames"].to_string();
    assert!(
        frames.contains("extra panics"),
        "the stack shows the test: {frames}"
    );
    l.command(&id, "stop").await;

    // Z6: a gutter Debug of one test in root.zig is one session with no steps.
    let one =
        json!({ "program": l.path("src/root.zig"), "is_test": true, "test_name": "basic add" });
    let id = l.start(one, vec![l.bp("src/root.zig", 4)], false).await;
    let s = l.next_stop(&id).await;
    eprintln!("Z6: {}", brief(&s));
    assert_eq!(
        (s["state"].as_str(), file_of(&s)),
        (Some("paused"), "root.zig".into()),
        "{}",
        brief(&s)
    );
    assert!(s["steps"].as_array().is_none_or(|a| a.is_empty()));
    l.command(&id, "stop").await;
}
