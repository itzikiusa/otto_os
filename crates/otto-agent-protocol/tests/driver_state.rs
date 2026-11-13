//! Driver state round trips (`export_state` → a fresh driver's
//! `import_state`): what a new daemon restores when it adopts a child that
//! outlived the old one. Mid-stream continuity against the real fake agents
//! is covered by `held_restart.rs`; pi has no fake, so its turn is replayed
//! here line by line.

use otto_agent_protocol::acp::AcpDriver;
use otto_agent_protocol::claude::ClaudeDriver;
use otto_agent_protocol::codex::CodexDriver;
use otto_agent_protocol::pi::PiDriver;
use otto_agent_protocol::{DriverEvent, LaunchContext, McpServerDef, StructuredDriver};
use otto_core::agent_runtime::{
    RuntimeAccessMode, RuntimePromptReq, TurnItemKind, TurnTerminalStatus,
};

fn ctx() -> LaunchContext {
    LaunchContext {
        mcp_servers: vec![McpServerDef {
            name: "otto".into(),
            command: "/bin/otto".into(),
            args: vec!["mcp".into()],
            env: vec![("T".into(), "1".into())],
        }],
        session_id: "s".into(),
        cwd: "/w".into(),
        model: Some("m".into()),
        resume: Some("r".into()),
        sandboxed: false,
        access_mode: RuntimeAccessMode::ApprovalRequired,
    }
}

type Make = fn() -> Box<dyn StructuredDriver>;

/// Every driver the daemon builds, fresh (the factory's view).
fn all() -> Vec<(&'static str, Make)> {
    vec![
        ("claude", || Box::new(ClaudeDriver::new())),
        ("codex", || Box::new(CodexDriver::with_program("codex"))),
        ("gemini", || Box::new(AcpDriver::gemini("gemini"))),
        ("agy", || Box::new(AcpDriver::agy("agy"))),
        ("grok", || Box::new(AcpDriver::grok())),
        ("opencode", || Box::new(AcpDriver::opencode())),
        ("pi", || Box::new(PiDriver::with_program("pi"))),
    ]
}

#[test]
fn every_driver_round_trips_its_state_and_keeps_its_launch_shape() {
    for (name, make) in all() {
        let mut d = make();
        let launch = d.launch(&ctx());
        let start = d.start(&ctx());
        assert!(!start.writes.is_empty(), "{name}: the handshake writes");
        let state = d
            .export_state()
            .unwrap_or_else(|| panic!("{name}: exportable"));
        assert!(
            !state.to_string().contains("/bin/otto"),
            "{name}: MCP servers (their credentials) are never exported"
        );
        let mut restored = make();
        restored.import_state(state.clone()).unwrap();
        assert_eq!(
            restored.export_state(),
            Some(state),
            "{name}: identical state"
        );
        assert_eq!(
            restored.launch(&ctx()),
            launch,
            "{name}: same provider config"
        );
        // Garbage is refused, never half-applied.
        assert!(
            make().import_state(serde_json::json!({"x": 1})).is_err(),
            "{name}"
        );
    }
}

fn sent(step: &otto_agent_protocol::DriverStep, i: usize) -> serde_json::Value {
    serde_json::from_str(&step.writes[i]).unwrap()
}

/// A pi turn interrupted by a "restart" after its first delta finishes in
/// the restored driver exactly as in the original.
#[test]
fn a_pi_turn_continues_in_a_restored_driver() {
    let mut d = PiDriver::with_program("pi");
    let ctx = LaunchContext {
        resume: None,
        access_mode: RuntimeAccessMode::FullAccess,
        ..ctx()
    };
    let s = d.start(&ctx);
    let st = sent(&s, 0);
    d.on_line(&format!(
        r#"{{"id":"{}","type":"response","command":"get_state","success":true,"data":{{"sessionId":"sess-1","model":{{"id":"gpt-5.5","provider":"openai-codex"}}}}}}"#,
        st["id"].as_str().unwrap()
    ));
    let s = d.prompt("t1", &RuntimePromptReq::of("hi")).unwrap();
    let p = sent(&s, s.writes.len() - 1);
    d.on_line(&format!(
        r#"{{"id":"{}","type":"response","command":"prompt","success":true,"data":{{"disposition":"started"}}}}"#,
        p["id"].as_str().unwrap()
    ));
    d.on_line(
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"Hel"}}"#,
    );

    let mut r = PiDriver::with_program("pi");
    r.import_state(d.export_state().unwrap()).unwrap();
    let rest = [
        r#"{"type":"message_update","assistantMessageEvent":{"type":"text_delta","delta":"lo"}}"#,
        r#"{"type":"tool_execution_start","toolCallId":"c1","toolName":"bash","args":{"command":"ls"}}"#,
    ];
    let mut texts = Vec::new();
    for line in rest {
        for ev in r.on_line(line).events {
            if let DriverEvent::TurnItem(i) = ev {
                if let TurnItemKind::AssistantMessage { text, .. } = i.kind {
                    texts.push(text);
                }
            }
        }
    }
    assert_eq!(texts.last().map(String::as_str), Some("Hello"), "{texts:?}");
    let end = r.on_line(
        r#"{"type":"agent_end","messages":[{"role":"assistant","stopReason":"stop"}],"willRetry":false}"#,
    );
    assert!(matches!(
        &end.events[0],
        DriverEvent::Terminal {
            status: TurnTerminalStatus::Completed,
            ..
        }
    ));
    // The next request id continues, never reusing one the child saw.
    let next = r.prompt("t2", &RuntimePromptReq::of("again")).unwrap();
    assert_ne!(sent(&next, next.writes.len() - 1)["id"], p["id"]);
}
