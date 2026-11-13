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
                        Ok(Some(
                            json!({ "threads": [{ "id": 1, "name": "goroutine 1" }] }),
                        )),
                    )
                    .await
                    .unwrap();
                }
                "stackTrace" => {
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
                    w.send_response(
                        req.seq,
                        "scopes",
                        Ok(Some(json!({
                            "scopes": [{ "name": "Locals", "variablesReference": 2001, "expensive": false }]
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
        },
        breakpoints: vec![IdeBreakpoint {
            path: canon_src.clone(),
            line: 6,
            enabled: true,
            condition: Some("x == 20".into()),
            hit_condition: None,
            log_message: None,
        }],
    };

    let started = mgr
        .start_session_with_stream(start_req, "dlv", client_io, true)
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
    .expect("timed out waiting for paused snapshot");

    assert_eq!(paused.stopped_reason.as_deref(), Some("breakpoint"));
    assert_eq!(paused.paused_line, Some(6));
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
