//! Round 5 7-provider subagent `parent_item_id` routing, `completed_at`,
//! `TaskSettlement`, `BackgroundTask.due_at`, and pre-handshake checks.

#![allow(clippy::disallowed_methods)]

use chrono::{Duration as ChronoDuration, Utc};
use otto_agent_protocol::acp::AcpDriver;
use otto_agent_protocol::claude::ClaudeDriver;
use otto_agent_protocol::codex::CodexDriver;
use otto_agent_protocol::driver::{
    ActiveTailChunk, ActiveTailKind, DriverEvent, LaunchContext, SidebandPayload, StructuredDriver,
};
use otto_agent_protocol::pi::PiDriver;
use otto_core::agent_runtime::{
    PromptImage, RuntimeAccessMode, RuntimePromptReq, TurnItem, TurnItemKind, TurnItemStatus,
};
use serde_json::{json, Value};

const VALID_PNG_B64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

fn launch_ctx(cwd: &str) -> LaunchContext {
    LaunchContext {
        mcp_servers: Vec::new(),
        session_id: "otto-r5-drv".into(),
        cwd: cwd.into(),
        model: None,
        resume: None,
        sandboxed: false,
        access_mode: RuntimeAccessMode::FullAccess,
    }
}

fn collect_items(events: &[DriverEvent]) -> Vec<TurnItem> {
    events
        .iter()
        .filter_map(|e| match e {
            DriverEvent::TurnItem(i) => Some(i.clone()),
            _ => None,
        })
        .collect()
}

fn acp_handshake_and_prompt(d: &mut AcpDriver, cwd: &str, sid: &str) -> u64 {
    d.start(&launch_ctx(cwd));
    d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
    d.on_line(&json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":sid}}).to_string());
    let p = d.prompt("t1", &RuntimePromptReq::of("audit")).unwrap();
    p.writes
        .iter()
        .find_map(|w| {
            let v: Value = serde_json::from_str(w).ok()?;
            (v.get("method")?.as_str()? == "session/prompt").then(|| v.get("id")?.as_u64())?
        })
        .expect("session/prompt rpc id")
}

#[test]
fn claude_codex_and_pi_emit_subagent_child_parent_ids_completed_at_and_settlements() {
    // 1. ClaudeDriver
    let mut claude = ClaudeDriver::new();
    claude.start(&launch_ctx("/w"));
    claude.on_line(&json!({"type":"system","subtype":"init","session_id":"c1"}).to_string());
    claude.prompt("t1", &RuntimePromptReq::of("audit")).unwrap();
    let s1 = claude.on_line(
        &json!({
            "type":"assistant",
            "message":{"id":"m1","content":[{"type":"tool_use","id":"tu_ag","name":"Agent","input":{"description":"Audit auth","subagent_type":"Explore","prompt":"Check","run_in_background":true}}]},
            "parent_tool_use_id":null
        })
        .to_string(),
    );
    assert!(collect_items(&s1.events)
        .iter()
        .any(|i| i.id == "t1:tool:tu_ag" && matches!(i.kind, TurnItemKind::Subagent { .. })));
    claude.on_line(
        &json!({
            "type":"system","subtype":"task_started","task_id":"bg_ag_1","tool_use_id":"tu_ag","task_type":"local_agent","is_backgrounded":true
        })
        .to_string(),
    );
    let s_child = claude.on_line(
        &json!({
            "type":"assistant",
            "message":{"id":"m_sub","content":[{"type":"tool_use","id":"sub_r1","name":"Read","input":{"file_path":"/w/lib.rs"}}]},
            "parent_tool_use_id":"tu_ag"
        })
        .to_string(),
    );
    assert!(collect_items(&s_child.events)
        .iter()
        .any(|i| i.parent_item_id.as_deref() == Some("t1:tool:tu_ag")));
    let s_done = claude.on_line(
        &json!({
            "type":"system","subtype":"task_notification","task_id":"bg_ag_1","tool_use_id":"tu_ag","status":"completed","summary":"Clean"
        })
        .to_string(),
    );
    let claude_items = collect_items(&s_done.events);
    assert!(claude_items.iter().any(|i| {
        i.id == "t1:tool:tu_ag" && i.status == TurnItemStatus::Completed && i.completed_at.is_some()
    }));
    assert!(claude_items.iter().any(|i| matches!(
        &i.kind,
        TurnItemKind::SystemNotice {
            task_settlement: Some(ts),
            ..
        } if ts.task_id == "bg_ag_1" && ts.kind == "agent" && ts.status == "completed"
    )));

    // 2. CodexDriver
    let mut codex = CodexDriver::with_program("codex");
    codex.start(&launch_ctx("/w"));
    codex.on_line(&json!({"id":1,"result":{}}).to_string());
    codex.on_line(&json!({"id":2,"result":{"thread":{"id":"th1"}}}).to_string());
    let cp = codex.prompt("t1", &RuntimePromptReq::of("audit")).unwrap();
    let req_id = serde_json::from_str::<Value>(&cp.writes[0]).unwrap()["id"].clone();
    codex.on_line(
        &json!({"id":req_id,"result":{"turn":{"id":"ct-root","items":[],"status":"inProgress"}}})
            .to_string(),
    );
    codex.on_line(
        &json!({
            "method":"item/started",
            "params":{"threadId":"th1","turnId":"ct-root","item":{"type":"collabAgentToolCall","id":"col-1","tool":"spawnAgent","role":"reviewer","prompt":"Inspect","status":"inProgress"}}
        })
        .to_string(),
    );
    codex.on_line(
        &json!({
            "method":"item/completed",
            "params":{"threadId":"th1","turnId":"ct-root","item":{"type":"collabAgentToolCall","id":"col-1","tool":"spawnAgent","role":"reviewer","prompt":"Inspect","receiverThreadIds":["th-sub-1"],"status":"completed"}}
        })
        .to_string(),
    );
    let cx_child = codex.on_line(
        &json!({
            "method":"item/started",
            "params":{"threadId":"th-sub-1","turnId":"ct-sub","item":{"type":"commandExecution","id":"c-sub","command":"rg auth","status":"inProgress"}}
        })
        .to_string(),
    );
    assert!(collect_items(&cx_child.events)
        .iter()
        .any(|i| i.parent_item_id.as_deref() == Some("t1:codex:col-1")));
    let cx_done = codex.on_line(
        &json!({
            "method":"turn/completed",
            "params":{"threadId":"th-sub-1","turn":{"id":"ct-sub","status":"completed"}}
        })
        .to_string(),
    );
    let cx_items = collect_items(&cx_done.events);
    assert!(cx_items.iter().any(|i| {
        i.id == "t1:codex:col-1"
            && i.status == TurnItemStatus::Completed
            && i.completed_at.is_some()
    }));
    assert!(cx_items.iter().any(|i| matches!(
        &i.kind,
        TurnItemKind::SystemNotice {
            task_settlement: Some(ts),
            ..
        } if ts.kind == "agent" && ts.status == "completed"
    )));

    // 3. PiDriver
    let mut pi = PiDriver::with_program("pi");
    pi.start(&launch_ctx("/w"));
    pi.on_line(
        &json!({"id":"init-state","type":"response","command":"get_state","success":true,"data":{"sessionId":"pi-1"}})
            .to_string(),
    );
    pi.prompt("t1", &RuntimePromptReq::of("audit")).unwrap();
    let pi_sub = pi.on_line(
        &json!({
            "type":"tool_execution_start","toolCallId":"pi_sub_1","toolName":"subagent","args":{"description":"Audit pi","prompt":"Check"}
        })
        .to_string(),
    );
    assert!(collect_items(&pi_sub.events)
        .iter()
        .any(|i| i.id == "t1:tool:pi_sub_1" && matches!(i.kind, TurnItemKind::Subagent { .. })));
    let pi_child = pi.on_line(
        &json!({
            "type":"tool_execution_start","toolCallId":"pi_r1","toolName":"read","args":{"path":"/w/pi.rs"}
        })
        .to_string(),
    );
    let pi_child_items = collect_items(&pi_child.events);
    assert!(pi_child_items.iter().any(|i| {
        i.id == "t1:tool:pi_r1" && i.parent_item_id.as_deref() == Some("t1:tool:pi_sub_1")
    }));
    assert!(pi_child_items
        .iter()
        .any(|i| matches!(&i.kind, TurnItemKind::Subagent { tool_uses: 1, .. })));
    pi.on_line(
        &json!({
            "type":"tool_execution_end","toolCallId":"pi_r1","toolName":"read","result":{"content":[{"type":"text","text":"ok"}]},"isError":false
        })
        .to_string(),
    );
    let pi_end = pi.on_line(
        &json!({
            "type":"tool_execution_end","toolCallId":"pi_sub_1","toolName":"subagent","result":{"content":[{"type":"text","text":"done"}]},"isError":false
        })
        .to_string(),
    );
    assert!(collect_items(&pi_end.events)
        .iter()
        .any(|i| i.id == "t1:tool:pi_sub_1" && i.completed_at.is_some()));
}

#[test]
fn agy_gemini_opencode_and_grok_emit_subagent_children_timers_and_prehandshake_checks() {
    // 4. AcpDriver::gemini pre-handshake image check + codebase_investigator subagent
    let mut gemini = AcpDriver::gemini("gemini");
    gemini.start(&launch_ctx("/w"));
    let mut img_req = RuntimePromptReq::of("look");
    img_req.images.push(PromptImage {
        mime: "image/png".into(),
        data_base64: VALID_PNG_B64.into(),
        name: Some("shot.png".into()),
    });
    let pre_err = gemini.prompt("t0", &img_req).unwrap_err();
    assert!(
        pre_err.0.contains("does not accept images"),
        "{}",
        pre_err.0
    );

    let mut gemini = AcpDriver::gemini("gemini");
    acp_handshake_and_prompt(&mut gemini, "/w", "gem-12345678");
    let g_sub = gemini.on_line(
        &json!({
            "jsonrpc":"2.0","method":"session/update",
            "params":{"sessionId":"gem-12345678","update":{"sessionUpdate":"tool_call","toolCallId":"g_inv","title":"codebase_investigator","status":"in_progress","rawInput":{"objective":"Trace auth"}}}
        })
        .to_string(),
    );
    assert!(collect_items(&g_sub.events)
        .iter()
        .any(|i| i.id == "t1:tool:g_inv" && matches!(i.kind, TurnItemKind::Subagent { .. })));
    let g_child = gemini.on_line(
        &json!({
            "jsonrpc":"2.0","method":"session/update",
            "params":{"sessionId":"gem-12345678","update":{"sessionUpdate":"tool_call","toolCallId":"g_read","title":"read_file","kind":"read","status":"completed","_meta":{"parentToolCallId":"g_inv"}}}
        })
        .to_string(),
    );
    assert!(collect_items(&g_child.events)
        .iter()
        .any(|i| i.parent_item_id.as_deref() == Some("t1:tool:g_inv")));
    let g_done = gemini.on_line(
        &json!({
            "jsonrpc":"2.0","method":"session/update",
            "params":{"sessionId":"gem-12345678","update":{"sessionUpdate":"tool_call_update","toolCallId":"g_inv","status":"completed","rawOutput":"Traced"}}
        })
        .to_string(),
    );
    assert!(collect_items(&g_done.events)
        .iter()
        .any(|i| i.id == "t1:tool:g_inv" && i.completed_at.is_some()));

    // 5. AcpDriver::agy subagent + schedule due_at + unrelated sender guard
    let mut agy = AcpDriver::agy("agy");
    let pid = acp_handshake_and_prompt(&mut agy, "/w", "11111111-2222-3333-4444-555555555555");
    agy.on_line(
        &json!({
            "jsonrpc":"2.0","method":"session/update",
            "params":{"sessionId":"11111111-2222-3333-4444-555555555555","update":{
                "sessionUpdate":"tool_call","toolCallId":"agy_sub","title":"invoke_subagent","status":"completed",
                "rawInput":{"TypeName":"research","Role":"Auditor","Prompt":"Inspect"},
                "rawOutput":{"conversationId":"aaaa1111-bbbb-2222-cccc-333333333333"}
            }}
        })
        .to_string(),
    );
    let sched = agy.on_line(
        &json!({
            "jsonrpc":"2.0","method":"session/update",
            "params":{"sessionId":"11111111-2222-3333-4444-555555555555","update":{
                "sessionUpdate":"tool_call","toolCallId":"sched_1","title":"default_api:schedule","status":"completed",
                "rawInput":{"DurationSeconds":45,"Prompt":"Check build"}
            }}
        })
        .to_string(),
    );
    assert!(sched.events.iter().any(|e| matches!(
        e,
        DriverEvent::BackgroundTasks(tasks)
            if tasks.iter().any(|t| t.id == "schedule:sched_1" && t.due_at.is_some())
    )));
    let tail_step = agy.on_sideband(SidebandPayload::Batch {
        primary: None,
        tails: vec![ActiveTailChunk {
            task_id: "aaaa1111-bbbb-2222-cccc-333333333333".into(),
            kind: ActiveTailKind::AgySubagentTranscript,
            using_fallback: false,
            next_offset: 200,
            bytes: format!(
                "{}\n",
                json!({"step_index":1,"type":"PLANNER_RESPONSE","tool_calls":[{"name":"read_file","args":{"Path":"/w/a.rs"}}]})
            )
            .into_bytes(),
        }],
        timer_tick: false,
    });
    assert!(collect_items(&tail_step.events)
        .iter()
        .any(|i| i.parent_item_id.as_deref() == Some("t1:tool:agy_sub")));
    agy.on_line(&json!({"jsonrpc":"2.0","id":pid,"result":{"stopReason":"end_turn"}}).to_string());

    let post_idle = (Utc::now() + ChronoDuration::seconds(3)).to_rfc3339();
    let peer_msg = format!(
        "{}\n",
        json!({
            "step_index":2,"type":"SYSTEM_MESSAGE","source":"SYSTEM","created_at":post_idle,
            "content":"<SYSTEM_MESSAGE>\n[Message] sender=unrelated-peer content=hello\n</SYSTEM_MESSAGE>"
        })
    );
    let peer_step = agy.on_sideband(SidebandPayload::AppendLines {
        resolved_path: None,
        using_fallback: false,
        next_offset: 300,
        bytes: peer_msg.into_bytes(),
    });
    assert!(!collect_items(&peer_step.events).iter().any(|i| matches!(
        &i.kind,
        TurnItemKind::SystemNotice {
            task_settlement: Some(ts),
            ..
        } if ts.task_id == "schedule:sched_1"
    )));

    let sub_wakeup = format!(
        "{}\n",
        json!({
            "step_index":3,"type":"SYSTEM_MESSAGE","source":"SYSTEM","created_at":post_idle,
            "content":"<SYSTEM_MESSAGE>\n[Message] sender=aaaa1111-bbbb-2222-cccc-333333333333 content=Report ready\n</SYSTEM_MESSAGE>"
        })
    );
    let sub_done = agy.on_sideband(SidebandPayload::AppendLines {
        resolved_path: None,
        using_fallback: false,
        next_offset: 600,
        bytes: sub_wakeup.into_bytes(),
    });
    assert!(collect_items(&sub_done.events).iter().any(|i| matches!(
        &i.kind,
        TurnItemKind::SystemNotice {
            task_settlement: Some(ts),
            ..
        } if ts.task_id == "aaaa1111-bbbb-2222-cccc-333333333333" && ts.status == "completed"
    )));

    // 6. AcpDriver::opencode child session SQLite rows -> parent_item_id
    let mut opencode = AcpDriver::opencode();
    acp_handshake_and_prompt(&mut opencode, "/w", "ses_root_1");
    opencode.on_line(
        &json!({
            "jsonrpc":"2.0","method":"session/update",
            "params":{"sessionId":"ses_root_1","update":{"sessionUpdate":"tool_call","toolCallId":"oc_t1","title":"task","status":"in_progress","rawInput":{"description":"Explore","subagent_type":"explore","prompt":"Find"}}}
        })
        .to_string(),
    );
    let oc_row = format!(
        "{}\n",
        json!({
            "id":"prt_c1","session_id":"ses_child_1","message_id":"m_c1",
            "time_updated": Utc::now().timestamp_millis() + 5_000,
            "role":"assistant",
            "data":{"type":"tool","tool":"bash","callID":"c_bash_1","state":{"status":"completed","input":{"command":"ls"},"output":"src"}}
        })
    );
    let oc_sb = opencode.on_sideband(SidebandPayload::OpencodeSqlite {
        stamp: (1, 64, 0, 0),
        rows_jsonl: oc_row.into_bytes(),
    });
    assert!(collect_items(&oc_sb.events)
        .iter()
        .any(|i| i.parent_item_id.as_deref() == Some("t1:tool:oc_t1")));

    // 7. AcpDriver::grok child session JSONL -> parent_item_id
    let mut grok = AcpDriver::grok();
    acp_handshake_and_prompt(&mut grok, "/w", "grok_root_1");
    grok.on_line(
        &json!({
            "jsonrpc":"2.0","method":"session/update",
            "params":{"sessionId":"grok_root_1","update":{"sessionUpdate":"tool_call","toolCallId":"gk_s1","title":"spawn_subagent","status":"in_progress","rawInput":{"description":"Scan","prompt":"Check"}}}
        })
        .to_string(),
    );
    let gk_line = format!(
        "{}\n",
        json!({
            "sessionId":"grok_child_1","parent_session_id":"grok_root_1",
            "update":{"sessionUpdate":"tool_call","toolCallId":"gk_c1","title":"read_file","status":"completed","rawInput":{"path":"/w/lib.rs"}}
        })
    );
    let gk_sb = grok.on_sideband(SidebandPayload::AppendLines {
        resolved_path: None,
        using_fallback: false,
        next_offset: gk_line.len() as u64,
        bytes: gk_line.into_bytes(),
    });
    assert!(collect_items(&gk_sb.events)
        .iter()
        .any(|i| i.parent_item_id.as_deref() == Some("t1:tool:gk_s1")));
}
