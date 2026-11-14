use super::*;
use crate::types::{IdeBreakpoint, IdeDebugLaunchSpec, IdeScope, IdeTargetSpec};

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

    mgr.command(
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
