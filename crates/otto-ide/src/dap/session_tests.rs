use std::sync::Mutex;
use std::time::Duration;

use serde_json::{json, Value};

use super::*;
use crate::dap::{DapMessage, DapReader, DapWriter};
use crate::types::{
    IdeBreakpoint, IdeDebugEvaluateReq, IdeDebugLaunchSpec, IdeDebugState, IdeScope, IdeTargetSpec,
    IdeVariable,
};

fn spawn_mock_dap_server(
    server_io: tokio::io::DuplexStream,
    canon_src_server: String,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let (sr, sw) = tokio::io::split(server_io);
        let mut r = DapReader::new(sr);
        let mut w = DapWriter::new(sw);
        let mut step_count = 0u32;

        while let Some(DapMessage::Request(req)) = r.next_message().await.unwrap() {
            match req.command.as_str() {
                "initialize" => {
                    w.send_response(
                        req.seq,
                        "initialize",
                        Ok(Some(json!({
                            "supportsConfigurationDoneRequest": true,
                            "supportsTerminateRequest": true,
                            "exceptionBreakpointFilters": [
                                { "filter": "unrecovered-panic", "label": "Panic" }
                            ]
                        }))),
                    )
                    .await
                    .unwrap();
                }
                "launch" => {
                    w.send_event("initialized", Some(json!({}))).await.unwrap();
                    w.send_response(req.seq, "launch", Ok(Some(json!({}))))
                        .await
                        .unwrap();
                }
                "setBreakpoints" => {
                    let bps = req
                        .arguments
                        .as_ref()
                        .and_then(|a| a.get("breakpoints"))
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    w.send_response(
                        req.seq,
                        "setBreakpoints",
                        Ok(Some(json!({ "breakpoints": bps }))),
                    )
                    .await
                    .unwrap();
                }
                "configurationDone" => {
                    w.send_response(req.seq, "configurationDone", Ok(Some(json!({}))))
                        .await
                        .unwrap();
                    w.send_event(
                        "output",
                        Some(json!({ "category": "stdout", "output": "=== RUN   TestAdd\n" })),
                    )
                    .await
                    .unwrap();
                    w.send_event(
                        "stopped",
                        Some(json!({ "reason": "breakpoint", "threadId": 1 })),
                    )
                    .await
                    .unwrap();
                }
                "threads" => {
                    w.send_response(
                        req.seq,
                        "threads",
                        Ok(Some(json!({
                            "threads": [
                                { "id": 1, "name": "goroutine 1 (running)" },
                                { "id": 2, "name": "goroutine 14 (chan receive)" }
                            ]
                        }))),
                    )
                    .await
                    .unwrap();
                }
                "stackTrace" => {
                    let tid = req
                        .arguments
                        .as_ref()
                        .and_then(|a| a.get("threadId"))
                        .and_then(Value::as_i64)
                        .unwrap_or(1);
                    if tid == 2 {
                        w.send_response(
                            req.seq,
                            "stackTrace",
                            Ok(Some(json!({
                                "stackFrames": [{
                                    "id": 2000,
                                    "name": "calc.workerLoop",
                                    "source": { "path": canon_src_server },
                                    "line": 4,
                                    "column": 1
                                }]
                            }))),
                        )
                        .await
                        .unwrap();
                        continue;
                    }
                    let line = if step_count == 0 { 6 } else { 7 };
                    w.send_response(
                        req.seq,
                        "stackTrace",
                        Ok(Some(json!({
                            "stackFrames": [{
                                "id": 1000,
                                "name": "calc.TestAdd",
                                "source": { "path": canon_src_server },
                                "line": line,
                                "column": 1
                            }]
                        }))),
                    )
                    .await
                    .unwrap();
                }
                "scopes" => {
                    let fid = req
                        .arguments
                        .as_ref()
                        .and_then(|a| a.get("frameId"))
                        .and_then(Value::as_i64)
                        .unwrap_or(1000);
                    let vref = if fid == 2000 { 2002 } else { 2001 };
                    w.send_response(
                        req.seq,
                        "scopes",
                        Ok(Some(json!({
                            "scopes": [{ "name": "Locals", "variablesReference": vref, "expensive": false }]
                        }))),
                    )
                    .await
                    .unwrap();
                }
                "variables" => {
                    let vref = req
                        .arguments
                        .as_ref()
                        .and_then(|a| a.get("variablesReference"))
                        .and_then(Value::as_i64)
                        .unwrap_or(0);
                    let vars = if vref == 2001 {
                        json!([
                            { "name": "x", "value": "20", "type": "int", "variablesReference": 0 },
                            { "name": "y", "value": "22", "type": "int", "variablesReference": 0 },
                            { "name": "sum", "value": "42", "type": "int", "variablesReference": 0 },
                            { "name": "meta", "value": "{ok: true}", "type": "struct", "variablesReference": 3001 }
                        ])
                    } else if vref == 2002 {
                        json!([
                            { "name": "x", "value": "999", "type": "int", "variablesReference": 0 }
                        ])
                    } else {
                        json!([
                            { "name": "ok", "value": "true", "type": "bool", "variablesReference": 0 }
                        ])
                    };
                    w.send_response(req.seq, "variables", Ok(Some(json!({ "variables": vars }))))
                        .await
                        .unwrap();
                }
                "evaluate" => {
                    let expr = req
                        .arguments
                        .as_ref()
                        .and_then(|a| a.get("expression"))
                        .and_then(Value::as_str)
                        .unwrap_or("");
                    w.send_response(
                        req.seq,
                        "evaluate",
                        Ok(Some(json!({
                            "result": format!("{expr} => 42"),
                            "type": "int",
                            "variablesReference": 0
                        }))),
                    )
                    .await
                    .unwrap();
                }
                "next" => {
                    step_count += 1;
                    w.send_response(req.seq, "next", Ok(Some(json!({}))))
                        .await
                        .unwrap();
                    w.send_event("stopped", Some(json!({ "reason": "step", "threadId": 1 })))
                        .await
                        .unwrap();
                }
                "terminate" => {
                    w.send_response(req.seq, "terminate", Ok(Some(json!({}))))
                        .await
                        .unwrap();
                }
                "disconnect" => {
                    w.send_response(req.seq, "disconnect", Ok(Some(json!({}))))
                        .await
                        .unwrap();
                    w.send_event("terminated", Some(json!({}))).await.unwrap();
                    break;
                }
                other => {
                    w.send_response(req.seq, other, Ok(Some(json!({}))))
                        .await
                        .unwrap();
                }
            }
        }
    })
}

#[tokio::test]
async fn full_mock_dap_lifecycle_over_duplex() {
    let tmp = tempfile::tempdir().unwrap();
    let src_file = tmp.path().join("calc_test.go");
    std::fs::write(
        &src_file,
        "package calc\n\nfunc TestAdd() {\n\tx := 20\n\ty := 22\n\tsum := x + y\n\t_ = sum\n}\n",
    )
    .unwrap();
    let canon_src = std::fs::canonicalize(&src_file)
        .unwrap()
        .to_string_lossy()
        .into_owned();

    let (client_io, server_io) = tokio::io::duplex(65536);
    let server_task = spawn_mock_dap_server(server_io, canon_src.clone());

    let mgr = DapManager::new(tmp.path().to_path_buf());
    let start_req = IdeStartDebugReq {
        target: IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned()),
        language: "go".into(),
        config_id: None,
        launch: IdeDebugLaunchSpec {
            program: canon_src.clone(),
            args: Vec::new(),
            cwd: Some(tmp.path().to_string_lossy().into_owned()),
            test_name: Some("TestAdd".into()),
            attach_port: None,
            env: BTreeMap::new(),
            ..IdeDebugLaunchSpec::default()
        },
        breakpoints: vec![IdeBreakpoint {
            path: canon_src.clone(),
            line: 6,
            enabled: true,
            condition: Some("x == 20".into()),
            hit_condition: None,
            log_message: None,
            verified: None,
            message: None,
        }],
        ..IdeStartDebugReq::default()
    };

    let started = mgr
        .start_session_with_stream(start_req, "dlv", client_io, true)
        .await
        .unwrap();
    assert_eq!(started.breakpoints[0].verified, Some(true));
    let mut sub = mgr.subscribe_events(&started.id).unwrap();

    let paused = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snap = mgr.get_session(&started.id).unwrap();
            if snap.state == IdeDebugState::Paused {
                return snap;
            }
            let _ = sub.recv().await;
        }
    })
    .await
    .expect("timed out waiting for paused snapshot");

    assert_eq!(paused.stopped_reason.as_deref(), Some("breakpoint"));
    assert_eq!(paused.stop_seq, 1);
    assert_eq!(paused.paused_line, Some(6));
    assert_eq!(paused.threads.len(), 2);
    assert_eq!(paused.stack_frames.len(), 1);
    assert_eq!(paused.scopes.len(), 1);
    assert!(paused.scopes[0]
        .variables
        .iter()
        .any(|v| v.name == "sum" && v.value == "42"));
    assert!(paused
        .inline_values
        .iter()
        .any(|iv| iv.line == 6 && iv.name == "sum" && iv.value == "42"));

    // Switch active goroutine (thread 2) via SelectFrame with `frame_id: None` and verify scopes/inline refresh!
    let switched = mgr
        .command(
            &started.id,
            IdeDebugCommandReq {
                action: IdeDebugAction::SelectFrame,
                thread_id: Some(2),
                frame_id: None,
                path: None,
                line: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(switched.active_thread_id, Some(2));
    assert_eq!(switched.active_frame_id, Some(2000));
    assert_eq!(switched.paused_line, Some(4));
    assert!(switched.scopes[0]
        .variables
        .iter()
        .any(|v| v.name == "x" && v.value == "999"));

    // Switch back to goroutine 1 before stepping.
    mgr.command(
        &started.id,
        IdeDebugCommandReq {
            action: IdeDebugAction::SelectFrame,
            thread_id: Some(1),
            frame_id: Some(1000),
            path: None,
            line: None,
        },
    )
    .await
    .unwrap();

    let children = mgr.expand_variables(&started.id, 3001).await.unwrap();
    assert_eq!(children.len(), 1);
    assert_eq!(children[0].name, "ok");
    assert_eq!(children[0].value, "true");

    let eval = mgr
        .evaluate(
            &started.id,
            IdeDebugEvaluateReq {
                expression: "x + y".into(),
                frame_id: None,
                context: Some("repl".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(eval.result, "x + y => 42");

    let step_ack = mgr
        .command(
            &started.id,
            IdeDebugCommandReq {
                action: IdeDebugAction::StepOver,
                thread_id: Some(1),
                frame_id: None,
                path: None,
                line: None,
            },
        )
        .await
        .unwrap();

    let stepped = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snap = mgr.get_session(&started.id).unwrap();
            if snap.state == IdeDebugState::Paused && snap.paused_line == Some(7) {
                return snap;
            }
            let _ = sub.recv().await;
        }
    })
    .await
    .expect("timed out waiting for step_over pause");
    assert_eq!(stepped.stopped_reason.as_deref(), Some("step"));
    assert_eq!(stepped.stop_seq, 2, "every stop bumps stop_seq");
    assert!(stepped.revision > paused.revision);
    assert!(stepped.revision >= step_ack.revision);

    let stopped = mgr
        .command(
            &started.id,
            IdeDebugCommandReq {
                action: IdeDebugAction::Stop,
                thread_id: None,
                frame_id: None,
                path: None,
                line: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(stopped.state, IdeDebugState::Terminated);
    server_task.await.unwrap();
}

#[test]
fn inline_values_matches_nearby_lines_and_skips_comments() {
    let source =
        "// count should not match here\nlet count = 7;\nlet total = count * 6;\nreturn total;\n";
    let scopes = vec![IdeScope {
        name: "Locals".into(),
        variables_reference: 1,
        variables: vec![
            IdeVariable {
                name: "count".into(),
                value: "7".into(),
                var_type: Some("i32".into()),
                variables_reference: 0,
                indexed_variables: None,
            },
            IdeVariable {
                name: "total".into(),
                value: "42".into(),
                var_type: Some("i32".into()),
                variables_reference: 0,
                indexed_variables: None,
            },
        ],
    }];
    let inline = compute_inline_values(source, 4, &scopes);
    assert!(!inline.iter().any(|v| v.line == 1));
    assert!(inline
        .iter()
        .any(|v| v.line == 2 && v.name == "count" && v.value == "7"));
    assert!(inline
        .iter()
        .any(|v| v.line == 3 && v.name == "total" && v.value == "42"));
}

/// Paused in one function, a function above it with the same local names shows nothing.
#[test]
fn inline_values_stay_inside_the_paused_function() {
    let source = "func TestA(t *testing.T) {\n\tgot := 1\n\t_ = got\n}\n\n\
                  func TestB(t *testing.T) {\n\tgot := 2\n\t_ = got\n}\n";
    let var = |name: &str, value: &str| IdeVariable {
        name: name.into(),
        value: value.into(),
        var_type: None,
        variables_reference: 0,
        indexed_variables: None,
    };
    let scopes = vec![IdeScope {
        name: "Locals".into(),
        variables_reference: 1,
        variables: vec![var("t", "*testing.T"), var("got", "2")],
    }];
    let lines: Vec<u32> = compute_inline_values(source, 8, &scopes)
        .iter()
        .map(|v| v.line)
        .collect();
    assert_eq!(lines, vec![6, 7, 8], "{lines:?}");
}

#[tokio::test]
async fn launch_compile_error_fails_fast_without_hanging_on_initialized() {
    let (client_io, server_io) = tokio::io::duplex(16384);
    tokio::spawn(async move {
        let (sr, sw) = tokio::io::split(server_io);
        let mut r = DapReader::new(sr);
        let mut w = DapWriter::new(sw);
        while let Some(DapMessage::Request(req)) = r.next_message().await.unwrap() {
            match req.command.as_str() {
                "initialize" => {
                    w.send_response(
                        req.seq,
                        "initialize",
                        Ok(Some(json!({ "supportsConfigurationDoneRequest": true }))),
                    )
                    .await
                    .unwrap();
                }
                "launch" => {
                    // Simulate `dlv dap` failing `go build` without ever emitting `initialized`!
                    w.send_response(
                        req.seq,
                        "launch",
                        Err("Failed to launch: build error: undefined: missingPkg".into()),
                    )
                    .await
                    .unwrap();
                    break;
                }
                _ => {}
            }
        }
    });

    let mgr = DapManager::new(PathBuf::from("/tmp/otto-dap-fastfail"));
    let req = IdeStartDebugReq {
        target: IdeTargetSpec::local("/tmp"),
        language: "go".into(),
        config_id: None,
        launch: IdeDebugLaunchSpec {
            program: "/tmp/main.go".into(),
            ..IdeDebugLaunchSpec::default()
        },
        breakpoints: Vec::new(),
        ..IdeStartDebugReq::default()
    };
    let t0 = std::time::Instant::now();
    let err = mgr
        .start_session_with_stream(req, "dlv", client_io, true)
        .await
        .expect_err("compile failure should error immediately");
    assert!(
        t0.elapsed() < Duration::from_secs(1),
        "launch failure took {:?} instead of failing fast",
        t0.elapsed()
    );
    assert!(err.to_string().contains("undefined: missingPkg"));
}

#[cfg(unix)]
#[test]
fn symlink_breakpoint_normalization_and_go_package_resolution_work() {
    use std::os::unix::fs::symlink;

    let tmp = tempfile::tempdir().unwrap();
    let real_ws = tmp.path().join("real_repo");
    let link_ws = tmp.path().join("symlink_repo");
    std::fs::create_dir_all(real_ws.join("cmd/go_admission")).unwrap();
    std::fs::write(
        real_ws.join("cmd/go_admission/main.go"),
        "package main\nfunc main() { runServer() }\n",
    )
    .unwrap();
    std::fs::write(
        real_ws.join("cmd/go_admission/routes.go"),
        "package main\nfunc runServer() {}\n",
    )
    .unwrap();
    symlink(&real_ws, &link_ws).unwrap();

    let target = IdeTargetSpec::local(link_ws.to_string_lossy().into_owned());
    let norm_bp = normalize_target_path(
        &target,
        &link_ws.join("cmd/go_admission/main.go").to_string_lossy(),
    );
    let canon_main = real_ws
        .join("cmd/go_admission/main.go")
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(norm_bp, canon_main);

    // Multi-file `package main` resolves `main.go` -> package dir so sibling `routes.go` compiles!
    let canon_pkg = real_ws
        .join("cmd/go_admission")
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        crate::dap::adapters::resolve_go_debug_program(&target, &canon_main, &canon_pkg),
        canon_pkg
    );
    // Workspace root with `cmd/go_admission` auto-discovers `cmd/go_admission`!
    let canon_root = real_ws
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    assert_eq!(
        crate::dap::adapters::resolve_go_debug_program(&target, &canon_root, &canon_root),
        canon_pkg
    );
}

#[cfg(unix)]
#[tokio::test]
async fn shell_dap_bridge_hits_breakpoint_inspects_vars_and_killpg_cleans_grandchildren() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let script = root.join("deploy.sh");
    std::fs::write(
        &script,
        "#!/usr/bin/env bash\nSERVICE_PORT=7700\nNEXT_PORT=$((SERVICE_PORT + 1))\necho \"port=$NEXT_PORT\"\n",
    )
    .unwrap();

    let mgr = DapManager::new(root.join(".otto-data"));
    let req = IdeStartDebugReq {
        target: IdeTargetSpec::local(root.to_string_lossy().into_owned()),
        language: "shell".into(),
        config_id: None,
        launch: IdeDebugLaunchSpec {
            program: script.to_string_lossy().into_owned(),
            args: Vec::new(),
            cwd: Some(root.to_string_lossy().into_owned()),
            test_name: None,
            attach_port: None,
            env: BTreeMap::new(),
            ..IdeDebugLaunchSpec::default()
        },
        breakpoints: vec![IdeBreakpoint {
            path: script.to_string_lossy().into_owned(),
            line: 3,
            enabled: true,
            condition: None,
            hit_condition: None,
            log_message: None,
            verified: None,
            message: None,
        }],
        ..IdeStartDebugReq::default()
    };

    let started = mgr
        .start_session(req, DapStartContext::new())
        .await
        .unwrap();
    let mut sub = mgr.subscribe_events(&started.id).unwrap();

    let paused = tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let snap = mgr.get_session(&started.id).unwrap();
            if snap.state == IdeDebugState::Paused {
                return snap;
            }
            let _ = sub.recv().await;
        }
    })
    .await
    .expect("shell debugger should pause at line 3 breakpoint");

    assert_eq!(paused.paused_line, Some(3));
    assert!(paused.scopes[0]
        .variables
        .iter()
        .any(|v| v.name == "SERVICE_PORT" && v.value == "7700"));

    let eval = mgr
        .evaluate(
            &started.id,
            IdeDebugEvaluateReq {
                expression: "$SERVICE_PORT".into(),
                frame_id: None,
                context: Some("repl".into()),
            },
        )
        .await
        .unwrap();
    assert_eq!(eval.result, "7700");

    let stopped = mgr
        .command(
            &started.id,
            IdeDebugCommandReq {
                action: IdeDebugAction::Stop,
                thread_id: None,
                frame_id: None,
                path: None,
                line: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(stopped.state, IdeDebugState::Terminated);
}

#[test]
fn debug_output_ring_and_terminated_session_eviction_enforce_caps() {
    let dummy_req = || IdeStartDebugReq {
        target: IdeTargetSpec::local("/tmp"),
        language: "go".into(),
        launch: IdeDebugLaunchSpec {
            program: "main.go".into(),
            ..IdeDebugLaunchSpec::default()
        },
        ..IdeStartDebugReq::default()
    };

    let mgr = DapManager::new(PathBuf::from("/tmp/otto-dap-test"));
    for i in 0..10u64 {
        let id = format!("dbg_term_{i:02}");
        let meta = (id.clone(), "Test".into(), "go".into(), "dlv".into());
        let entry = SessionEntry::new(i, meta, Vec::new(), dummy_req(), DapStartContext::new());
        entry.mutate_and_broadcast(|snap| snap.state = IdeDebugState::Terminated);
        mgr.sessions.insert(id, entry);
        evict_terminated_sessions(&mgr.sessions);
    }

    let live_meta = ("dbg_live".into(), "Live".into(), "go".into(), "dlv".into());
    let live = SessionEntry::new(
        99,
        live_meta,
        Vec::new(),
        dummy_req(),
        DapStartContext::new(),
    );
    for i in 0..(MAX_DEBUG_OUTPUT + 250) {
        live.append_output("stdout", format!("line-{i}"));
    }
    mgr.sessions.insert("dbg_live".into(), Arc::clone(&live));
    evict_terminated_sessions(&mgr.sessions);

    assert_eq!(mgr.sessions.len(), MAX_TERMINATED_DEBUG_SESSIONS + 1);
    assert!(mgr.get_session("dbg_term_00").is_err());
    assert!(mgr.get_session("dbg_term_05").is_err());
    assert!(mgr.get_session("dbg_term_06").is_ok());
    assert!(mgr.get_session("dbg_term_09").is_ok());

    let snap = mgr.get_session("dbg_live").unwrap();
    assert_eq!(snap.output.len(), MAX_DEBUG_OUTPUT);
    assert_eq!(snap.output[0].text, "line-250");
    assert_eq!(
        snap.output[MAX_DEBUG_OUTPUT - 1].text,
        format!("line-{}", MAX_DEBUG_OUTPUT + 249)
    );
}

#[test]
fn go_test_run_pattern_anchors_subtest_segments_without_double_escaping() {
    use crate::dap::adapters_go_py::format_go_test_run_pattern;

    assert_eq!(format_go_test_run_pattern("TestAdd"), "^TestAdd$");
    assert_eq!(format_go_test_run_pattern("^TestAdd$"), "^TestAdd$");
    assert_eq!(
        format_go_test_run_pattern("^TestParse$/^empty_input$"),
        "^TestParse$/^empty_input$"
    );
    assert_eq!(
        format_go_test_run_pattern(r"^TestParse$/^case\.1$"),
        r"^TestParse$/^case\.1$"
    );
    assert_eq!(
        format_go_test_run_pattern("TestParse/case.1"),
        r"^TestParse$/^case\.1$"
    );
    assert_eq!(
        format_go_test_run_pattern("^TestParse/case(1)$"),
        r"^TestParse$/^case\(1\)$"
    );
    assert_eq!(
        format_go_test_run_pattern("TestSuite/group/case+1"),
        r"^TestSuite$/^group$/^case\+1$"
    );
}

#[tokio::test]
async fn exception_breakpoints_and_streamed_snapshot_events_over_duplex() {
    let tmp = tempfile::tempdir().unwrap();
    let src_file = tmp.path().join("panic_test.go");
    std::fs::write(
        &src_file,
        "package p\nfunc run() {\n\tx := 1\n\tpanic(x)\n}\n",
    )
    .unwrap();
    let canon_src = std::fs::canonicalize(&src_file)
        .unwrap()
        .to_string_lossy()
        .into_owned();

    let (client_io, server_io) = tokio::io::duplex(65536);
    let recorded_bp_args: Arc<Mutex<Vec<Value>>> = Arc::new(Mutex::new(Vec::new()));
    let recorded_exc_filters: Arc<Mutex<Vec<Vec<String>>>> = Arc::new(Mutex::new(Vec::new()));
    let bp_sink = Arc::clone(&recorded_bp_args);
    let exc_sink = Arc::clone(&recorded_exc_filters);
    let canon_for_srv = canon_src.clone();

    let server_task = tokio::spawn(async move {
        let (sr, sw) = tokio::io::split(server_io);
        let mut r = DapReader::new(sr);
        let mut w = DapWriter::new(sw);
        while let Some(DapMessage::Request(req)) = r.next_message().await.unwrap() {
            match req.command.as_str() {
                "initialize" => {
                    w.send_response(
                        req.seq,
                        "initialize",
                        Ok(Some(json!({
                            "supportsConfigurationDoneRequest": true,
                            "supportsTerminateRequest": true,
                            "supportsHitConditionalBreakpoints": true,
                            "supportsLogPoints": true,
                            "exceptionBreakpointFilters": [
                                { "filter": "unrecovered-panic", "label": "Panic" }
                            ]
                        }))),
                    )
                    .await
                    .unwrap();
                }
                "launch" => {
                    w.send_event("initialized", Some(json!({}))).await.unwrap();
                    w.send_response(req.seq, "launch", Ok(Some(json!({}))))
                        .await
                        .unwrap();
                }
                "setBreakpoints" => {
                    if let Some(args) = &req.arguments {
                        bp_sink.lock().unwrap().push(args.clone());
                    }
                    let bps = req
                        .arguments
                        .as_ref()
                        .and_then(|a| a.get("breakpoints"))
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default();
                    w.send_response(
                        req.seq,
                        "setBreakpoints",
                        Ok(Some(json!({ "breakpoints": bps }))),
                    )
                    .await
                    .unwrap();
                }
                "setExceptionBreakpoints" => {
                    let filters: Vec<String> = req
                        .arguments
                        .as_ref()
                        .and_then(|a| a.get("filters"))
                        .and_then(Value::as_array)
                        .map(|arr| {
                            arr.iter()
                                .filter_map(|v| v.as_str().map(String::from))
                                .collect()
                        })
                        .unwrap_or_default();
                    let enabled = !filters.is_empty();
                    exc_sink.lock().unwrap().push(filters);
                    w.send_response(req.seq, "setExceptionBreakpoints", Ok(Some(json!({}))))
                        .await
                        .unwrap();
                    if enabled {
                        w.send_event(
                            "stopped",
                            Some(json!({ "reason": "exception", "threadId": 1 })),
                        )
                        .await
                        .unwrap();
                    }
                }
                "threads" => {
                    w.send_response(
                        req.seq,
                        "threads",
                        Ok(Some(json!({
                            "threads": [{ "id": 1, "name": "goroutine 1" }]
                        }))),
                    )
                    .await
                    .unwrap();
                }
                "stackTrace" => {
                    w.send_response(
                        req.seq,
                        "stackTrace",
                        Ok(Some(json!({
                            "stackFrames": [{
                                "id": 100,
                                "name": "p.run",
                                "source": { "path": canon_for_srv },
                                "line": 4,
                                "column": 1
                            }]
                        }))),
                    )
                    .await
                    .unwrap();
                }
                "scopes" => {
                    w.send_response(
                        req.seq,
                        "scopes",
                        Ok(Some(json!({
                            "scopes": [{ "name": "Locals", "variablesReference": 10, "expensive": false }]
                        }))),
                    )
                    .await
                    .unwrap();
                }
                "variables" => {
                    w.send_response(
                        req.seq,
                        "variables",
                        Ok(Some(json!({
                            "variables": [{ "name": "x", "value": "1", "type": "int", "variablesReference": 0 }]
                        }))),
                    )
                    .await
                    .unwrap();
                }
                "disconnect" => {
                    w.send_response(req.seq, "disconnect", Ok(Some(json!({}))))
                        .await
                        .unwrap();
                    break;
                }
                other => {
                    w.send_response(req.seq, other, Ok(Some(json!({}))))
                        .await
                        .unwrap();
                }
            }
        }
    });

    let mgr = DapManager::new(tmp.path().to_path_buf());
    let start_req = IdeStartDebugReq {
        target: IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned()),
        language: "go".into(),
        launch: IdeDebugLaunchSpec {
            program: canon_src.clone(),
            cwd: Some(tmp.path().to_string_lossy().into_owned()),
            ..IdeDebugLaunchSpec::default()
        },
        breakpoints: vec![
            IdeBreakpoint {
                path: canon_src.clone(),
                line: 3,
                enabled: true,
                condition: None,
                hit_condition: Some("3".into()),
                log_message: None,
                verified: None,
                message: None,
            },
            IdeBreakpoint {
                path: canon_src.clone(),
                line: 4,
                enabled: true,
                condition: None,
                hit_condition: None,
                log_message: Some("x={x}".into()),
                verified: None,
                message: None,
            },
        ],
        ..IdeStartDebugReq::default()
    };

    let started = mgr
        .start_session_with_stream(start_req, "mock-go", client_io, true)
        .await
        .unwrap();
    let mut sub = mgr.subscribe_events(&started.id).unwrap();

    mgr.set_breakpoints(
        &started.id,
        IdeSetBreakpointsReq {
            breakpoints: started.breakpoints.clone(),
            break_on_exceptions: true,
        },
    )
    .await
    .unwrap();

    let mut observed_paused: Option<IdeDebugSessionSnapshot> = None;
    tokio::time::timeout(Duration::from_secs(5), async {
        while let Ok(ev) = sub.recv().await {
            if ev.state == IdeDebugState::Paused
                && ev.stopped_reason.as_deref() == Some("exception")
            {
                observed_paused = Some(ev);
                break;
            }
        }
    })
    .await
    .expect("timed out waiting for exception stop over subscribe_events");

    let snap = observed_paused.expect("exception snapshot");
    assert_eq!(snap.paused_line, Some(4));
    assert_eq!(snap.stopped_reason.as_deref(), Some("exception"));

    {
        let bp_calls = recorded_bp_args.lock().unwrap();
        assert!(!bp_calls.is_empty());
        let bps = bp_calls[0]["breakpoints"].as_array().unwrap();
        assert_eq!(bps[0]["hitCondition"].as_str(), Some("3"));
        assert_eq!(bps[1]["logMessage"].as_str(), Some("x={x}"));
    }
    {
        let exc_calls = recorded_exc_filters.lock().unwrap();
        assert!(
            exc_calls
                .iter()
                .any(|f| f == &vec!["unrecovered-panic".to_string()]),
            "expected setExceptionBreakpoints with advertised filter, got {exc_calls:?}"
        );
    }

    let stopped = mgr
        .command(
            &started.id,
            IdeDebugCommandReq {
                action: IdeDebugAction::Stop,
                thread_id: None,
                frame_id: None,
                path: None,
                line: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(stopped.state, IdeDebugState::Terminated);
    let term_ev = tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            let ev = sub.recv().await.unwrap();
            if ev.state == IdeDebugState::Terminated {
                return ev;
            }
        }
    })
    .await
    .expect("expected Terminated snapshot on subscribe_events");
    assert_eq!(term_ev.state, IdeDebugState::Terminated);
    let _ = server_task.await;
}

#[cfg(unix)]
#[tokio::test]
async fn shell_dap_logpoints_hit_conditions_and_restart_preserve_live_events() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().canonicalize().unwrap();
    let script = root.join("loop.sh");
    // Line 1: shebang
    // Line 2: for ITER in 1 2 3 4; do
    // Line 3:   STEP="s-$ITER"       <-- non-pausing logpoint (`log-iter={ITER}`)
    // Line 4:   LAST="$STEP"          <-- hit_condition = "3" (pauses only on 3rd hit!)
    // Line 5: done
    std::fs::write(
        &script,
        "#!/usr/bin/env bash\nfor ITER in 1 2 3 4; do\n  STEP=\"s-$ITER\"\n  LAST=\"$STEP\"\ndone\n",
    )
    .unwrap();

    let mgr = DapManager::new(root.join(".otto-data"));
    let req = IdeStartDebugReq {
        target: IdeTargetSpec::local(root.to_string_lossy().into_owned()),
        language: "shell".into(),
        launch: IdeDebugLaunchSpec {
            program: script.to_string_lossy().into_owned(),
            cwd: Some(root.to_string_lossy().into_owned()),
            ..IdeDebugLaunchSpec::default()
        },
        breakpoints: vec![
            IdeBreakpoint {
                path: script.to_string_lossy().into_owned(),
                line: 3,
                enabled: true,
                condition: None,
                hit_condition: None,
                log_message: Some("log-iter={ITER}".into()),
                verified: None,
                message: None,
            },
            IdeBreakpoint {
                path: script.to_string_lossy().into_owned(),
                line: 4,
                enabled: true,
                condition: None,
                hit_condition: Some("3".into()),
                log_message: None,
                verified: None,
                message: None,
            },
        ],
        ..IdeStartDebugReq::default()
    };

    let started = mgr
        .start_session(req, DapStartContext::new())
        .await
        .unwrap();
    let mut sub = mgr.subscribe_events(&started.id).unwrap();

    let first_pause = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            let snap = mgr.get_session(&started.id).unwrap();
            if snap.state == IdeDebugState::Paused {
                return snap;
            }
            let _ = sub.recv().await;
        }
    })
    .await
    .expect("shell debugger should pause on 3rd hit at line 4");

    assert_eq!(first_pause.paused_line, Some(4));
    assert!(
        first_pause.scopes[0]
            .variables
            .iter()
            .any(|v| v.name == "ITER" && v.value == "3"),
        "expected hit_condition=3 to skip iterations 1 and 2 and pause at ITER=3, got {:?}",
        first_pause.scopes[0].variables
    );
    for expected_log in ["log-iter=1", "log-iter=2", "log-iter=3"] {
        assert!(
            first_pause
                .output
                .iter()
                .any(|o| o.category == "console" && o.text.contains(expected_log)),
            "expected non-pausing logpoint output '{expected_log}', got {:?}",
            first_pause.output
        );
    }

    // Restart the session and verify the retiring bridge's `terminated` / `__eof__`
    // does not clobber the new run before or after it reaches the 3rd hit again.
    mgr.command(
        &started.id,
        IdeDebugCommandReq {
            action: IdeDebugAction::Restart,
            thread_id: None,
            frame_id: None,
            path: None,
            line: None,
        },
    )
    .await
    .unwrap();

    let mut saw_clobber_terminated = false;
    let second_pause = tokio::time::timeout(Duration::from_secs(8), async {
        loop {
            match tokio::time::timeout(Duration::from_millis(50), sub.recv()).await {
                Ok(Ok(ev)) => {
                    if ev.state == IdeDebugState::Terminated {
                        saw_clobber_terminated = true;
                    }
                    if ev.state == IdeDebugState::Paused {
                        return ev;
                    }
                }
                _ => {
                    let snap = mgr.get_session(&started.id).unwrap();
                    if snap.state == IdeDebugState::Paused {
                        return snap;
                    }
                }
            }
        }
    })
    .await
    .expect("restarted shell session should pause at line 4 on 3rd hit");

    assert!(
        !saw_clobber_terminated,
        "Restart must not emit a spurious Terminated state from the retired connection"
    );
    assert_eq!(second_pause.paused_line, Some(4));
    assert!(second_pause.scopes[0]
        .variables
        .iter()
        .any(|v| v.name == "ITER" && v.value == "3"));

    let stopped = mgr
        .command(
            &started.id,
            IdeDebugCommandReq {
                action: IdeDebugAction::Stop,
                thread_id: None,
                frame_id: None,
                path: None,
                line: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(stopped.state, IdeDebugState::Terminated);
}

#[test]
fn breakpoint_relocation_on_one_line_is_skipped_but_a_loop_revisit_is_not() {
    use super::super::session_inspect::is_same_line_relocation;
    let key = (1, 3, Some("/w/src/main.rs".to_string()), 8);
    let first = (key.clone(), 0x950);
    // `println!` compiled to three locations: the 2nd and 3rd are relocations.
    assert!(is_same_line_relocation(Some(&first), &key, 0x95c));
    // A loop coming back to the line restarts at the first address.
    assert!(!is_same_line_relocation(Some(&first), &key, 0x950));
    // Another line, another depth (recursion) or no previous stop: a real stop.
    assert!(!is_same_line_relocation(
        Some(&first),
        &(1, 3, key.2.clone(), 9),
        0x960
    ));
    assert!(!is_same_line_relocation(
        Some(&first),
        &(1, 4, key.2.clone(), 8),
        0x960
    ));
    assert!(!is_same_line_relocation(None, &key, 0x95c));
}
