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
                title_auto: false,
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

#[test]
fn projection_streaming_ids_tracks_streaming_and_settled_items_in_o1() {
    let mut proj = super::Projection::default();
    for idx in 0..500u32 {
        let mut settled = item(
            format!("settled-{idx}"),
            TurnItemStatus::Completed,
            None,
            TurnItemKind::AssistantMessage {
                message_id: format!("m-{idx}"),
                text: "done".into(),
                streaming: false,
            },
        );
        settled.ordinal = idx + 1;
        proj.apply(
            &RuntimeEventBody::TurnItemUpdated { turn_item: settled },
            u64::from(idx + 1),
        );
    }
    assert!(proj.streaming_ids.is_empty());

    let mut live_a = item(
        "stream-a".into(),
        TurnItemStatus::Running,
        None,
        TurnItemKind::AssistantMessage {
            message_id: "ma".into(),
            text: "part 1".into(),
            streaming: true,
        },
    );
    live_a.ordinal = 501;
    let mut live_b = item(
        "stream-b".into(),
        TurnItemStatus::Running,
        None,
        TurnItemKind::Reasoning {
            text: "thinking".into(),
            streaming: true,
        },
    );
    live_b.ordinal = 502;
    proj.apply(
        &RuntimeEventBody::TurnItemUpdated {
            turn_item: live_a.clone(),
        },
        501,
    );
    proj.apply(
        &RuntimeEventBody::TurnItemUpdated { turn_item: live_b },
        502,
    );
    assert_eq!(proj.streaming_ids.len(), 2);
    assert!(proj.streaming_ids.contains("stream-a"));
    assert!(proj.streaming_ids.contains("stream-b"));

    // Settling `stream-a` removes it in O(1) while leaving `stream-b`.
    live_a.status = TurnItemStatus::Completed;
    if let TurnItemKind::AssistantMessage { streaming, .. } = &mut live_a.kind {
        *streaming = false;
    }
    proj.apply(
        &RuntimeEventBody::TurnItemUpdated { turn_item: live_a },
        503,
    );
    assert_eq!(proj.streaming_ids.len(), 1);
    assert!(proj.streaming_ids.contains("stream-b"));
}

#[tokio::test]
async fn holder_checkpoint_throttles_streaming_deltas_and_flushes_immediately_on_critical_events() {
    use crate::holder::{HolderClient, HolderEvent, LineStream, SequencedLine};

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
            turn_id: Some("t-held".into()),
            thought_text: String::new(),
            msg_text: String::new(),
        }))
    });
    let rt = AgentRuntime::new(
        AgentRuntimeRepo::new(pool),
        Arc::clone(&host) as Arc<dyn RuntimeHost>,
        drivers,
        RuntimeConfig {
            stream_coalesce: Duration::ZERO,
            attachments_dir: dir.path().join("att"),
            ..RuntimeConfig::default()
        },
    );
    let sid: Id = "held-throttle-test".into();
    rt.inner
        .repo
        .create(otto_state::NewRuntimeSession {
            id: sid.clone(),
            workspace_id: "w1".into(),
            provider: RuntimeProvider::Claude,
            title: "Held".into(),
            cwd,
            model: None,
            access_mode: RuntimeAccessMode::FullAccess,
            created_by: "u1".into(),
            parent_id: None,
            meta: None,
        })
        .await
        .unwrap();

    let live = rt.live(&sid).await.unwrap();
    let (client, cp_rx) = HolderClient::for_test();
    let (ev_tx, ev_rx) = tokio::sync::mpsc::channel(64);
    let epoch = {
        let mut st = live.lock();
        st.epoch = 1;
        st.turn = Some(super::ActiveTurn {
            id: "t-held".into(),
            origin: otto_core::agent_runtime::TurnOrigin::User,
            interrupting: false,
            stalled: false,
        });
        st.proc = Some(super::Proc {
            epoch: 1,
            request_ids: super::RequestIds::new(),
            pid: 99_999,
            transport: super::ProcTransport::Held {
                client,
                acked_seq: 0,
                claude_login: None,
                last_checkpoint_at: None,
                checkpoint_dirty: false,
                checkpoint_writes: 0,
            },
            driver: Box::new(TimedShellDriver {
                turn_id: Some("t-held".into()),
                thought_text: String::new(),
                msg_text: String::new(),
            }),
        });
        1
    };
    let pump_handle = tokio::spawn(super::held_proc::pump_held(
        Arc::clone(&rt.inner),
        Arc::clone(&live),
        sid.clone(),
        epoch,
        ev_rx,
        None,
    ));

    // Push 15 rapid streaming delta lines back-to-back (< 250ms window).
    for seq in 1..=15u64 {
        ev_tx
            .send(HolderEvent::Line(SequencedLine {
                seq,
                stream: LineStream::Out,
                line: format!("MSG:chunk-{seq} "),
                overlong_bytes: None,
            }))
            .await
            .unwrap();
    }
    tokio::time::sleep(Duration::from_millis(40)).await;
    {
        let st = live.lock();
        assert_eq!(st.proj.streaming_ids.len(), 1);
        let Some(super::Proc {
            transport:
                super::ProcTransport::Held {
                    acked_seq,
                    checkpoint_dirty,
                    checkpoint_writes,
                    ..
                },
            ..
        }) = st.proc.as_ref()
        else {
            panic!("expected Held proc");
        };
        assert_eq!(*acked_seq, 15);
        assert_eq!(
            *checkpoint_writes, 1,
            "first delta writes once; subsequent deltas within 250ms coalesce without duplicate writes"
        );
        assert!(*checkpoint_dirty, "trailing deltas mark checkpoint dirty");
    }

    // After the 250ms throttle window elapses, `pump_held`'s timer flushes the dirty checkpoint.
    tokio::time::sleep(Duration::from_millis(260)).await;
    let writes_after_timer = {
        let st = live.lock();
        let Some(super::Proc {
            transport:
                super::ProcTransport::Held {
                    checkpoint_dirty,
                    checkpoint_writes,
                    ..
                },
            ..
        }) = st.proc.as_ref()
        else {
            panic!("expected Held proc");
        };
        assert!(!*checkpoint_dirty, "timer flushed dirty checkpoint");
        assert_eq!(*checkpoint_writes, 2);
        assert_eq!(cp_rx.borrow().as_ref().unwrap().acked_seq, 15);
        assert_eq!(cp_rx.borrow().as_ref().unwrap().streaming_items.len(), 1);
        *checkpoint_writes
    };

    // Critical transition (`TURN_DONE` -> `DriverEvent::Terminal`) bypasses throttle immediately!
    ev_tx
        .send(HolderEvent::Line(SequencedLine {
            seq: 16,
            stream: LineStream::Out,
            line: "MSG_DONE".into(),
            overlong_bytes: None,
        }))
        .await
        .unwrap();
    ev_tx
        .send(HolderEvent::Line(SequencedLine {
            seq: 17,
            stream: LineStream::Out,
            line: "TURN_DONE".into(),
            overlong_bytes: None,
        }))
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_millis(30)).await;
    {
        let st = live.lock();
        assert!(st.proj.streaming_ids.is_empty());
        let Some(super::Proc {
            transport:
                super::ProcTransport::Held {
                    checkpoint_dirty,
                    checkpoint_writes,
                    ..
                },
            ..
        }) = st.proc.as_ref()
        else {
            panic!("expected Held proc");
        };
        assert!(
            *checkpoint_writes > writes_after_timer,
            "TurnTerminal flushes an immediate unthrottled checkpoint"
        );
        assert!(!*checkpoint_dirty);
        assert_eq!(cp_rx.borrow().as_ref().unwrap().acked_seq, 17);
        assert!(cp_rx.borrow().as_ref().unwrap().streaming_items.is_empty());
    }

    drop(ev_tx);
    let _ = pump_handle.await;
    rt.close(&sid).await.unwrap();
}

struct FailingExitDriver;

impl StructuredDriver for FailingExitDriver {
    fn provider(&self) -> RuntimeProvider {
        RuntimeProvider::Codex
    }

    fn launch(&self, _ctx: &LaunchContext) -> LaunchSpec {
        LaunchSpec {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "echo READY; read -r _l; echo boom >&2; exit 3".into(),
            ],
            ..LaunchSpec::default()
        }
    }

    fn start(&mut self, _ctx: &LaunchContext) -> DriverStep {
        DriverStep::default()
    }

    fn prompt(
        &mut self,
        _turn_id: &str,
        req: &RuntimePromptReq,
    ) -> std::result::Result<DriverStep, DriverError> {
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
        if line == "READY" {
            step.push(DriverEvent::Ready {
                native_thread_id: Some("f-native".into()),
                model: None,
            });
        }
        step
    }

    fn on_exit(&mut self, _code: Option<i32>) -> DriverStep {
        DriverStep::default()
    }
}

#[tokio::test]
async fn closed_and_failed_sessions_evict_from_live_map_after_persistence() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap().to_string();
    let pool = test_pool().await;
    let host = Arc::new(LibTestHost {
        root: cwd.clone(),
        events: Mutex::new(Vec::new()),
        notify: tokio::sync::Notify::new(),
    });
    let drivers: Arc<dyn DriverFactory> = Arc::new(|p| -> Option<Box<dyn StructuredDriver>> {
        match p {
            RuntimeProvider::Codex => Some(Box::new(FailingExitDriver)),
            _ => Some(Box::new(TimedShellDriver {
                turn_id: None,
                thought_text: String::new(),
                msg_text: String::new(),
            })),
        }
    });
    let rt = AgentRuntime::new(
        AgentRuntimeRepo::new(pool),
        Arc::clone(&host) as Arc<dyn RuntimeHost>,
        drivers,
        RuntimeConfig {
            close_grace: Duration::from_millis(20),
            attachments_dir: dir.path().join("att"),
            ..RuntimeConfig::default()
        },
    );

    // 1. Closed session evicts from `rt.inner.live` and stays clean across history reads.
    let s_closed = rt
        .create(
            &"u1".into(),
            &"w1".into(),
            CreateRuntimeSessionReq {
                title_auto: false,
                provider: Some(RuntimeProvider::Claude),
                cwd: Some(cwd.clone()),
                title: None,
                model: None,
                access_mode: Some(RuntimeAccessMode::FullAccess),
                meta: None,
            },
        )
        .await
        .unwrap();
    assert!(rt.inner.live.contains_key(&s_closed.id));
    rt.close(&s_closed.id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while rt.inner.live.contains_key(&s_closed.id) {
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    })
    .await
    .expect("closed session must be evicted from AgentRuntime::live");

    let _ = rt.replay(&s_closed.id, 0).await.unwrap();
    assert!(
        !rt.inner.live.contains_key(&s_closed.id),
        "reading closed session history must not re-pin it in AgentRuntime::live"
    );

    // 2. Failed session whose child exits non-zero evicts once persisted and tasks exit.
    let s_failed = rt
        .create(
            &"u1".into(),
            &"w1".into(),
            CreateRuntimeSessionReq {
                title_auto: false,
                provider: Some(RuntimeProvider::Codex),
                cwd: Some(cwd),
                title: None,
                model: None,
                access_mode: Some(RuntimeAccessMode::FullAccess),
                meta: None,
            },
        )
        .await
        .unwrap();
    rt.prompt(&s_failed.id, "crash-now").await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while rt.inner.live.contains_key(&s_failed.id) {
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    })
    .await
    .expect("failed session must be evicted from AgentRuntime::live after persistence");
    assert_eq!(
        rt.get(&s_failed.id).await.unwrap().status,
        otto_core::agent_runtime::RuntimeSessionStatus::Dormant
    );
}

/// T3 Code #39049: a released thread keeps nothing loaded once its agent
/// process is gone (projection, replay ring, persister); a read loads it again.
#[tokio::test]
async fn released_sessions_leave_the_live_map_once_their_process_exits() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().to_str().unwrap().to_string();
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
        AgentRuntimeRepo::new(test_pool().await),
        Arc::clone(&host) as Arc<dyn RuntimeHost>,
        drivers,
        RuntimeConfig {
            close_grace: Duration::from_millis(20),
            attachments_dir: dir.path().join("att"),
            ..RuntimeConfig::default()
        },
    );
    let s = rt
        .create(
            &"u1".into(),
            &"w1".into(),
            CreateRuntimeSessionReq {
                title_auto: false,
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
    rt.prompt(&s.id, "hi").await.unwrap();
    tokio::time::timeout(Duration::from_secs(3), async {
        while !host
            .events
            .lock()
            .unwrap()
            .iter()
            .any(|e| matches!(e.body, RuntimeEventBody::TurnTerminal { .. }))
        {
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    })
    .await
    .expect("the turn ends");

    let later = Utc::now() + chrono::TimeDelta::minutes(31);
    assert_eq!(
        rt.release_idle_processes(30, later).await,
        vec![s.id.clone()]
    );
    tokio::time::timeout(Duration::from_secs(3), async {
        while rt.inner.live.contains_key(&s.id) {
            tokio::time::sleep(Duration::from_millis(15)).await;
        }
    })
    .await
    .expect("a released session must leave AgentRuntime::live once its process exits");
    assert_eq!(
        rt.get(&s.id).await.unwrap().status,
        otto_core::agent_runtime::RuntimeSessionStatus::Dormant,
        "released, not closed"
    );
}

#[tokio::test]
async fn prompt_validates_attachment_magic_bytes_and_rejects_dash_prefixed_models() {
    use otto_core::agent_runtime::{PromptFile, PromptImage};

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
            attachments_dir: dir.path().join("att"),
            ..RuntimeConfig::default()
        },
    );

    let mk_req = |model: Option<&str>| CreateRuntimeSessionReq {
        title_auto: false,
        provider: Some(RuntimeProvider::Claude),
        cwd: Some(cwd.clone()),
        title: None,
        model: model.map(str::to_string),
        access_mode: Some(RuntimeAccessMode::FullAccess),
        meta: None,
    };
    let bad_create = rt
        .create(
            &"u1".into(),
            &"w1".into(),
            mk_req(Some("-oProxyCommand=evil")),
        )
        .await;
    assert!(matches!(bad_create, Err(otto_core::Error::Invalid(_))));

    let s = rt
        .create(&"u1".into(), &"w1".into(), mk_req(None))
        .await
        .unwrap();

    // Dash-prefixed model rejected on prompt.
    let mut dash_model = RuntimePromptReq::of("hello");
    dash_model.model = Some("--evil-flag".into());
    assert!(matches!(
        rt.prompt_with(&s.id, &dash_model).await,
        Err(otto_core::Error::Invalid(_))
    ));

    // SVG rejected on prompt (`UnsupportedMedia`).
    let mut svg_req = RuntimePromptReq::of("svg");
    svg_req.images.push(PromptImage {
        mime: "image/svg+xml".into(),
        data_base64: "PHN2Zz48L3N2Zz4=".into(),
        name: None,
    });
    assert!(matches!(
        rt.prompt_with(&s.id, &svg_req).await,
        Err(otto_core::Error::UnsupportedMedia(_))
    ));
    // Spoofed PNG bytes ("test") rejected by magic-byte check.
    let mut spoof_png = RuntimePromptReq::of("png");
    spoof_png.images.push(PromptImage {
        mime: "image/png".into(),
        data_base64: "dGVzdA==".into(),
        name: None,
    });
    assert!(matches!(
        rt.prompt_with(&s.id, &spoof_png).await,
        Err(otto_core::Error::Invalid(_))
    ));
    // Spoofed PDF bytes ("test") rejected by magic-byte check.
    let mut spoof_pdf = RuntimePromptReq::of("pdf");
    spoof_pdf.files.push(PromptFile {
        path: None,
        name: "doc.pdf".into(),
        mime: "application/pdf".into(),
        data_base64: "dGVzdA==".into(),
    });
    assert!(matches!(
        rt.prompt_with(&s.id, &spoof_pdf).await,
        Err(otto_core::Error::Invalid(_))
    ));

    rt.close(&s.id).await.unwrap();
}
