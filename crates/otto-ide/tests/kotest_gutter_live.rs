//! Live Kotest gutter Run / Debug (rc.6 B4) through the IDE HTTP routes, against the real
//! Gradle (wrapper) and Maven builds of a Kotest 6 project: every style's deepest test runs
//! alone (a failing sibling next to it proves the siblings did not run), containers run
//! their subtree, a disabled test runs when clicked, data rows run through their container,
//! and Debug stops on a breakpoint in the chosen test's lambda but never in its sibling's.
//!
//! Run by hand (nothing is downloaded by the test; the builds fetch their dependencies):
//! `OTTO_LIVE_KOTEST_GRADLE=<gradle fixture> OTTO_LIVE_KOTEST_MAVEN=<maven fixture>
//!  OTTO_LIVE_KOTEST_OLD=<Kotest 5 fixture> OTTO_LIVE_KOTEST_ENV='{"JAVA_HOME":…,"PATH":…}'
//!  cargo test -p otto-ide --test kotest_gutter_live -- --ignored --nocapture --test-threads=1`

#![allow(clippy::disallowed_methods)] // process sampling in a live test

mod common;

use std::time::{Duration, Instant};

use axum::http::Method;
use axum::Router;
use common::{build_test_app, send_json};
use serde_json::{json, Value};

struct Live {
    app: Router,
    root: String,
    env: Value,
    _tmp: tempfile::TempDir,
}

async fn live(var: &str) -> Option<Live> {
    let (Ok(root), Ok(env)) = (std::env::var(var), std::env::var("OTTO_LIVE_KOTEST_ENV")) else {
        eprintln!("skip: set {var} and OTTO_LIVE_KOTEST_ENV");
        return None;
    };
    let tmp = tempfile::tempdir().unwrap();
    let (_ctx, app) = build_test_app(tmp.path().join("data"), &tmp.path().join("state.db")).await;
    Some(Live {
        app,
        root,
        env: serde_json::from_str(&env).unwrap(),
        _tmp: tmp,
    })
}

impl Live {
    fn spec(&self, name: &str) -> String {
        format!("{}/src/test/kotlin/com/acme/{name}.kt", self.root)
    }

    fn target(&self) -> Value {
        json!({ "kind": "local", "root": self.root })
    }

    async fn gutter(&self, spec: &str) -> Vec<Value> {
        let body = json!({ "target": self.target(), "path": self.spec(spec) });
        let (status, resp) = send_json(&self.app, Method::POST, "/ide/gutter", Some(body)).await;
        assert!(status.is_success(), "gutter: {status} {resp}");
        resp["items"].as_array().unwrap().clone()
    }

    async fn item(&self, spec: &str, label: &str) -> Value {
        let items = self.gutter(spec).await;
        items
            .iter()
            .find(|i| i["label"] == label)
            .unwrap_or_else(|| panic!("no item '{label}' in {spec}: {:?}", labels(&items)))
            .clone()
    }

    /// Run an item the way the UI's gutter Run does (launchGutterAction).
    async fn run(&self, item: &Value) -> Value {
        let spec = &item["run_spec"];
        let adhoc = json!({
            "id": format!("gutter-{}", item["line"]), "name": item["label"],
            "runner": spec["runner"], "args": [], "cwd": spec["cwd"],
            "env_overrides": self.env, "inherit_shell_env": false,
            "target_path": spec["target_path"], "test_filter": spec["test_filter"],
            "test_path": spec["test_path"],
        });
        let body = json!({ "target": self.target(), "adhoc": adhoc });
        let (status, resp) = send_json(&self.app, Method::POST, "/ide/runs", Some(body)).await;
        assert!(status.is_success(), "run: {status} {resp}");
        let id = resp["run"]["id"].as_str().unwrap().to_string();
        let deadline = Instant::now() + Duration::from_secs(240);
        loop {
            let (_, r) = send_json(&self.app, Method::GET, &format!("/ide/runs/{id}"), None).await;
            if r["run"]["status"] != "running" && r["run"]["status"] != "queued" {
                return r["run"].clone();
            }
            assert!(
                Instant::now() < deadline,
                "run did not end: {}",
                tail(&r["run"])
            );
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
    }

    async fn start_debug(&self, item: &Value, bps: Vec<(String, u32)>) -> (u16, Value) {
        let spec = &item["run_spec"];
        let launch = json!({
            "program": spec["debug_program"], "args": [], "cwd": spec["cwd"],
            "test_name": spec["test_filter"], "test_path": spec["test_path"],
            "env": self.env, "inherit_shell_env": false,
        });
        let bps: Vec<Value> = bps
            .into_iter()
            .map(|(p, l)| json!({ "path": p, "line": l, "enabled": true }))
            .collect();
        let body = json!({ "target": self.target(), "language": "kotlin", "launch": launch, "breakpoints": bps });
        let (status, resp) =
            send_json(&self.app, Method::POST, "/ide/debug/sessions", Some(body)).await;
        (status.as_u16(), resp)
    }

    async fn snap(&self, id: &str) -> Value {
        let (_, r) = send_json(
            &self.app,
            Method::GET,
            &format!("/ide/debug/sessions/{id}"),
            None,
        )
        .await;
        r["session"].clone()
    }

    async fn next_stop(&self, id: &str) -> Value {
        let deadline = Instant::now() + Duration::from_secs(240);
        loop {
            let s = self.snap(id).await;
            if matches!(s["state"].as_str(), Some("paused" | "terminated" | "error")) {
                return s;
            }
            assert!(
                Instant::now() < deadline,
                "no stop: {} bps={} output={}",
                s["state"],
                s["breakpoints"],
                s["output"]
                    .as_array()
                    .map(|o| o
                        .iter()
                        .filter_map(|e| e["text"].as_str())
                        .collect::<String>())
                    .unwrap_or_default()
            );
            tokio::time::sleep(Duration::from_millis(300)).await;
        }
    }

    async fn command(&self, id: &str, action: &str) {
        let (status, resp) = send_json(
            &self.app,
            Method::POST,
            &format!("/ide/debug/sessions/{id}/command"),
            Some(json!({ "action": action })),
        )
        .await;
        assert!(status.is_success(), "{action}: {status} {resp}");
    }

    async fn eval(&self, id: &str, expr: &str) -> String {
        let (_, r) = send_json(
            &self.app,
            Method::POST,
            &format!("/ide/debug/sessions/{id}/evaluate"),
            Some(json!({ "expression": expr })),
        )
        .await;
        r["result"].as_str().unwrap_or_default().to_string()
    }
}

fn labels(items: &[Value]) -> Vec<String> {
    items
        .iter()
        .map(|i| i["label"].as_str().unwrap_or("").to_string())
        .collect()
}

fn ran(run: &Value) -> Vec<String> {
    run["tests"]
        .as_array()
        .unwrap()
        .iter()
        .map(|t| {
            format!(
                "{}:{}",
                t["status"].as_str().unwrap_or(""),
                t["name"].as_str().unwrap_or("")
            )
        })
        .collect()
}

fn tail(run: &Value) -> String {
    let out = run["output"].as_array().unwrap();
    out[out.len().saturating_sub(12)..]
        .iter()
        .map(|l| l.as_str().unwrap_or(""))
        .collect::<Vec<_>>()
        .join(" | ")
}

/// Test JVMs (Gradle's worker, Surefire's fork) still alive.
fn test_jvms() -> Vec<String> {
    let out = std::process::Command::new("ps")
        .args(["-axo", "pid=,command="])
        .output()
        .unwrap();
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter(|l| {
            l.contains("Gradle Test Executor")
                || l.contains("surefirebooter")
                || l.contains("jdwp=transport")
        })
        .filter(|l| !l.contains("ps -axo"))
        .map(str::to_string)
        .collect()
}

const DEEPEST: &[(&str, &str, &str)] = &[
    (
        "LoginFunSpec",
        "a user -- logs in -- sees dashboard",
        "sees dashboard",
    ),
    ("CalcStringSpec", "adds numbers", "adds numbers"),
    (
        "CartDescribeSpec",
        "a cart -- with items -- totals the prices",
        "totals the prices",
    ),
    (
        "StackBehaviorSpec",
        "a stack -- an item is pushed -- the top is the item",
        "the top is the item",
    ),
    (
        "ShouldStyleSpec",
        "a list -- when empty -- have size zero",
        "have size zero",
    ),
    (
        "WordStyleSpec",
        "a queue -- empty -- have no head",
        "have no head",
    ),
    ("FreeStyleSpec", "outer -- inner -- the leaf", "the leaf"),
    (
        "FeatureStyleSpec",
        "login -- with a password -- works",
        "works",
    ),
    ("ExpectStyleSpec", "math -- addition -- adds", "adds"),
    ("AnnotationStyleSpec", "addsNumbers", "addsNumbers"),
];

async fn run_matrix(l: &Live, styles: &[(&str, &str, &str)]) {
    for (spec, label, leaf) in styles {
        let run = l.run(&l.item(spec, label).await).await;
        let tests = ran(&run);
        eprintln!(
            "R1 {spec} '{label}': status={} tests={tests:?}",
            run["status"]
        );
        eprintln!("R1 {spec}: {}", run["output"][0]);
        assert_eq!(run["status"], "passed", "{spec}: {}", tail(&run));
        assert_eq!(
            tests.len(),
            1,
            "{spec}: only the chosen test runs: {tests:?}"
        );
        assert!(
            tests[0].starts_with("passed:") && tests[0].ends_with(leaf),
            "{tests:?}"
        );
    }
}

async fn debug_leaf(
    l: &Live,
    spec: &str,
    label: &str,
    (hit, sibling): (u32, u32),
    local: &str,
    value: &str,
) {
    let item = l.item(spec, label).await;
    let file = l.spec(spec);
    let (status, resp) = l
        .start_debug(&item, vec![(file.clone(), hit), (file.clone(), sibling)])
        .await;
    assert!(status < 300, "debug start: {status} {resp}");
    let id = resp["session"]["id"].as_str().unwrap().to_string();
    let s = l.next_stop(&id).await;
    eprintln!(
        "D1 {spec} '{label}': state={} at {}:{} error={}",
        s["state"],
        s["paused_file"]
            .as_str()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .unwrap_or(""),
        s["paused_line"],
        s["error"]
    );
    assert_eq!(s["state"], "paused", "{}", s["error"]);
    assert_eq!(s["paused_line"], hit);
    assert!(s["paused_file"]
        .as_str()
        .unwrap()
        .ends_with(&format!("{spec}.kt")));
    let v = l.eval(&id, local).await;
    eprintln!("D1 {spec}: evaluate {local} = {v}");
    assert!(v.contains(value), "{local} = {v}");
    l.command(&id, "step_over").await;
    let s = l.next_stop(&id).await;
    eprintln!(
        "D1 {spec}: step_over -> {}:{}",
        s["state"], s["paused_line"]
    );
    assert_eq!(s["state"], "paused");
    assert_ne!(s["paused_line"], sibling);
    l.command(&id, "continue").await;
    let s = l.next_stop(&id).await;
    let out: Vec<String> = s["output"]
        .as_array()
        .map(|o| {
            o.iter()
                .filter_map(|e| e["text"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    let joined = out.join("");
    for line in joined.lines().filter(|x| {
        x.contains("Attaching")
            || x.contains("Listening")
            || x.contains("BUILD")
            || x.contains("Tests run")
    }) {
        eprintln!("D1 {spec} console: {line}");
    }
    assert_eq!(
        s["state"], "terminated",
        "the sibling's breakpoint at {sibling} must never stop: {}:{}",
        s["state"], s["paused_line"]
    );
    tokio::time::sleep(Duration::from_secs(4)).await;
    let left = test_jvms();
    eprintln!("D1 {spec}: test JVMs left: {left:?}");
    assert!(left.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn kotest_gradle_runs_and_debugs_one_test() {
    let Some(l) = live("OTTO_LIVE_KOTEST_GRADLE").await else {
        return;
    };
    let login = labels(&l.gutter("LoginFunSpec").await);
    eprintln!("G0 LoginFunSpec gutter: {login:?}");
    assert!(login.contains(&"parsing".to_string()));
    assert!(!login
        .iter()
        .any(|x| x.contains("a.dot") || x.contains("parsing -- ")));

    if std::env::var("OTTO_LIVE_KOTEST_QUICK").is_err() {
        run_matrix(&l, DEEPEST).await;
    }

    let run = l
        .run(&l.item("LoginFunSpec", "a user -- logs in").await)
        .await;
    eprintln!(
        "R2 'a user -- logs in': status={} tests={:?}",
        run["status"],
        ran(&run)
    );
    assert_eq!(ran(&run).len(), 2);
    let run = l.run(&l.item("LoginFunSpec", "disabled one").await).await;
    eprintln!(
        "R3 'disabled one': status={} tests={:?}",
        run["status"],
        ran(&run)
    );
    assert_eq!(ran(&run), vec!["passed:disabled one"]);
    let run = l.run(&l.item("LoginFunSpec", "parsing").await).await;
    eprintln!(
        "R4 'parsing': status={} tests={:?}",
        run["status"],
        ran(&run)
    );
    assert_eq!(ran(&run), vec!["passed:a", "passed:b"]);
    let mut gone = l
        .item("LoginFunSpec", "a user -- logs in -- sees dashboard")
        .await;
    gone["run_spec"]["test_path"] = json!(["a user", "logs out"]);
    let run = l.run(&gone).await;
    eprintln!("R-none: status={} tail={}", run["status"], tail(&run));
    assert_eq!(run["status"], "failed");
    assert!(tail(&run).contains("Kotest ran no test named 'a user -- logs out'"));

    debug_leaf(
        &l,
        "LoginFunSpec",
        "a user -- logs in -- sees dashboard",
        (13, 17),
        "size",
        "2",
    )
    .await;
    debug_leaf(
        &l,
        "CartDescribeSpec",
        "a cart -- with items -- totals the prices",
        (11, 14),
        "total",
        "5",
    )
    .await;
    debug_leaf(
        &l,
        "StackBehaviorSpec",
        "a stack -- an item is pushed -- the top is the item",
        (12, 15),
        "stack",
        "ArrayDeque",
    )
    .await;

    // P1: something else holds 5005.
    let hold = std::net::TcpListener::bind("127.0.0.1:5005").unwrap();
    let (_, resp) = l
        .start_debug(
            &l.item("LoginFunSpec", "a user -- other leaf").await,
            Vec::new(),
        )
        .await;
    let s = l.next_stop(resp["session"]["id"].as_str().unwrap()).await;
    eprintln!("P1: state={} error={}", s["state"], s["error"]);
    drop(hold);
    assert_eq!(s["state"], "error");
    assert!(s["error"].as_str().unwrap().contains("Port 5005 is in use"));
    let left = test_jvms();
    eprintln!("P1: test JVMs left: {left:?}");
    assert!(left.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn kotest_maven_runs_and_debugs_one_test() {
    let Some(l) = live("OTTO_LIVE_KOTEST_MAVEN").await else {
        return;
    };
    run_matrix(&l, &DEEPEST[..4]).await;
    let run = l
        .run(&l.item("CartDescribeSpec", "a cart -- with items").await)
        .await;
    eprintln!(
        "M-R2 'a cart -- with items': status={} tests={:?}",
        run["status"],
        ran(&run)
    );
    assert_eq!(ran(&run).len(), 2);
    debug_leaf(
        &l,
        "LoginFunSpec",
        "a user -- logs in -- sees dashboard",
        (13, 17),
        "size",
        "2",
    )
    .await;
}

#[tokio::test(flavor = "multi_thread")]
#[ignore]
async fn kotest_5_keeps_spec_items_with_the_note() {
    let Some(l) = live("OTTO_LIVE_KOTEST_OLD").await else {
        return;
    };
    let items = l.gutter("OldSpec").await;
    eprintln!(
        "V1 OldSpec gutter: {:?} note={}",
        labels(&items),
        items[0]["note"]
    );
    assert_eq!(labels(&items), vec!["OldSpec"]);
    assert_eq!(
        items[0]["note"],
        "Single-test runs need Kotest 6.1+ (this project uses 5.9.1)."
    );
}
