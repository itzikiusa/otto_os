use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use otto_core::agent_runtime::{
    CreateRuntimeSessionReq, ProviderTurnStatus, RuntimeAccessMode, RuntimeEvent, RuntimeEventBody,
    RuntimePromptReq, RuntimeProvider, RuntimeSession, TurnItemKind, TurnItemStatus,
    TurnTerminalStatus,
};
use otto_core::domain::Workspace;
use otto_core::{Id, Result};
use otto_state::{AgentRuntimeRepo, DbPool};

use super::{AgentRuntime, RuntimeConfig};
use crate::acp::AcpDriver;
use crate::claude::ClaudeDriver;
use crate::codex::CodexDriver;
use crate::driver::{
    item, DriverError, DriverEvent, DriverFactory, DriverStep, LaunchContext, LaunchSpec,
    McpDelivery, McpServerDef, StructuredDriver,
};
use crate::host::RuntimeHost;
use crate::pi::PiDriver;

struct LibTestHost {
    root: String,
    events: Mutex<Vec<Arc<RuntimeEvent>>>,
    notify: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl RuntimeHost for LibTestHost {
    async fn workspace(&self, id: &Id) -> Result<Workspace> {
        Ok(Workspace {
            id: id.clone(),
            name: "W".into(),
            root_path: self.root.clone(),
            settings: serde_json::json!({}),
            archived: false,
            created_at: Utc::now(),
        })
    }

    async fn sandboxed(&self, _s: &RuntimeSession) -> bool {
        false
    }

    async fn prepare_spawn(
        &self,
        _ws: &Workspace,
        _s: &RuntimeSession,
        _spec: &mut LaunchSpec,
        _mcp: McpDelivery,
    ) -> Result<Vec<McpServerDef>> {
        Ok(Vec::new())
    }

    fn publish(&self, _s: &RuntimeSession, event: Arc<RuntimeEvent>) {
        self.events.lock().unwrap().push(event);
        self.notify.notify_waiters();
    }

    async fn retire(&self, _s: &RuntimeSession) {}
}

struct TimedShellDriver {
    turn_id: Option<String>,
    thought_text: String,
    msg_text: String,
}

impl StructuredDriver for TimedShellDriver {
    fn provider(&self) -> RuntimeProvider {
        RuntimeProvider::Claude
    }

    fn launch(&self, _ctx: &LaunchContext) -> LaunchSpec {
        // `/bin/sh` echoes ready, then reads each prompt line and emits timed
        // reasoning and message chunks separated by real elapsed time.
        let script = r#"
echo "READY"
while IFS= read -r _line; do
  echo "THINK:first "
  sleep 0.03
  echo "THINK:second "
  sleep 0.03
  echo "THINK:third"
  sleep 0.03
  echo "THINK_DONE"
  sleep 0.02
  echo "MSG:hello "
  sleep 0.03
  echo "MSG:world"
  sleep 0.03
  echo "MSG_DONE"
  echo "TURN_DONE"
done
"#;
        LaunchSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
            ..LaunchSpec::default()
        }
    }

    fn start(&mut self, _ctx: &LaunchContext) -> DriverStep {
        DriverStep::default()
    }

    fn prompt(
        &mut self,
        turn_id: &str,
        req: &RuntimePromptReq,
    ) -> std::result::Result<DriverStep, DriverError> {
        self.turn_id = Some(turn_id.to_string());
        self.thought_text.clear();
        self.msg_text.clear();
        let mut step = DriverStep::default();
        step.write(req.text.clone());
        Ok(step)
    }

    fn interrupt(&mut self) -> DriverStep {
        DriverStep::default()
    }

    fn respond(
        &mut self,
        request_id: &str,
        _decision: otto_core::agent_runtime::ApprovalDecision,
    ) -> std::result::Result<DriverStep, DriverError> {
        Err(DriverError(format!("unknown request {request_id}")))
    }

    fn on_line(&mut self, line: &str) -> DriverStep {
        let mut step = DriverStep::default();
        let tid = self.turn_id.clone().unwrap_or_default();
        if line == "READY" {
            step.push(DriverEvent::Ready {
                native_thread_id: Some("native-1".into()),
                model: Some("test-model".into()),
            });
        } else if let Some(chunk) = line.strip_prefix("THINK:") {
            self.thought_text.push_str(chunk);
            // Intentionally construct via `driver::item` with a fresh `Utc::now()`
            // on every chunk, mirroring real drivers before `upsert_item` normalizes.
            step.push(DriverEvent::TurnItem(item(
                format!("{tid}:think"),
                TurnItemStatus::Running,
                None,
                TurnItemKind::Reasoning {
                    text: self.thought_text.clone(),
                    streaming: true,
                },
            )));
        } else if line == "THINK_DONE" {
            step.push(DriverEvent::TurnItem(item(
                format!("{tid}:think"),
                TurnItemStatus::Completed,
                None,
                TurnItemKind::Reasoning {
                    text: self.thought_text.clone(),
                    streaming: false,
                },
            )));
        } else if let Some(chunk) = line.strip_prefix("MSG:") {
            self.msg_text.push_str(chunk);
            step.push(DriverEvent::TurnItem(item(
                format!("{tid}:msg"),
                TurnItemStatus::Running,
                None,
                TurnItemKind::AssistantMessage {
                    message_id: format!("{tid}:msg"),
                    text: self.msg_text.clone(),
                    streaming: true,
                },
            )));
        } else if line == "MSG_DONE" {
            step.push(DriverEvent::TurnItem(item(
                format!("{tid}:msg"),
                TurnItemStatus::Completed,
                None,
                TurnItemKind::AssistantMessage {
                    message_id: format!("{tid}:msg"),
                    text: self.msg_text.clone(),
                    streaming: false,
                },
            )));
        } else if line == "TURN_DONE" {
            self.turn_id = None;
            step.push(DriverEvent::Terminal {
                status: TurnTerminalStatus::Completed,
                failure: None,
                reason: None,
            });
        }
        step
    }

    fn on_exit(&mut self, _code: Option<i32>) -> DriverStep {
        DriverStep::default()
    }
}

async fn test_pool() -> DbPool {
    let opts = sqlx::sqlite::SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = sqlx::sqlite::SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .unwrap();
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .unwrap();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) \
         VALUES ('u1', 'u', 'x', 'U', 0, ?)",
    )
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, created_at) VALUES ('w1', 'W', '/tmp', ?)",
    )
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    pool.into()
}

#[tokio::test]
async fn streaming_and_coalesced_upserts_preserve_initial_started_at_across_completion() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap().to_string();
    let pool = test_pool().await;
    let host = Arc::new(LibTestHost {
        root: cwd.clone(),
        events: Mutex::new(Vec::new()),
        notify: tokio::sync::Notify::new(),
    });
    let drivers: Arc<dyn DriverFactory> = Arc::new(|_| -> Option<Box<dyn StructuredDriver>> {
        Some(Box::new(TimedShellDriver {
            turn_id: None,
            thought_text: String::new(),
            msg_text: String::new(),
        }))
    });
    let rt = AgentRuntime::new(
        AgentRuntimeRepo::new(pool),
        Arc::clone(&host) as Arc<dyn RuntimeHost>,
        drivers,
        RuntimeConfig {
            // Long coalesce window so intermediate streaming chunks go through `st.held`.
            stream_coalesce: Duration::from_secs(30),
            attachments_dir: dir.path().join("att"),
            ..RuntimeConfig::default()
        },
    );

    let session = rt
        .create(
            &"u1".into(),
            &"w1".into(),
            CreateRuntimeSessionReq {
                provider: Some(RuntimeProvider::Claude),
                cwd: Some(cwd),
                title: None,
                model: None,
                access_mode: Some(RuntimeAccessMode::FullAccess),
                meta: None,
            },
        )
        .await
        .unwrap();

    let resp = rt.prompt(&session.id, "run").await.unwrap();

    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            let done = host.events.lock().unwrap().iter().any(|e| {
                matches!(
                    &e.body,
                    RuntimeEventBody::TurnTerminal { turn_id, .. } if *turn_id == resp.turn_id
                )
            });
            if done {
                break;
            }
            host.notify.notified().await;
        }
    })
    .await
    .expect("timed out waiting for turn completion");

    let snap = rt.live(&session.id).await.unwrap().lock().proj.snapshot();
    let turn = snap
        .turns
        .iter()
        .find(|t| t.id == resp.turn_id)
        .expect("turn present");
    assert_eq!(turn.status, ProviderTurnStatus::Completed);

    let user = snap
        .turn_items
        .iter()
        .find(|i| matches!(i.kind, TurnItemKind::UserMessage { .. }))
        .expect("user message item present");
    assert_eq!(user.status, TurnItemStatus::Completed);
    assert_eq!(user.started_at, turn.started_at);
    assert_eq!(user.completed_at, user.started_at);

    let reasoning = snap
        .turn_items
        .iter()
        .find(|i| matches!(i.kind, TurnItemKind::Reasoning { .. }))
        .expect("reasoning item present");
    assert_eq!(reasoning.status, TurnItemStatus::Completed);
    let r_start = reasoning.started_at.expect("reasoning started_at");
    let r_end = reasoning.completed_at.expect("reasoning completed_at");
    assert!(
        r_end > r_start,
        "expected completed_at ({r_end}) > started_at ({r_start}) for multi-chunk reasoning"
    );
    assert!(
        (r_end - r_start).num_milliseconds() >= 40,
        "expected real elapsed thinking duration >= 40ms, got {}ms",
        (r_end - r_start).num_milliseconds()
    );

    let assistant = snap
        .turn_items
        .iter()
        .find(|i| matches!(i.kind, TurnItemKind::AssistantMessage { .. }))
        .expect("assistant message item present");
    assert_eq!(assistant.status, TurnItemStatus::Completed);
    let a_start = assistant.started_at.expect("assistant started_at");
    let a_end = assistant.completed_at.expect("assistant completed_at");
    assert!(
        a_end > a_start,
        "expected completed_at ({a_end}) > started_at ({a_start}) for multi-chunk assistant message"
    );
    assert!(
        a_start >= r_end,
        "assistant message started ({a_start}) after reasoning completed ({r_end})"
    );

    // Every emitted update for the reasoning item must carry the exact same `started_at`.
    let reasoning_starts: Vec<_> = host
        .events
        .lock()
        .unwrap()
        .iter()
        .filter_map(|e| match &e.body {
            RuntimeEventBody::TurnItemUpdated { turn_item } if turn_item.id == reasoning.id => {
                Some(turn_item.started_at)
            }
            _ => None,
        })
        .collect();
    assert!(reasoning_starts.len() >= 2);
    for seen in &reasoning_starts {
        assert_eq!(*seen, Some(r_start));
    }

    rt.close(&session.id).await.unwrap();
}

#[test]
fn drivers_preserve_block_and_item_start_timestamps_before_first_delta() {
    let ctx = LaunchContext {
        mcp_servers: Vec::new(),
        session_id: "s".into(),
        cwd: "/tmp".into(),
        model: None,
        resume: None,
        sandboxed: false,
        access_mode: RuntimeAccessMode::FullAccess,
    };

    // Codex summary-only reasoning (`item/started` with empty text, then `item/completed` with summary).
    let mut codex = CodexDriver::with_program("codex");
    let init = codex.start(&ctx);
    let init_id: serde_json::Value = serde_json::from_str(&init.writes[0]).unwrap();
    let thread = codex.on_line(&format!(
        r#"{{"jsonrpc":"2.0","id":{},"result":{{"userAgent":"codex"}}}}"#,
        init_id["id"]
    ));
    let thread_id: serde_json::Value = serde_json::from_str(&thread.writes[1]).unwrap();
    codex.on_line(&format!(
        r#"{{"jsonrpc":"2.0","id":{},"result":{{"thread":{{"id":"th1"}}}}}}"#,
        thread_id["id"]
    ));
    codex.prompt("t1", &RuntimePromptReq::of("hi")).unwrap();
    let started = codex.on_line(
        r#"{"jsonrpc":"2.0","method":"item/started","params":{"item":{"id":"r1","type":"reasoning"}}}"#,
    );
    assert!(started.events.is_empty());
    std::thread::sleep(Duration::from_millis(15));
    let finished = codex.on_line(
        r#"{"jsonrpc":"2.0","method":"item/completed","params":{"item":{"id":"r1","type":"reasoning","summary":["thought carefully"]}}}"#,
    );
    let Some(DriverEvent::TurnItem(codex_item)) = finished.events.first() else {
        panic!("expected reasoning TurnItem on item/completed");
    };
    assert!(codex_item.completed_at.unwrap() > codex_item.started_at.unwrap());

    // Claude `content_block_start` -> delay -> `thinking_delta` -> `content_block_stop`.
    let mut claude = ClaudeDriver::new();
    claude.prompt("t1", &RuntimePromptReq::of("hi")).unwrap();
    claude.on_line(
        r#"{"type":"stream_event","event":{"type":"message_start","message":{"id":"m1"}}}"#,
    );
    claude.on_line(
        r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"thinking"}}}"#,
    );
    std::thread::sleep(Duration::from_millis(15));
    let delta = claude.on_line(
        r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hmm"}}}"#,
    );
    std::thread::sleep(Duration::from_millis(15));
    let stop = claude
        .on_line(r#"{"type":"stream_event","event":{"type":"content_block_stop","index":0}}"#);
    let Some(DriverEvent::TurnItem(d_item)) = delta.events.first() else {
        panic!("expected delta TurnItem");
    };
    let Some(DriverEvent::TurnItem(s_item)) = stop.events.first() else {
        panic!("expected closed TurnItem");
    };
    assert_eq!(d_item.started_at, s_item.started_at);
    assert!(s_item.completed_at.unwrap() > s_item.started_at.unwrap());

    // ACP thought chunks -> message chunk closes thought preserving initial `started_at`.
    let mut acp = AcpDriver::gemini("gemini");
    let init = acp.start(&ctx);
    let init_id: serde_json::Value = serde_json::from_str(&init.writes[0]).unwrap();
    let new_sess = acp.on_line(&format!(
        r#"{{"jsonrpc":"2.0","id":{},"result":{{"protocolVersion":1}}}}"#,
        init_id["id"]
    ));
    let sess_id: serde_json::Value = serde_json::from_str(&new_sess.writes[0]).unwrap();
    acp.on_line(&format!(
        r#"{{"jsonrpc":"2.0","id":{},"result":{{"sessionId":"s1"}}}}"#,
        sess_id["id"]
    ));
    acp.prompt("t1", &RuntimePromptReq::of("hi")).unwrap();
    let c1 = acp.on_line(
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"a"}}}}"#,
    );
    std::thread::sleep(Duration::from_millis(15));
    let c2 = acp.on_line(
        r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"b"}}}}"#,
    );
    let Some(DriverEvent::TurnItem(acp_open)) = c1.events.first() else {
        panic!("expected open thought item");
    };
    let Some(DriverEvent::TurnItem(acp_closed)) = c2.events.first() else {
        panic!("expected closed thought item");
    };
    assert_eq!(acp_open.started_at, acp_closed.started_at);
    assert!(acp_closed.completed_at.unwrap() > acp_closed.started_at.unwrap());

    // Pi `thinking_start` -> delay -> `thinking_delta` -> `message_end`.
    let mut pi = PiDriver::with_program("pi");
    let st = pi.start(&ctx);
    let st_id: serde_json::Value = serde_json::from_str(&st.writes[0]).unwrap();
    pi.on_line(&format!(
        r#"{{"id":"{}","type":"response","command":"get_state","success":true,"data":{{"sessionId":"p1"}}}}"#,
        st_id["id"].as_str().unwrap()
    ));
    pi.prompt("t1", &RuntimePromptReq::of("hi")).unwrap();
    pi.on_line(r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_start"}}"#);
    std::thread::sleep(Duration::from_millis(15));
    let p_delta = pi.on_line(
        r#"{"type":"message_update","assistantMessageEvent":{"type":"thinking_delta","delta":"deep"}}"#,
    );
    let p_end = pi.on_line(r#"{"type":"message_end"}"#);
    let Some(DriverEvent::TurnItem(pi_open)) = p_delta.events.first() else {
        panic!("expected open pi thought");
    };
    let Some(DriverEvent::TurnItem(pi_closed)) = p_end.events.first() else {
        panic!("expected closed pi thought");
    };
    assert_eq!(pi_open.started_at, pi_closed.started_at);
    assert!(pi_closed.completed_at.unwrap() > pi_closed.started_at.unwrap());
}
