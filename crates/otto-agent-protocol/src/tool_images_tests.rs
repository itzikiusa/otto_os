//! Real provider frames through the public driver boundary.
use crate::{
    acp::AcpDriver, claude::ClaudeDriver, codex::CodexDriver, DriverEvent, DriverStep,
    LaunchContext, StructuredDriver,
};
use otto_core::agent_runtime::{RuntimeAccessMode, RuntimePromptReq, TurnItemKind};
use serde_json::{json, Value};

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+a5V8AAAAASUVORK5CYII=";
fn ctx() -> LaunchContext {
    LaunchContext {
        mcp_servers: vec![],
        session_id: "s".into(),
        cwd: "/w".into(),
        model: None,
        resume: None,
        sandboxed: false,
        access_mode: RuntimeAccessMode::default(),
    }
}
fn feed(d: &mut dyn StructuredDriver, v: Value) -> DriverStep {
    d.on_line(&v.to_string())
}
fn assert_images(step: &DriverStep, count: usize, text: &str) {
    let item = step
        .events
        .iter()
        .find_map(|e| match e {
            DriverEvent::TurnItem(item)
                if matches!(item.kind, TurnItemKind::DynamicTool { .. }) =>
            {
                Some(item)
            }
            _ => None,
        })
        .expect("dynamic tool event");
    let TurnItemKind::DynamicTool { images, output, .. } = &item.kind else {
        unreachable!()
    };
    assert_eq!(images.len(), count);
    assert!(
        output.as_deref().unwrap_or_default().contains(text),
        "{output:?}"
    );
    assert!(!output.as_deref().unwrap_or_default().contains(PNG));
    // Serialized projections used by both live WS and persisted snapshots retain images.
    let encoded = serde_json::to_value(item).unwrap();
    let roundtrip: otto_core::agent_runtime::TurnItem = serde_json::from_value(encoded).unwrap();
    assert_eq!(*item, roundtrip);
}

#[test]
fn claude_mcp_mixed_result_preserves_inline_images() {
    let mut d = ClaudeDriver::new();
    let init = d.start(&ctx());
    let request: Value = serde_json::from_str(&init.writes[0]).unwrap();
    feed(
        &mut d,
        json!({"type":"control_response","response":{"subtype":"success","request_id":request["request_id"],"response":{}}}),
    );
    d.prompt("t1", &RuntimePromptReq::of("screenshot")).unwrap();
    feed(
        &mut d,
        json!({"type":"assistant","message":{"id":"m1","content":[{"type":"tool_use","id":"tool1","name":"mcp__preview__screenshot","input":{}}]}}),
    );
    let s = feed(
        &mut d,
        json!({"type":"user","message":{"content":[{"type":"tool_result","tool_use_id":"tool1","content":[{"type":"text","text":"Captured"},{"type":"image","source":{"type":"base64","media_type":"image/png","data":PNG}}]}]}}),
    );
    assert_images(&s, 1, "Captured");
}

#[test]
fn codex_mcp_result_preserves_images_and_metadata_but_rejects_invalid_images() {
    let mut d = CodexDriver::with_program("codex");
    d.start(&ctx());
    feed(&mut d, json!({"id":1,"result":{"userAgent":"fake"}}));
    feed(
        &mut d,
        json!({"id":2,"result":{"thread":{"id":"th1"},"model":"fake"}}),
    );
    d.prompt("t1", &RuntimePromptReq::of("screenshot")).unwrap();
    let s = feed(
        &mut d,
        json!({"method":"item/completed","params":{"threadId":"th1","turnId":"turn1","item":{
        "type":"mcpToolCall","id":"tool1","server":"preview","tool":"screenshot","arguments":{},"status":"completed","result":{
            "content":[{"type":"text","text":"Captured"},{"type":"image","mimeType":"image/png","data":PNG},{"type":"image","mimeType":"image/png","data":PNG},{"type":"image","mimeType":"image/svg+xml","data":PNG},{"type":"image","mimeType":"image/png","data":"!!!!"}],"structuredContent":{"width":1}
        }}}}),
    );
    assert_images(&s, 2, "Captured");
    assert_images(&s, 2, "\"width\":1");
    assert_images(&s, 2, "Image omitted");
}

#[test]
fn acp_opencode_images_replace_on_update_and_raw_attachments_are_supported() {
    let mut d = AcpDriver::opencode();
    d.start(&ctx());
    feed(
        &mut d,
        json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}),
    );
    feed(
        &mut d,
        json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}),
    );
    d.prompt("t1", &RuntimePromptReq::of("screenshot")).unwrap();
    let s = feed(
        &mut d,
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{
        "sessionUpdate":"tool_call","toolCallId":"tool1","title":"Screenshot","kind":"other","status":"in_progress","content":[
            {"type":"content","content":{"type":"text","text":"Captured"}},
            {"type":"content","content":{"type":"image","mimeType":"image/png","data":PNG}}]}}}),
    );
    assert_images(&s, 1, "Captured");
    let s = feed(
        &mut d,
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{
        "sessionUpdate":"tool_call_update","toolCallId":"tool1","status":"completed","content":[{"type":"content","content":{"type":"text","text":"Replaced"}}]}}}),
    );
    assert_images(&s, 0, "Replaced");
    let s = feed(
        &mut d,
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{
        "sessionUpdate":"tool_call_update","toolCallId":"tool1","rawOutput":{"output":"Screenshot attachment","attachments":[{"type":"file","mime":"image/png","url":format!("data:image/png;base64,{PNG}")}]}}}}),
    );
    assert_images(&s, 1, "Screenshot attachment");
}

/// MCP links, embedded resources and audio read as what they are (no base64
/// in the text), for Codex and for ACP agents — ACP used to drop them.
#[test]
fn mcp_links_resources_and_audio_read_as_words_for_codex_and_acp() {
    let blocks = json!([
        {"type":"resource_link","uri":"https://x/report.pdf","name":"report.pdf","description":"The report"},
        {"type":"resource","resource":{"uri":"file:///notes.md","mimeType":"text/markdown","text":"# Notes"}},
        {"type":"resource","resource":{"uri":"file:///a.zip","mimeType":"application/zip","blob":"UEsDBAoAAAAAAA=="}},
        {"type":"audio","mimeType":"audio/wav","data":"UklGRiQAAABXQVZF"}
    ]);
    let mut d = CodexDriver::with_program("codex");
    d.start(&ctx());
    feed(&mut d, json!({"id":1,"result":{"userAgent":"fake"}}));
    feed(
        &mut d,
        json!({"id":2,"result":{"thread":{"id":"th1"},"model":"fake"}}),
    );
    d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
    let s = feed(
        &mut d,
        json!({"method":"item/completed","params":{"threadId":"th1","turnId":"turn1","item":{
        "type":"mcpToolCall","id":"tool1","server":"probe","tool":"probe_link","arguments":{},"status":"completed","result":{"content":blocks}}}}),
    );
    for want in [
        "Link: report.pdf — https://x/report.pdf (The report)",
        "file:///notes.md:\n# Notes",
        "Resource: file:///a.zip (application/zip",
        "Audio (audio/wav",
    ] {
        assert_images(&s, 0, want);
    }
    let mut a = AcpDriver::opencode();
    a.start(&ctx());
    feed(
        &mut a,
        json!({"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}),
    );
    feed(
        &mut a,
        json!({"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}),
    );
    a.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
    let content: Vec<Value> = blocks
        .as_array()
        .unwrap()
        .iter()
        .map(|b| json!({"type":"content","content":b}))
        .collect();
    let s = feed(
        &mut a,
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{
        "sessionUpdate":"tool_call","toolCallId":"tool1","title":"probe_link","kind":"other","status":"completed","content":content}}}),
    );
    assert_images(&s, 0, "Link: report.pdf — https://x/report.pdf");
    // An MCP result handed over as rawOutput only (no content) is read too.
    let s = feed(
        &mut a,
        json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{
        "sessionUpdate":"tool_call","toolCallId":"tool2","title":"probe_echo","kind":"other","status":"completed",
        "rawOutput":{"content":[{"type":"text","text":"probe says: hi"}],"isError":false}}}}),
    );
    assert_images(&s, 0, "probe says: hi");
}

#[test]
fn legacy_dynamic_tool_needs_no_images_field() {
    let kind: TurnItemKind = serde_json::from_value(
        json!({"type":"dynamic_tool","tool_name":"legacy","input":{},"output":"text"}),
    )
    .unwrap();
    let TurnItemKind::DynamicTool { images, .. } = kind else {
        unreachable!()
    };
    assert!(images.is_empty());
}
