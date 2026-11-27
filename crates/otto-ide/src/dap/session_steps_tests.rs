//! Debug All step engine: scripted mock adapters over loopback TCP, no toolchains.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures_util::future::BoxFuture;
use otto_core::{Error, Result};
use serde_json::{json, Value};
use tokio::net::TcpListener;

use super::*;
use crate::dap::adapters::{DapAdapterTransport, ResolvedDapAdapter};
use crate::dap::wire::{DapMessage, DapReader, DapWriter};
use crate::dap::{DapManager, DapStartContext};
use crate::types::{
    IdeBreakpoint, IdeDebugAction, IdeDebugCommandReq, IdeDebugLaunchSpec, IdeDebugSessionSnapshot,
    IdeDebugState, IdeDebugStepState, IdeSetBreakpointsReq, IdeStartDebugReq, IdeTargetSpec,
};

/// What one mock step's debuggee does.
#[derive(Clone, Copy)]
struct Script {
    /// Stop at the first breakpoint after `configurationDone` (when one is set).
    pause: bool,
    exit_code: i64,
    output: &'static str,
}

#[derive(Default)]
struct Record {
    /// `(step program, setBreakpoints lines)` per adapter connection, in order.
    set_bps: Vec<(String, Vec<u64>)>,
    resolves: Vec<String>,
}

/// Plans the given steps; each step program boots a scripted adapter (or fails
/// to start when it has no script).
struct MockResolver {
    steps: Vec<&'static str>,
    ports: HashMap<String, u16>,
    record: Arc<Mutex<Record>>,
}

impl DebugResolver for MockResolver {
    fn plan<'a>(
        &'a self,
        req: &'a IdeStartDebugReq,
        _ctx: &'a DapStartContext,
    ) -> BoxFuture<'a, Result<Option<DebugPlan>>> {
        let steps = self
            .steps
            .iter()
            .map(|p| {
                let mut r = req.clone();
                r.launch.program = (*p).to_string();
                r.launch.debug_step = true;
                PlannedStep {
                    label: format!("{p} (net10.0)"),
                    req: r,
                    kind: StepKind::VsTest,
                    error: None,
                    program: (*p).to_string(),
                }
            })
            .collect();
        Box::pin(async move {
            Ok(Some(DebugPlan {
                title: "Debug all tests · 3 test projects".into(),
                steps,
                output: vec!["Reading test projects…".into()],
            }))
        })
    }

    fn resolve<'a>(
        &'a self,
        req: &'a IdeStartDebugReq,
        _ctx: &'a DapStartContext,
    ) -> BoxFuture<'a, Result<ResolvedDapAdapter>> {
        let program = req.launch.program.clone();
        self.record.lock().unwrap().resolves.push(program.clone());
        let port = self.ports.get(&program).copied();
        Box::pin(async move {
            let port =
                port.ok_or_else(|| Error::Invalid(format!("{program} cannot start\nmore")))?;
            Ok(ResolvedDapAdapter {
                adapter_name: "mock".into(),
                language: "csharp".into(),
                transport: DapAdapterTransport::DirectTcpAttach {
                    host: "127.0.0.1".into(),
                    port,
                },
                initialize_args: json!({}),
                launch_command: "launch".into(),
                launch_args: json!({}),
                normalized_cwd: String::new(),
                initial_output: Vec::new(),
            })
        })
    }
}

async fn serve_step(
    listener: TcpListener,
    program: String,
    script: Script,
    record: Arc<Mutex<Record>>,
) {
    while let Ok((sock, _)) = listener.accept().await {
        let (program, record) = (program.clone(), Arc::clone(&record));
        tokio::spawn(async move {
            let (r, w) = sock.into_split();
            let mut reader = DapReader::new(r);
            let mut w = DapWriter::new(w);
            let mut has_bp = false;
            record
                .lock()
                .unwrap()
                .set_bps
                .push((program.clone(), Vec::new()));
            while let Ok(Some(DapMessage::Request(req))) = reader.next_message().await {
                let args = req.arguments.clone().unwrap_or(json!({}));
                let body = match req.command.as_str() {
                    "initialize" => json!({
                        "supportsConfigurationDoneRequest": true,
                        "supportsTerminateRequest": true
                    }),
                    "setBreakpoints" => {
                        let bps = args["breakpoints"].as_array().cloned().unwrap_or_default();
                        let lines: Vec<u64> =
                            bps.iter().filter_map(|b| b["line"].as_u64()).collect();
                        has_bp |= !lines.is_empty();
                        if let Some(last) = record.lock().unwrap().set_bps.last_mut() {
                            last.1.extend(lines);
                        }
                        let verified: Vec<Value> = bps
                            .iter()
                            .map(|b| json!({ "verified": true, "line": b["line"] }))
                            .collect();
                        json!({ "breakpoints": verified })
                    }
                    "threads" => json!({ "threads": [{ "id": 1, "name": "main" }] }),
                    "stackTrace" => json!({ "stackFrames": [{
                        "id": 1, "name": "Calc.Add", "line": 3, "column": 1,
                        "source": { "path": "/ws/Calc.cs" }
                    }] }),
                    "scopes" => json!({ "scopes": [] }),
                    _ => json!({}),
                };
                let _ = w.send_response(req.seq, &req.command, Ok(Some(body))).await;
                let finish = match req.command.as_str() {
                    "launch" => {
                        let _ = w.send_event("initialized", Some(json!({}))).await;
                        false
                    }
                    "configurationDone" if script.pause && has_bp => {
                        let _ = w
                            .send_event(
                                "stopped",
                                Some(json!({ "reason": "breakpoint", "threadId": 1 })),
                            )
                            .await;
                        false
                    }
                    "configurationDone" | "continue" => true,
                    "disconnect" => break,
                    _ => false,
                };
                if finish {
                    let out = json!({ "category": "stdout", "output": script.output });
                    let _ = w.send_event("output", Some(out)).await;
                    let _ = w
                        .send_event("exited", Some(json!({ "exitCode": script.exit_code })))
                        .await;
                    let _ = w.send_event("terminated", Some(json!({}))).await;
                }
            }
        });
    }
}

async fn manager(
    steps: &[(&'static str, Option<Script>)],
) -> (DapManager, Arc<Mutex<Record>>, tempfile::TempDir) {
    let tmp = tempfile::tempdir().unwrap();
    let record = Arc::new(Mutex::new(Record::default()));
    let mut ports = HashMap::new();
    for (program, script) in steps {
        if let Some(script) = script {
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            ports.insert(program.to_string(), listener.local_addr().unwrap().port());
            tokio::spawn(serve_step(
                listener,
                program.to_string(),
                *script,
                Arc::clone(&record),
            ));
        }
    }
    let resolver = MockResolver {
        steps: steps.iter().map(|(p, _)| *p).collect(),
        ports,
        record: Arc::clone(&record),
    };
    let mgr = DapManager::new(tmp.path().to_path_buf()).with_resolver(Arc::new(resolver));
    (mgr, record, tmp)
}

fn start_req(root: &std::path::Path, breakpoints: Vec<IdeBreakpoint>) -> IdeStartDebugReq {
    IdeStartDebugReq {
        target: IdeTargetSpec::local(root.to_string_lossy().into_owned()),
        language: "csharp".into(),
        launch: IdeDebugLaunchSpec {
            program: root.join("All.sln").to_string_lossy().into_owned(),
            is_test: true,
            ..IdeDebugLaunchSpec::default()
        },
        breakpoints,
        ..IdeStartDebugReq::default()
    }
}

fn bp(line: u32) -> IdeBreakpoint {
    IdeBreakpoint {
        path: "/ws/Calc.cs".into(),
        line,
        ..IdeBreakpoint::default()
    }
}

async fn wait_for(
    mgr: &DapManager,
    id: &str,
    what: &str,
    ok: impl Fn(&IdeDebugSessionSnapshot) -> bool,
) -> IdeDebugSessionSnapshot {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let snap = mgr.get_session(id).unwrap();
        if ok(&snap) {
            return snap;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {what}: {:?} {:?}",
            snap.state,
            snap.steps
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

fn cmd(action: IdeDebugAction) -> IdeDebugCommandReq {
    IdeDebugCommandReq {
        action,
        thread_id: None,
        frame_id: None,
        path: None,
        line: None,
    }
}

fn states(snap: &IdeDebugSessionSnapshot) -> Vec<IdeDebugStepState> {
    snap.steps.iter().map(|s| s.state).collect()
}

const PASS: Script = Script {
    pause: false,
    exit_code: 0,
    output: "Passed!  - Failed:     0, Passed:     3, Skipped:     0, Total:     3\n",
};
const FAIL: Script = Script {
    pause: false,
    exit_code: 1,
    output: "  Failed AddFails [3 ms]\nFailed!  - Failed:     1, Passed:    16, Skipped:     0, Total:    17\n",
};
const PAUSE: Script = Script {
    pause: true,
    exit_code: 0,
    output: "",
};

#[tokio::test]
async fn steps_run_in_order_and_record_pass_fail_and_could_not_start() {
    let (mgr, _rec, tmp) =
        manager(&[("Core", Some(PASS)), ("Api", Some(FAIL)), ("Mtp", None)]).await;
    let ctx = DapStartContext::new().with_caller_is_root(true);
    let (snap, _rx) = mgr
        .start_session_async(start_req(tmp.path(), Vec::new()), ctx)
        .unwrap();
    let done = wait_for(&mgr, &snap.id, "the run to end", |s| {
        s.state == IdeDebugState::Terminated
    })
    .await;
    assert_eq!(done.title, "Debug all tests · 3 test projects");
    assert_eq!(
        states(&done),
        vec![
            IdeDebugStepState::Passed,
            IdeDebugStepState::Failed,
            IdeDebugStepState::Error
        ]
    );
    assert_eq!(done.steps[0].summary.as_deref(), Some("3 passed"));
    assert_eq!(done.steps[1].summary.as_deref(), Some("1 of 17 failed"));
    assert_eq!(done.steps[1].failed_tests, vec!["AddFails".to_string()]);
    assert_eq!(done.steps[1].exit_code, Some(1));
    assert_eq!(done.steps[2].summary.as_deref(), Some("Mtp cannot start"));
    assert_eq!(done.current_step, None);
    let text: String = done.output.iter().map(|o| o.text.as_str()).collect();
    let seps: Vec<usize> = [
        "── Step 1/3 · Core (net10.0) ──",
        "── Step 2/3 · Api (net10.0) ──",
        "── Step 3/3 · Mtp (net10.0) ──",
    ]
    .iter()
    .map(|s| {
        text.find(s)
            .unwrap_or_else(|| panic!("missing {s} in {text}"))
    })
    .collect();
    assert!(seps.windows(2).all(|w| w[0] < w[1]), "{text}");
    assert!(
        text.contains(
            "3 steps · 1 passed · 1 failed · 1 step couldn't start (Api (net10.0): 1 of 17 failed)"
        ),
        "{text}"
    );
}

#[tokio::test]
async fn a_breakpoint_added_while_step_one_is_paused_is_sent_to_step_two() {
    let (mgr, rec, tmp) = manager(&[("Core", Some(PAUSE)), ("Api", Some(PASS))]).await;
    let ctx = DapStartContext::new().with_caller_is_root(true);
    let (snap, _rx) = mgr
        .start_session_async(start_req(tmp.path(), vec![bp(3)]), ctx)
        .unwrap();
    let id = snap.id;
    wait_for(&mgr, &id, "step 1 to pause", |s| {
        s.state == IdeDebugState::Paused
    })
    .await;
    let edited = IdeSetBreakpointsReq {
        breakpoints: vec![
            IdeBreakpoint {
                enabled: false,
                ..bp(3)
            },
            bp(7),
        ],
        break_on_exceptions: false,
    };
    mgr.set_breakpoints(&id, edited).await.unwrap();
    mgr.command(&id, cmd(IdeDebugAction::Continue))
        .await
        .unwrap();
    wait_for(&mgr, &id, "the run to end", |s| {
        s.state == IdeDebugState::Terminated
    })
    .await;
    let rec = rec.lock().unwrap();
    let api: Vec<&Vec<u64>> = rec
        .set_bps
        .iter()
        .filter(|(p, _)| p == "Api")
        .map(|(_, l)| l)
        .collect();
    assert_eq!(
        api,
        vec![&vec![7]],
        "step 2 gets the edited breakpoints only"
    );
}

#[tokio::test]
async fn skip_ends_the_step_early_and_stop_skips_the_rest() {
    let (mgr, _rec, tmp) = manager(&[
        ("Core", Some(PASS)),
        ("Api", Some(PAUSE)),
        ("Mtp", Some(PAUSE)),
        ("Last", Some(PASS)),
    ])
    .await;
    let ctx = DapStartContext::new().with_caller_is_root(true);
    let (snap, _rx) = mgr
        .start_session_async(start_req(tmp.path(), vec![bp(3)]), ctx)
        .unwrap();
    let id = snap.id;
    wait_for(&mgr, &id, "step 2 to pause", |s| {
        s.state == IdeDebugState::Paused && s.current_step == Some(1)
    })
    .await;
    mgr.command(&id, cmd(IdeDebugAction::SkipStep))
        .await
        .unwrap();
    let s = wait_for(&mgr, &id, "step 3 to pause", |s| {
        s.state == IdeDebugState::Paused && s.current_step == Some(2)
    })
    .await;
    assert_eq!(s.steps[1].state, IdeDebugStepState::Skipped);
    let stopped = mgr.command(&id, cmd(IdeDebugAction::Stop)).await.unwrap();
    assert_eq!(stopped.state, IdeDebugState::Terminated);
    assert_eq!(
        states(&stopped),
        vec![
            IdeDebugStepState::Passed,
            IdeDebugStepState::Skipped,
            IdeDebugStepState::Skipped,
            IdeDebugStepState::Skipped
        ]
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let later = mgr.get_session(&id).unwrap();
    assert_eq!(
        later.state,
        IdeDebugState::Terminated,
        "no step starts after Stop"
    );
    assert_eq!(later.steps[3].started_ms, None);
}

#[tokio::test]
async fn restart_runs_the_whole_plan_again_from_step_one() {
    let (mgr, rec, tmp) = manager(&[("Core", Some(PASS)), ("Api", Some(PAUSE))]).await;
    let ctx = DapStartContext::new().with_caller_is_root(true);
    let (snap, _rx) = mgr
        .start_session_async(start_req(tmp.path(), vec![bp(3)]), ctx)
        .unwrap();
    let id = snap.id;
    wait_for(&mgr, &id, "step 2 to pause", |s| {
        s.state == IdeDebugState::Paused && s.current_step == Some(1)
    })
    .await;
    mgr.command(&id, cmd(IdeDebugAction::Restart))
        .await
        .unwrap();
    let again = wait_for(&mgr, &id, "step 2 to pause again", |s| {
        s.state == IdeDebugState::Paused && s.current_step == Some(1)
    })
    .await;
    assert_eq!(again.steps[0].state, IdeDebugStepState::Passed);
    assert_eq!(
        rec.lock().unwrap().resolves,
        vec!["Core", "Api", "Core", "Api"],
        "Restart starts again at step 1"
    );
    mgr.command(&id, cmd(IdeDebugAction::Stop)).await.unwrap();
}

#[test]
fn step_results_read_the_runners_summaries_and_exit_codes() {
    let zig_pass = step_result(StepKind::Zig, Some(0), "All 12 tests passed.\n");
    assert_eq!(
        (zig_pass.state, zig_pass.summary.as_deref()),
        (IdeDebugStepState::Passed, Some("12 passed"))
    );
    let zig_out = "1/3 extra.test.extra passes...OK\n2/3 extra.test.extra fails...expected 6, found 5\nFAIL (TestExpectedEqual)\n/x/extra.zig:11:5: in test.extra fails\n3/3 extra.test.extra panics...OK\n2 passed; 0 skipped; 1 failed.\n";
    let zig_fail = step_result(StepKind::Zig, Some(1), zig_out);
    assert_eq!(
        (zig_fail.state, zig_fail.summary.as_deref()),
        (IdeDebugStepState::Failed, Some("1 of 3 failed"))
    );
    assert_eq!(zig_fail.failed_tests, vec!["extra fails".to_string()]);
    let mtp = "Test run summary: Passed!\n  total: 3\n  failed: 0\n  succeeded: 3\n  skipped: 0\n";
    let r = step_result(StepKind::Mtp, Some(0), mtp);
    assert_eq!(
        (r.state, r.summary.as_deref()),
        (IdeDebugStepState::Passed, Some("3 passed"))
    );
    assert_eq!(
        step_result(StepKind::Mtp, Some(8), "").state,
        IdeDebugStepState::NoTests
    );
    assert_eq!(
        step_result(StepKind::Mtp, Some(2), "").state,
        IdeDebugStepState::Failed
    );
    let crash = step_result(StepKind::VsTest, Some(134), "");
    assert_eq!(
        (crash.state, crash.summary.as_deref()),
        (IdeDebugStepState::Error, Some("exited with code 134"))
    );
    let none = step_result(
        StepKind::VsTest,
        Some(0),
        "Passed!  - Failed:     0, Passed:     0, Skipped:     0, Total:     0\n",
    );
    assert_eq!(none.state, IdeDebugStepState::NoTests);
}
