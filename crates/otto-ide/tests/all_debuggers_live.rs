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

use std::time::{Duration, Instant};

use axum::http::{Method, StatusCode};
use axum::Router;
use common::{build_test_app, ensure_bins_or_skip, extended_path, find_bin, send_json};
use serde_json::{json, Value};

/// Variable names only: a failure message must never print values, which
/// for the shell bridge include the whole process environment (secrets).
fn var_names<'a>(vars: &[&'a serde_json::Value]) -> Vec<&'a str> {
    vars.iter().filter_map(|v| v["name"].as_str()).collect()
}

/// A session snapshot without its variables (see [`var_names`]).
fn brief(sess: &serde_json::Value) -> String {
    format!(
        "state={} paused_line={} title={} error={}",
        sess["state"], sess["paused_line"], sess["title"], sess["error"]
    )
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

/// Budget for a session's FIRST pause: it includes the build (javac, cargo,
/// go) and the debuggee's cold start, which took over 20 s on a loaded CI
/// runner. A liveness bound, not a speed bound.
const FIRST_PAUSE: Duration = Duration::from_secs(90);

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
                panic!("debug session {sid} exited before pausing: {}", brief(sess));
            }
            _ => {}
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for debug session {sid} to pause; snapshot={}",
            brief(sess)
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
            panic!(
                "debug session {sid} exited before emitting '{marker}': {}",
                brief(sess)
            );
        }
        assert!(
            Instant::now() < deadline,
            "timed out waiting for '{marker}' in debug session {sid}; snapshot={}",
            brief(sess)
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
            "timed out waiting for debug session {sid} to terminate; snapshot={}",
            brief(sess)
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
    if !ensure_bins_or_skip(&["go", "dlv"]) {
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
    let snap1 = wait_until_paused(&app, &sid, FIRST_PAUSE).await;
    assert_eq!(
        snap1["paused_line"],
        11,
        "unexpected Go paused_line: {}",
        brief(&snap1)
    );
    let vars1 = all_vars(&snap1);
    assert!(
        vars1
            .iter()
            .any(|v| v["name"] == "base" && v["value"] == "21"),
        "expected base=21 in Go locals, got {:?}",
        var_names(&vars1)
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
    if !ensure_bins_or_skip(&["java", "javac"]) {
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
    let snap1 = wait_until_paused(&app, &sid, FIRST_PAUSE).await;
    assert_eq!(
        snap1["paused_line"],
        16,
        "unexpected Java paused_line: {}",
        brief(&snap1)
    );
    let vars = all_vars(&snap1);
    assert!(
        vars.iter()
            .any(|v| v["name"] == "count" && v["value"] == "42"),
        "vars={:?}",
        var_names(&vars)
    );
    assert!(
        vars.iter()
            .any(|v| v["name"] == "label" && v["value"].as_str().unwrap_or("").contains("otto")),
        "vars={:?}",
        var_names(&vars)
    );

    // Expand `item` object fields (`sku`, `qty`) via `POST /ide/debug/sessions/{id}/variables`:
    let item_ref = vars
        .iter()
        .find(|v| v["name"] == "item")
        .and_then(|v| v["variables_reference"].as_i64())
        .unwrap_or(0);
    assert!(
        item_ref > 0,
        "expected expandable object reference for item: {:?}",
        var_names(&vars)
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
    if !ensure_bins_or_skip(&["python3"]) {
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

    let snap1 = wait_until_paused(&app, &sid, FIRST_PAUSE).await;
    assert_eq!(snap1["paused_line"], 6);
    let vars1 = all_vars(&snap1);
    assert!(
        vars1
            .iter()
            .any(|v| v["name"] == "amount" && v["value"] == "42"),
        "vars1={:?}",
        var_names(&vars1)
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
    if !ensure_bins_or_skip(&["node"]) {
        return;
    }
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

    let snap = wait_until_paused(app, &sid, FIRST_PAUSE).await;
    assert_eq!(
        snap["paused_line"],
        bp_line,
        "{lang} paused_line mismatch: {}",
        brief(&snap)
    );
    let vars = all_vars(&snap);
    assert!(
        vars.iter()
            .any(|v| v["name"] == "count" && v["value"] == "42"),
        "vars={:?}",
        var_names(&vars)
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
    if !ensure_bins_or_skip(&["cargo", "lldb"]) {
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
            // CI strips symbols from Otto itself. The debuggee needs locals and line tables.
            "launch": { "program": main_str, "cwd": crate_str, "env": { "CARGO_PROFILE_DEV_DEBUG": "2" } },
            "breakpoints": [{ "path": main_str, "line": 3 }],
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "Rust start_debug failed: {start}");
    let sid = start["session"]["id"].as_str().unwrap().to_string();

    let snap1 = wait_until_paused(&app, &sid, FIRST_PAUSE).await;
    assert_eq!(
        snap1["paused_line"],
        3,
        "Rust paused_line mismatch: {}",
        brief(&snap1)
    );
    let vars1 = all_vars(&snap1);
    assert!(
        vars1
            .iter()
            .any(|v| v["name"] == "score" && v["value"].as_str().unwrap_or("").contains("42")),
        "expected score=42 in Rust locals: {:?}",
        var_names(&vars1)
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

    for (fname, shebang, shell) in [
        ("run.sh", "#!/bin/bash", "bash"),
        ("run.zsh", "#!/bin/zsh", "zsh"),
    ] {
        // Ubuntu CI runners ship no zsh; the bridge then rightly fails with
        // "Zsh shell ('zsh') not found", so skip that leg like other toolchains.
        if !ensure_bins_or_skip(&[shell]) {
            continue;
        }
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

        let snap1 = wait_until_paused(&app, &sid, FIRST_PAUSE).await;
        assert_eq!(
            snap1["paused_line"],
            3,
            "{fname} wrong bp line: {}",
            brief(&snap1)
        );
        let vars1 = all_vars(&snap1);
        assert!(
            vars1.iter().any(|v| v["name"] == "TARGET_PORT"
                && v["value"].as_str().unwrap_or("").contains("7700")),
            "{fname} missing TARGET_PORT in vars: {:?}",
            var_names(&vars1)
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

#[tokio::test]
async fn live_shell_bridge_stops_in_a_file_sourced_through_dotdot() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("sh-src");
    std::fs::create_dir_all(ws.join("scripts")).unwrap();
    std::fs::create_dir_all(ws.join("lib")).unwrap();
    let (_ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let ws_str = ws.to_string_lossy().into_owned();
    // The macOS temp dir is itself behind a symlink (/var → /private/var), so this also covers
    // a symlinked root.
    let lib = ws.join("lib/x.sh");
    std::fs::write(&lib, "greet() {\n  GREETED=1\n  echo \"GREET_DONE\"\n}\n").unwrap();
    let lib_str = lib.to_string_lossy().into_owned();

    for (fname, shebang, shell) in [
        ("run.sh", "#!/bin/bash", "bash"),
        ("run.zsh", "#!/bin/zsh", "zsh"),
    ] {
        if !ensure_bins_or_skip(&[shell]) {
            continue;
        }
        let script = ws.join("scripts").join(fname);
        std::fs::write(
            &script,
            format!("{shebang}\nsource \"{ws_str}/scripts/../lib/x.sh\"\ngreet\n"),
        )
        .unwrap();
        let (st, start) = send_json(
            &app,
            Method::POST,
            "/ide/debug/sessions",
            Some(json!({
                "target": { "kind": "local", "root": ws_str },
                "language": "shell",
                "launch": { "program": script.to_string_lossy(), "cwd": ws_str },
                "breakpoints": [{ "path": lib_str, "line": 2 }],
            })),
        )
        .await;
        assert_eq!(st, StatusCode::OK, "{fname} start_debug failed: {start}");
        let sid = start["session"]["id"].as_str().unwrap().to_string();
        let snap = wait_until_paused(&app, &sid, FIRST_PAUSE).await;
        assert_eq!(snap["paused_line"], 2, "{fname} wrong bp: {}", brief(&snap));
        send_cmd(&app, &sid, "continue").await;
        let end = wait_until_terminated(&app, &sid, Duration::from_secs(10)).await;
        assert!(joined_output(&end).contains("GREET_DONE"), "{fname}: {end}");
    }
}

#[tokio::test]
async fn live_zig_lldb_debugs_types_watch_set_variable_tests_and_panic() {
    if !ensure_bins_or_skip(&["zig", "lldb"]) {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("zig-sample");
    std::fs::create_dir_all(ws.join("src")).unwrap();
    let main_zig = ws.join("src/main.zig");
    std::fs::write(
        &main_zig,
        r#"const std = @import("std");

const Point = struct {
    x: i32,
    y: i32,
};

const MyErr = error{Boom};

fn add(a: i32, b: i32) i32 {
    const total = a + b;
    return total;
}

pub fn main() !void {
    var counter: i32 = 4;
    counter += 1;
    const greeting: []const u8 = "hello zig";
    const nums: []const i32 = &.{ 10, 20, 30 };
    var opt_some: ?i32 = null;
    opt_some = counter + 37;
    var opt_none: ?i32 = 1;
    opt_none = null;
    const res_ok: MyErr!i32 = if (counter > 0) counter + 2 else MyErr.Boom;
    const res_err: MyErr!i32 = if (counter < 0) counter else MyErr.Boom;
    const pt = Point{ .x = 3, .y = 4 };
    const sum = add(counter, pt.x);
    std.debug.print("ZIG_DONE:{d}:{s}:{d}:{d}:{d}:{d}\n", .{ counter, greeting, sum, nums.len, opt_some orelse -1, opt_none orelse -1 });
    _ = .{ res_ok catch 0, res_err catch 0 };
}

test "addition works" {
    const result = add(19, 23);
    try std.testing.expectEqual(@as(i32, 42), result);
}
"#,
    )
    .unwrap();

    let (_ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let ws_str = ws.to_string_lossy().into_owned();
    let main_str = main_zig.to_string_lossy().into_owned();

    // 1. Main program: breakpoint on line 27 (`const sum = add(counter, pt.x);`)
    let (st, start) = send_json(
        &app,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "language": "zig",
            "launch": { "program": main_str, "cwd": ws_str },
            "breakpoints": [{ "path": main_str, "line": 27 }],
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "Zig start_debug failed: {start}");
    let sid = start["session"]["id"].as_str().unwrap().to_string();

    let snap1 = wait_until_paused(&app, &sid, FIRST_PAUSE).await;
    assert_eq!(
        snap1["paused_line"],
        27,
        "Zig paused_line mismatch: {}",
        brief(&snap1)
    );
    let vars1 = all_vars(&snap1);
    let var_val = |name: &str| -> String {
        vars1
            .iter()
            .find(|v| v["name"] == name)
            .and_then(|v| v["value"].as_str())
            .unwrap_or("")
            .to_string()
    };
    assert!(
        var_val("counter").contains("5"),
        "expected counter=5, names={:?}",
        var_names(&vars1)
    );
    assert!(
        var_val("greeting").contains("hello zig"),
        "expected Zig []const u8 pretty-printer for greeting, got '{}'",
        var_val("greeting")
    );
    assert!(
        var_val("nums").contains("10") && var_val("nums").contains("30"),
        "expected Zig []const i32 pretty-printer for nums, got '{}'",
        var_val("nums")
    );
    assert!(
        var_val("opt_some").contains("42"),
        "expected Zig ?i32 pretty-printer for opt_some, got '{}'",
        var_val("opt_some")
    );
    assert!(
        var_val("opt_none").contains("null"),
        "expected Zig ?i32 pretty-printer for opt_none=null, got '{}'",
        var_val("opt_none")
    );
    assert!(
        var_val("res_ok").contains("7"),
        "expected Zig error union pretty-printer for res_ok, got '{}'",
        var_val("res_ok")
    );
    assert!(
        var_val("res_err").contains("error") || var_val("res_err").contains("Boom"),
        "expected Zig error union pretty-printer for res_err, got '{}'",
        var_val("res_err")
    );

    // Watch / evaluate expression
    let (st_ev, ev_body) = send_json(
        &app,
        Method::POST,
        &format!("/ide/debug/sessions/{sid}/evaluate"),
        Some(json!({ "expression": "counter + 10" })),
    )
    .await;
    assert_eq!(st_ev, StatusCode::OK, "Zig evaluate failed: {ev_body}");
    assert!(
        ev_body["result"].as_str().unwrap_or("").contains("15"),
        "expected 15 from counter + 10, got {ev_body}"
    );

    // Set variable via `POST /ide/debug/sessions/{id}/set-variable`
    let locals_ref = snap1["scopes"][0]["variables_reference"]
        .as_i64()
        .unwrap_or(0);
    assert!(
        locals_ref > 0,
        "expected Locals scope variables_reference > 0"
    );
    let (st_sv, sv_body) = send_json(
        &app,
        Method::POST,
        &format!("/ide/debug/sessions/{sid}/set-variable"),
        Some(json!({
            "variables_reference": locals_ref,
            "name": "counter",
            "value": "99",
        })),
    )
    .await;
    assert_eq!(st_sv, StatusCode::OK, "Zig set-variable failed: {sv_body}");
    assert!(
        sv_body["value"].as_str().unwrap_or("").contains("99"),
        "expected 99 after set-variable, got {sv_body}"
    );

    // Step into `add(counter, pt.x)` and step over
    send_cmd(&app, &sid, "step_into").await;
    let snap2 = wait_until_paused(&app, &sid, Duration::from_secs(10)).await;
    let paused2 = snap2["paused_line"].as_u64().unwrap_or(0);
    assert!(
        (10..=13).contains(&paused2),
        "expected step_into inside add() (lines 10..13), got line {paused2}: {}",
        brief(&snap2)
    );

    send_cmd(&app, &sid, "continue").await;
    let end = wait_until_terminated(&app, &sid, Duration::from_secs(15)).await;
    assert_eq!(
        end["state"], "terminated",
        "Zig session ended in error: {end}"
    );
    let out = joined_output(&end);
    assert!(
        out.contains("ZIG_DONE:99:hello zig:102:3:42:-1"),
        "expected mutated counter=99 and sum=102 in Zig output, got: {out}"
    );

    // 2. Single `test "addition works"` debug via `test_name` filter
    let (st_t, start_t) = send_json(
        &app,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "language": "zig",
            "launch": {
                "program": main_str,
                "cwd": ws_str,
                "test_name": "addition works",
            },
            "breakpoints": [{ "path": main_str, "line": 34 }],
        })),
    )
    .await;
    assert_eq!(
        st_t,
        StatusCode::OK,
        "Zig test start_debug failed: {start_t}"
    );
    let sid_t = start_t["session"]["id"].as_str().unwrap().to_string();
    let snap_t = wait_until_paused(&app, &sid_t, FIRST_PAUSE).await;
    assert_eq!(
        snap_t["paused_line"],
        34,
        "Zig test paused_line mismatch: {}",
        brief(&snap_t)
    );
    let vars_t = all_vars(&snap_t);
    assert!(
        vars_t
            .iter()
            .any(|v| v["name"] == "result" && v["value"].as_str().unwrap_or("").contains("42")),
        "expected result=42 in Zig test locals: {vars_t:?}"
    );
    send_cmd(&app, &sid_t, "continue").await;
    let end_t = wait_until_terminated(&app, &sid_t, Duration::from_secs(15)).await;
    assert_eq!(
        end_t["state"], "terminated",
        "Zig test debug session ended in error: {end_t}"
    );

    // 3. Panic breakpoint (`break_on_exceptions: true` catches `@panic(...)`)
    let panic_zig = ws.join("src/panic_demo.zig");
    std::fs::write(
        &panic_zig,
        "pub fn main() void {\n    @panic(\"deliberate zig boom\");\n}\n",
    )
    .unwrap();
    let panic_str = panic_zig.to_string_lossy().into_owned();
    let (st_p, start_p) = send_json(
        &app,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "language": "zig",
            "launch": { "program": panic_str, "cwd": ws_str },
            "breakpoints": [],
            "break_on_exceptions": true,
        })),
    )
    .await;
    assert_eq!(
        st_p,
        StatusCode::OK,
        "Zig panic start_debug failed: {start_p}"
    );
    let sid_p = start_p["session"]["id"].as_str().unwrap().to_string();
    let snap_p = wait_until_paused(&app, &sid_p, FIRST_PAUSE).await;
    assert_eq!(
        snap_p["state"],
        "paused",
        "expected Zig panic session to pause on @panic: {}",
        brief(&snap_p)
    );
    send_cmd(&app, &sid_p, "stop").await;
}

#[tokio::test]
async fn live_cpp_cmake_lldb_stl_watch_set_variable_attach_and_exceptions() {
    if !ensure_bins_or_skip(&["cmake", "clang++", "lldb"]) {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("cpp-sample");
    std::fs::create_dir_all(ws.join("include")).unwrap();
    std::fs::create_dir_all(ws.join("src")).unwrap();

    std::fs::write(
        ws.join("CMakeLists.txt"),
        r#"cmake_minimum_required(VERSION 3.16)
project(cpp_sample LANGUAGES CXX)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_CXX_STANDARD_REQUIRED ON)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)

add_executable(cpp_sample src/main.cpp src/math.cpp)
target_include_directories(cpp_sample PRIVATE include)
target_compile_options(cpp_sample PRIVATE -g -O0)

enable_testing()
add_test(NAME sample_runs COMMAND cpp_sample)
"#,
    )
    .unwrap();

    let math_hpp = ws.join("include/math.hpp");
    std::fs::write(
        &math_hpp,
        "#pragma once\nnamespace demo {\nint add(int a, int b);\n}\n",
    )
    .unwrap();

    let math_cpp = ws.join("src/math.cpp");
    std::fs::write(
        &math_cpp,
        "#include \"math.hpp\"\nnamespace demo {\nint add(int a, int b) {\n    int total = a + b;\n    return total;\n}\n}\n",
    )
    .unwrap();

    let main_cpp = ws.join("src/main.cpp");
    std::fs::write(
        &main_cpp,
        r#"#include <iostream>
#include <map>
#include <optional>
#include <string>
#include <vector>
#include "math.hpp"

int main() {
    int counter = 4;
    counter += 1;
    std::string greeting = "hello cpp";
    std::vector<int> nums = {10, 20, 30};
    std::map<std::string, int> scores = {{"alpha", 7}, {"beta", 9}};
    std::optional<int> opt_some = counter + 37;
    std::optional<int> opt_none = std::nullopt;
    int sum = demo::add(counter, 3);
    std::cout << "CPP_DONE:" << counter << ":" << greeting << ":" << sum
              << ":" << nums.size() << ":" << scores.size()
              << ":" << opt_some.value_or(-1) << ":" << opt_none.value_or(-1) << "\n";
    return 0;
}
"#,
    )
    .unwrap();

    let (_ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let ws_str = ws.to_string_lossy().into_owned();
    let main_str = main_cpp.to_string_lossy().into_owned();

    // 0. Run CMake configure+build via `/ide/runs` (verifying CMake runner & compile_commands.json)
    let (st_cfg, run_cfg) = send_json(
        &app,
        Method::POST,
        "/ide/runs",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "adhoc": {
                "id": "cmake-cfg-build",
                "name": "CMake Configure & Build",
                "runner": "cmake",
                "args": ["-S", ".", "-B", "build", "-DCMAKE_BUILD_TYPE=Debug", "-DCMAKE_EXPORT_COMPILE_COMMANDS=ON"],
                "cwd": ws_str,
            }
        })),
    )
    .await;
    assert_eq!(
        st_cfg,
        StatusCode::OK,
        "CMake configure run failed: {run_cfg}"
    );
    let cfg_id = run_cfg["run"]["id"].as_str().unwrap().to_string();
    let cfg_deadline = Instant::now() + Duration::from_secs(30);
    let mut cfg_state = run_cfg["run"].clone();
    while Instant::now() < cfg_deadline && cfg_state["status"] == "running" {
        tokio::time::sleep(Duration::from_millis(120)).await;
        let (_, r) = send_json(&app, Method::GET, &format!("/ide/runs/{cfg_id}"), None).await;
        cfg_state = r["run"].clone();
    }
    assert_eq!(
        cfg_state["status"], "passed",
        "CMake configure failed: {cfg_state}"
    );
    assert!(
        ws.join("build/compile_commands.json").exists(),
        "expected CMake to generate build/compile_commands.json"
    );

    // Build via CMake runner
    let (st_bld, run_bld) = send_json(
        &app,
        Method::POST,
        "/ide/runs",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "adhoc": {
                "id": "cmake-build",
                "name": "CMake Build",
                "runner": "cmake",
                "args": ["--build", "build"],
                "cwd": ws_str,
            }
        })),
    )
    .await;
    assert_eq!(st_bld, StatusCode::OK, "CMake build run failed: {run_bld}");
    let bld_id = run_bld["run"]["id"].as_str().unwrap().to_string();
    let bld_deadline = Instant::now() + Duration::from_secs(30);
    let mut bld_state = run_bld["run"].clone();
    while Instant::now() < bld_deadline && bld_state["status"] == "running" {
        tokio::time::sleep(Duration::from_millis(120)).await;
        let (_, r) = send_json(&app, Method::GET, &format!("/ide/runs/{bld_id}"), None).await;
        bld_state = r["run"].clone();
    }
    assert_eq!(
        bld_state["status"], "passed",
        "CMake build failed: {bld_state}"
    );

    // Run CTest via `/ide/runs` and verify structured test result parsing
    let (st_ct, run_ct) = send_json(
        &app,
        Method::POST,
        "/ide/runs",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "adhoc": {
                "id": "ctest-all",
                "name": "CTest",
                "runner": "ctest",
                "args": ["--test-dir", "build", "--output-on-failure"],
                "cwd": ws_str,
            }
        })),
    )
    .await;
    assert_eq!(st_ct, StatusCode::OK, "CTest run failed: {run_ct}");
    let ct_id = run_ct["run"]["id"].as_str().unwrap().to_string();
    let ct_deadline = Instant::now() + Duration::from_secs(20);
    let mut ct_state = run_ct["run"].clone();
    while Instant::now() < ct_deadline && ct_state["status"] == "running" {
        tokio::time::sleep(Duration::from_millis(120)).await;
        let (_, r) = send_json(&app, Method::GET, &format!("/ide/runs/{ct_id}"), None).await;
        ct_state = r["run"].clone();
    }
    assert_eq!(ct_state["status"], "passed", "CTest failed: {ct_state}");
    assert!(
        ct_state["tests"].as_array().is_some_and(|arr| arr
            .iter()
            .any(|t| t["name"] == "sample_runs" && t["status"] == "passed")),
        "expected parsed CTest sample_runs result: {ct_state}"
    );

    // 1. Debug built CMake binary: breakpoint at line 16, verify STL locals, watch, set-variable, step into math.cpp
    let bin_str = ws.join("build/cpp_sample").to_string_lossy().into_owned();
    let (st, start) = send_json(
        &app,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "language": "cpp",
            "launch": { "program": bin_str, "cwd": ws_str },
            "breakpoints": [{ "path": main_str, "line": 16 }],
        })),
    )
    .await;
    assert_eq!(st, StatusCode::OK, "C++ start_debug failed: {start}");
    let sid = start["session"]["id"].as_str().unwrap().to_string();

    let snap1 = wait_until_paused(&app, &sid, FIRST_PAUSE).await;
    assert_eq!(
        snap1["paused_line"],
        16,
        "C++ paused_line mismatch: {}",
        brief(&snap1)
    );
    let vars1 = all_vars(&snap1);
    let var_val = |name: &str| -> String {
        vars1
            .iter()
            .find(|v| v["name"] == name)
            .and_then(|v| v["value"].as_str())
            .unwrap_or("")
            .to_string()
    };
    assert!(
        var_val("counter").contains("5"),
        "expected counter=5, got '{}', names={:?}",
        var_val("counter"),
        var_names(&vars1)
    );
    assert!(
        var_val("greeting").contains("hello cpp"),
        "expected STL std::string value for greeting, got '{}'",
        var_val("greeting")
    );
    let nums_v = var_val("nums");
    assert!(
        nums_v.contains("size=3") || (nums_v.contains("10") && nums_v.contains("30")),
        "expected STL std::vector summary for nums, got '{nums_v}'"
    );
    let scores_v = var_val("scores");
    assert!(
        scores_v.contains("size=2") || scores_v.contains("alpha"),
        "expected STL std::map summary for scores, got '{scores_v}'"
    );
    let opt_some_v = var_val("opt_some");
    assert!(
        opt_some_v.contains("42") || opt_some_v.contains("Has Value=true"),
        "expected STL std::optional value for opt_some, got '{opt_some_v}'"
    );

    // Watch / evaluate expression
    let (st_ev, ev_body) = send_json(
        &app,
        Method::POST,
        &format!("/ide/debug/sessions/{sid}/evaluate"),
        Some(json!({ "expression": "counter + 10" })),
    )
    .await;
    assert_eq!(st_ev, StatusCode::OK, "C++ evaluate failed: {ev_body}");
    assert!(
        ev_body["result"].as_str().unwrap_or("").contains("15"),
        "expected 15 from counter + 10, got {ev_body}"
    );

    // Set variable via `POST /ide/debug/sessions/{id}/set-variable`
    let locals_ref = snap1["scopes"][0]["variables_reference"]
        .as_i64()
        .unwrap_or(0);
    assert!(
        locals_ref > 0,
        "expected Locals scope variables_reference > 0"
    );
    let (st_sv, sv_body) = send_json(
        &app,
        Method::POST,
        &format!("/ide/debug/sessions/{sid}/set-variable"),
        Some(json!({
            "variables_reference": locals_ref,
            "name": "counter",
            "value": "99",
        })),
    )
    .await;
    assert_eq!(st_sv, StatusCode::OK, "C++ set-variable failed: {sv_body}");
    assert!(
        sv_body["value"].as_str().unwrap_or("").contains("99"),
        "expected 99 after set-variable, got {sv_body}"
    );

    // Step into `demo::add(counter, 3)` in `src/math.cpp`
    send_cmd(&app, &sid, "step_into").await;
    let snap2 = wait_until_paused(&app, &sid, Duration::from_secs(10)).await;
    let paused2 = snap2["paused_line"].as_u64().unwrap_or(0);
    let paused2_file = snap2["paused_file"].as_str().unwrap_or("");
    assert!(
        paused2_file.ends_with("math.cpp") && (3..=5).contains(&paused2),
        "expected step_into inside src/math.cpp lines 3..=5, got {paused2_file}:{paused2}: {}",
        brief(&snap2)
    );

    // Continue to exit and verify mutated output
    send_cmd(&app, &sid, "continue").await;
    let end = wait_until_terminated(&app, &sid, Duration::from_secs(15)).await;
    assert_eq!(
        end["state"], "terminated",
        "C++ session ended in error: {end}"
    );
    let out = joined_output(&end);
    assert!(
        out.contains("CPP_DONE:99:hello cpp:102:3:2:42:-1"),
        "expected mutated counter=99 and sum=102 in C++ output, got: {out}"
    );

    // 2. C++ exception breakpoint (`break_on_exceptions: true` catches `throw std::runtime_error`)
    let throw_cpp = ws.join("src/throw_demo.cpp");
    std::fs::write(
        &throw_cpp,
        "#include <stdexcept>\nint main() {\n    throw std::runtime_error(\"deliberate cpp boom\");\n}\n",
    )
    .unwrap();
    let throw_str = throw_cpp.to_string_lossy().into_owned();
    let (st_ex, start_ex) = send_json(
        &app,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "language": "cpp",
            "launch": { "program": throw_str, "cwd": ws_str },
            "breakpoints": [],
            "break_on_exceptions": true,
        })),
    )
    .await;
    assert_eq!(
        st_ex,
        StatusCode::OK,
        "C++ exception start_debug failed: {start_ex}"
    );
    let sid_ex = start_ex["session"]["id"].as_str().unwrap().to_string();
    let snap_ex = wait_until_paused(&app, &sid_ex, FIRST_PAUSE).await;
    assert_eq!(
        snap_ex["state"],
        "paused",
        "expected C++ exception session to pause on throw: {}",
        brief(&snap_ex)
    );
    send_cmd(&app, &sid_ex, "stop").await;

    // 3. Live attach by PID (`attach_pid`)
    let attach_src = ws.join("src/attach_demo.cpp");
    let attach_bin = ws.join("build/attach_demo");
    std::fs::write(
        &attach_src,
        "#include <chrono>\n#include <thread>\nint main() {\n    volatile int spin = 1;\n    while (spin) {\n        std::this_thread::sleep_for(std::chrono::milliseconds(20));\n    }\n    return 0;\n}\n",
    )
    .unwrap();
    let attach_src_canon = attach_src.canonicalize().unwrap();
    let compile_attach = tokio::process::Command::new("clang++")
        .args(["-std=c++20", "-g", "-O0"])
        .arg(&attach_src_canon)
        .arg("-o")
        .arg(&attach_bin)
        .status()
        .await
        .expect("compile attach_demo");
    assert!(
        compile_attach.success(),
        "clang++ attach_demo compile failed"
    );

    let mut child = std::process::Command::new(&attach_bin)
        .spawn()
        .expect("spawn attach_demo");
    let child_pid = child.id();
    tokio::time::sleep(Duration::from_millis(120)).await;

    let attach_src_str = attach_src_canon.to_string_lossy().into_owned();
    let (st_att, start_att) = send_json(
        &app,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "language": "cpp",
            "launch": {
                "program": attach_bin.to_string_lossy().into_owned(),
                "cwd": ws_str,
                "attach_pid": child_pid,
            },
            "breakpoints": [{ "path": attach_src_str, "line": 6 }],
        })),
    )
    .await;
    assert_eq!(
        st_att,
        StatusCode::OK,
        "C++ attach_pid start_debug failed: {start_att}"
    );
    let sid_att = start_att["session"]["id"].as_str().unwrap().to_string();
    let snap_att = wait_until_paused(&app, &sid_att, FIRST_PAUSE).await;
    assert_eq!(
        snap_att["state"],
        "paused",
        "expected LLDB attach_pid to pause running process: {}",
        brief(&snap_att)
    );
    assert_eq!(
        snap_att["paused_line"],
        6,
        "expected attach_pid breakpoint hit at attach_demo.cpp:6: {}",
        brief(&snap_att)
    );
    send_cmd(&app, &sid_att, "stop").await;
    let _ = child.kill();
    let _ = child.wait();
}

#[tokio::test]
async fn live_cpp_clangd_lsp_completion_hover_definition_and_header_switch() {
    if !ensure_bins_or_skip(&["cmake", "clangd"]) {
        return;
    }
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
    use tokio::process::Command;

    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("clangd-sample");
    std::fs::create_dir_all(ws.join("include")).unwrap();
    std::fs::create_dir_all(ws.join("src")).unwrap();

    std::fs::write(
        ws.join("CMakeLists.txt"),
        r#"cmake_minimum_required(VERSION 3.16)
project(clangd_sample LANGUAGES CXX)
set(CMAKE_CXX_STANDARD 20)
set(CMAKE_EXPORT_COMPILE_COMMANDS ON)
add_executable(clangd_sample src/main.cpp src/math.cpp)
target_include_directories(clangd_sample PRIVATE include)
"#,
    )
    .unwrap();
    let math_hpp = ws.join("include/math.hpp");
    std::fs::write(
        &math_hpp,
        "#pragma once\nnamespace demo {\n/// Adds two integers.\nint add_numbers(int a, int b);\n}\n",
    )
    .unwrap();
    let math_cpp = ws.join("src/math.cpp");
    std::fs::write(
        &math_cpp,
        "#include \"math.hpp\"\nnamespace demo {\nint add_numbers(int a, int b) { return a + b; }\n}\n",
    )
    .unwrap();
    let main_cpp = ws.join("src/main.cpp");
    let main_src = "#include \"math.hpp\"\nint main() {\n    int v = demo::add_numbers(2, 3);\n    return v;\n}\n";
    std::fs::write(&main_cpp, main_src).unwrap();

    let st = Command::new("cmake")
        .args([
            "-S",
            ".",
            "-B",
            "build",
            "-DCMAKE_EXPORT_COMPILE_COMMANDS=ON",
        ])
        .current_dir(&ws)
        .status()
        .await
        .expect("run cmake");
    assert!(st.success(), "cmake configure for clangd fixture failed");
    assert!(ws.join("build/compile_commands.json").exists());

    // Verify Otto's built-in LSP registry resolves `cpp` -> `clangd`
    let canon_ws = ws.canonicalize().unwrap();
    let resolved = otto_ide::lsp_host::servers::resolve_lang("cpp")
        .expect("expected built-in cpp/clangd entry in LSP registry");
    assert!(resolved.available, "expected clangd to be available");
    assert!(
        resolved.command.ends_with("clangd"),
        "expected clangd command, got {}",
        resolved.command
    );
    let clangd_bin = std::path::PathBuf::from(&resolved.command);

    let mut clangd_args = resolved.args.clone();
    clangd_args.push(format!(
        "--compile-commands-dir={}",
        canon_ws.join("build").to_string_lossy()
    ));

    let mut child = Command::new(&clangd_bin)
        .args(&clangd_args)
        .current_dir(&canon_ws)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("spawn clangd");

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    async fn send_lsp(stdin: &mut tokio::process::ChildStdin, body: &Value) {
        let s = body.to_string();
        let frame = format!("Content-Length: {}\r\n\r\n{}", s.len(), s);
        stdin.write_all(frame.as_bytes()).await.unwrap();
        stdin.flush().await.unwrap();
    }

    async fn read_lsp_response(
        stdout: &mut BufReader<tokio::process::ChildStdout>,
        want_id: i64,
    ) -> Value {
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            let mut content_len: usize = 0;
            loop {
                let mut line = String::new();
                let n = tokio::time::timeout(Duration::from_secs(10), stdout.read_line(&mut line))
                    .await
                    .expect("clangd read header timeout")
                    .unwrap();
                assert!(n > 0, "clangd exited unexpectedly");
                if line == "\r\n" || line == "\n" {
                    break;
                }
                if let Some(rest) = line.strip_prefix("Content-Length:") {
                    content_len = rest.trim().parse().unwrap();
                }
            }
            let mut buf = vec![0u8; content_len];
            stdout.read_exact(&mut buf).await.unwrap();
            let v: Value = serde_json::from_slice(&buf).unwrap();
            if v.get("id").and_then(|x| x.as_i64()) == Some(want_id) {
                return v;
            }
        }
        panic!("timed out waiting for clangd response id={want_id}");
    }

    let root_uri = format!("file://{}", canon_ws.to_string_lossy());
    let main_uri = format!("file://{}", canon_ws.join("src/main.cpp").to_string_lossy());
    let math_cpp_uri = format!("file://{}", canon_ws.join("src/math.cpp").to_string_lossy());
    let math_src = std::fs::read_to_string(&math_cpp).unwrap();

    send_lsp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "processId": std::process::id(),
                "rootUri": root_uri,
                "capabilities": {}
            }
        }),
    )
    .await;
    let init = read_lsp_response(&mut stdout, 1).await;
    assert!(
        init["result"]["capabilities"].is_object(),
        "clangd initialize failed: {init}"
    );

    send_lsp(
        &mut stdin,
        &json!({ "jsonrpc": "2.0", "method": "initialized", "params": {} }),
    )
    .await;
    send_lsp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": { "uri": main_uri, "languageId": "cpp", "version": 1, "text": main_src }
            }
        }),
    )
    .await;
    send_lsp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "method": "textDocument/didOpen",
            "params": {
                "textDocument": { "uri": math_cpp_uri, "languageId": "cpp", "version": 1, "text": math_src }
            }
        }),
    )
    .await;

    // Hover on `add_numbers` at line 2, char 20
    send_lsp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 2,
            "method": "textDocument/hover",
            "params": {
                "textDocument": { "uri": main_uri },
                "position": { "line": 2, "character": 20 }
            }
        }),
    )
    .await;
    let hover = read_lsp_response(&mut stdout, 2).await;
    let hover_str = hover.to_string();
    assert!(
        hover_str.contains("add_numbers") && hover_str.contains("int"),
        "expected clangd hover to mention add_numbers signature: {hover_str}"
    );

    // Definition of `add_numbers` -> include/math.hpp or src/math.cpp
    send_lsp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "textDocument/definition",
            "params": {
                "textDocument": { "uri": main_uri },
                "position": { "line": 2, "character": 20 }
            }
        }),
    )
    .await;
    let def = read_lsp_response(&mut stdout, 3).await;
    assert!(
        def.to_string().contains("math.hpp") || def.to_string().contains("math.cpp"),
        "expected clangd definition to jump to math.hpp/math.cpp: {def}"
    );

    // Completion after `demo::` (line 2, char 18)
    send_lsp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "textDocument/completion",
            "params": {
                "textDocument": { "uri": main_uri },
                "position": { "line": 2, "character": 18 }
            }
        }),
    )
    .await;
    let comp = read_lsp_response(&mut stdout, 4).await;
    assert!(
        comp.to_string().contains("add_numbers"),
        "expected clangd completion after demo:: to include add_numbers: {comp}"
    );

    // Header/Source switch (`textDocument/switchSourceHeader`) on src/math.cpp -> include/math.hpp
    send_lsp(
        &mut stdin,
        &json!({
            "jsonrpc": "2.0",
            "id": 5,
            "method": "textDocument/switchSourceHeader",
            "params": { "uri": math_cpp_uri }
        }),
    )
    .await;
    let sw = read_lsp_response(&mut stdout, 5).await;
    assert!(
        sw["result"]
            .as_str()
            .is_some_and(|u| u.ends_with("include/math.hpp")),
        "expected clangd switchSourceHeader to return include/math.hpp, got: {sw}"
    );

    let _ = child.kill().await;
}

#[tokio::test]
async fn live_kotlin_jvm_jdwp_debug_and_mixed_java_kotlin_sources() {
    // Kotlin compiles to JVM bytecode and reuses Otto's Java JDWP bridge (`language: "kotlin"`).
    // Even when `kotlinc` is not installed on the host, we verify the REAL live JVM JDWP debug path
    // end-to-end with the installed JDK (`javac -g` + `java`) using a `*Kt` class + sibling Java helper
    // in a mixed `src/main/kotlin` + `src/main/java` module layout.
    if !ensure_bins_or_skip(&["java", "javac"]) {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("kt-jvm-app");
    let kt_pkg = ws.join("src/main/kotlin/com/acme");
    let java_pkg = ws.join("src/main/java/com/acme");
    let classes_dir = ws.join("build/classes/kotlin/main");
    std::fs::create_dir_all(&kt_pkg).unwrap();
    std::fs::create_dir_all(&java_pkg).unwrap();
    std::fs::create_dir_all(&classes_dir).unwrap();

    // Mixed Java helper called from the Kotlin-style `*Kt` entry class
    let helper_java = java_pkg.join("JavaHelper.java");
    std::fs::write(
        &helper_java,
        r#"package com.acme;

public class JavaHelper {
    public static int boost(int n) {
        int out = n * 2;
        return out;
    }
}
"#,
    )
    .unwrap();

    // Kotlin-convention `*Kt` class (compiled with javac -g so the test runs on hosts without kotlinc)
    let app_kt_java = java_pkg.join("AppKt.java");
    std::fs::write(
        &app_kt_java,
        r#"package com.acme;

public final class AppKt {
    public static void main(String[] args) {
        int base = 21;
        int boosted = JavaHelper.boost(base);
        System.out.println("KT_JDWP_DONE:" + boosted);
    }
}
"#,
    )
    .unwrap();

    // Mirror `.kt` file in `src/main/kotlin` to confirm mixed source-root inclusion in the adapter
    std::fs::write(
        kt_pkg.join("Placeholder.kt"),
        "package com.acme\n\nfun helper(): Int = 42\n",
    )
    .unwrap();

    let compile_status = tokio::process::Command::new("javac")
        .args([
            "-g",
            "-d",
            &classes_dir.to_string_lossy(),
            &helper_java.to_string_lossy(),
            &app_kt_java.to_string_lossy(),
        ])
        .status()
        .await
        .expect("run javac");
    assert!(
        compile_status.success(),
        "javac failed for Kotlin JDWP test"
    );

    let (_ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let ws_str = ws.to_string_lossy().into_owned();
    let app_kt_java_str = app_kt_java.to_string_lossy().into_owned();

    // Launch with `language: "kotlin"` pointing to `com.acme.AppKt` + compiled classpath
    let (st, start) = send_json(
        &app,
        Method::POST,
        "/ide/debug/sessions",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "language": "kotlin",
            "launch": {
                "program": "com.acme.AppKt",
                "cwd": ws_str,
                "classpath": [classes_dir.to_string_lossy()],
            },
            "breakpoints": [{ "path": app_kt_java_str, "line": 6 }],
        })),
    )
    .await;
    assert_eq!(
        st,
        StatusCode::OK,
        "Kotlin JDWP start_debug failed: {start}"
    );
    let sid = start["session"]["id"].as_str().unwrap().to_string();

    let snap1 = wait_until_paused(&app, &sid, FIRST_PAUSE).await;
    assert_eq!(
        snap1["paused_line"],
        6,
        "Kotlin JDWP paused_line mismatch: {}",
        brief(&snap1)
    );
    let vars1 = all_vars(&snap1);
    assert!(
        vars1
            .iter()
            .any(|v| v["name"] == "base" && v["value"] == "21"),
        "expected base=21 in Kotlin JDWP locals: {:?}",
        var_names(&vars1)
    );

    // Evaluate `base + 9`
    let (st_ev, ev_body) = send_json(
        &app,
        Method::POST,
        &format!("/ide/debug/sessions/{sid}/evaluate"),
        Some(json!({ "expression": "base + 9" })),
    )
    .await;
    assert_eq!(st_ev, StatusCode::OK);
    assert_eq!(ev_body["result"], "30");

    // Step into JavaHelper.boost (mixed Kotlin/Java stepping)
    send_cmd(&app, &sid, "step_into").await;
    let snap2 = wait_until_paused(&app, &sid, Duration::from_secs(10)).await;
    assert!(
        snap2["paused_file"]
            .as_str()
            .unwrap_or("")
            .ends_with("JavaHelper.java"),
        "expected step_into JavaHelper.java, got {snap2}"
    );

    send_cmd(&app, &sid, "continue").await;
    let end = wait_until_terminated(&app, &sid, Duration::from_secs(15)).await;
    assert_eq!(
        end["state"], "terminated",
        "Kotlin JDWP session ended in error: {end}"
    );
    let out = joined_output(&end);
    assert!(
        out.contains("KT_JDWP_DONE:42"),
        "expected KT_JDWP_DONE:42 in output, got: {out}"
    );
}

#[tokio::test]
async fn live_spring_boot_actuator_devtools_and_config_intelligence() {
    if !ensure_bins_or_skip(&["javac", "java"]) {
        return;
    }
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("spring-live-app");
    let java_pkg = ws.join("src/main/java/com/acme/boot");
    let res_dir = ws.join("src/main/resources");
    let classes_dir = ws.join("target/classes");
    std::fs::create_dir_all(&java_pkg).unwrap();
    std::fs::create_dir_all(&res_dir).unwrap();
    std::fs::create_dir_all(&classes_dir).unwrap();

    std::fs::write(
        ws.join("pom.xml"),
        r#"<project>
  <modelVersion>4.0.0</modelVersion>
  <groupId>com.acme</groupId>
  <artifactId>spring-live-app</artifactId>
  <version>1.0.0</version>
  <dependencies>
    <dependency><groupId>org.springframework.boot</groupId><artifactId>spring-boot-starter-web</artifactId></dependency>
    <dependency><groupId>org.springframework.boot</groupId><artifactId>spring-boot-starter-actuator</artifactId></dependency>
    <dependency><groupId>org.springframework.boot</groupId><artifactId>spring-boot-devtools</artifactId></dependency>
  </dependencies>
</project>
"#,
    )
    .unwrap();

    let props_file = res_dir.join("application.properties");
    std::fs::write(
        &props_file,
        "server.port=8089\nspring.redis.host=127.0.0.1\nspring.jpa.show-sql=notabool\n",
    )
    .unwrap();

    let app_file = java_pkg.join("LiveActuatorApp.java");
    std::fs::write(
        &app_file,
        r#"package com.acme.boot;

import com.sun.net.httpserver.HttpServer;
import java.io.OutputStream;
import java.net.InetSocketAddress;
import java.nio.charset.StandardCharsets;

// @SpringBootApplication
public class LiveActuatorApp {
    // @Value("${server.port:8080}")
    private int port = 8089;

    public static void main(String[] args) throws Exception {
        HttpServer srv = HttpServer.create(new InetSocketAddress("127.0.0.1", 0), 0);
        int boundPort = srv.getAddress().getPort();
        srv.createContext("/actuator", ex -> {
            String path = ex.getRequestURI().getPath();
            String body = switch (path) {
                case "/actuator" -> "{\"_links\":{\"self\":{\"href\":\"http://127.0.0.1:" + boundPort + "/actuator\"},\"health\":{},\"mappings\":{},\"beans\":{},\"env\":{}}}";
                case "/actuator/health" -> "{\"status\":\"UP\",\"components\":{\"db\":{\"status\":\"UP\",\"details\":{\"database\":\"H2\"}}}}";
                case "/actuator/mappings" -> "{\"contexts\":{\"application\":{\"mappings\":{\"dispatcherServlets\":{\"dispatcherServlet\":[{\"handler\":\"com.acme.boot.HelloController#hello()\",\"details\":{\"requestMappingConditions\":{\"patterns\":[\"/api/hello\"],\"methods\":[\"GET\"]}}}]}}}}}";
                case "/actuator/beans" -> "{\"contexts\":{\"application\":{\"beans\":{\"helloController\":{\"type\":\"com.acme.boot.HelloController\",\"scope\":\"singleton\",\"dependencies\":[\"greetingService\"]}}}}}";
                case "/actuator/env" -> "{\"activeProfiles\":[\"dev\"],\"propertySources\":[{\"name\":\"application.properties\",\"properties\":{\"server.port\":{\"value\":\"8089\"},\"spring.datasource.password\":{\"value\":\"topsecret123\"}}}]}";
                default -> "{}";
            };
            byte[] bytes = body.getBytes(StandardCharsets.UTF_8);
            ex.getResponseHeaders().add("Content-Type", "application/json");
            ex.sendResponseHeaders(200, bytes.length);
            try (OutputStream os = ex.getResponseBody()) {
                os.write(bytes);
            }
        });
        srv.start();
        System.out.println("ACTUATOR_PORT=" + boundPort);
        System.out.flush();
        Thread.sleep(20000);
    }
}
"#,
    )
    .unwrap();

    let javac_st = tokio::process::Command::new("javac")
        .args([
            "-g",
            "-d",
            &classes_dir.to_string_lossy(),
            &app_file.to_string_lossy(),
        ])
        .status()
        .await
        .expect("run javac");
    assert!(javac_st.success(), "javac failed for LiveActuatorApp");

    // Launch real JDK 27 Actuator mock server on 127.0.0.1:0
    let mut child = tokio::process::Command::new("java")
        .args([
            "-cp",
            &classes_dir.to_string_lossy(),
            "com.acme.boot.LiveActuatorApp",
        ])
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .expect("spawn java LiveActuatorApp");

    use tokio::io::{AsyncBufReadExt, BufReader};
    let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
    let port_line = tokio::time::timeout(Duration::from_secs(10), lines.next_line())
        .await
        .expect("timeout waiting for ACTUATOR_PORT")
        .expect("read stdout")
        .expect("eof before ACTUATOR_PORT");
    let bound_port: u16 = port_line
        .trim()
        .strip_prefix("ACTUATOR_PORT=")
        .expect("ACTUATOR_PORT prefix")
        .parse()
        .expect("parse port");

    let (_ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("db.sqlite")).await;
    let ws_str = ws.to_string_lossy().into_owned();
    let props_str = props_file.to_string_lossy().into_owned();
    let app_str = app_file.to_string_lossy().into_owned();

    // 1. Build tools model detects Spring Boot workspace
    let (st_bt, bt_body) = send_json(
        &app,
        Method::POST,
        "/ide/build-tools/model",
        Some(json!({ "target": { "kind": "local", "root": ws_str }, "roots": [ws_str] })),
    )
    .await;
    assert_eq!(st_bt, StatusCode::OK, "build-tools model failed: {bt_body}");
    assert_eq!(bt_body["spring_boot"]["detected"], true);
    assert_eq!(bt_body["spring_boot"]["has_actuator"], true);
    assert_eq!(bt_body["spring_boot"]["has_devtools"], true);
    assert_eq!(bt_body["spring_boot"]["server_port"], 8089);

    // 2. Live Actuator inspection against real JDK 27 HTTP server on 127.0.0.1:<bound_port>
    let (st_act, act_body) = send_json(
        &app,
        Method::POST,
        "/ide/spring/actuator",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "port": bound_port,
            "base_path": "/actuator"
        })),
    )
    .await;
    assert_eq!(
        st_act,
        StatusCode::OK,
        "actuator inspect failed: {act_body}"
    );
    assert_eq!(act_body["reachable"], true);
    assert_eq!(act_body["status"], "UP");
    assert_eq!(act_body["endpoints"][0]["method"], "GET");
    assert_eq!(act_body["endpoints"][0]["path"], "/api/hello");
    assert_eq!(act_body["beans"][0]["name"], "helloController");
    assert_eq!(act_body["active_profiles"][0], "dev");
    let env_list = act_body["env_properties"].as_array().unwrap();
    let pw_entry = env_list
        .iter()
        .find(|p| p["key"] == "spring.datasource.password")
        .expect("password entry present");
    assert_eq!(pw_entry["redacted"], true);
    assert_eq!(pw_entry["value"], "******");

    // 3. SSRF guard rejects non-loopback Actuator URL with 403 Forbidden
    let (st_ssrf, _) = send_json(
        &app,
        Method::POST,
        "/ide/spring/actuator",
        Some(json!({ "base_url": "http://169.254.169.254/latest/meta-data" })),
    )
    .await;
    assert_eq!(st_ssrf, StatusCode::FORBIDDEN);

    // 4. DevTools restart creates .reloadtrigger inside target/classes
    let (st_dt, dt_body) = send_json(
        &app,
        Method::POST,
        "/ide/spring/devtools-restart",
        Some(json!({ "target": { "kind": "local", "root": ws_str } })),
    )
    .await;
    assert_eq!(st_dt, StatusCode::OK, "devtools restart failed: {dt_body}");
    assert_eq!(dt_body["ok"], true);
    let trigger_path = std::path::PathBuf::from(dt_body["trigger_path"].as_str().unwrap());
    assert!(trigger_path.is_file(), "expected trigger file on disk");
    assert!(trigger_path.ends_with("target/classes/.reloadtrigger"));

    // 5. Spring config diagnostics + hover + definition over HTTP routes
    let (st_diag, diag_body) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/diagnostics",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": props_str,
        })),
    )
    .await;
    assert_eq!(st_diag, StatusCode::OK);
    let diags = diag_body["diagnostics"].as_array().unwrap();
    assert!(
        diags
            .iter()
            .any(|d| d["code"] == "spring_deprecated_property"),
        "expected deprecated diagnostic: {diags:?}"
    );
    assert!(
        diags.iter().any(|d| d["code"] == "spring_invalid_type"),
        "expected invalid boolean diagnostic: {diags:?}"
    );

    let (st_hov, hov_body) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/hover",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": app_str,
            "line": 10,
            "column": 20,
        })),
    )
    .await;
    assert_eq!(st_hov, StatusCode::OK);
    assert_eq!(hov_body["symbol"], "server.port");

    let (st_def, def_body) = send_json(
        &app,
        Method::POST,
        "/ide/lsp/definition",
        Some(json!({
            "target": { "kind": "local", "root": ws_str },
            "path": app_str,
            "line": 10,
            "column": 20,
        })),
    )
    .await;
    assert_eq!(st_def, StatusCode::OK);
    assert!(def_body["locations"][0]["path"]
        .as_str()
        .unwrap_or("")
        .ends_with("application.properties"));

    let _ = child.kill().await;
}
