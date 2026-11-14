//! Live end-to-end debugger integration tests across all 6 debuggable languages:
//! 1. Go (`dlv dap`) — macOS `/var/folders` <-> `/private/var/folders` canonicalization,
//!    `StepInto` `go.mod` `replace` sibling repo, and live mid-run `PUT .../breakpoints`.
//! 2. Java (`java-jdwp-dap` binary JDWP client) — multi-module Maven project (`toolkit` + `service-app`),
//!    pre-launch & mid-run breakpoints, `int`/`String`/object field expansion, `evaluate`, and `StepInto`.
//! 3. Python (`py_bridge`) — `/var/folders` symlink canonicalization, locals + dict expansion,
//!    `evaluate`, `StepOver`, and live mid-run breakpoints.
//! 4. JavaScript & TypeScript (`node_bridge` V8 Inspector WebSocket CDP) — both `.js` and `.ts`
//!    (`--experimental-strip-types`), object property expansion, `evaluate`, `StepOver`, and `Continue`.
//! 5. Rust (`lldb-python-dap` / `codelldb` / `lldb-dap`) — `cargo build --message-format=json`
//!    artifact discovery, breakpoint hit, locals, `StepOver`, and `Continue`.
//! 6. Shell (`shell_bridge`) — both `bash` (`DEBUG` trap) and `zsh` (`TRAPDEBUG` + `funcfiletrace`),
//!    variable inspection, `evaluate`, and live mid-run breakpoints.

mod common;

use std::path::Path;
use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use axum::Router;
use common::{build_test_app, send_json};
use serde_json::{json, Value};

fn extended_path() -> String {
    let cur = std::env::var("PATH").unwrap_or_default();
    let home = std::env::var("HOME").unwrap_or_default();
    format!("{home}/go/bin:/opt/homebrew/bin:/usr/local/bin:/usr/bin:/bin:{cur}")
}

fn find_bin(name: &str) -> Option<String> {
    for dir in extended_path().split(':').filter(|s| !s.is_empty()) {
        let cand = Path::new(dir).join(name);
        if cand.is_file() {
            return Some(cand.to_string_lossy().into_owned());
        }
    }
    None
}

fn all_vars(snap: &Value) -> Vec<&Value> {
    snap["scopes"]
        .as_array()
        .map(|scopes| {
            scopes
                .iter()
                .filter_map(|sc| sc["variables"].as_array())
                .flat_map(|v| v.iter())
                .collect()
        })
        .unwrap_or_default()
}

fn joined_output(snap: &Value) -> String {
    snap["output"]
        .as_array()
        .map(|arr| {
            arr.iter()
                .filter_map(|e| e["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .unwrap_or_default()
}

async fn wait_until_paused(app: &Router, sid: &str, timeout: Duration) -> Value {
    let deadline = Instant::now() + timeout;
    loop {
        let (st, body) = send_json(
            app,
            Method::GET,
            &format!("/ide/debug/sessions/{sid}"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let sess = &body["session"];
        match sess["state"].as_str().unwrap_or("") {
            "paused" => return sess.clone(),
            "terminated" | "error" => {
                panic!("debug session {sid} exited before pausing: {sess}");
            }
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for debug session {sid} to pause; snapshot={sess}"
        );
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
}

async fn wait_until_output_contains(app: &Router, sid: &str, marker: &str, timeout: Duration) {
    let deadline = Instant::now() + timeout;
    loop {
        let (st, body) = send_json(
            app,
            Method::GET,
            &format!("/ide/debug/sessions/{sid}"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let sess = &body["session"];
        if joined_output(sess).contains(marker) {
            return;
        }
        if matches!(sess["state"].as_str(), Some("terminated" | "error")) {
            panic!("debug session {sid} exited before emitting '{marker}': {sess}");
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for '{marker}' in debug session {sid}; snapshot={sess}"
        );
        tokio::time::sleep(Duration::from_millis(40)).await;
    }
}

async fn wait_until_terminated(app: &Router, sid: &str, timeout: Duration) -> Value {
    let deadline = Instant::now() + timeout;
    loop {
        let (st, body) = send_json(
            app,
            Method::GET,
            &format!("/ide/debug/sessions/{sid}"),
            None,
        )
        .await;
        assert_eq!(st, StatusCode::OK);
        let sess = &body["session"];
        if matches!(sess["state"].as_str(), Some("terminated" | "error")) {
            return sess.clone();
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for debug session {sid} to terminate; snapshot={sess}"
        );
        tokio::time::sleep(Duration::from_millis(60)).await;
    }
}

async fn send_cmd(app: &Router, sid: &str, action: &str) {
    let (st, body) = send_json(
        app,
        Method::POST,
        &format!("/ide/debug/sessions/{sid}/command"),
        Some(json!({ "action": action })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "debug command {action} failed: {body}");
}

async fn set_breakpoints(app: &Router, sid: &str, path: &str, lines: &[u32]) {
    let bps: Vec<Value> = lines
        .iter()
        .map(|&line| json!({ "path": path, "line": line }))
        .collect();
    let (st, body) = send_json(
        app,
        Method::PUT,
        &format!("/ide/debug/sessions/{sid}/breakpoints"),
        Some(json!({ "breakpoints": bps })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "set_breakpoints failed: {body}");
}

#[tokio::test]
async fn live_go_dlv_dap_breakpoints_step_into_replace_module_and_midrun_breakpoints() {
    if find_bin("go").is_none() || find_bin("dlv").is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    // Keep the uncanonicalized `/var/folders/...` path from `tempdir()` to exercise symlink normalization.
    let app_dir = tmp.path().join("go-app");
    let dep_dir = tmp.path().join("localdep");
    std::fs::create_dir_all(&app_dir).unwrap();
    std::fs::create_dir_all(&dep_dir).unwrap();

    std::fs::write(
        dep_dir.join("go.mod"),
        "module example.com/localdep\n\ngo 1.21\n",
    )
    .unwrap();
    std::fs::write(
        dep_dir.join("lib.go"),
        "package localdep\n\nfunc Compute(x int) int {\n\ty := x * 2\n\treturn y\n}\n",
    )
    .unwrap();

    std::fs::write(
        app_dir.join("go.mod"),
        "module example.com/goapp\n\ngo 1.21\n\nrequire example.com/localdep v0.0.0\nreplace example.com/localdep => ../localdep\n",
    )
    .unwrap();
    let main_go = app_dir.join("main.go");
    std::fs::write(
        &main_go,
        r#"package main

import (
	"fmt"
	"time"
	"example.com/localdep"
)

func main() {
	base := 21
	doubled := localdep.Compute(base)
	fmt.Println("LOOP_READY")
	for i := 0; i < 200; i++ {
		time.Sleep(35 * time.Millisecond)
		doubled += 1
	}
	fmt.Printf("GO_DONE:%d\n", doubled)
}
"#,
    )
    .unwrap();

    let (_ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let app_str = app_dir.to_string_lossy().into_owned();
    let main_str = main_go.to_string_lossy().into_owned();

    let _ = send_json(
        &app,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": app_str },
            "env_vars": [{ "key": "PATH", "value": extended_path(), "secret": false }],
        })),
    )
    .await;

    let (st, start) = send_json(
        &app,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": app_str },
            "language": "go",
            "launch": { "program": main_str, "cwd": app_str },
            "breakpoints": [{ "path": main_str, "line": 11 }],
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "Go start_debug failed: {start}");
    let sid = start["session"]["id"].as_str().unwrap().to_string();

    // 1. Pauses at line 11 (`doubled := localdep.Compute(base)`):
    let snap1 = wait_until_paused(&app, &sid, Duration::from_secs(25)).await;
    assert_eq!(
        snap1["paused_line"], 11,
        "unexpected Go paused_line: {snap1}"
    );
    let vars1 = all_vars(&snap1);
    assert!(
        vars1
            .iter()
            .any(|v| v["name"] == "base" && v["value"] == "21"),
        "expected base=21 in Go locals, got {vars1:?}"
    );

    // 2. `StepInto` jumps into `../localdep/lib.go`:
    send_cmd(&app, &sid, "step_into").await;
    let snap2 = wait_until_paused(&app, &sid, Duration::from_secs(10)).await;
    assert!(
        snap2["paused_file"]
            .as_str()
            .unwrap_or("")
            .ends_with("localdep/lib.go"),
        "expected step_into localdep/lib.go, got {snap2}"
    );

    // 3. Resume until `LOOP_READY` confirms loop entry, then set a live mid-run breakpoint on line 15 (`doubled += 1`):
    send_cmd(&app, &sid, "continue").await;
    wait_until_output_contains(&app, &sid, "LOOP_READY", Duration::from_secs(10)).await;
    set_breakpoints(&app, &sid, &main_str, &[15]).await;
    let snap3 = wait_until_paused(&app, &sid, Duration::from_secs(10)).await;
    assert_eq!(
        snap3["paused_line"], 15,
        "expected mid-run Go pause at line 15: {snap3}"
    );

    // Clear breakpoints and continue to clean exit:
    set_breakpoints(&app, &sid, &main_str, &[]).await;
    send_cmd(&app, &sid, "continue").await;
    let end = wait_until_terminated(&app, &sid, Duration::from_secs(15)).await;
    assert_eq!(
        end["state"], "terminated",
        "Go session ended in error: {end}"
    );
    let out = joined_output(&end);
    assert!(out.contains("GO_DONE:"), "missing GO_DONE output: {out}");
}

#[tokio::test]
async fn live_java_jdwp_multi_module_breakpoints_locals_fields_eval_step_and_midrun() {
    if find_bin("java").is_none() || find_bin("javac").is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let toolkit = tmp.path().join("toolkit");
    let app_dir = tmp.path().join("service-app");
    std::fs::create_dir_all(toolkit.join("src/main/java/com/acme/tk")).unwrap();
    std::fs::create_dir_all(app_dir.join("src/main/java/com/acme/app")).unwrap();

    std::fs::write(
        toolkit.join("pom.xml"),
        "<project><modelVersion>4.0.0</modelVersion><groupId>com.acme</groupId><artifactId>toolkit</artifactId><version>1.0.0</version></project>\n",
    )
    .unwrap();
    let calc_java = toolkit.join("src/main/java/com/acme/tk/Calculator.java");
    std::fs::write(
        &calc_java,
        "package com.acme.tk;\n\npublic class Calculator {\n    public static int doubleIt(int n) {\n        int r = n * 2;\n        return r;\n    }\n}\n",
    )
    .unwrap();

    std::fs::write(
        app_dir.join("pom.xml"),
        "<project><modelVersion>4.0.0</modelVersion><groupId>com.acme</groupId><artifactId>service-app</artifactId><version>1.0.0</version><dependencies><dependency><groupId>com.acme</groupId><artifactId>toolkit</artifactId><version>1.0.0</version></dependency></dependencies></project>\n",
    )
    .unwrap();
    let main_java = app_dir.join("src/main/java/com/acme/app/AppMain.java");
    std::fs::write(
        &main_java,
        r#"package com.acme.app;

import com.acme.tk.Calculator;

public class AppMain {
    static class OrderItem {
        final String sku;
        final int qty;
        OrderItem(String sku, int qty) { this.sku = sku; this.qty = qty; }
    }

    public static void main(String[] args) throws Exception {
        int count = 42;
        String label = "otto";
        OrderItem item = new OrderItem("PRO-1", 3);
        int result = Calculator.doubleIt(count);
        System.out.println("LOOP_READY"); System.out.flush();
        for (int i = 0; i < 200; i++) {
            Thread.sleep(35);
            result += 0;
        }
        System.out.println("JAVA_DONE:" + label + ":" + result + ":" + item.sku);
    }
}
"#,
    )
    .unwrap();

    let (_ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let svc_str = app_dir.to_string_lossy().into_owned();
    let tk_str = toolkit.to_string_lossy().into_owned();
    let main_str = main_java.to_string_lossy().into_owned();

    let (st, start) = send_json(
        &app,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": svc_str },
            "project_folders": [tk_str],
            "language": "java",
            "launch": { "program": main_str, "cwd": svc_str },
            "breakpoints": [{ "path": main_str, "line": 16 }],
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "Java start_debug failed: {start}");
    let sid = start["session"]["id"].as_str().unwrap().to_string();

    // 1. Pauses at line 16 (`int result = Calculator.doubleIt(count);`):
    let snap1 = wait_until_paused(&app, &sid, Duration::from_secs(20)).await;
    assert_eq!(
        snap1["paused_line"], 16,
        "unexpected Java paused_line: {snap1}"
    );
    let vars = all_vars(&snap1);
    assert!(
        vars.iter()
            .any(|v| v["name"] == "count" && v["value"] == "42"),
        "vars={vars:?}"
    );
    assert!(
        vars.iter()
            .any(|v| v["name"] == "label" && v["value"].as_str().unwrap_or("").contains("otto")),
        "vars={vars:?}"
    );

    // Expand `item` object fields (`sku`, `qty`) via `POST /ide/debug/sessions/{id}/variables`:
    let item_ref = vars
        .iter()
        .find(|v| v["name"] == "item")
        .and_then(|v| v["variables_reference"].as_i64())
        .unwrap_or(0);
    assert!(
        item_ref > 0,
        "expected expandable object reference for item: {vars:?}"
    );
    let (st_vars, vars_body) = send_json(
        &app,
        Method::POST,
        &format!("/ide/debug/sessions/{sid}/variables"),
        Some(json!({ "variables_reference": item_ref })),
    )
    .await;
    assert_eq!(st_vars, StatusCode::OK);
    let fields = vars_body["variables"].as_array().unwrap();
    assert!(
        fields
            .iter()
            .any(|f| f["name"] == "qty" && f["value"] == "3"),
        "fields={fields:?}"
    );

    // Evaluate `count + 8` via `POST /ide/debug/sessions/{id}/evaluate`:
    let (st_ev, ev_body) = send_json(
        &app,
        Method::POST,
        &format!("/ide/debug/sessions/{sid}/evaluate"),
        Some(json!({ "expression": "count + 8" })),
    )
    .await;
    assert_eq!(st_ev, StatusCode::OK);
    assert_eq!(ev_body["result"], "50");

    // 2. `StepInto` jumps into sibling `toolkit` `Calculator.doubleIt`:
    send_cmd(&app, &sid, "step_into").await;
    let snap2 = wait_until_paused(&app, &sid, Duration::from_secs(10)).await;
    assert!(
        snap2["paused_file"]
            .as_str()
            .unwrap_or("")
            .ends_with("Calculator.java"),
        "expected step_into Calculator.java, got {snap2}"
    );

    // 3. Resume until `LOOP_READY` confirms loop entry, then set live mid-run breakpoint on line 20 (`result += 0;`):
    send_cmd(&app, &sid, "continue").await;
    wait_until_output_contains(&app, &sid, "LOOP_READY", Duration::from_secs(10)).await;
    set_breakpoints(&app, &sid, &main_str, &[20]).await;
    let snap3 = wait_until_paused(&app, &sid, Duration::from_secs(10)).await;
    assert_eq!(snap3["paused_line"], 20);

    set_breakpoints(&app, &sid, &main_str, &[]).await;
    send_cmd(&app, &sid, "continue").await;
    let end = wait_until_terminated(&app, &sid, Duration::from_secs(15)).await;
    assert_eq!(
        end["state"], "terminated",
        "Java session ended in error: {end}"
    );
    let out = joined_output(&end);
    assert!(
        out.contains("JAVA_DONE:otto:84:PRO-1"),
        "missing JAVA_DONE output: {out}"
    );
    assert!(
        !app_dir.join(".otto-java-debug-classes").exists()
            && !toolkit.join(".otto-java-debug-classes").exists(),
        "Java debug compile must write under data_dir, never inside user repo cwd"
    );
}

#[tokio::test]
async fn live_python_bdb_bridge_breakpoints_locals_eval_step_and_midrun() {
    if find_bin("python3").is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("py-app");
    std::fs::create_dir_all(&ws).unwrap();
    let script = ws.join("worker.py");
    std::fs::write(
        &script,
        r#"import time

def run_job():
    amount = 42
    payload = {"env": "prod"}
    doubled = amount * 2
    print("LOOP_READY", flush=True)
    for i in range(200):
        time.sleep(0.035)
        doubled += 0
    print(f"PY_DONE:{doubled}:{payload['env']}", flush=True)

if __name__ == "__main__":
    run_job()
"#,
    )
    .unwrap();

    let (_ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let ws_str = ws.to_string_lossy().into_owned();
    let script_str = script.to_string_lossy().into_owned();

    let (st, start) = send_json(
        &app,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "language": "python",
            "launch": { "program": script_str, "cwd": ws_str },
            "breakpoints": [{ "path": script_str, "line": 6 }],
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "Python start_debug failed: {start}");
    let sid = start["session"]["id"].as_str().unwrap().to_string();

    let snap1 = wait_until_paused(&app, &sid, Duration::from_secs(15)).await;
    assert_eq!(snap1["paused_line"], 6);
    let vars1 = all_vars(&snap1);
    assert!(
        vars1
            .iter()
            .any(|v| v["name"] == "amount" && v["value"] == "42"),
        "vars1={vars1:?}"
    );

    let (st_ev, ev_body) = send_json(
        &app,
        Method::POST,
        &format!("/ide/debug/sessions/{sid}/evaluate"),
        Some(json!({ "expression": "amount * 2" })),
    )
    .await;
    assert_eq!(st_ev, StatusCode::OK);
    assert_eq!(ev_body["result"], "84");

    send_cmd(&app, &sid, "step_over").await;
    let snap2 = wait_until_paused(&app, &sid, Duration::from_secs(10)).await;
    assert_eq!(snap2["paused_line"], 7);

    // Resume until `LOOP_READY` confirms loop entry, then set live mid-run breakpoint on line 10 (`doubled += 0`):
    send_cmd(&app, &sid, "continue").await;
    wait_until_output_contains(&app, &sid, "LOOP_READY", Duration::from_secs(10)).await;
    set_breakpoints(&app, &sid, &script_str, &[10]).await;
    let snap3 = wait_until_paused(&app, &sid, Duration::from_secs(10)).await;
    assert_eq!(snap3["paused_line"], 10);

    set_breakpoints(&app, &sid, &script_str, &[]).await;
    send_cmd(&app, &sid, "continue").await;
    let end = wait_until_terminated(&app, &sid, Duration::from_secs(15)).await;
    assert_eq!(
        end["state"], "terminated",
        "Python session ended in error: {end}"
    );
    let out = joined_output(&end);
    assert!(out.contains("PY_DONE:84:prod"), "missing PY_DONE: {out}");
}

#[tokio::test]
async fn live_node_inspector_cdp_bridge_debugs_both_js_and_typescript() {
    let Some(node_bin) = find_bin("node") else {
        return;
    };
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("node-app");
    std::fs::create_dir_all(&ws).unwrap();

    let (_ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let ws_str = ws.to_string_lossy().into_owned();
    let _ = send_json(
        &app,
        Method::PUT,
        "/ide/environments",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "env_vars": [{ "key": "PATH", "value": extended_path(), "secret": false }],
        })),
    )
    .await;

    // 1. JavaScript (`app.js`):
    let js_file = ws.join("app.js");
    std::fs::write(
        &js_file,
        "function compute() {\n  const count = 42;\n  const user = { role: 'admin' };\n  const next = count + 1;\n  console.log('JS_DONE:' + next + ':' + user.role);\n}\ncompute();\n",
    )
    .unwrap();
    run_single_node_debug_assertion(
        &app,
        &ws_str,
        &js_file.to_string_lossy(),
        "javascript",
        4,
        "JS_DONE:43:admin",
    )
    .await;

    // 2. TypeScript (`app.ts` via Node 22+ `--experimental-strip-types`):
    let ver_out = tokio::process::Command::new(&node_bin)
        .arg("--version")
        .output()
        .await
        .ok();
    let node_major = ver_out
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| {
            s.trim()
                .trim_start_matches('v')
                .split('.')
                .next()?
                .parse::<u32>()
                .ok()
        })
        .unwrap_or(0);
    if node_major >= 22 {
        let ts_file = ws.join("app.ts");
        std::fs::write(
            &ts_file,
            "interface User { role: string }\nfunction computeTs(): void {\n  const count: number = 42;\n  const user: User = { role: 'admin' };\n  const next: number = count + 1;\n  console.log('TS_DONE:' + next + ':' + user.role);\n}\ncomputeTs();\n",
        )
        .unwrap();
        run_single_node_debug_assertion(
            &app,
            &ws_str,
            &ts_file.to_string_lossy(),
            "typescript",
            5,
            "TS_DONE:43:admin",
        )
        .await;
    }
}

async fn run_single_node_debug_assertion(
    app: &Router,
    ws_str: &str,
    file_str: &str,
    lang: &str,
    bp_line: u32,
    expected_marker: &str,
) {
    let (st, start) = send_json(
        app,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "language": lang,
            "launch": { "program": file_str, "cwd": ws_str },
            "breakpoints": [{ "path": file_str, "line": bp_line }],
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "{lang} start_debug failed: {start}");
    let sid = start["session"]["id"].as_str().unwrap().to_string();

    let snap = wait_until_paused(app, &sid, Duration::from_secs(15)).await;
    assert_eq!(
        snap["paused_line"], bp_line,
        "{lang} paused_line mismatch: {snap}"
    );
    let vars = all_vars(&snap);
    assert!(
        vars.iter()
            .any(|v| v["name"] == "count" && v["value"] == "42"),
        "vars={vars:?}"
    );

    let (st_ev, ev_body) = send_json(
        app,
        Method::POST,
        &format!("/ide/debug/sessions/{sid}/evaluate"),
        Some(json!({ "expression": "count + 10" })),
    )
    .await;
    assert_eq!(st_ev, StatusCode::OK);
    assert_eq!(ev_body["result"], "52");

    send_cmd(app, &sid, "step_over").await;
    let snap2 = wait_until_paused(app, &sid, Duration::from_secs(10)).await;
    assert_eq!(snap2["paused_line"], bp_line + 1);

    send_cmd(app, &sid, "continue").await;
    let end = wait_until_terminated(app, &sid, Duration::from_secs(15)).await;
    assert_eq!(
        end["state"], "terminated",
        "{lang} session ended in error: {end}"
    );
    let out = joined_output(&end);
    assert!(
        out.contains(expected_marker),
        "missing {expected_marker}: {out}"
    );
}

#[tokio::test]
async fn live_rust_lldb_bridge_builds_cargo_artifact_hits_breakpoint_reads_locals_and_steps() {
    if find_bin("cargo").is_none() || find_bin("lldb").is_none() {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let crate_dir = tmp.path().join("rs-sample");
    std::fs::create_dir_all(crate_dir.join("src")).unwrap();
    std::fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\nname = \"rs-sample\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    let main_rs = crate_dir.join("src/main.rs");
    std::fs::write(
        &main_rs,
        "fn main() {\n    let score: i32 = 42;\n    let bonus: i32 = score + 8;\n    println!(\"RS_DONE:{bonus}\");\n}\n",
    )
    .unwrap();

    let (_ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let crate_str = crate_dir.to_string_lossy().into_owned();
    let main_str = main_rs.to_string_lossy().into_owned();

    let (st, start) = send_json(
        &app,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": crate_str },
            "language": "rust",
            "launch": { "program": main_str, "cwd": crate_str },
            "breakpoints": [{ "path": main_str, "line": 3 }],
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "Rust start_debug failed: {start}");
    let sid = start["session"]["id"].as_str().unwrap().to_string();

    let snap1 = wait_until_paused(&app, &sid, Duration::from_secs(30)).await;
    assert_eq!(
        snap1["paused_line"], 3,
        "Rust paused_line mismatch: {snap1}"
    );
    let vars1 = all_vars(&snap1);
    assert!(
        vars1
            .iter()
            .any(|v| v["name"] == "score" && v["value"].as_str().unwrap_or("").contains("42")),
        "expected score=42 in Rust locals: {vars1:?}"
    );

    send_cmd(&app, &sid, "step_over").await;
    let snap2 = wait_until_paused(&app, &sid, Duration::from_secs(10)).await;
    assert_eq!(snap2["paused_line"], 4);

    send_cmd(&app, &sid, "continue").await;
    let end = wait_until_terminated(&app, &sid, Duration::from_secs(15)).await;
    assert_eq!(
        end["state"], "terminated",
        "Rust debug session ended in error: {end}"
    );
    let out = joined_output(&end);
    assert!(
        out.contains("RS_DONE:50"),
        "expected RS_DONE:50 in Rust debug output, got: {out}"
    );
}

#[tokio::test]
async fn live_shell_bridge_debugs_both_bash_and_zsh_with_midrun_breakpoints() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("sh-app");
    std::fs::create_dir_all(&ws).unwrap();
    let (_ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let ws_str = ws.to_string_lossy().into_owned();

    for (fname, shebang) in [("run.sh", "#!/bin/bash"), ("run.zsh", "#!/bin/zsh")] {
        let script = ws.join(fname);
        std::fs::write(
            &script,
            format!(
                "{shebang}\nTARGET_PORT=7700\nNEXT_PORT=7701\necho \"LOOP_READY\"\nfor i in {{1..200}}; do\n  sleep 0.035\n  LAST_ITER=$i\ndone\necho \"SH_DONE:$TARGET_PORT\"\n"
            ),
        )
        .unwrap();
        let script_str = script.to_string_lossy().into_owned();

        let (st, start) = send_json(
            &app,
            Method::POST,
            "/ide/debug/sessions",
            Some(json!({
                "target": { "kind": "local", "root": ws_str },
                "language": "shell",
                "launch": { "program": script_str, "cwd": ws_str },
                "breakpoints": [{ "path": script_str, "line": 3 }],
            })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{fname} start_debug failed: {start}");
        let sid = start["session"]["id"].as_str().unwrap().to_string();

        let snap1 = wait_until_paused(&app, &sid, Duration::from_secs(10)).await;
        assert_eq!(snap1["paused_line"], 3, "{fname} wrong bp line: {snap1}");
        let vars1 = all_vars(&snap1);
        assert!(
            vars1.iter().any(|v| v["name"] == "TARGET_PORT"
                && v["value"].as_str().unwrap_or("").contains("7700")),
            "{fname} missing TARGET_PORT in vars: {vars1:?}"
        );

        let (st_ev, ev_body) = send_json(
            &app,
            Method::POST,
            &format!("/ide/debug/sessions/{sid}/evaluate"),
            Some(json!({ "expression": "$TARGET_PORT" })),
        )
        .await;
        assert_eq!(st_ev, StatusCode::OK);
        assert!(ev_body["result"].as_str().unwrap_or("").contains("7700"));

        // Resume until `LOOP_READY` confirms loop entry, then set live mid-run breakpoint on line 7 (`LAST_ITER=$i`):
        send_cmd(&app, &sid, "continue").await;
        wait_until_output_contains(&app, &sid, "LOOP_READY", Duration::from_secs(10)).await;
        set_breakpoints(&app, &sid, &script_str, &[7]).await;
        let snap2 = wait_until_paused(&app, &sid, Duration::from_secs(10)).await;
        assert_eq!(
            snap2["paused_line"], 7,
            "{fname} missed mid-run bp: {snap2}"
        );

        set_breakpoints(&app, &sid, &script_str, &[]).await;
        send_cmd(&app, &sid, "continue").await;
        let end = wait_until_terminated(&app, &sid, Duration::from_secs(10)).await;
        assert_eq!(end["state"], "terminated", "{fname} ended in error: {end}");
        let out = joined_output(&end);
        assert!(
            out.contains("SH_DONE:7700"),
            "{fname} missing SH_DONE: {out}"
        );
    }
}
