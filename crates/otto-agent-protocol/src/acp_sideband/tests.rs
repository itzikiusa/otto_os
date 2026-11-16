use chrono::{Duration as ChronoDuration, Utc};
use otto_core::agent_runtime::{
    RuntimeAccessMode, RuntimePromptReq, RuntimeProvider, TurnItemKind, TurnItemStatus,
    TurnTerminalStatus,
};
use serde_json::{json, Value};

use crate::acp::AcpDriver;
use crate::driver::{
    ActiveTailChunk, ActiveTailKind, DriverEvent, LaunchContext, SidebandPayload, SidebandSource,
    StructuredDriver,
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
        doc: snap1,
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
        doc: snap2,
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
        has_more: false,
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
        has_more: false,
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

fn rfc3339_from_ms(ms: u64) -> String {
    chrono::DateTime::<Utc>::from_timestamp_millis(ms as i64)
        .expect("valid timestamp")
        .to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

#[test]
fn p1_1_grace_window_and_turn_ownership_suppress_prompted_tail_flushes() {
    // 1. Gemini: `idle - 15ms`, `idle + 20ms`, and structural `"user"` turn ownership
    //    (`idle + 400ms` & `idle + 900ms`) emit zero duplicates; `idle + 2000ms` emits `auto-1`.
    let mut gem = AcpDriver::gemini("gemini");
    gem.set_sideband_root_dir("/tmp/test-gemini-p1-1");
    let gem_sid = "a753ef73-62fb-46e4-a2b6-b973e8f278b1";
    handshake_and_finish_turn1(&mut gem, "/workspace", gem_sid);
    let gem_idle = gem.snapshot_state()["sideband"]["idle_since_ms"]
        .as_u64()
        .expect("gemini idle_since_ms");
    let gem_path =
        std::path::PathBuf::from("/tmp/test-gemini-p1-1/tmp/ws/chats/session-a753ef73.json");

    let gem_prompted_flush1 = json!({
        "sessionId": gem_sid,
        "messages": [
            {"id": "m_minus15", "type": "gemini", "timestamp": rfc3339_from_ms(gem_idle - 15), "content": "Chunk before idle"},
            {"id": "m_plus20", "type": "gemini", "timestamp": rfc3339_from_ms(gem_idle + 20), "content": "Chunk within 50ms grace"},
            {"id": "u1", "type": "user", "timestamp": rfc3339_from_ms(gem_idle - 200), "content": "start bg"},
            {"id": "m_owned", "type": "gemini", "timestamp": rfc3339_from_ms(gem_idle + 400), "content": "Prompted answer part 1"}
        ]
    });
    assert!(
        gem.on_sideband(SidebandPayload::GeminiJson {
            resolved_path: gem_path.clone(),
            stamp: (1, 100),
            doc: gem_prompted_flush1,
        })
        .events
        .is_empty(),
        "Gemini prompted messages at idle-15, idle+20, and user-owned idle+400 must be baselined"
    );

    // Final disk flush of `m_owned` lands even later (`idle + 900`), plus genuine autonomous `m_auto` at `idle + 2000`.
    let gem_auto = json!({
        "sessionId": gem_sid,
        "messages": [
            {"id": "m_minus15", "type": "gemini", "timestamp": rfc3339_from_ms(gem_idle - 15), "content": "Chunk before idle"},
            {"id": "m_plus20", "type": "gemini", "timestamp": rfc3339_from_ms(gem_idle + 20), "content": "Chunk within 50ms grace"},
            {"id": "u1", "type": "user", "timestamp": rfc3339_from_ms(gem_idle - 200), "content": "start bg"},
            {"id": "m_owned", "type": "gemini", "timestamp": rfc3339_from_ms(gem_idle + 900), "content": "Prompted answer part 1 and tail"},
            {"id": "m_auto", "type": "gemini", "timestamp": rfc3339_from_ms(gem_idle + 2_000), "content": "Autonomous Gemini wakeup"}
        ]
    });
    let gem_step = gem.on_sideband(SidebandPayload::GeminiJson {
        resolved_path: gem_path,
        stamp: (2, 200),
        doc: gem_auto,
    });
    assert!(gem_step
        .events
        .iter()
        .any(|e| matches!(e, DriverEvent::TurnStarted { turn_id } if turn_id == "auto-1")));
    assert!(!gem_step.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::AssistantMessage { text, .. } if text.contains("Prompted"))
    )));
    assert!(gem_step.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::AssistantMessage { text, .. } if text == "Autonomous Gemini wakeup")
    )));

    // 2. OpenCode: `idle - 15ms`, `idle + 20ms`, and `msg_time_created <= idle + 250ms`
    //    emit zero duplicates; genuine autonomous message at `idle + 2000ms` emits `auto-1`.
    let mut oc = AcpDriver::opencode();
    oc.set_sideband_root_dir("/tmp/test-opencode-p1-1");
    let oc_sid = "ses_262ad3c69ffeB7b1E1bC997L5s";
    handshake_and_finish_turn1(&mut oc, "/workspace", oc_sid);
    let oc_idle = oc.snapshot_state()["sideband"]["idle_since_ms"]
        .as_u64()
        .expect("opencode idle_since_ms") as i64;

    let oc_prompted = format!(
        "{}\n{}\n{}\n",
        json!({
            "id": "p_minus15",
            "message_id": "msg_p1",
            "msg_time_created": oc_idle - 300,
            "time_updated": oc_idle - 15,
            "role": "assistant",
            "data": {"type": "text", "text": "OpenCode before idle"}
        }),
        json!({
            "id": "p_plus20",
            "message_id": "msg_p1",
            "msg_time_created": oc_idle - 300,
            "time_updated": oc_idle + 20,
            "role": "assistant",
            "data": {"type": "text", "text": "OpenCode within 50ms"}
        }),
        json!({
            "id": "p_late_flush",
            "message_id": "msg_p1",
            "msg_time_created": oc_idle - 100,
            "time_updated": oc_idle + 450,
            "role": "assistant",
            "data": {"type": "text", "text": "OpenCode late SQLite flush"}
        }),
    );
    assert!(
        oc.on_sideband(SidebandPayload::OpencodeSqlite {
            stamp: (1, 100, 1, 50),
            rows_jsonl: oc_prompted.into_bytes(),
            has_more: false,
        })
        .events
        .is_empty(),
        "OpenCode prompted parts at idle-15, idle+20, and msg_time_created<=idle+250 must be baselined"
    );

    let oc_auto = format!(
        "{}\n",
        json!({
            "id": "p_auto",
            "message_id": "msg_auto_1",
            "msg_time_created": oc_idle + 2_000,
            "time_updated": oc_idle + 2_000,
            "role": "assistant",
            "data": {"type": "text", "text": "OpenCode autonomous wakeup"}
        })
    );
    let oc_step = oc.on_sideband(SidebandPayload::OpencodeSqlite {
        stamp: (1, 100, 2, 100),
        rows_jsonl: oc_auto.into_bytes(),
        has_more: false,
    });
    assert!(oc_step
        .events
        .iter()
        .any(|e| matches!(e, DriverEvent::TurnStarted { turn_id } if turn_id == "auto-1")));

    // 3. Grok: `idle - 15ms` and `idle + 20ms` emit zero duplicates; `idle + 2000ms` emits `auto-1`.
    let mut grok = AcpDriver::grok();
    grok.set_sideband_root_dir("/tmp/test-grok-p1-1");
    let grok_sid = "02aa2a78-9d8f-4e1a-a67c-a40a96b34c33";
    handshake_and_finish_turn1(&mut grok, "/workspace", grok_sid);
    let grok_idle = grok.snapshot_state()["sideband"]["idle_since_ms"]
        .as_u64()
        .expect("grok idle_since_ms");

    let grok_prompted = format!(
        "{}\n{}\n",
        json!({"sessionId":grok_sid,"_meta":{"agentTimestampMs":(grok_idle - 15) as f64},"update":{
            "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Grok -15ms"}
        }}),
        json!({"sessionId":grok_sid,"_meta":{"agentTimestampMs":(grok_idle + 20) as f64},"update":{
            "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Grok +20ms"}
        }}),
    );
    assert!(
        grok.on_sideband(SidebandPayload::AppendLines {
            resolved_path: None,
            using_fallback: false,
            next_offset: grok_prompted.len() as u64,
            bytes: grok_prompted.into_bytes(),
        })
        .events
        .is_empty(),
        "Grok chunks at idle-15 and idle+20 must be baselined"
    );

    let grok_auto = format!(
        "{}\n",
        json!({"sessionId":grok_sid,"_meta":{"agentTimestampMs":(grok_idle + 2_000) as f64},"update":{
            "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Grok +2000ms"}
        }})
    );
    let grok_step = grok.on_sideband(SidebandPayload::AppendLines {
        resolved_path: None,
        using_fallback: false,
        next_offset: 1024,
        bytes: grok_auto.into_bytes(),
    });
    assert!(grok_step
        .events
        .iter()
        .any(|e| matches!(e, DriverEvent::TurnStarted { turn_id } if turn_id == "auto-1")));
}

#[test]
fn p1_2_interrupt_autonomous_turn_and_retired_tool_updates_preserve_sideband() {
    let mut d = AcpDriver::agy("agy");
    d.set_sideband_root_dir("/tmp/test-agy-p1-2");
    let sid = "eaf6440b-3dbd-4661-9f38-63372630db72";
    d.start(&launch_ctx("/workspace"));
    d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
    d.on_line(&json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":sid}}).to_string());

    // Prompted turn 1 starts a tool `bg_tool_t1` with a large 128 KiB output, then finishes.
    let p1 = d.prompt("t1", &RuntimePromptReq::of("run job")).unwrap();
    let pid = serde_json::from_str::<Value>(&p1.writes[0]).unwrap()["id"]
        .as_u64()
        .unwrap();
    d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call",
            "toolCallId":"bg_tool_t1",
            "title":"Long build",
            "kind":"execute",
            "status":"in_progress",
            "rawInput":{"command":"make -j8"}
        }}})
        .to_string(),
    );
    let big_output = "x".repeat(128 * 1024);
    d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call_update",
            "toolCallId":"bg_tool_t1",
            "status":"completed",
            "rawOutput":{"stdout":big_output}
        }}})
        .to_string(),
    );
    // P0-1.4: completed tool entry's heavy output is compacted before `PromptResponse`.
    let mid_turn_state_bytes = serde_json::to_string(&d.snapshot_state()).unwrap().len();
    assert!(
        mid_turn_state_bytes < 8_192,
        "completed tool output must be compacted out of driver_state (got {mid_turn_state_bytes} bytes)"
    );
    d.on_line(&json!({"jsonrpc":"2.0","id":pid,"result":{"stopReason":"end_turn"}}).to_string());

    // Part B: During idle gap, a late `tool_call_update` for retired `bg_tool_t1` (even WITH title)
    // and an unknown nameless `tool_call_update` must be dropped cleanly without disabling sideband
    // or synthesizing an empty DynamicTool card.
    let late_retired = d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call_update",
            "toolCallId":"bg_tool_t1",
            "status":"completed",
            "title":"Long build"
        }}})
        .to_string(),
    );
    assert!(late_retired.events.is_empty());
    let late_nameless = d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call_update",
            "toolCallId":"unknown_idle_tool",
            "status":"completed"
        }}})
        .to_string(),
    );
    assert!(late_nameless.events.is_empty());
    assert!(
        d.sideband_poll().is_some(),
        "late retired/nameless tool_call_update must not set native_autonomous_seen"
    );

    // Part A: Start `auto-1` via sideband, interrupt it, and verify `cancelling` resets to `false`
    // so `auto-2` can still start via sideband!
    let post_ts = (Utc::now() + ChronoDuration::seconds(2)).to_rfc3339();
    let wake1 = format!(
        "{}\n",
        json!({
            "step_index": 20,
            "type": "SYSTEM_MESSAGE",
            "source": "SYSTEM",
            "created_at": post_ts,
            "content": "Timer 1"
        })
    );
    let s1 = d.on_sideband(SidebandPayload::AppendLines {
        resolved_path: None,
        using_fallback: false,
        next_offset: 512,
        bytes: wake1.into_bytes(),
    });
    assert!(s1
        .events
        .iter()
        .any(|e| matches!(e, DriverEvent::TurnStarted { turn_id } if turn_id == "auto-1")));

    let int_step = d.interrupt();
    assert!(int_step.events.iter().any(|e| matches!(
        e,
        DriverEvent::Terminal {
            status: TurnTerminalStatus::Interrupted,
            ..
        }
    )));
    assert_eq!(d.snapshot_state()["cancelling"], json!(false));

    let wake2 = format!(
        "{}\n{}\n",
        json!({
            "step_index": 21,
            "type": "SYSTEM_MESSAGE",
            "source": "SYSTEM",
            "created_at": post_ts,
            "content": "Timer 2"
        }),
        json!({
            "step_index": 22,
            "type": "PLANNER_RESPONSE",
            "source": "MODEL",
            "created_at": post_ts,
            "content": "Auto-2 answer"
        }),
    );
    let s2 = d.on_sideband(SidebandPayload::AppendLines {
        resolved_path: None,
        using_fallback: false,
        next_offset: 1024,
        bytes: wake2.into_bytes(),
    });
    assert!(s2
        .events
        .iter()
        .any(|e| matches!(e, DriverEvent::TurnStarted { turn_id } if turn_id == "auto-2")));
    let _ = d.on_idle_settle();

    // In prompted turn `t2`, once pre-prompt drain ends on fresh output, a self-describing
    // single-frame `tool_call_update` with non-empty `title` passes through!
    let _ = d.prompt("t2", &RuntimePromptReq::of("follow up")).unwrap();
    let _ = d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Starting t2 work"}
        }}})
        .to_string(),
    );
    let single_frame_tool = d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call_update",
            "toolCallId":"t2_self_describing",
            "title":"Read Cargo.toml",
            "kind":"read",
            "status":"completed"
        }}})
        .to_string(),
    );
    assert!(
        !single_frame_tool.events.is_empty(),
        "legitimate self-describing tool_call_update in prompted turn after drain must not be dropped"
    );
}

#[test]
fn p1_3_cancelled_agy_and_grok_prompted_turns_do_not_block_autonomous_wakeups() {
    // 1. Agy: cancelled `CORTEX_STEP_TYPE_USER_INPUT` without terminal `PLANNER_RESPONSE`
    //    must not block a subsequent post-idle `CORTEX_STEP_TYPE_SYSTEM_MESSAGE`.
    let mut agy = AcpDriver::agy("agy");
    agy.set_sideband_root_dir("/tmp/test-agy-p1-3");
    let agy_sid = "eaf6440b-3dbd-4661-9f38-63372630db72";
    agy.start(&launch_ctx("/workspace"));
    agy.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
    agy.on_line(&json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":agy_sid}}).to_string());
    let p1 = agy
        .prompt("t1", &RuntimePromptReq::of("cancel me"))
        .unwrap();
    let pid = serde_json::from_str::<Value>(&p1.writes[0]).unwrap()["id"]
        .as_u64()
        .unwrap();
    let _ = agy.interrupt();
    agy.on_line(&json!({"jsonrpc":"2.0","id":pid,"result":{"stopReason":"cancelled"}}).to_string());

    let agy_idle = agy.snapshot_state()["sideband"]["idle_since_ms"]
        .as_u64()
        .expect("agy idle_since_ms");
    let agy_lines = format!(
        "{}\n{}\n{}\n",
        json!({
            "step_index": 1,
            "type": "CORTEX_STEP_TYPE_USER_INPUT",
            "source": "CORTEX_STEP_SOURCE_USER_EXPLICIT",
            "created_at": rfc3339_from_ms(agy_idle - 200),
            "content": "cancel me"
        }),
        json!({
            "step_index": 2,
            "type": "CORTEX_STEP_TYPE_SYSTEM_MESSAGE",
            "source": "CORTEX_STEP_SOURCE_SYSTEM",
            "created_at": rfc3339_from_ms(agy_idle + 2_000),
            "content": "Subagent finished"
        }),
        json!({
            "step_index": 3,
            "type": "CORTEX_STEP_TYPE_PLANNER_RESPONSE",
            "source": "CORTEX_STEP_SOURCE_MODEL",
            "created_at": rfc3339_from_ms(agy_idle + 2_100),
            "content": "Handled wakeup after cancelled prompt"
        }),
    );
    let agy_step = agy.on_sideband(SidebandPayload::AppendLines {
        resolved_path: None,
        using_fallback: false,
        next_offset: agy_lines.len() as u64,
        bytes: agy_lines.into_bytes(),
    });
    assert!(agy_step
        .events
        .iter()
        .any(|e| matches!(e, DriverEvent::TurnStarted { turn_id } if turn_id == "auto-1")));
    assert!(agy_step.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::AssistantMessage { text, .. }
                if text == "Handled wakeup after cancelled prompt")
    )));

    // 2. Grok: cancelled `user_message` without `turn_completed` must clear `in_prompted_turn`
    //    when post-idle autonomous records arrive.
    let mut grok = AcpDriver::grok();
    grok.set_sideband_root_dir("/tmp/test-grok-p1-3");
    let grok_sid = "02aa2a78-9d8f-4e1a-a67c-a40a96b34c33";
    grok.start(&launch_ctx("/workspace"));
    grok.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
    grok.on_line(&json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":grok_sid}}).to_string());
    let gp1 = grok.prompt("t1", &RuntimePromptReq::of("abort")).unwrap();
    let gpid = serde_json::from_str::<Value>(&gp1.writes[0]).unwrap()["id"]
        .as_u64()
        .unwrap();
    let _ = grok.interrupt();
    grok.on_line(
        &json!({"jsonrpc":"2.0","id":gpid,"result":{"stopReason":"cancelled"}}).to_string(),
    );

    let grok_idle = grok.snapshot_state()["sideband"]["idle_since_ms"]
        .as_u64()
        .expect("grok idle_since_ms");
    let grok_lines = format!(
        "{}\n{}\n{}\n",
        json!({"sessionId":grok_sid,"_meta":{"agentTimestampMs":(grok_idle - 200) as f64},"update":{
            "sessionUpdate":"user_message","content":{"type":"text","text":"abort"}
        }}),
        json!({"sessionId":grok_sid,"_meta":{"agentTimestampMs":(grok_idle + 2_000) as f64},"update":{
            "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Grok woke after cancel"}
        }}),
        json!({"sessionId":grok_sid,"_meta":{"agentTimestampMs":(grok_idle + 2_050) as f64},"update":{
            "sessionUpdate":"turn_completed","stop_reason":"end_turn"
        }}),
    );
    let grok_step = grok.on_sideband(SidebandPayload::AppendLines {
        resolved_path: None,
        using_fallback: false,
        next_offset: grok_lines.len() as u64,
        bytes: grok_lines.into_bytes(),
    });
    assert!(grok_step
        .events
        .iter()
        .any(|e| matches!(e, DriverEvent::TurnStarted { turn_id } if turn_id == "auto-1")));
    assert!(grok_step.events.iter().any(|e| matches!(
        e,
        DriverEvent::Terminal {
            status: TurnTerminalStatus::Completed,
            ..
        }
    )));
}

#[test]
fn perf_p1_2_adaptive_poll_interval_and_negative_lookup_backoff() {
    let mut sb = super::AcpSideband::for_provider(RuntimeProvider::Gemini);
    sb.set_root_dir(Some("/tmp/test-gemini-perf".into()));
    sb.finish_prompted_turn();
    let base = sb.poll_interval_at(0);
    let _ = base;

    let mut d = AcpDriver::gemini("gemini");
    d.set_sideband_root_dir("/tmp/test-gemini-perf");
    let sid = "a753ef73-62fb-46e4-a2b6-b973e8f278b1";
    handshake_and_finish_turn1(&mut d, "/workspace", sid);

    let first_poll = d.sideband_poll().expect("initial gemini poll");
    assert_eq!(first_poll.interval, std::time::Duration::from_millis(400));
    match first_poll.source {
        SidebandSource::GeminiJson { allow_scan, .. } => assert!(allow_scan),
        other => panic!("unexpected source: {other:?}"),
    }

    // Negative lookup (`SidebandPayload::Unresolved`) arms 3s scan backoff (`allow_scan == false`).
    assert!(d.on_sideband(SidebandPayload::Unresolved).events.is_empty());
    let backoff_poll = d.sideband_poll().expect("backoff gemini poll");
    assert!(backoff_poll.interval > std::time::Duration::from_millis(400));
    match backoff_poll.source {
        SidebandSource::GeminiJson { allow_scan, .. } => {
            assert!(
                !allow_scan,
                "unresolved lookup must suppress read_dir scan during 3s backoff"
            );
        }
        other => panic!("unexpected source: {other:?}"),
    }

    // Adaptive cadence steps 400ms (<10s) -> 1500ms (>=10s) -> 3000ms (>=60s) -> 400ms on activity.
    let mut cadence = super::AcpSideband::for_provider(RuntimeProvider::Agy);
    cadence.finish_prompted_turn();
    let t0 = super::burst::now_ms();
    assert_eq!(
        cadence.poll_interval_at(t0 + 5_000),
        std::time::Duration::from_millis(400)
    );
    assert_eq!(
        cadence.poll_interval_at(t0 + 15_000),
        std::time::Duration::from_millis(1500)
    );
    assert_eq!(
        cadence.poll_interval_at(t0 + 65_000),
        std::time::Duration::from_millis(3000)
    );
    cadence.reset_for_prompt();
    cadence.finish_prompted_turn();
    let t1 = super::burst::now_ms();
    assert_eq!(
        cadence.poll_interval_at(t1 + 100),
        std::time::Duration::from_millis(400)
    );
}

#[test]
fn agy_subagent_transcript_tailing_background_log_slash_id_and_timer_expiry() {
    let mut d = AcpDriver::agy("agy");
    d.set_sideband_root_dir("/tmp/test-agy-subagent-brain");
    let sid = "eaf6440b-3dbd-4661-9f38-63372630db72";
    d.start(&launch_ctx("/workspace"));
    d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
    d.on_line(&json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":sid}}).to_string());

    let p1 = d.prompt("t1", &RuntimePromptReq::of("delegate")).unwrap();
    let pid: u64 = p1
        .writes
        .iter()
        .find_map(|w| {
            let v: Value = serde_json::from_str(w).ok()?;
            (v.get("method")?.as_str()? == "session/prompt").then(|| v.get("id")?.as_u64())?
        })
        .expect("prompt id");

    // 1. Launch `invoke_subagent`: starts as Running Subagent card and stays Running
    //    even after its launch RPC returns `conversationId` + `logAbsoluteUri`.
    let inv_start = d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call",
            "toolCallId":"inv-1",
            "title":"invoke_subagent",
            "kind":"other",
            "status":"in_progress",
            "rawInput":{"TypeName":"self","Role":"Backend Fixer","Prompt":"Fix the timer bug"}
        }}})
        .to_string(),
    );
    assert!(inv_start.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if i.id == "t1:tool:inv-1"
                && i.status == TurnItemStatus::Running
                && matches!(
                    &i.kind,
                    TurnItemKind::Subagent {
                        tool_name,
                        description: Some(desc),
                        subagent_type: Some(st),
                        prompt: Some(pr),
                        background: true,
                        ..
                    } if tool_name == "invoke_subagent"
                        && desc == "Backend Fixer"
                        && st == "self"
                        && pr == "Fix the timer bug"
                )
    )));

    let sub_out = json!({
        "conversationId": "70426041-f05b-45ac-ba8d-6544e59c0aa7",
        "logAbsoluteUri": "file:///tmp/test-agy-subagent-brain/70426041-f05b-45ac-ba8d-6544e59c0aa7/.system_generated/logs/transcript.jsonl"
    })
    .to_string();
    let inv_done = d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call_update",
            "toolCallId":"inv-1",
            "status":"completed",
            "content":[{"type":"content","content":{"type":"text","text":sub_out}}]
        }}})
        .to_string(),
    );
    assert!(
        inv_done.events.iter().any(|e| matches!(
            e,
            DriverEvent::TurnItem(i)
                if i.id == "t1:tool:inv-1" && i.status == TurnItemStatus::Running
        )),
        "async invoke_subagent card must remain Running after launch RPC returns"
    );
    assert!(inv_done.events.iter().any(|e| matches!(
        e,
        DriverEvent::BackgroundTasks(tasks)
            if tasks.iter().any(|t| t.id == "70426041-f05b-45ac-ba8d-6544e59c0aa7" && t.kind == "agent")
    )));

    // 2. Launch background `run_command` with `/` in task ID + `file://.../*.log` URI.
    d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call",
            "toolCallId":"cmd-1",
            "title":"run_command",
            "kind":"execute",
            "status":"in_progress",
            "rawInput":{"CommandLine":"python3 -m http.server"}
        }}})
        .to_string(),
    );
    let cmd_bg = d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call_update",
            "toolCallId":"cmd-1",
            "status":"completed",
            "content":[{"type":"content","content":{"type":"text","text":
                "Command is still running in the background as a background task with task id: eaf6440b-3dbd-4661-9f38-63372630db72/task-9459\nTask logs are available at: file:///tmp/test-agy-subagent-brain/eaf6440b-3dbd-4661-9f38-63372630db72/.system_generated/tasks/task-9459.log"
            }}]
        }}})
        .to_string(),
    );
    assert!(cmd_bg.events.iter().any(|e| matches!(
        e,
        DriverEvent::BackgroundTasks(tasks)
            if tasks.iter().any(|t| t.id == "eaf6440b-3dbd-4661-9f38-63372630db72/task-9459" && t.kind == "command")
    )));

    // 3. One-shot `schedule` timer (`DurationSeconds: 0`, no `CronExpression`) tracks `kind == "timer"` + `due_at`.
    d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call",
            "toolCallId":"sch-1",
            "title":"schedule",
            "kind":"other",
            "status":"in_progress",
            "rawInput":{"DurationSeconds":0,"Prompt":"Check build"}
        }}})
        .to_string(),
    );
    let sch_done = d.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call_update",
            "toolCallId":"sch-1",
            "status":"completed",
            "content":[{"type":"content","content":{"type":"text","text":"Scheduled timer"}}]
        }}})
        .to_string(),
    );
    assert!(sch_done.events.iter().any(|e| matches!(
        e,
        DriverEvent::BackgroundTasks(tasks)
            if tasks.iter().any(|t| t.id == "schedule:sch-1" && t.kind == "timer")
    )));

    // Finish Turn 1; active tails & expired timer are advertised on `sideband_poll()`.
    d.on_line(&json!({"jsonrpc":"2.0","id":pid,"result":{"stopReason":"end_turn"}}).to_string());
    let poll = d.sideband_poll().expect("sideband poll with active tails");
    assert!(poll.timer_expired, "0s one-shot timer should be expired");
    assert_eq!(poll.active_tails.len(), 2);

    // 4. Deliver sideband batch: `.log` output + child `transcript.jsonl` + timer tick.
    let child_transcript = format!(
        "{}\n{}\n{}\n",
        json!({
            "step_index": 1,
            "type": "PLANNER_RESPONSE",
            "status": "DONE",
            "tool_calls": [
                {"name": "read_file", "args": {"Path": "/workspace/src/lib.rs"}},
                {"name": "send_message", "args": {"Recipient": sid, "Message": "Timer bug resolved cleanly."}}
            ]
        }),
        json!({
            "step_index": 2,
            "type": "GENERIC",
            "status": "DONE",
            "content": "fn main() {}"
        }),
        json!({
            "step_index": 3,
            "type": "PLANNER_RESPONSE",
            "status": "DONE",
            "content": "Done."
        }),
    );
    let batch = d.on_sideband(SidebandPayload::Batch {
        primary: None,
        tails: vec![
            ActiveTailChunk {
                task_id: "eaf6440b-3dbd-4661-9f38-63372630db72/task-9459".into(),
                kind: ActiveTailKind::CommandLog,
                using_fallback: false,
                next_offset: 18,
                bytes: b"Serving HTTP on 0\n".to_vec(),
            },
            ActiveTailChunk {
                task_id: "70426041-f05b-45ac-ba8d-6544e59c0aa7".into(),
                kind: ActiveTailKind::AgySubagentTranscript,
                using_fallback: true,
                next_offset: child_transcript.len() as u64,
                bytes: child_transcript.into_bytes(),
            },
        ],
        timer_tick: true,
    });

    // Background `.log` emitted live `CommandOutput`.
    assert!(batch.events.iter().any(|e| matches!(
        e,
        DriverEvent::CommandOutput { item_id, chunk }
            if item_id == "t1:tool:cmd-1" && chunk == "Serving HTTP on 0\n"
    )));
    // Child transcript emitted child `TurnItem`s with `parent_item_id == Some("t1:tool:inv-1")`.
    assert!(batch.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if i.parent_item_id.as_deref() == Some("t1:tool:inv-1")
    )));
    // Subagent card settled to `Completed` with `tool_uses == 2` and `send_message` output!
    assert!(batch.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if i.id == "t1:tool:inv-1"
                && i.status == TurnItemStatus::Completed
                && matches!(
                    &i.kind,
                    TurnItemKind::Subagent {
                        tool_uses: 2,
                        output: Some(out),
                        ..
                    } if out == "Timer bug resolved cleanly."
                )
    )));
    // Both timer and subagent emitted their background settlement `SystemNotice`s.
    let has_notice = |evs: &[DriverEvent], pred: fn(&str) -> bool| {
        evs.iter().any(|e| {
            matches!(
                e,
                DriverEvent::TurnItem(i)
                    if matches!(&i.kind, TurnItemKind::SystemNotice { message, .. } if pred(message))
            )
        })
    };
    assert!(has_notice(&batch.events, |m| {
        m.starts_with("Background timer finished:")
    }));
    assert!(has_notice(&batch.events, |m| {
        m == "Background agent finished: Backend Fixer"
    }));

    // 5. Turn 2: `manage_task(Action="kill")` and `manage_subagents(Action="kill")`.
    d.prompt("t2", &RuntimePromptReq::of("cleanup")).unwrap();
    let upd = |id: &str, title: &str, input: Value, content: Value| {
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":sid,"update":{
            "sessionUpdate":"tool_call","toolCallId":id,"title":title,"kind":"other",
            "status":"completed","rawInput":input,"content":content
        }}})
        .to_string()
    };
    d.on_line(&upd(
        "inv-2",
        "invoke_subagent",
        json!({"TypeName":"research","Role":"Helper","Prompt":"Research"}),
        json!([{"type":"content","content":{"type":"text","text":"{\"conversationId\":\"sub-to-kill\"}"}}]),
    ));
    let kill_cmd = d.on_line(&upd(
        "mk-1",
        "manage_task",
        json!({"Action":"kill","TaskId":"eaf6440b-3dbd-4661-9f38-63372630db72/task-9459"}),
        json!([]),
    ));
    assert!(has_notice(&kill_cmd.events, |m| {
        m == "Background command stopped: python3 -m http.server"
    }));

    let kill_sub = d.on_line(&upd(
        "ms-1",
        "manage_subagents",
        json!({"Action":"kill","ConversationIds":["sub-to-kill"]}),
        json!([]),
    ));
    assert!(kill_sub.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if i.id == "t2:tool:inv-2" && i.status == TurnItemStatus::Cancelled
    )));
    assert!(kill_sub.events.iter().any(|e| matches!(
        e,
        DriverEvent::BackgroundTasks(tasks) if tasks.is_empty()
    )));
}

#[test]
fn acp_grok_gemini_opencode_subagent_promotion_and_child_routing() {
    // 1. OpenCode / Gemini `task` (`subagent_type`) + child `_meta.parentToolUseId` & child `sessionId`.
    let mut oc = AcpDriver::opencode();
    oc.set_sideband_root_dir("/tmp/test-oc-sub");
    let root_sid = "ses_root_1234567890";
    oc.start(&launch_ctx("/workspace"));
    oc.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
    oc.on_line(&json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":root_sid}}).to_string());
    oc.prompt("t1", &RuntimePromptReq::of("audit")).unwrap();

    let sub_start = oc.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":root_sid,"update":{
            "sessionUpdate":"tool_call",
            "toolCallId":"task-call-1",
            "title":"task",
            "kind":"other",
            "status":"in_progress",
            "rawInput":{"subagent_type":"explore","description":"Explore API","prompt":"Find handlers"}
        }}})
        .to_string(),
    );
    assert!(sub_start.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if i.id == "t1:tool:task-call-1"
                && matches!(
                    &i.kind,
                    TurnItemKind::Subagent {
                        tool_name,
                        description: Some(d),
                        subagent_type: Some(st),
                        background: false,
                        ..
                    } if tool_name == "task" && d == "Explore API" && st == "explore"
                )
    )));

    // Child tool routed via `_meta.parentToolUseId`.
    let child_tool = oc.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{
            "sessionId":root_sid,
            "_meta":{"parentToolUseId":"task-call-1"},
            "update":{
                "sessionUpdate":"tool_call",
                "toolCallId":"child-read-1",
                "title":"Read routes.rs",
                "kind":"read",
                "status":"completed"
            }
        }})
        .to_string(),
    );
    assert!(child_tool.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if i.parent_item_id.as_deref() == Some("t1:tool:task-call-1")
                && i.title.as_deref() == Some("Read routes.rs")
    )));
    assert!(child_tool.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if i.id == "t1:tool:task-call-1"
                && matches!(
                    &i.kind,
                    TurnItemKind::Subagent {
                        tool_uses: 1,
                        last_tool: Some(lt),
                        ..
                    } if lt == "Read routes.rs"
                )
    )));

    // Child message routed via distinct child `sessionId`.
    let child_msg = oc.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{
            "sessionId":"ses_child_999",
            "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Found 12 endpoints"}}
        }})
        .to_string(),
    );
    assert!(child_msg.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if i.parent_item_id.as_deref() == Some("t1:tool:task-call-1")
                && matches!(&i.kind, TurnItemKind::AssistantMessage { text, .. } if text == "Found 12 endpoints")
    )));

    // Synchronous `task` completes directly when its tool call finishes.
    let sub_end = oc.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":root_sid,"update":{
            "sessionUpdate":"tool_call_update","toolCallId":"task-call-1","status":"completed",
            "content":[{"type":"content","content":{"type":"text","text":"12 endpoints audited"}}]
        }}})
        .to_string(),
    );
    assert!(sub_end.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if i.id == "t1:tool:task-call-1"
                && i.status == TurnItemStatus::Completed
                && matches!(&i.kind, TurnItemKind::Subagent { output: Some(o), .. } if o == "12 endpoints audited")
    )));

    // 2. Grok async `spawn_subagent` (`Agent ID: grok-sub-7`) + `get_command_or_subagent_output` settlement.
    let mut grok = AcpDriver::grok();
    grok.set_sideband_root_dir("/tmp/test-grok-sub");
    let gsid = "02aa2a78-9d8f-4e1a-a67c-a40a96b34c33";
    grok.start(&launch_ctx("/workspace"));
    grok.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
    grok.on_line(&json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":gsid}}).to_string());
    grok.prompt("t1", &RuntimePromptReq::of("spawn")).unwrap();

    let g_launch = grok.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":gsid,"update":{
            "sessionUpdate":"tool_call",
            "toolCallId":"gsub-1",
            "title":"spawn_subagent",
            "kind":"other",
            "status":"completed",
            "rawInput":{"agent_type":"coder","description":"Grok Coder","prompt":"Write tests"},
            "content":[{"type":"content","content":{"type":"text","text":"Background subagent launched. Agent ID: grok-sub-7"}}]
        }}})
        .to_string(),
    );
    assert!(g_launch.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if i.id == "t1:tool:gsub-1" && i.status == TurnItemStatus::Running
    )));
    assert!(g_launch.events.iter().any(|e| matches!(
        e,
        DriverEvent::BackgroundTasks(tasks)
            if tasks.iter().any(|t| t.id == "grok-sub-7" && t.kind == "agent")
    )));

    let g_poll = grok.on_line(
        &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":gsid,"update":{
            "sessionUpdate":"tool_call",
            "toolCallId":"gpoll-1",
            "title":"get_command_or_subagent_output",
            "kind":"other",
            "status":"completed",
            "rawInput":{"id":"grok-sub-7"},
            "content":[{"type":"content","content":{"type":"text","text":"{\"status\":\"completed\",\"output\":\"All tests green\"}"}}]
        }}})
        .to_string(),
    );
    assert!(g_poll.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if i.id == "t1:tool:gsub-1"
                && i.status == TurnItemStatus::Completed
                && matches!(&i.kind, TurnItemKind::Subagent { output: Some(o), .. } if o == "All tests green")
    )));
    assert!(g_poll.events.iter().any(|e| matches!(
        e,
        DriverEvent::TurnItem(i)
            if matches!(
                &i.kind,
                TurnItemKind::SystemNotice { message, .. }
                    if message == "Background agent finished: Grok Coder"
            )
    )));
}
