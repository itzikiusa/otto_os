use super::*;

fn ctx(resume: Option<&str>, access_mode: RuntimeAccessMode) -> LaunchContext {
    LaunchContext {
        mcp_servers: Vec::new(),
        session_id: "s".into(),
        cwd: "/w".into(),
        model: Some("openai-codex/gpt-5.5".into()),
        resume: resume.map(str::to_string),
        sandboxed: false,
        access_mode,
    }
}

fn sent(step: &DriverStep, i: usize) -> Value {
    serde_json::from_str(&step.writes[i]).unwrap()
}

#[test]
fn launch_maps_resume_model_and_access_to_tools() {
    let d = PiDriver::with_program("pi");
    let a = d
        .launch(&ctx(Some("abc"), RuntimeAccessMode::ApprovalRequired))
        .args;
    assert_eq!(
        a,
        [
            "--mode",
            "rpc",
            "--session",
            "abc",
            "--model",
            "openai-codex/gpt-5.5",
            "--tools",
            "read,grep,find,ls",
            "--exclude-tools",
            "mcp__*"
        ]
    );
    let full = d.launch(&ctx(None, RuntimeAccessMode::FullAccess)).args;
    assert!(!full.iter().any(|a| a == "--tools" || a == "--session"));
}

#[test]
fn handshake_turn_tools_and_settle() {
    let mut d = PiDriver::with_program("pi");
    let s = d.start(&ctx(None, RuntimeAccessMode::FullAccess));
    let st = sent(&s, 0);
    assert_eq!(st["type"], "get_state");
    let s = d.on_line(&format!(
        r#"{{"id":"{}","type":"response","command":"get_state","success":true,"data":{{"sessionId":"sess-1","model":{{"id":"gpt-5.5","provider":"openai-codex"}}}}}}"#,
        st["id"].as_str().unwrap()
    ));
    assert!(
        matches!(&s.events[0], DriverEvent::Ready { native_thread_id: Some(t), model: Some(m) } if t == "sess-1" && m == "openai-codex/gpt-5.5")
    );

    let mut req = RuntimePromptReq::of("hi");
    req.effort = Some("max".into());
    let s = d.prompt("t1", &req).unwrap();
    assert_eq!(sent(&s, 0)["level"], "max");
    let p = sent(&s, 1);
    assert_eq!(
        (p["type"].as_str(), p["message"].as_str()),
        (Some("prompt"), Some("hi"))
    );
    d.on_line(&format!(
        r#"{{"id":"{}","type":"response","command":"prompt","success":true,"data":{{"disposition":"started"}}}}"#,
        p["id"].as_str().unwrap()
    ));
    let s = d.on_line(
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Hel"}}"#,
    );
    assert!(
        matches!(&s.events[0], DriverEvent::TurnItem(i) if matches!(&i.kind, TurnItemKind::AssistantMessage { text, streaming: true, .. } if text == "Hel"))
    );
    d.on_line(
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"lo"}}"#,
    );
    let s = d.on_line(r#"{"type":"tool_execution_start","toolCallId":"c1","toolName":"bash","args":{"command":"ls"}}"#);
    assert!(
        matches!(&s.events[0], DriverEvent::TurnItem(i) if matches!(&i.kind, TurnItemKind::AssistantMessage { text, streaming: false, .. } if text == "Hello"))
    );
    assert!(
        matches!(&s.events[1], DriverEvent::TurnItem(i) if matches!(&i.kind, TurnItemKind::CommandExecution { input, .. } if input == "ls"))
    );
    let s = d.on_line(r#"{"type":"tool_execution_end","toolCallId":"c1","toolName":"bash","result":{"content":[{"type":"text","text":"a.rs"}]},"isError":false}"#);
    assert!(
        matches!(&s.events[0], DriverEvent::TurnItem(i) if i.status == TurnItemStatus::Completed && matches!(&i.kind, TurnItemKind::CommandExecution { output: Some(o), .. } if o == "a.rs"))
    );
    let s = d.on_line(
        r#"{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop"}],"willRetry":false}"#,
    );
    assert!(matches!(
        &s.events[0],
        DriverEvent::Terminal {
            status: TurnTerminalStatus::Completed,
            ..
        }
    ));
    // A later agent_settled finds no open turn.
    assert!(d
        .on_line(r#"{"type":"agent_settled","aborted":false}"#)
        .events
        .is_empty());
}

/// Once pi has sent `agent_settled` (1.x), `agent_end` no longer ends the
/// turn — `agent_settled` does, with the stashed outcome. The lines are
/// pi 1.5.0's for a provider whose OAuth refresh fails (trimmed).
#[test]
fn a_settling_pi_ends_turns_at_agent_settled_with_the_runs_outcome() {
    let mut d = PiDriver::with_program("pi");
    d.start(&ctx(None, RuntimeAccessMode::FullAccess));
    d.prompt("t1", &RuntimePromptReq::of("hi")).unwrap();
    let s = d.on_line(
        r#"{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop"}],"willRetry":false}"#,
    );
    assert!(matches!(
        &s.events[0],
        DriverEvent::Terminal {
            status: TurnTerminalStatus::Completed,
            ..
        }
    ));
    // Learns that this pi settles; nothing is open any more.
    assert!(d
        .on_line(r#"{"type":"agent_settled","aborted":false}"#)
        .events
        .is_empty());

    d.prompt("t2", &RuntimePromptReq::of("again")).unwrap();
    let s = d.on_line(r#"{"type":"message_end","message":{"role":"assistant","content":[],"stopReason":"error","errorMessage":"OAuth refresh failed for openai-codex"}}"#);
    assert!(s.events.is_empty());
    let s = d.on_line(r#"{"type":"agent_end","messages":[{"role":"system","content":""},{"role":"user","content":[]},{"role":"assistant","content":[],"stopReason":"error","errorMessage":"OAuth refresh failed for openai-codex"}],"willRetry":false}"#);
    assert!(s.events.is_empty(), "agent_end must wait for agent_settled");
    let s = d.on_line(r#"{"type":"agent_settled","aborted":false}"#);
    assert!(
        matches!(&s.events[0], DriverEvent::Terminal { status: TurnTerminalStatus::Failed, failure: Some(f), .. } if f.message == "OAuth refresh failed for openai-codex")
    );

    // An abort wins over the stashed outcome.
    d.prompt("t3", &RuntimePromptReq::of("go")).unwrap();
    d.on_line(
        r#"{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop"}],"willRetry":false}"#,
    );
    let s = d.on_line(r#"{"type":"agent_settled","aborted":true}"#);
    assert!(matches!(
        &s.events[0],
        DriverEvent::Terminal {
            status: TurnTerminalStatus::Interrupted,
            ..
        }
    ));
}

#[test]
fn a_refused_prompt_fails_and_dialogs_are_cancelled() {
    let mut d = PiDriver::with_program("pi");
    d.start(&ctx(None, RuntimeAccessMode::FullAccess));
    let s = d.prompt("t1", &RuntimePromptReq::of("hi")).unwrap();
    let p = sent(&s, 0);
    let s = d.on_line(&format!(
        r#"{{"id":"{}","type":"response","command":"prompt","success":false,"error":"No model"}}"#,
        p["id"].as_str().unwrap()
    ));
    assert!(
        matches!(&s.events[0], DriverEvent::Terminal { status: TurnTerminalStatus::Failed, failure: Some(f), .. } if f.message == "No model")
    );
    let s = d
        .on_line(r#"{"type":"extension_ui_request","id":"u1","method":"confirm","title":"Sure?"}"#);
    let r = sent(&s, 0);
    assert_eq!(
        (r["type"].as_str(), r["cancelled"].as_bool()),
        (Some("extension_ui_response"), Some(true))
    );
}

#[test]
fn autonomous_agent_start_after_ready_emits_turn_started_and_streams_thinking_and_text() {
    let mut d = PiDriver::with_program("pi");
    let st = sent(&d.start(&ctx(None, RuntimeAccessMode::FullAccess)), 0);
    d.on_line(&format!(
        r#"{{"id":"{}","type":"response","command":"get_state","success":true,"data":{{"sessionId":"sess-1"}}}}"#,
        st["id"].as_str().unwrap()
    ));
    let s = d.on_line(r#"{"type":"agent_start"}"#);
    assert!(matches!(&s.events[0], DriverEvent::TurnStarted { turn_id } if turn_id == "auto-1"));
    let th = d.on_line(
        r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","delta":"pondering"}}"#,
    );
    assert!(
        matches!(&th.events[0], DriverEvent::TurnItem(i) if matches!(&i.kind, TurnItemKind::Reasoning { text, streaming: true } if text == "pondering"))
    );
    let tx = d.on_line(
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"autonomous answer"}}"#,
    );
    assert!(
        matches!(&tx.events[1], DriverEvent::TurnItem(i) if matches!(&i.kind, TurnItemKind::AssistantMessage { text, streaming: true, .. } if text == "autonomous answer"))
    );
    d.on_line(r#"{"type":"message_end","message":{"role":"assistant","stopReason":"stop"}}"#);
    let fin = d.on_line(
        r#"{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop"}],"willRetry":false}"#,
    );
    assert!(fin.events.iter().any(|e| matches!(
        e,
        DriverEvent::Terminal {
            status: TurnTerminalStatus::Completed,
            ..
        }
    )));
}

#[test]
fn steer_sends_rpc_steer_mid_turn_and_surfaces_refusal_without_ending_turn() {
    use otto_core::agent_runtime::{PromptFile, PromptImage};

    let mut d = PiDriver::with_program("pi");
    let st = sent(&d.start(&ctx(None, RuntimeAccessMode::FullAccess)), 0);
    assert!(
        d.steer(&RuntimePromptReq::of("not ready")).is_err(),
        "steer before get_state ready must fail"
    );
    d.on_line(&format!(
        r#"{{"id":"{}","type":"response","command":"get_state","success":true,"data":{{"sessionId":"sess-1"}}}}"#,
        st["id"].as_str().unwrap()
    ));
    assert!(
        d.steer(&RuntimePromptReq::of("idle")).is_err(),
        "steer without a running turn must fail"
    );

    let p = sent(&d.prompt("t1", &RuntimePromptReq::of("work")).unwrap(), 0);
    d.on_line(&format!(
        r#"{{"id":"{}","type":"response","command":"prompt","success":true}}"#,
        p["id"].as_str().unwrap()
    ));

    assert!(d.steer(&RuntimePromptReq::default()).is_err());
    let with_file = RuntimePromptReq {
        files: vec![PromptFile {
            mime: "application/pdf".into(),
            name: "a.pdf".into(),
            data_base64: "JVBERi0=".into(),
        }],
        ..RuntimePromptReq::of("pdf")
    };
    assert!(d.steer(&with_file).is_err());

    let steer_req = RuntimePromptReq {
        effort: Some("high".into()),
        images: vec![PromptImage {
            mime: "image/png".into(),
            data_base64: "iVBORw0KGgo=".into(),
        }],
        ..RuntimePromptReq::of("check edge case")
    };
    let s = d.steer(&steer_req).unwrap();
    assert_eq!(s.writes.len(), 2);
    assert_eq!(sent(&s, 0)["type"], "set_thinking_level");
    assert_eq!(sent(&s, 0)["level"], "high");
    let st_cmd = sent(&s, 1);
    assert_eq!(st_cmd["type"], "steer");
    assert_eq!(st_cmd["message"], "check edge case");
    assert_eq!(
        st_cmd["images"],
        json!([{"type": "image", "data": "iVBORw0KGgo=", "mimeType": "image/png"}])
    );

    // A refused steer emits `DriverEvent::Error` without terminating `t1`.
    let bad = sent(&d.steer(&RuntimePromptReq::of("/extcmd")).unwrap(), 0);
    let err_step = d.on_line(&format!(
        r#"{{"id":"{}","type":"response","command":"steer","success":false,"error":"Extension commands cannot be steered"}}"#,
        bad["id"].as_str().unwrap()
    ));
    assert!(err_step.events.iter().any(|e| matches!(
        e,
        DriverEvent::Error(f) if f.message.contains("Extension commands")
    )));
    assert!(!err_step
        .events
        .iter()
        .any(|e| matches!(e, DriverEvent::Terminal { .. })));

    let ok_step = d.on_line(&format!(
        r#"{{"id":"{}","type":"response","command":"steer","success":true}}"#,
        st_cmd["id"].as_str().unwrap()
    ));
    assert!(ok_step.events.is_empty());

    let done = d.on_line(
        r#"{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop"}],"willRetry":false}"#,
    );
    assert!(done.events.iter().any(|e| matches!(
        e,
        DriverEvent::Terminal {
            status: TurnTerminalStatus::Completed,
            ..
        }
    )));
}

#[test]
fn snapshot_and_restore_preserve_pi_streaming_text_tools_and_settlement_mode() {
    let mut d1 = PiDriver::with_program("pi");
    let st = sent(&d1.start(&ctx(None, RuntimeAccessMode::FullAccess)), 0);
    d1.on_line(&format!(
        r#"{{"id":"{}","type":"response","command":"get_state","success":true,"data":{{"sessionId":"sess-pi-1"}}}}"#,
        st["id"].as_str().unwrap()
    ));
    let p = sent(&d1.prompt("t1", &RuntimePromptReq::of("build")).unwrap(), 0);
    d1.on_line(&format!(
        r#"{{"id":"{}","type":"response","command":"prompt","success":true}}"#,
        p["id"].as_str().unwrap()
    ));
    d1.on_line(
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Half "}}"#,
    );
    d1.on_line(
        r#"{"type":"tool_execution_start","toolCallId":"tc-1","toolName":"bash","args":{"command":"ls"}}"#,
    );
    d1.on_line(
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Half "}}"#,
    );

    let snap = d1.snapshot_state();
    let mut d2 = PiDriver::with_program("pi");
    d2.restore_state(&snap).unwrap();

    let tool_end = d2.on_line(
        r#"{"type":"tool_execution_end","toolCallId":"tc-1","toolName":"bash","result":"ok","isError":false}"#,
    );
    assert!(tool_end.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i) if i.id == "t1:tool:tc-1" && i.status == TurnItemStatus::Completed
    )));
    let text2 = d2.on_line(
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"done"}}"#,
    );
    assert!(text2.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::AssistantMessage { text, streaming: true, .. } if text == "Half done")
    )));
    let end = d2.on_line(
        r#"{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop"}],"willRetry":false}"#,
    );
    assert!(end.events.iter().any(|e| matches!(
        e,
        DriverEvent::Terminal {
            status: TurnTerminalStatus::Completed,
            ..
        }
    )));
}

#[test]
fn subagent_and_background_bash_emit_subagent_cards_bg_tasks_and_notices() {
    let mut d = PiDriver::with_program("pi");
    let st = sent(&d.start(&ctx(None, RuntimeAccessMode::FullAccess)), 0);
    d.on_line(&format!(
        r#"{{"id":"{}","type":"response","command":"get_state","success":true,"data":{{"sessionId":"sess-pi-bg"}}}}"#,
        st["id"].as_str().unwrap()
    ));
    let p = sent(
        &d.prompt("t1", &RuntimePromptReq::of("fan out")).unwrap(),
        0,
    );
    d.on_line(&format!(
        r#"{{"id":"{}","type":"response","command":"prompt","success":true}}"#,
        p["id"].as_str().unwrap()
    ));

    // 1. Synchronous `task` / `subagent` tool emits `TurnItemKind::Subagent`, updates
    //    `tool_uses`/`last_tool` on `tool_execution_update`, and completes with `output`.
    let sub_start = d.on_line(
        r#"{"type":"tool_execution_start","toolCallId":"sub-1","toolName":"task","args":{"subagent_type":"explorer","description":"Map callers","prompt":"Find all callers of login()"}}"#,
    );
    assert!(sub_start.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if i.id == "t1:tool:sub-1"
                && i.status == TurnItemStatus::Running
                && matches!(
                    &i.kind,
                    TurnItemKind::Subagent {
                        tool_name,
                        description: Some(desc),
                        subagent_type: Some(st),
                        prompt: Some(pr),
                        background: false,
                        ..
                    } if tool_name == "task"
                        && desc == "Map callers"
                        && st == "explorer"
                        && pr == "Find all callers of login()"
                )
    )));

    let sub_upd = d.on_line(
        r#"{"type":"tool_execution_update","toolCallId":"sub-1","toolName":"task","partialResult":{"details":{"toolUses":3,"lastTool":"grep"}}}"#,
    );
    assert!(sub_upd.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if i.id == "t1:tool:sub-1"
                && matches!(
                    &i.kind,
                    TurnItemKind::Subagent {
                        tool_uses: 3,
                        last_tool: Some(lt),
                        ..
                    } if lt == "grep"
                )
    )));

    let sub_end = d.on_line(
        r#"{"type":"tool_execution_end","toolCallId":"sub-1","toolName":"task","result":{"content":[{"type":"text","text":"3 callers found"}]},"isError":false}"#,
    );
    assert!(sub_end.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if i.id == "t1:tool:sub-1"
                && i.status == TurnItemStatus::Completed
                && matches!(&i.kind, TurnItemKind::Subagent { output: Some(o), .. } if o == "3 callers found")
    )));

    // 2. Async `subagent` (`run_in_background: true`) stays Running on launch handle
    //    return, tracks `BackgroundTasks`, and settles on `subagent_output`.
    let bg_sub_start = d.on_line(
        r#"{"type":"tool_execution_start","toolCallId":"sub-bg","toolName":"subagent","args":{"agent_type":"worker","description":"Background Worker","prompt":"Audit tests","run_in_background":true}}"#,
    );
    assert!(bg_sub_start.events.iter().any(|e| matches!(
        e,
        DriverEvent::BackgroundTasks(tasks)
            if tasks.iter().any(|t| t.id == "sub-bg" && t.kind == "agent" && t.label == "Background Worker")
    )));
    let bg_sub_launched = d.on_line(
        r#"{"type":"tool_execution_end","toolCallId":"sub-bg","toolName":"subagent","result":{"details":{"taskId":"pi-agent-99","status":"running"},"content":[{"type":"text","text":"Started"}]},"isError":false}"#,
    );
    assert!(bg_sub_launched.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i) if i.id == "t1:tool:sub-bg" && i.status == TurnItemStatus::Running
    )));
    assert!(bg_sub_launched.events.iter().any(|e| matches!(
        e,
        DriverEvent::BackgroundTasks(tasks) if tasks.iter().any(|t| t.id == "pi-agent-99")
    )));

    let bg_sub_settled = d.on_line(
        r#"{"type":"tool_execution_start","toolCallId":"poll-1","toolName":"subagent_output","args":{"taskId":"pi-agent-99"}}"#,
    );
    let _ = bg_sub_settled;
    let bg_sub_done = d.on_line(
        r#"{"type":"tool_execution_end","toolCallId":"poll-1","toolName":"subagent_output","result":{"details":{"status":"completed"},"content":[{"type":"text","text":"Audit finished"}]},"isError":false}"#,
    );
    assert!(bg_sub_done.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if i.id == "t1:tool:sub-bg"
                && i.status == TurnItemStatus::Completed
                && matches!(&i.kind, TurnItemKind::Subagent { output: Some(o), .. } if o == "Audit finished")
    )));
    assert!(bg_sub_done.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::SystemNotice { message } if message == "Background agent finished: Background Worker")
    )));

    // 3. Background `bash` (`run_in_background: true`) streams `CommandOutput` and settles with notice.
    let bg_bash_start = d.on_line(
        r#"{"type":"tool_execution_start","toolCallId":"bash-bg","toolName":"bash","args":{"command":"./run_suite.sh","run_in_background":true}}"#,
    );
    assert!(bg_bash_start.events.iter().any(|e| matches!(
        e,
        DriverEvent::BackgroundTasks(tasks)
            if tasks.iter().any(|t| t.id == "bash-bg" && t.kind == "command")
    )));
    let bg_bash_upd = d.on_line(
        r#"{"type":"tool_execution_update","toolCallId":"bash-bg","toolName":"bash","partialResult":{"content":[{"type":"text","text":"step 1 ok\n"}]}}"#,
    );
    assert!(bg_bash_upd.events.iter().any(|e| matches!(
        e,
        DriverEvent::CommandOutput { item_id, chunk }
            if item_id == "t1:tool:bash-bg" && chunk == "step 1 ok\n"
    )));
    let bg_bash_end = d.on_line(
        r#"{"type":"tool_execution_end","toolCallId":"bash-bg","toolName":"bash","result":{"content":[{"type":"text","text":"step 1 ok\nall passed\n"}]},"isError":false}"#,
    );
    assert!(bg_bash_end.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::SystemNotice { message } if message == "Background command finished: ./run_suite.sh")
    )));
    assert!(bg_bash_end.events.iter().any(|e| matches!(
        e,
        DriverEvent::BackgroundTasks(tasks) if tasks.is_empty()
    )));
}
