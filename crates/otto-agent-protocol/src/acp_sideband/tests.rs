use chrono::{Duration as ChronoDuration, Utc};
use otto_core::agent_runtime::{
    RuntimeAccessMode, RuntimePromptReq, RuntimeProvider, TurnItemKind, TurnItemStatus,
    TurnTerminalStatus,
};
use serde_json::{json, Value};

use crate::acp::AcpDriver;
use crate::driver::{
    DriverEvent, LaunchContext, SidebandPayload, SidebandSource, StructuredDriver,
};

fn launch_ctx(cwd: &str) -> LaunchContext {
    LaunchContext {
        mcp_servers: Vec::new(),
        session_id: "otto-sess-1".into(),
        cwd: cwd.into(),
        model: None,
        resume: None,
        sandboxed: false,
        access_mode: RuntimeAccessMode::FullAccess,
    }
}

fn handshake_and_finish_turn1(d: &mut AcpDriver, cwd: &str, sid: &str) {
    d.start(&launch_ctx(cwd));
    d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
    d.on_line(&json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":sid}}).to_string());
    let p1 = d.prompt("t1", &RuntimePromptReq::of("start bg")).unwrap();
    let pid: u64 = p1
        .writes
        .iter()
        .find_map(|w| {
            let v: Value = serde_json::from_str(w).ok()?;
            (v.get("method")?.as_str()? == "session/prompt").then(|| v.get("id")?.as_u64())?
        })
        .expect("prompt id");
    d.on_line(&json!({"jsonrpc":"2.0","id":pid,"result":{"stopReason":"end_turn"}}).to_string());
}

#[test]
fn agy_sideband_ignores_pre_idle_stream_retry_and_deduplicates_burst() {
    let mut d = AcpDriver::agy("agy");
    d.set_sideband_root_dir("/tmp/test-agy-brain");
    let sid = "eaf6440b-3dbd-4661-9f38-63372630db72";
    handshake_and_finish_turn1(&mut d, "/workspace", sid);

    let pre_idle = (Utc::now() - ChronoDuration::seconds(15)).to_rfc3339();
    let post_idle = (Utc::now() + ChronoDuration::seconds(2)).to_rfc3339();

    // Mid-turn stream retry written BEFORE turn 1 finished must be ignored.
    let retry_lines = format!(
        "{}\n{}\n",
        json!({
            "step_index": 1,
            "type": "ERROR_MESSAGE",
            "source": "SYSTEM",
            "created_at": pre_idle,
            "content": "Error: The stream was interrupted. Retrying..."
        }),
        json!({
            "step_index": 2,
            "type": "PLANNER_RESPONSE",
            "source": "MODEL",
            "created_at": pre_idle,
            "thinking": "Old turn 1 thinking",
            "content": "Old turn 1 answer"
        }),
    );
    let ignored = d.on_sideband(SidebandPayload::AppendLines {
        resolved_path: None,
        using_fallback: false,
        next_offset: retry_lines.len() as u64,
        bytes: retry_lines.into_bytes(),
    });
    assert!(
        ignored.events.is_empty(),
        "pre-idle transcript lines must not trigger false autonomous turns"
    );

    // Real between-prompt timer wakeup arrives after turn 1 settled.
    let wake_lines = format!(
        "{}\n{}\n{}\n{}\n",
        json!({
            "step_index": 3,
            "type": "SYSTEM_MESSAGE",
            "source": "SYSTEM",
            "created_at": post_idle,
            "content": "Scheduled timer fired"
        }),
        json!({
            "step_index": 4,
            "type": "PLANNER_RESPONSE",
            "source": "MODEL",
            "created_at": post_idle,
            "thinking": "Checking health",
            "tool_calls": [{
                "id": "call_1",
                "name": "default_api:run_command",
                "args": {"CommandLine": "\"cargo test\"", "toolSummary": "\"Run tests\""}
            }]
        }),
        json!({
            "step_index": 5,
            "type": "GENERIC",
            "source": "MODEL",
            "status": "DONE",
            "created_at": post_idle,
            "id": "call_1",
            "content": "test result: ok."
        }),
        json!({
            "step_index": 6,
            "type": "PLANNER_RESPONSE",
            "source": "MODEL",
            "created_at": post_idle,
            "content": "All tests passed!",
            "input_tokens": 100,
            "output_tokens": 25,
            "cache_read_tokens": 40,
            "thinking_tokens": 10
        }),
    );
    let step = d.on_sideband(SidebandPayload::AppendLines {
        resolved_path: None,
        using_fallback: false,
        next_offset: 4096,
        bytes: wake_lines.into_bytes(),
    });
    assert!(step
        .events
        .iter()
        .any(|e| matches!(e, DriverEvent::TurnStarted { turn_id } if turn_id == "auto-1")));
    assert!(step.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::CommandExecution { input, output: Some(out), .. }
                if input == "cargo test" && out == "test result: ok.")
                && i.status == TurnItemStatus::Completed
    )));
    assert!(step.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::AssistantMessage { text, streaming: false, .. }
                if text == "All tests passed!")
    )));
    assert!(d.needs_idle_turn_settle());
    let settled = d.on_idle_settle();
    assert!(settled.events.iter().any(|e| matches!(
        e,
        DriverEvent::Terminal {
            status: TurnTerminalStatus::Completed,
            ..
        }
    )));

    // Legacy checkpoint alias (`agy_tail.brain_dir`) restores cleanly.
    let mut snap = d.snapshot_state();
    if let Some(obj) = snap.as_object_mut() {
        if let Some(mut sb) = obj.remove("sideband") {
            if let Some(sb_obj) = sb.as_object_mut() {
                if let Some(root) = sb_obj.remove("root_dir") {
                    sb_obj.insert("brain_dir".into(), root);
                }
            }
            obj.insert("agy_tail".into(), sb);
        }
    }
    let mut restored = AcpDriver::agy("agy");
    restored.restore_state(&snap).unwrap();

    // Next user prompt replays the buffered autonomous burst before fresh output.
    let _ = restored
        .prompt("t2", &RuntimePromptReq::of("status?"))
        .unwrap();
    for replayed in [
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"Checking health"}
        }}}),
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call","toolCallId":"replayed_tc","title":"Run tests","kind":"execute","status":"in_progress"
        }}}),
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call_update","toolCallId":"replayed_tc","status":"completed"
        }}}),
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"⚠ The stream was interrupted. Retrying...\n"}
        }}}),
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"All tests passed!"}
        }}}),
    ] {
        assert!(
            restored.on_line(&replayed.to_string()).events.is_empty(),
            "replayed burst frame must be suppressed: {replayed}"
        );
    }
    let fresh = restored.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Fresh turn 2 reply"}
        }}})
        .to_string(),
    );
    assert!(fresh.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::AssistantMessage { text, .. } if text == "Fresh turn 2 reply")
    )));
}

#[test]
fn grok_raw_updates_and_sideband_updates_jsonl_settle_and_deduplicate() {
    let mut d = AcpDriver::grok();
    d.set_sideband_root_dir("/tmp/test-grok-home");
    let sid = "02aa2a78-9d8f-4e1a-a67c-a40a96b34c33";
    d.start(&launch_ctx("/Users/test/my_proj"));
    d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
    d.on_line(&json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":sid}}).to_string());

    // Prompted turn with Grok's untyped raw `tool_call` (no `kind` or `status`) + `turn_completed`.
    let p1 = d.prompt("t1", &RuntimePromptReq::of("run ls")).unwrap();
    let pid = serde_json::from_str::<Value>(&p1.writes[0]).unwrap()["id"]
        .as_u64()
        .unwrap();
    let tc = d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call",
            "toolCallId":"grok_tc_1",
            "title":"bash",
            "rawInput":{"command":"ls -la","is_background":true}
        }}})
        .to_string(),
    );
    assert!(tc.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::CommandExecution { input, .. } if input == "ls -la")
                && i.status == TurnItemStatus::Running
    )));
    let tcu = d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call_update",
            "toolCallId":"grok_tc_1",
            "status":"completed",
            "rawOutput":{"stdout":"total 8"}
        }}})
        .to_string(),
    );
    assert!(tcu
        .events
        .iter()
        .any(|e| matches!(e, DriverEvent::BackgroundTasks(tasks) if tasks.len() == 1)));
    let tc_done = d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"turn_completed",
            "stop_reason":"end_turn",
            "usage":{"inputTokens":50,"outputTokens":12,"cachedReadTokens":20,"reasoningTokens":4}
        }}})
        .to_string(),
    );
    assert!(tc_done.events.iter().any(
        |e| matches!(e, DriverEvent::Usage(u) if u.input_tokens == 50 && u.output_tokens == 12)
    ));
    assert!(!tc_done
        .events
        .iter()
        .any(|e| matches!(e, DriverEvent::Terminal { .. })));
    d.on_line(&json!({"jsonrpc":"2.0","id":pid,"result":{"stopReason":"end_turn"}}).to_string());

    // Check Grok's percent-encoded sideband path.
    let poll = d.sideband_poll().expect("grok sideband poll");
    match poll.source {
        SidebandSource::AppendLines { path, .. } => {
            assert!(path.to_string_lossy().contains(
                "%2FUsers%2Ftest%2Fmy_proj/02aa2a78-9d8f-4e1a-a67c-a40a96b34c33/updates.jsonl"
            ));
        }
        other => panic!("unexpected source: {other:?}"),
    }

    // Between-prompt autonomous wakeup in `updates.jsonl` settles immediately on `turn_completed`.
    let ts_future = (Utc::now().timestamp_millis() + 2_000) as f64;
    let grok_jsonl = format!(
        "{}\n{}\n{}\n{}\n{}\n",
        json!({"sessionId":sid,"_meta":{"agentTimestampMs":ts_future},"update":{
            "sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"Grok bg thought"}
        }}),
        json!({"sessionId":sid,"_meta":{"agentTimestampMs":ts_future},"update":{
            "sessionUpdate":"tool_call","toolCallId":"bg_1","title":"bash","rawInput":{"command":"echo hi"}
        }}),
        json!({"sessionId":sid,"_meta":{"agentTimestampMs":ts_future},"update":{
            "sessionUpdate":"tool_call_update","toolCallId":"bg_1","status":"completed","rawOutput":"hi"
        }}),
        json!({"sessionId":sid,"_meta":{"agentTimestampMs":ts_future},"update":{
            "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Grok finished bg"}
        }}),
        json!({"sessionId":sid,"_meta":{"agentTimestampMs":ts_future},"update":{
            "sessionUpdate":"turn_completed","stop_reason":"end_turn",
            "usage":{"inputTokens":30,"outputTokens":9}
        }}),
    );
    let bg_step = d.on_sideband(SidebandPayload::AppendLines {
        resolved_path: None,
        using_fallback: false,
        next_offset: grok_jsonl.len() as u64,
        bytes: grok_jsonl.into_bytes(),
    });
    assert!(bg_step
        .events
        .iter()
        .any(|e| matches!(e, DriverEvent::TurnStarted { turn_id } if turn_id == "auto-1")));
    assert!(bg_step.events.iter().any(|e| matches!(
        e,
        DriverEvent::Terminal {
            status: TurnTerminalStatus::Completed,
            ..
        }
    )));
    assert!(!d.needs_idle_turn_settle());

    // Next prompt suppresses replayed Grok chunks and emits fresh reply.
    let _ = d.prompt("t2", &RuntimePromptReq::of("next")).unwrap();
    assert!(d
        .on_line(
            &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
                "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Grok finished bg"}
            }}})
            .to_string()
        )
        .events
        .is_empty());
}

#[test]
fn gemini_sideband_diffs_session_json_snapshots_and_deduplicates_burst() {
    let mut d = AcpDriver::gemini("gemini");
    d.set_sideband_root_dir("/tmp/test-gemini-home");
    let sid = "a753ef73-62fb-46e4-a2b6-b973e8f278b1";
    handshake_and_finish_turn1(&mut d, "/workspace", sid);

    let pre_ts = (Utc::now() - ChronoDuration::seconds(10)).to_rfc3339();
    let post_ts = (Utc::now() + ChronoDuration::seconds(2)).to_rfc3339();
    let path = std::path::PathBuf::from("/tmp/test-gemini-home/tmp/ws/chats/session-a753ef73.json");

    // Snapshot 1: old message (ignored) + new message with running tool call.
    let snap1 = json!({
        "sessionId": sid,
        "messages": [
            {"id": "m0", "type": "gemini", "timestamp": pre_ts, "content": "Old turn 1"},
            {
                "id": "m1",
                "type": "gemini",
                "timestamp": post_ts,
                "thoughts": [{"subject": "Plan", "description": "Inspecting status"}],
                "toolCalls": [{
                    "id": "gem_tc_1",
                    "name": "run_shell_command",
                    "args": {"command": "git status"},
                    "status": "running"
                }],
                "content": ""
            }
        ]
    });
    let s1 = d.on_sideband(SidebandPayload::GeminiJson {
        resolved_path: path.clone(),
        stamp: (1, 100),
        bytes: serde_json::to_vec(&snap1).unwrap(),
    });
    assert!(s1
        .events
        .iter()
        .any(|e| matches!(e, DriverEvent::TurnStarted { turn_id } if turn_id == "auto-1")));
    assert!(
        !d.needs_idle_turn_settle(),
        "running tool must block idle settle"
    );

    // Snapshot 2: tool completes and final content + tokens arrive.
    let snap2 = json!({
        "sessionId": sid,
        "messages": [
            {"id": "m0", "type": "gemini", "timestamp": pre_ts, "content": "Old turn 1"},
            {
                "id": "m1",
                "type": "gemini",
                "timestamp": post_ts,
                "thoughts": [{"subject": "Plan", "description": "Inspecting status"}],
                "toolCalls": [{
                    "id": "gem_tc_1",
                    "name": "run_shell_command",
                    "args": {"command": "git status"},
                    "status": "success",
                    "resultDisplay": "On branch feat/ide"
                }],
                "content": "Workspace is clean.",
                "tokens": {"input": 80, "output": 15, "cached": 10, "thoughts": 5}
            }
        ]
    });
    let s2 = d.on_sideband(SidebandPayload::GeminiJson {
        resolved_path: path,
        stamp: (2, 200),
        bytes: serde_json::to_vec(&snap2).unwrap(),
    });
    assert!(s2.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::AssistantMessage { text, .. } if text == "Workspace is clean.")
    )));
    assert!(d.needs_idle_turn_settle());
    let _ = d.on_idle_settle();

    // Next prompt suppresses replayed Gemini output.
    let _ = d.prompt("t2", &RuntimePromptReq::of("continue")).unwrap();
    assert!(d
        .on_line(
            &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
                "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Workspace is clean."}
            }}})
            .to_string()
        )
        .events
        .is_empty());
}

#[test]
fn opencode_sideband_diffs_in_place_part_updates_and_deduplicates_burst() {
    let mut d = AcpDriver::opencode();
    d.set_sideband_root_dir("/tmp/test-opencode-data");
    let sid = "ses_262ad3c69ffeB7b1E1bC997L5s";
    handshake_and_finish_turn1(&mut d, "/workspace", sid);

    let ts = Utc::now().timestamp_millis() + 2_000;
    let batch1 = format!(
        "{}\n{}\n",
        json!({
            "id": "prt_1",
            "message_id": "msg_auto",
            "time_created": ts,
            "time_updated": ts,
            "role": "assistant",
            "data": {"type": "reasoning", "text": "Checking build"}
        }),
        json!({
            "id": "prt_2",
            "message_id": "msg_auto",
            "time_created": ts,
            "time_updated": ts,
            "role": "assistant",
            "data": {
                "type": "tool",
                "callID": "oc_call_1",
                "tool": "bash",
                "state": {"status": "running", "input": {"command": "cargo check"}}
            }
        }),
    );
    let s1 = d.on_sideband(SidebandPayload::OpencodeSqlite {
        stamp: (1, 100, 1, 50),
        rows_jsonl: batch1.into_bytes(),
    });
    assert!(s1
        .events
        .iter()
        .any(|e| matches!(e, DriverEvent::TurnStarted { turn_id } if turn_id == "auto-1")));
    assert!(!d.needs_idle_turn_settle());

    // In-place SQLite UPDATE on `prt_2` (`completed`) + streaming `prt_3` text + `step-finish`.
    let batch2 = format!(
        "{}\n{}\n{}\n",
        json!({
            "id": "prt_2",
            "message_id": "msg_auto",
            "time_created": ts,
            "time_updated": ts + 50,
            "role": "assistant",
            "data": {
                "type": "tool",
                "callID": "oc_call_1",
                "tool": "bash",
                "state": {"status": "completed", "input": {"command": "cargo check"}, "output": "Finished dev"}
            }
        }),
        json!({
            "id": "prt_3",
            "message_id": "msg_auto",
            "time_created": ts + 60,
            "time_updated": ts + 80,
            "role": "assistant",
            "data": {"type": "text", "text": "Build succeeded.", "time": {"start": ts + 60, "end": ts + 80}}
        }),
        json!({
            "id": "prt_4",
            "message_id": "msg_auto",
            "time_created": ts + 90,
            "time_updated": ts + 90,
            "role": "assistant",
            "data": {
                "type": "step-finish",
                "reason": "stop",
                "tokens": {"input": 120, "output": 30, "reasoning": 8, "cache": {"read": 50}}
            }
        }),
    );
    let s2 = d.on_sideband(SidebandPayload::OpencodeSqlite {
        stamp: (1, 100, 2, 120),
        rows_jsonl: batch2.into_bytes(),
    });
    assert!(s2.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::CommandExecution { output: Some(out), .. } if out == "Finished dev")
                && i.status == TurnItemStatus::Completed
    )));
    assert!(s2.events.iter().any(|e| matches!(
        e,
        DriverEvent::Terminal {
            status: TurnTerminalStatus::Completed,
            ..
        }
    )));
    assert!(!d.needs_idle_turn_settle());

    // Next prompt suppresses replayed OpenCode burst.
    let _ = d.prompt("t2", &RuntimePromptReq::of("deploy")).unwrap();
    assert!(d
        .on_line(
            &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
                "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Build succeeded."}
            }}})
            .to_string()
        )
        .events
        .is_empty());
}

#[test]
fn custom_acp_server_adopts_otto_acp_sideband_file_env() {
    let mut d = AcpDriver::new(RuntimeProvider::Gemini, "custom-acp", Vec::new());
    let mut spec = d.launch(&launch_ctx("/workspace"));
    spec.env.push((
        "OTTO_ACP_SIDEBAND_FILE".into(),
        "/tmp/custom-acp-updates.jsonl".into(),
    ));
    d.adopt_spec(&mut spec);
    assert!(
        !spec.env.iter().any(|(k, _)| k == "OTTO_ACP_SIDEBAND_FILE"),
        "OTTO_ACP_SIDEBAND_FILE must be consumed by adopt_spec"
    );
    handshake_and_finish_turn1(&mut d, "/workspace", "custom-sid-1");
    let poll = d.sideband_poll().expect("custom sideband poll");
    match poll.source {
        SidebandSource::AppendLines { path, .. } => {
            assert_eq!(
                path,
                std::path::PathBuf::from("/tmp/custom-acp-updates.jsonl")
            );
        }
        other => panic!("unexpected source: {other:?}"),
    }
}

#[test]
fn agy_sideband_survives_mid_settle_wakeup_and_suppresses_orphan_burst_updates() {
    let mut d = AcpDriver::agy("agy");
    d.set_sideband_root_dir("/tmp/test-agy-brain");
    let sid = "eaf6440b-3dbd-4661-9f38-63372630db72";
    handshake_and_finish_turn1(&mut d, "/workspace", sid);

    // Whole-second RFC3339 (`...Z`) in the SAME wall-clock second right after
    // `finish_prompted_turn()` must NOT be dropped (`parse_rfc3339_end_ms`).
    let same_sec = Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string();
    let wake1 = format!(
        "{}\n{}\n{}\n",
        json!({
            "step_index": 10,
            "type": "SYSTEM_MESSAGE",
            "source": "SYSTEM",
            "created_at": same_sec,
            "content": "Subagent finished"
        }),
        json!({
            "step_index": 11,
            "type": "PLANNER_RESPONSE",
            "source": "MODEL",
            "created_at": same_sec,
            "thinking": "Wakeup 1 thought",
            "tool_calls": [{
                "id": "call_wake_1",
                "name": "default_api:run_command",
                "args": {"CommandLine": "\"echo one\"", "toolSummary": "\"Echo one\""}
            }]
        }),
        json!({
            "step_index": 12,
            "type": "GENERIC",
            "source": "MODEL",
            "status": "DONE",
            "created_at": same_sec,
            "id": "call_wake_1",
            "content": "one"
        }),
    );
    let s1 = d.on_sideband(SidebandPayload::AppendLines {
        resolved_path: None,
        using_fallback: false,
        next_offset: 1024,
        bytes: wake1.into_bytes(),
    });
    assert!(s1
        .events
        .iter()
        .any(|e| matches!(e, DriverEvent::TurnStarted { turn_id } if turn_id == "auto-1")));

    // Finish auto-1's planner response, then immediately land auto-2's
    // `<SYSTEM_MESSAGE>` BEFORE `on_idle_settle()` fires (and also exercise
    // `on_turn_settled` between wakeups).
    let finish1 = format!(
        "{}\n",
        json!({
            "step_index": 13,
            "type": "PLANNER_RESPONSE",
            "source": "MODEL",
            "created_at": same_sec,
            "content": "Wakeup 1 summary"
        })
    );
    let _ = d.on_sideband(SidebandPayload::AppendLines {
        resolved_path: None,
        using_fallback: false,
        next_offset: 1536,
        bytes: finish1.into_bytes(),
    });
    assert!(d.needs_idle_turn_settle());
    let _ = d.on_idle_settle();

    // Wakeup 2 starts with `<SYSTEM_MESSAGE>`: must emit `TurnStarted(auto-2)`
    // immediately and hold `needs_idle_turn_settle() == false` while thinking!
    let wake2_header = format!(
        "{}\n",
        json!({
            "step_index": 14,
            "type": "SYSTEM_MESSAGE",
            "source": "SYSTEM",
            "created_at": same_sec,
            "content": "Timer fired"
        })
    );
    let s2_open = d.on_sideband(SidebandPayload::AppendLines {
        resolved_path: None,
        using_fallback: false,
        next_offset: 2048,
        bytes: wake2_header.into_bytes(),
    });
    assert!(s2_open
        .events
        .iter()
        .any(|e| matches!(e, DriverEvent::TurnStarted { turn_id } if turn_id == "auto-2")));
    assert!(
        !d.needs_idle_turn_settle(),
        "auto-2 must stay open while awaiting planner response"
    );

    // Even if an errored planner + stream retry happens during auto-2,
    // `in_autonomous_gap` survives and emits the recovered wakeup live.
    let wake2_body = format!(
        "{}\n{}\n{}\n",
        json!({
            "step_index": 15,
            "type": "PLANNER_RESPONSE",
            "source": "MODEL",
            "status": "ERROR",
            "created_at": same_sec,
            "thinking": "Partial retry thought"
        }),
        json!({
            "step_index": 16,
            "type": "ERROR_MESSAGE",
            "source": "SYSTEM",
            "created_at": same_sec,
            "content": "Stream interrupted, retrying..."
        }),
        json!({
            "step_index": 17,
            "type": "PLANNER_RESPONSE",
            "source": "MODEL",
            "created_at": same_sec,
            "thinking": "Wakeup 2 thought",
            "content": "Wakeup 2 complete"
        }),
    );
    let s2_body = d.on_sideband(SidebandPayload::AppendLines {
        resolved_path: None,
        using_fallback: false,
        next_offset: 3072,
        bytes: wake2_body.into_bytes(),
    });
    assert!(s2_body.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::AssistantMessage { text, streaming: false, .. }
                if text == "Wakeup 2 complete")
    )));
    assert!(d.needs_idle_turn_settle());
    let _ = d.on_idle_settle();

    // User sends prompt `t2`. `agy` replays:
    //  1) An orphan `tool_call_update` for a `schedule` timer started in `t1`
    //  2) All thoughts, tool calls, warning banner, and messages from auto-1 & auto-2
    let _ = d
        .prompt("t2", &RuntimePromptReq::of("where do we stand?"))
        .unwrap();
    for replay in [
        json!({"sessionUpdate":"tool_call_update","toolCallId":"call_orphan_schedule_from_t1","status":"completed","title":"Run schedule"}),
        json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"Wakeup 1 thought"}}),
        json!({"sessionUpdate":"tool_call","toolCallId":"call_wake_1","title":"Echo one","kind":"execute","status":"in_progress"}),
        json!({"sessionUpdate":"tool_call_update","toolCallId":"call_wake_1","status":"completed"}),
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Wakeup 1 summary"}}),
        json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"Partial retry thought"}}),
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"⚠ Stream interrupted, retrying..."}}),
        json!({"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"Wakeup 2 thought"}}),
        json!({"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Wakeup 2 complete"}}),
    ] {
        let ev = d.on_line(
            &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":replay}})
                .to_string(),
        );
        assert!(
            ev.events.is_empty(),
            "replayed burst frame leaked into t2: {replay:?} -> {:?}",
            ev.events
        );
    }

    // Real `t2` tool call, update, and answer pass through intact!
    let real_call = d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call","toolCallId":"call_t2_real","title":"Check status","kind":"execute","status":"in_progress"
        }}})
        .to_string(),
    );
    assert_eq!(real_call.events.len(), 1);
    let real_up = d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call_update","toolCallId":"call_t2_real","status":"completed"
        }}})
        .to_string(),
    );
    assert!(!real_up.events.is_empty());
    let real_msg = d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"All green and deployed."}
        }}})
        .to_string(),
    );
    assert!(real_msg.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::AssistantMessage { text, .. } if text == "All green and deployed.")
    )));
}
