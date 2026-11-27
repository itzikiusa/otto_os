//! Live Debug All Tests for .NET (rc.6 A2): one solution with an xUnit project,
//! an NUnit project on two frameworks and an MSTest project on
//! Microsoft.Testing.Platform, debugged through the IDE HTTP routes with the
//! real `dotnet`, VSTest and `netcoredbg`. One step per (project, framework),
//! one test host at a time, breakpoints carried between steps, Skip and Stop.
//!
//! Run by hand (it needs the toolchains; nothing is downloaded):
//! `OTTO_LIVE_DOTNET=<dir with dotnet (SDK 11 + runtime 10)> OTTO_LIVE_NETCOREDBG=<netcoredbg binary>
//!  OTTO_LIVE_NUGET=<NUGET_PACKAGES dir> cargo test -p otto-ide --test dotnet_debug_steps_live -- --ignored --nocapture`

#![allow(clippy::disallowed_methods)] // fixture copies and process sampling in a live test

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
    env: Value,
    _tmp: tempfile::TempDir,
}

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap().flatten() {
        let p = e.path();
        let name = e.file_name();
        if matches!(name.to_str(), Some("bin" | "obj")) {
            continue;
        }
        if p.is_dir() {
            copy_dir(&p, &to.join(&name));
        } else {
            std::fs::copy(&p, to.join(&name)).unwrap();
        }
    }
}

async fn live() -> Option<Live> {
    let (Ok(dotnet), Ok(ncdbg), Ok(nuget)) = (
        std::env::var("OTTO_LIVE_DOTNET"),
        std::env::var("OTTO_LIVE_NETCOREDBG"),
        std::env::var("OTTO_LIVE_NUGET"),
    ) else {
        eprintln!("skip: set OTTO_LIVE_DOTNET, OTTO_LIVE_NETCOREDBG and OTTO_LIVE_NUGET");
        return None;
    };
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let tc = data.join("toolchains/dotnet");
    std::fs::create_dir_all(&tc).unwrap();
    std::os::unix::fs::symlink(&dotnet, tc.join("current")).unwrap();
    std::os::unix::fs::symlink(Path::new(&ncdbg).parent().unwrap(), tc.join("netcoredbg")).unwrap();
    let root = tmp.path().join("repo");
    copy_dir(
        &Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/dotnet-multi-tests"),
        &root,
    );
    let (_ctx, app) = build_test_app(data.clone(), &tmp.path().join("state.db")).await;
    let env = json!({
        "DOTNET_ROOT": dotnet,
        "NUGET_PACKAGES": nuget,
        "DOTNET_CLI_HOME": tmp.path().join("home").to_string_lossy(),
        "DOTNET_CLI_TELEMETRY_OPTOUT": "1",
        "DOTNET_NOLOGO": "1",
    });
    Some(Live {
        app,
        root,
        env,
        _tmp: tmp,
    })
}

impl Live {
    fn path(&self, rel: &str) -> String {
        self.root.join(rel).to_string_lossy().into_owned()
    }

    fn bp(&self, rel: &str, line: u32, enabled: bool) -> Value {
        json!({ "path": self.path(rel), "line": line, "enabled": enabled })
    }

    async fn start(&self, launch: Value, bps: Vec<Value>) -> String {
        let mut launch = launch;
        launch["env"] = self.env.clone();
        launch["cwd"] = json!(self.root.to_string_lossy());
        let body = json!({
            "target": { "kind": "local", "root": self.root.to_string_lossy() },
            "language": "csharp",
            "launch": launch,
            "breakpoints": bps,
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

    /// Wait for a pause or the end; `hosts` tracks the most test hosts seen at once.
    async fn next_stop(&self, id: &str, hosts: &mut usize) -> Value {
        let deadline = Instant::now() + Duration::from_secs(300);
        loop {
            *hosts = (*hosts).max(self.processes("testhost").len());
            let s = self.snap(id).await;
            if matches!(s["state"].as_str(), Some("paused" | "terminated" | "error")) {
                return s;
            }
            assert!(Instant::now() < deadline, "no stop: {}", brief(&s));
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    }

    /// This fixture's processes whose command line contains `what`.
    fn processes(&self, what: &str) -> Vec<String> {
        let out = std::process::Command::new("ps")
            .args(["-axo", "pid=,command="])
            .output()
            .unwrap();
        let root = self.root.to_string_lossy().into_owned();
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .filter(|l| {
                l.contains(what)
                    && (l.contains(&root)
                        || l.contains("/private")
                            && l.contains(root.trim_start_matches("/private")))
            })
            .map(str::to_string)
            .collect()
    }
}

fn brief(s: &Value) -> String {
    format!(
        "state={} step={} paused={}:{} error={} steps={}",
        s["state"],
        s["current_step"],
        s["paused_file"]
            .as_str()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .unwrap_or(""),
        s["paused_line"],
        s["error"],
        s["steps"]
    )
}

fn step_label(s: &Value) -> String {
    let i = s["current_step"].as_u64().unwrap_or(99) as usize;
    s["steps"][i]["label"].as_str().unwrap_or("?").to_string()
}

fn where_paused(s: &Value) -> String {
    format!(
        "{}:{}",
        s["paused_file"]
            .as_str()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .unwrap_or(""),
        s["paused_line"]
    )
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
#[ignore = "live: needs dotnet, netcoredbg and a NuGet cache (see the module doc)"]
async fn dotnet_debug_all_tests_runs_one_step_per_project_and_framework() {
    let Some(l) = live().await else { return };
    let all = json!({ "program": l.path("All.sln"), "is_test": true });

    // D1 + D2: four steps in order, each stops in Calc.Add, one test host at a time.
    let id = l
        .start(all.clone(), vec![l.bp("src/Calc/Calc.cs", 7, true)])
        .await;
    let mut hosts = 0usize;
    let mut stops: Vec<(String, String)> = Vec::new();
    let end = loop {
        let s = l.next_stop(&id, &mut hosts).await;
        if s["state"] != "paused" {
            break s;
        }
        stops.push((step_label(&s), where_paused(&s)));
        l.command(&id, "continue").await;
    };
    eprintln!("D1 stops: {stops:?}");
    eprintln!("D1 max test hosts at once: {hosts}");
    eprintln!("D1 end: {}", brief(&end));
    let text = console(&end);
    for line in text.lines().filter(|l| {
        l.starts_with("── Step")
            || l.contains(" steps ·")
            || l.contains("Failed ")
            || l.contains("Passed!")
            || l.contains("Failed!")
            || l.contains("total:")
    }) {
        eprintln!("console: {line}");
    }
    assert_eq!(end["state"], "terminated", "{}", brief(&end));
    assert_eq!(end["title"], "Debug all tests · 3 test projects");
    let labels: Vec<&str> = end["steps"]
        .as_array()
        .unwrap()
        .iter()
        .map(|s| s["label"].as_str().unwrap())
        .collect();
    assert_eq!(
        labels,
        [
            "Core.Tests (net10.0)",
            "Api.Tests (net10.0)",
            "Api.Tests (net11.0)",
            "Mtp.Tests (net10.0)"
        ]
    );
    for label in &labels {
        assert!(
            stops
                .iter()
                .any(|(st, at)| st == label && at == "Calc.cs:7"),
            "{label} never stopped in Calc.Add: {stops:?}"
        );
    }
    assert!(hosts <= 1, "{hosts} test hosts ran at once");
    let st = |i: usize| {
        (
            end["steps"][i]["state"].as_str().unwrap().to_string(),
            end["steps"][i]["summary"]
                .as_str()
                .unwrap_or("")
                .to_string(),
        )
    };
    assert_eq!(st(0), ("passed".into(), "3 passed".into()));
    assert_eq!(st(1).0, "passed");
    assert_eq!(st(2), ("failed".into(), "1 of 2 failed".into()));
    assert!(
        end["steps"][2]["failed_tests"]
            .to_string()
            .contains("FailsOnlyOnNet11"),
        "{}",
        end["steps"][2]
    );
    assert_eq!(st(3), ("passed".into(), "1 passed".into()));
    assert!(text.contains("4 steps · 3 passed · 1 failed"), "{text}");

    // D3: a breakpoint added while step 1 is paused (and the first one disabled)
    // is the only one step 2 stops at.
    let id = l
        .start(all.clone(), vec![l.bp("src/Calc/Calc.cs", 7, true)])
        .await;
    let s = l.next_stop(&id, &mut hosts).await;
    assert_eq!(
        (s["current_step"].as_u64(), where_paused(&s)),
        (Some(0), "Calc.cs:7".into()),
        "{}",
        brief(&s)
    );
    l.set_bps(
        &id,
        vec![
            l.bp("src/Calc/Calc.cs", 7, false),
            l.bp("tests/Api.Tests/ApiTests.cs", 9, true),
        ],
    )
    .await;
    l.command(&id, "continue").await;
    let s = l.next_stop(&id, &mut hosts).await;
    eprintln!("D3 next stop: {}", brief(&s));
    assert_eq!(
        (s["current_step"].as_u64(), where_paused(&s)),
        (Some(1), "ApiTests.cs:9".into()),
        "{}",
        brief(&s)
    );
    l.command(&id, "stop").await;

    // D4: Skip during step 2 starts step 3; no host of step 2 is left.
    let id = l
        .start(
            all.clone(),
            vec![l.bp("tests/Api.Tests/ApiTests.cs", 9, true)],
        )
        .await;
    let s = l.next_stop(&id, &mut hosts).await;
    assert_eq!(
        (s["current_step"].as_u64(), where_paused(&s)),
        (Some(1), "ApiTests.cs:9".into()),
        "{}",
        brief(&s)
    );
    let step2_hosts = l.processes("testhost");
    eprintln!("D4 hosts in step 2: {}", step2_hosts.len());
    l.command(&id, "skip_step").await;
    let s = l.next_stop(&id, &mut hosts).await;
    eprintln!("D4 after skip: {}", brief(&s));
    assert_eq!(
        (s["current_step"].as_u64(), where_paused(&s)),
        (Some(2), "ApiTests.cs:9".into()),
        "{}",
        brief(&s)
    );
    assert_eq!(s["steps"][1]["state"], "skipped");
    let pid = |l: &String| l.split_whitespace().next().unwrap_or("").to_string();
    let now: Vec<String> = l.processes("testhost").iter().map(pid).collect();
    assert!(
        step2_hosts.iter().map(pid).all(|p| !now.contains(&p)),
        "a step 2 host is left: {step2_hosts:?} / {now:?}"
    );

    // D5: Stop during step 3: terminated, step 4 skipped, no process of the run left.
    let stopped = l.command(&id, "stop").await;
    assert_eq!(stopped["state"], "terminated");
    assert_eq!(stopped["steps"][3]["state"], "skipped");
    tokio::time::sleep(Duration::from_secs(2)).await;
    let left: Vec<String> = ["testhost", "dotnet test", "netcoredbg"]
        .iter()
        .flat_map(|w| l.processes(w))
        .collect();
    assert!(left.is_empty(), "left running: {left:?}");

    // D7: the failed test's Debug: one session for that test on net11.0.
    let one = json!({
        "program": l.path("tests/Api.Tests/Api.Tests.csproj"),
        "is_test": true,
        "test_name": "FailsOnlyOnNet11",
        "test_framework": "net11.0",
    });
    let id = l
        .start(one, vec![l.bp("tests/Api.Tests/ApiTests.cs", 18, true)])
        .await;
    let s = l.next_stop(&id, &mut hosts).await;
    eprintln!("D7: {}", brief(&s));
    assert_eq!(
        (s["state"].as_str(), where_paused(&s)),
        (Some("paused"), "ApiTests.cs:18".into()),
        "{}",
        brief(&s)
    );
    assert!(
        s["steps"].as_array().is_none_or(|a| a.is_empty()),
        "a single test has no steps"
    );
    l.command(&id, "stop").await;

    // D9: a single-project .NET Test stays one session with no steps.
    let core = json!({ "program": l.path("tests/Core.Tests/Core.Tests.csproj"), "is_test": true });
    let id = l.start(core, vec![l.bp("src/Calc/Calc.cs", 7, true)]).await;
    let s = l.next_stop(&id, &mut hosts).await;
    eprintln!("D9: {}", brief(&s));
    assert_eq!(
        (s["state"].as_str(), where_paused(&s)),
        (Some("paused"), "Calc.cs:7".into()),
        "{}",
        brief(&s)
    );
    assert!(s["steps"].as_array().is_none_or(|a| a.is_empty()));
    l.command(&id, "stop").await;
}
