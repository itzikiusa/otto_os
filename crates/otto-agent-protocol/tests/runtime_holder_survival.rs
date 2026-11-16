//! End-to-end integration test for detached `otto-runtime-holder` survival
//! across `AgentRuntime::shutdown_for_restart` and `recover_and_adopt`.

#![allow(clippy::disallowed_methods)]

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use otto_agent_protocol::claude::ClaudeDriver;
use otto_agent_protocol::holder::{list_holder_sockets, HolderLauncher};
use otto_agent_protocol::{
    AgentRuntime, DriverFactory, LaunchSpec, McpDelivery, McpServerDef, RuntimeConfig, RuntimeHost,
    StructuredDriver,
};
use otto_core::agent_runtime::{
    CreateRuntimeSessionReq, ProviderSessionStatus, ProviderTurnStatus, RuntimeEvent,
    RuntimeEventBody, RuntimeProvider, RuntimeReplay, RuntimeSession, TurnItemKind, TurnItemStatus,
    TurnTerminalStatus,
};
use otto_core::domain::Workspace;
use otto_core::{Id, Result};
use otto_state::{AgentRuntimeRepo, DbPool};

macro_rules! fake_bin {
    ($name:literal) => {
        std::env::var(concat!("NEXTEST_BIN_EXE_", $name))
            .unwrap_or_else(|_| env!(concat!("CARGO_BIN_EXE_", $name)).to_string())
    };
}

const WAIT: Duration = Duration::from_secs(15);

struct TestHost {
    root: String,
    events: Mutex<Vec<Arc<RuntimeEvent>>>,
    notify: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl RuntimeHost for TestHost {
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
        spec: &mut LaunchSpec,
        _mcp: McpDelivery,
    ) -> Result<Vec<McpServerDef>> {
        spec.env.push(("OTTO_PREPARED".into(), "1".into()));
        Ok(Vec::new())
    }

    fn publish(&self, _s: &RuntimeSession, event: Arc<RuntimeEvent>) {
        self.events.lock().unwrap().push(event);
        self.notify.notify_waiters();
    }
}

impl TestHost {
    fn new(root: &Path) -> Arc<Self> {
        Arc::new(Self {
            root: root.to_string_lossy().into_owned(),
            events: Mutex::default(),
            notify: tokio::sync::Notify::new(),
        })
    }

    fn all(&self) -> Vec<RuntimeEvent> {
        self.events
            .lock()
            .unwrap()
            .iter()
            .map(|e| (**e).clone())
            .collect()
    }

    async fn wait(&self, pred: impl Fn(&RuntimeEvent) -> bool) -> RuntimeEvent {
        let deadline = tokio::time::Instant::now() + WAIT;
        loop {
            let notified = self.notify.notified();
            tokio::pin!(notified);
            notified.as_mut().enable();
            if let Some(hit) = self.all().into_iter().find(|e| pred(e)) {
                return hit;
            }
            if tokio::time::timeout_at(deadline, notified).await.is_err() {
                panic!(
                    "timed out waiting for runtime event; seen: {:#?}",
                    self.all()
                );
            }
        }
    }
}

fn drivers() -> Arc<dyn DriverFactory> {
    Arc::new(|p: RuntimeProvider| -> Option<Box<dyn StructuredDriver>> {
        match p {
            RuntimeProvider::Claude => Some(Box::new(ClaudeDriver::with_program(fake_bin!(
                "otto-fake-claude"
            )))),
            _ => None,
        }
    })
}

async fn open_db(path: &Path) -> DbPool {
    let pool = otto_state::open(path).await.unwrap();
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        "INSERT OR IGNORE INTO users (id, username, password_hash, display_name, is_root, created_at) \
         VALUES ('u1', 'u', 'x', 'U', 0, ?)",
    )
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    sqlx::query(
        "INSERT OR IGNORE INTO workspaces (id, name, root_path, created_at) \
         VALUES ('w1', 'W', '/tmp', ?)",
    )
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    pool
}

fn build_held_runtime(pool: &DbPool, host: &Arc<TestHost>, holders_dir: &Path) -> AgentRuntime {
    let config = RuntimeConfig {
        attachments_dir: Path::new(&host.root).join("attachments"),
        holders_dir: Some(holders_dir.to_path_buf()),
        holder_launcher: Some(HolderLauncher::for_bin(fake_bin!("otto-runtime-holder"))),
        ..RuntimeConfig::default()
    };
    AgentRuntime::new(
        AgentRuntimeRepo::new(pool.clone()),
        Arc::clone(host) as Arc<dyn RuntimeHost>,
        drivers(),
        config,
    )
}

async fn wait_file(path: &Path) {
    let deadline = tokio::time::Instant::now() + WAIT;
    while !path.exists() {
        assert!(
            tokio::time::Instant::now() < deadline,
            "timed out waiting for {}",
            path.display()
        );
        tokio::time::sleep(Duration::from_millis(15)).await;
    }
}

fn pid_is_alive(pid: u32) -> bool {
    // SAFETY: signal 0 probes process existence without delivering a signal.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[tokio::test]
async fn runtime_holder_survives_daemon_restart_streams_gap_output_and_runs_next_turn() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let holders_dir = dir.path().join("runtime-holders");
    let gate_dir = dir.path().join("gates");
    std::fs::create_dir_all(&gate_dir).unwrap();

    // Daemon generation 1: spawn held Claude session and begin a multi-phase streaming turn.
    let pool1 = open_db(&db_path).await;
    let host1 = TestHost::new(dir.path());
    let rt1 = build_held_runtime(&pool1, &host1, &holders_dir);

    let session = rt1
        .create(
            &"u1".into(),
            &"w1".into(),
            CreateRuntimeSessionReq {
                title_auto: false,
                provider: Some(RuntimeProvider::Claude),
                cwd: Some(dir.path().to_string_lossy().into_owned()),
                title: Some("Surviving Thread".into()),
                model: None,
                access_mode: None,
                meta: None,
            },
        )
        .await
        .unwrap();

    host1
        .wait(|e| {
            matches!(
                &e.body,
                RuntimeEventBody::ProviderSessionUpdated { provider_session }
                    if provider_session.status == ProviderSessionStatus::Ready
            )
        })
        .await;

    let turn1 = rt1
        .prompt(
            &session.id,
            format!("gated {}", gate_dir.display()).as_str(),
        )
        .await
        .unwrap();

    wait_file(&gate_dir.join("phase1_ready")).await;
    host1
        .wait(|e| {
            matches!(
                &e.body,
                RuntimeEventBody::TurnItemUpdated { turn_item }
                    if matches!(
                        &turn_item.kind,
                        TurnItemKind::AssistantMessage { text, streaming: true, .. }
                            if text == "alpha beta "
                    )
            )
        })
        .await;

    let pid1: u32 = std::fs::read_to_string(gate_dir.join("pid_turn1"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert!(
        pid_is_alive(pid1),
        "child process must be running before restart"
    );

    // Graceful daemon restart: flush checkpoint + detach without killing child or holder.
    rt1.shutdown_for_restart().await;
    drop(rt1);
    drop(pool1);

    assert!(
        pid_is_alive(pid1),
        "child CLI process must remain alive while daemon is stopped"
    );
    assert_eq!(
        list_holder_sockets(&holders_dir).len(),
        1,
        "detached runtime-holder socket must remain on disk across restart"
    );

    // Daemon-down gap: emit streaming output while NO AgentRuntime is attached.
    std::fs::write(gate_dir.join("emit_gap"), b"1").unwrap();
    wait_file(&gate_dir.join("gap_emitted")).await;
    // Give runtime-holder a brief moment to ingest the stdout line into its ring.
    tokio::time::sleep(Duration::from_millis(40)).await;

    // Daemon generation 2: open state DB, adopt live holders, and replay gap output.
    let pool2 = open_db(&db_path).await;
    let host2 = TestHost::new(dir.path());
    let rt2 = build_held_runtime(&pool2, &host2, &holders_dir);

    let adopted = rt2.recover_and_adopt().await.unwrap();
    assert!(
        adopted.contains(&session.id),
        "recover_and_adopt must report the live held session id for MCP token preservation"
    );
    assert!(
        pid_is_alive(pid1),
        "child CLI process must still have the same PID after adoption"
    );

    // Let the in-flight turn emit its final delta and finish on the adopted runtime.
    std::fs::write(gate_dir.join("finish_turn"), b"1").unwrap();
    let fin1 = host2
        .wait(|e| matches!(&e.body, RuntimeEventBody::TurnTerminal { .. }))
        .await;
    assert!(
        matches!(
            &fin1.body,
            RuntimeEventBody::TurnTerminal {
                turn_id,
                status: TurnTerminalStatus::Completed,
                ..
            } if turn_id == &turn1.turn_id
        ),
        "in-flight turn must finish Completed rather than Interrupted: {fin1:#?}"
    );

    // Send a second turn on the adopted runtime and verify the exact same child PID handles it.
    let pid2_file = gate_dir.join("pid_turn2");
    let turn2 = rt2
        .prompt(&session.id, format!("pid {}", pid2_file.display()).as_str())
        .await
        .unwrap();
    let fin2 = host2
        .wait(|e| {
            matches!(
                &e.body,
                RuntimeEventBody::TurnTerminal { turn_id, .. } if turn_id == &turn2.turn_id
            )
        })
        .await;
    assert!(matches!(
        &fin2.body,
        RuntimeEventBody::TurnTerminal {
            status: TurnTerminalStatus::Completed,
            ..
        }
    ));

    let pid2: u32 = std::fs::read_to_string(&pid2_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    assert_eq!(
        pid1, pid2,
        "second turn after restart must execute on the original live child process"
    );

    rt2.flush(&session.id).await;
    let RuntimeReplay::Snapshot { snapshot, .. } = rt2.replay(&session.id, 0).await.unwrap() else {
        panic!("expected full snapshot from seq 0");
    };
    assert_eq!(
        snapshot.turns.iter().map(|t| t.status).collect::<Vec<_>>(),
        vec![ProviderTurnStatus::Completed, ProviderTurnStatus::Completed],
        "both pre-restart and post-restart turns must be Completed"
    );

    let assistant_texts: Vec<String> = snapshot
        .turn_items
        .iter()
        .filter_map(|item| match (&item.status, &item.kind) {
            (
                TurnItemStatus::Completed,
                TurnItemKind::AssistantMessage {
                    text,
                    streaming: false,
                    ..
                },
            ) => Some(text.clone()),
            _ => None,
        })
        .collect();
    assert_eq!(
        assistant_texts,
        vec![
            "alpha beta gamma delta".to_string(),
            "second-turn-ok".to_string(),
        ],
        "pre-restart, daemon-down gap, and post-adopt output must assemble with zero loss"
    );

    // Explicit shutdown terminates the holder + child and removes the socket.
    rt2.shutdown().await;
    for _ in 0..50 {
        if !pid_is_alive(pid1) && list_holder_sockets(&holders_dir).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(
        list_holder_sockets(&holders_dir).is_empty(),
        "explicit runtime.shutdown() must unlink holder sockets"
    );
}

#[tokio::test]
async fn agy_sideband_tails_autonomous_wakeup_live_and_deduplicates_burst() {
    use otto_agent_protocol::acp::AcpDriver;
    use otto_core::agent_runtime::{RuntimeAccessMode, TurnOrigin};

    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("ws");
    let brain_dir = tmp.path().join("brain");
    let holders_dir = tmp.path().join("holders");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&brain_dir).unwrap();
    std::fs::create_dir_all(&holders_dir).unwrap();

    // Fake `agy acp` script:
    // 1. Answers initialize + session/new ("conv-e2e-1").
    // 2. On Prompt 1: writes USER_INPUT + PLANNER_RESPONSE to transcript_full.jsonl,
    //    emits AgentMessageChunk("Turn 1 done.") + PromptResponse, then stays
    //    COMPLETELY SILENT on stdout while writing an autonomous wakeup
    //    (SYSTEM_MESSAGE + PLANNER_RESPONSE + GENERIC + final PLANNER_RESPONSE)
    //    only to transcript_full.jsonl!
    // 3. On Prompt 2 ("status?"): dumps the buffered autonomous session/update
    //    frames in a burst BEFORE emitting Turn 2's real message ("Status green.").
    let script = tmp.path().join("fake_agy.py");
    std::fs::write(
        &script,
        r#"#!/usr/bin/env python3
import json, os, sys, time

brain = sys.argv[1]
sid = "conv-e2e-1"
log_dir = os.path.join(brain, sid, ".system_generated", "logs")
os.makedirs(log_dir, exist_ok=True)
tpath = os.path.join(log_dir, "transcript_full.jsonl")

def append_step(obj):
    with open(tpath, "a", encoding="utf-8") as f:
        f.write(json.dumps(obj) + "\n")
        f.flush()

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

prompt_num = 0
for raw in sys.stdin:
    raw = raw.strip()
    if not raw:
        continue
    msg = json.loads(raw)
    mid = msg.get("id")
    method = msg.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": mid, "result": {"protocolVersion": 1, "agentCapabilities": {"loadSession": True}}})
    elif method == "session/new":
        send({"jsonrpc": "2.0", "id": mid, "result": {"sessionId": sid, "modes": {"currentModeId": "default", "availableModes": [{"id": "default"}]}}})
    elif method == "session/prompt":
        prompt_num += 1
        if prompt_num == 1:
            append_step({"step_index": 0, "source": "USER_EXPLICIT", "type": "USER_INPUT", "content": "go"})
            append_step({"step_index": 1, "source": "MODEL", "type": "PLANNER_RESPONSE", "content": "Turn 1 done."})
            send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": sid, "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Turn 1 done."}}}})
            send({"jsonrpc": "2.0", "id": mid, "result": {"stopReason": "end_turn"}})
            time.sleep(0.15)
            append_step({"step_index": 2, "source": "SYSTEM", "type": "SYSTEM_MESSAGE", "content": "Subagent finished"})
            append_step({"step_index": 3, "source": "MODEL", "type": "PLANNER_RESPONSE", "thinking": "Inspecting subagent report.", "tool_calls": [{"name": "run_command", "args": {"CommandLine": "\"echo ok\"", "toolSummary": "\"Verify build\""}}]})
            time.sleep(0.10)
            append_step({"step_index": 4, "source": "MODEL", "type": "GENERIC", "status": "DONE", "content": "ok\n"})
            append_step({"step_index": 5, "source": "MODEL", "type": "PLANNER_RESPONSE", "thinking": "Verified.", "content": "Autonomous work finished live!"})
        else:
            # Flush buffered between-prompt notifications first (exact agy acp bug):
            for up in [
                {"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "Inspecting subagent report."}},
                {"sessionUpdate": "tool_call", "toolCallId": "call_buf_99", "title": "Verify build", "kind": "execute", "status": "in_progress"},
                {"sessionUpdate": "tool_call_update", "toolCallId": "call_buf_99", "status": "completed"},
                {"sessionUpdate": "agent_thought_chunk", "content": {"type": "text", "text": "Verified."}},
                {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Autonomous work finished live!"}},
            ]:
                send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": sid, "update": up}})
            append_step({"step_index": 6, "source": "USER_EXPLICIT", "type": "USER_INPUT", "content": "status?"})
            append_step({"step_index": 7, "source": "MODEL", "type": "PLANNER_RESPONSE", "content": "Status green."})
            send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": sid, "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": "Status green."}}}})
            send({"jsonrpc": "2.0", "id": mid, "result": {"stopReason": "end_turn"}})
"#,
    )
    .unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let pool = open_db(&tmp.path().join("state.db")).await;
    let host = TestHost::new(&root);
    let script_str = script.display().to_string();
    let brain_str = brain_dir.display().to_string();
    let agy_drivers: Arc<dyn DriverFactory> = Arc::new(
        move |p: RuntimeProvider| -> Option<Box<dyn StructuredDriver>> {
            if p != RuntimeProvider::Agy {
                return None;
            }
            let mut d = AcpDriver::new(
                RuntimeProvider::Agy,
                "python3",
                vec![script_str.clone(), brain_str.clone()],
            );
            d.set_agy_brain_dir(brain_str.clone());
            Some(Box::new(d))
        },
    );

    let cfg = RuntimeConfig {
        holders_dir: Some(holders_dir.clone()),
        holder_launcher: Some(HolderLauncher::for_bin(fake_bin!("otto-runtime-holder"))),
        autonomous_settle: Duration::from_millis(250),
        stream_coalesce: Duration::ZERO,
        ..RuntimeConfig::default()
    };
    let rt = AgentRuntime::new(
        AgentRuntimeRepo::new(pool.clone()),
        Arc::clone(&host) as Arc<dyn RuntimeHost>,
        Arc::clone(&agy_drivers),
        cfg.clone(),
    );

    let session = rt
        .create(
            &"u1".to_string(),
            &"w1".to_string(),
            CreateRuntimeSessionReq {
                title_auto: false,
                provider: Some(RuntimeProvider::Agy),
                title: Some("agy-tail".into()),
                cwd: Some(root.display().to_string()),
                model: None,
                access_mode: Some(RuntimeAccessMode::FullAccess),
                meta: None,
            },
        )
        .await
        .unwrap();

    // Prompt 1 completes normally.
    let p1 = rt.prompt(&session.id, "go").await.unwrap();
    host.wait(|ev| {
        matches!(
            &ev.body,
            RuntimeEventBody::TurnTerminal {
                turn_id,
                status: TurnTerminalStatus::Completed,
                ..
            } if turn_id == &p1.turn_id
        )
    })
    .await;

    // Without ANY second prompt or stdout from the child, the runtime tails
    // transcript_full.jsonl, streams the autonomous turn, and settles it!
    host.wait(|ev| {
        matches!(
            &ev.body,
            RuntimeEventBody::TurnTerminal {
                turn_id,
                status: TurnTerminalStatus::Completed,
                ..
            } if turn_id != &p1.turn_id
        )
    })
    .await;

    // Prompt 2 triggers the buffered burst on stdout, which must be absorbed!
    let p2 = rt.prompt(&session.id, "status?").await.unwrap();
    host.wait(|ev| {
        matches!(
            &ev.body,
            RuntimeEventBody::TurnTerminal {
                turn_id,
                status: TurnTerminalStatus::Completed,
                ..
            } if turn_id == &p2.turn_id
        )
    })
    .await;

    rt.flush(&session.id).await;
    rt.shutdown_for_restart().await;
    drop(rt);

    let rt2 = AgentRuntime::new(
        AgentRuntimeRepo::new(pool.clone()),
        Arc::clone(&host) as Arc<dyn RuntimeHost>,
        agy_drivers,
        cfg,
    );
    let adopted = rt2.recover_and_adopt().await.unwrap();
    assert!(adopted.contains(&session.id));

    let RuntimeReplay::Snapshot { snapshot, .. } = rt2.replay(&session.id, 0).await.unwrap() else {
        panic!("expected snapshot after adopt");
    };
    assert_eq!(
        snapshot.turns.len(),
        3,
        "expected User + Agent + User turns"
    );
    assert_eq!(snapshot.turns[0].origin, TurnOrigin::User);
    assert_eq!(snapshot.turns[1].origin, TurnOrigin::Agent);
    assert_eq!(snapshot.turns[2].origin, TurnOrigin::User);

    let msgs_by_turn = |tid: &str| -> Vec<String> {
        snapshot
            .turn_items
            .iter()
            .filter(|i| i.turn_id.as_deref() == Some(tid))
            .filter_map(|i| match &i.kind {
                TurnItemKind::AssistantMessage { text, .. } => Some(text.clone()),
                _ => None,
            })
            .collect()
    };
    assert_eq!(msgs_by_turn(&snapshot.turns[0].id), vec!["Turn 1 done."]);
    assert_eq!(
        msgs_by_turn(&snapshot.turns[1].id),
        vec!["Autonomous work finished live!"]
    );
    assert_eq!(
        msgs_by_turn(&snapshot.turns[2].id),
        vec!["Status green."],
        "Turn 2 must NOT contain duplicated burst messages from Turn 1's autonomous gap"
    );

    let turn2_tools = snapshot
        .turn_items
        .iter()
        .filter(|i| {
            i.turn_id.as_deref() == Some(snapshot.turns[2].id.as_str())
                && matches!(i.kind, TurnItemKind::CommandExecution { .. })
        })
        .count();
    assert_eq!(
        turn2_tools, 0,
        "Turn 2 must NOT contain duplicated burst tool calls"
    );

    rt2.shutdown().await;
}

#[tokio::test]
async fn grok_gemini_opencode_sideband_and_coalesce_flush_live_without_prompt() {
    use otto_agent_protocol::acp::AcpDriver;
    use otto_core::agent_runtime::{ProviderTurn, RuntimeAccessMode, TurnItem, TurnOrigin};

    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("workspace");
    let holders_dir = dir.path().join("holders");
    let grok_home = dir.path().join("grok-home");
    let gemini_home = dir.path().join("gemini-home");
    let opencode_dir = dir.path().join("opencode-data");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::create_dir_all(&holders_dir).unwrap();
    std::fs::create_dir_all(&grok_home).unwrap();
    std::fs::create_dir_all(&gemini_home).unwrap();
    std::fs::create_dir_all(&opencode_dir).unwrap();

    let script = dir.path().join("mock_multi_acp.py");
    std::fs::write(
        &script,
        r#"import json, os, sqlite3, sys, time, urllib.parse
from datetime import datetime, timezone, timedelta

mode = sys.argv[1]
home = sys.argv[2]
cwd = sys.argv[3]
sid = {
    "grok": "02aa2a78-9d8f-4e1a-a67c-a40a96b34c33",
    "gemini": "a753ef73-62fb-46e4-a2b6-b973e8f278b1",
    "opencode": "ses_262ad3c69ffeB7b1E1bC997L5s",
}[mode]
prompt_count = 0

def send(obj):
    sys.stdout.write(json.dumps(obj) + "\n")
    sys.stdout.flush()

for raw in sys.stdin:
    line = raw.strip()
    if not line:
        continue
    msg = json.loads(line)
    mid = msg.get("id")
    method = msg.get("method")
    if method == "initialize":
        send({"jsonrpc": "2.0", "id": mid, "result": {"protocolVersion": 1, "agentCapabilities": {"loadSession": True}}})
    elif method in ("session/new", "session/load"):
        send({"jsonrpc": "2.0", "id": mid, "result": {"sessionId": sid}})
    elif method in ("session/set_mode", "session/set_model", "session/set_config_option"):
        send({"jsonrpc": "2.0", "id": mid, "result": {}})
    elif method == "session/prompt":
        prompt_count += 1
        if prompt_count == 1:
            send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": sid, "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": f"{mode} turn 1 ok"}
            }}})
            send({"jsonrpc": "2.0", "id": mid, "result": {"stopReason": "end_turn"}})
            time.sleep(0.12)
            now_ms = int(time.time() * 1000) + 500
            iso_now = (datetime.now(timezone.utc) + timedelta(milliseconds=500)).isoformat()
            if mode == "grok":
                enc = urllib.parse.quote(cwd, safe="-_.~")
                sdir = os.path.join(home, "sessions", enc, sid)
                os.makedirs(sdir, exist_ok=True)
                with open(os.path.join(sdir, "updates.jsonl"), "a", encoding="utf-8") as f:
                    for ch in ("Grok ", "auto ", "flushed!"):
                        f.write(json.dumps({"sessionId": sid, "_meta": {"agentTimestampMs": now_ms}, "update": {
                            "sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": ch}
                        }}) + "\n")
                    f.write(json.dumps({"sessionId": sid, "_meta": {"agentTimestampMs": now_ms + 10}, "update": {
                        "sessionUpdate": "turn_completed", "stop_reason": "end_turn",
                        "usage": {"inputTokens": 40, "outputTokens": 10}
                    }}) + "\n")
                    f.flush()
            elif mode == "gemini":
                with open(os.path.join(home, "projects.json"), "w", encoding="utf-8") as pf:
                    json.dump({"projects": {cwd: "ws_slug"}}, pf)
                cdir = os.path.join(home, "tmp", "ws_slug", "chats")
                os.makedirs(cdir, exist_ok=True)
                with open(os.path.join(cdir, f"session-2026-{sid[:8]}.json"), "w", encoding="utf-8") as sf:
                    json.dump({
                        "sessionId": sid,
                        "messages": [{
                            "id": "gm_auto_1",
                            "type": "gemini",
                            "timestamp": iso_now,
                            "content": "Gemini auto flushed!",
                            "tokens": {"input": 30, "output": 8}
                        }]
                    }, sf)
            elif mode == "opencode":
                dbp = os.path.join(home, "opencode.db")
                conn = sqlite3.connect(dbp)
                conn.execute("CREATE TABLE IF NOT EXISTS message (id TEXT PRIMARY KEY, session_id TEXT, data TEXT)")
                conn.execute("CREATE TABLE IF NOT EXISTS part (id TEXT PRIMARY KEY, message_id TEXT, session_id TEXT, time_created INTEGER, time_updated INTEGER, data TEXT)")
                conn.execute("INSERT OR REPLACE INTO message VALUES (?, ?, ?)", ("m_auto", sid, json.dumps({"role": "assistant"})))
                conn.execute("INSERT OR REPLACE INTO part VALUES (?, ?, ?, ?, ?, ?)", (
                    "p_txt", "m_auto", sid, now_ms, now_ms,
                    json.dumps({"type": "text", "text": "OpenCode auto "})
                ))
                conn.commit()
                time.sleep(0.05)
                conn.execute("UPDATE part SET time_updated = ?, data = ? WHERE id = 'p_txt'", (
                    now_ms + 20,
                    json.dumps({"type": "text", "text": "OpenCode auto flushed!", "time": {"start": now_ms, "end": now_ms + 20}})
                ))
                conn.execute("INSERT OR REPLACE INTO part VALUES (?, ?, ?, ?, ?, ?)", (
                    "p_fin", "m_auto", sid, now_ms + 30, now_ms + 30,
                    json.dumps({"type": "step-finish", "reason": "stop", "tokens": {"input": 25, "output": 7}})
                ))
                conn.commit()
                conn.close()
        else:
            # Replay buffered autonomous text before fresh Turn 2 text
            replayed = {
                "grok": "Grok auto flushed!",
                "gemini": "Gemini auto flushed!",
                "opencode": "OpenCode auto flushed!",
            }[mode]
            send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": sid, "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": replayed}
            }}})
            send({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": sid, "update": {
                "sessionUpdate": "agent_message_chunk",
                "content": {"type": "text", "text": f"{mode} turn 2 clean"}
            }}})
            send({"jsonrpc": "2.0", "id": mid, "result": {"stopReason": "end_turn"}})
"#,
    )
    .unwrap();

    let pool = open_db(&dir.path().join("state.db")).await;
    let host = TestHost::new(&root);
    let script_s = script.display().to_string();
    let cwd_s = root.display().to_string();
    let grok_s = grok_home.display().to_string();
    let gem_s = gemini_home.display().to_string();
    let oc_s = opencode_dir.display().to_string();

    let drivers: Arc<dyn DriverFactory> = Arc::new(
        move |provider: RuntimeProvider| -> Option<Box<dyn StructuredDriver>> {
            let (mode, home) = match provider {
                RuntimeProvider::Grok => ("grok", grok_s.clone()),
                RuntimeProvider::Gemini => ("gemini", gem_s.clone()),
                RuntimeProvider::Opencode => ("opencode", oc_s.clone()),
                _ => return None,
            };
            let mut d = AcpDriver::new(
                provider,
                "python3",
                vec![script_s.clone(), mode.into(), home.clone(), cwd_s.clone()],
            );
            d.set_sideband_root_dir(home);
            Some(Box::new(d))
        },
    );

    let cfg = RuntimeConfig {
        holders_dir: Some(holders_dir),
        holder_launcher: Some(HolderLauncher::for_bin(fake_bin!("otto-runtime-holder"))),
        autonomous_settle: Duration::from_millis(250),
        // Non-zero coalesce window verifies `upsert_item` wakes `pump_held` on held items!
        stream_coalesce: Duration::from_millis(40),
        ..RuntimeConfig::default()
    };
    let rt = AgentRuntime::new(
        AgentRuntimeRepo::new(pool),
        Arc::clone(&host) as Arc<dyn RuntimeHost>,
        drivers,
        cfg,
    );

    for (provider, expected_auto, expected_t2) in [
        (
            RuntimeProvider::Grok,
            "Grok auto flushed!",
            "grok turn 2 clean",
        ),
        (
            RuntimeProvider::Gemini,
            "Gemini auto flushed!",
            "gemini turn 2 clean",
        ),
        (
            RuntimeProvider::Opencode,
            "OpenCode auto flushed!",
            "opencode turn 2 clean",
        ),
    ] {
        let session = rt
            .create(
                &"u1".to_string(),
                &"w1".to_string(),
                CreateRuntimeSessionReq {
                    title_auto: false,
                    provider: Some(provider),
                    title: Some(format!("{}-sideband", provider.as_str())),
                    cwd: Some(root.display().to_string()),
                    model: None,
                    access_mode: Some(RuntimeAccessMode::FullAccess),
                    meta: None,
                },
            )
            .await
            .unwrap();

        let p1 = rt.prompt(&session.id, "start").await.unwrap();
        let sid = session.id.clone();
        let p1_tid = p1.turn_id.clone();
        host.wait(|ev| {
            ev.session_id == sid
                && matches!(
                    &ev.body,
                    RuntimeEventBody::TurnTerminal {
                        turn_id,
                        status: TurnTerminalStatus::Completed,
                        ..
                    } if *turn_id == p1_tid
                )
        })
        .await;

        // Autonomous sideband turn streams & settles without any user prompt!
        host.wait(|ev| {
            ev.session_id == sid
                && matches!(
                    &ev.body,
                    RuntimeEventBody::TurnTerminal {
                        turn_id,
                        status: TurnTerminalStatus::Completed,
                        ..
                    } if *turn_id != p1_tid
                )
        })
        .await;

        let p2 = rt.prompt(&session.id, "next").await.unwrap();
        let p2_tid = p2.turn_id.clone();
        host.wait(|ev| {
            ev.session_id == sid
                && matches!(
                    &ev.body,
                    RuntimeEventBody::TurnTerminal {
                        turn_id,
                        status: TurnTerminalStatus::Completed,
                        ..
                    } if *turn_id == p2_tid
                )
        })
        .await;

        rt.flush(&session.id).await;
        let (turns, turn_items): (Vec<ProviderTurn>, Vec<TurnItem>) =
            match rt.replay(&session.id, 0).await.unwrap() {
                RuntimeReplay::Snapshot { snapshot, .. } => (snapshot.turns, snapshot.turn_items),
                RuntimeReplay::Events { events, .. } => {
                    let mut turns: Vec<ProviderTurn> = Vec::new();
                    let mut items: Vec<TurnItem> = Vec::new();
                    for ev in events {
                        match ev.body {
                            RuntimeEventBody::ProviderTurnUpdated { provider_turn } => {
                                if let Some(existing) =
                                    turns.iter_mut().find(|t| t.id == provider_turn.id)
                                {
                                    *existing = provider_turn;
                                } else {
                                    turns.push(provider_turn);
                                }
                            }
                            RuntimeEventBody::TurnItemUpdated { turn_item } => {
                                if let Some(existing) =
                                    items.iter_mut().find(|i| i.id == turn_item.id)
                                {
                                    *existing = turn_item;
                                } else {
                                    items.push(turn_item);
                                }
                            }
                            _ => {}
                        }
                    }
                    (turns, items)
                }
            };

        assert_eq!(turns.len(), 3, "provider={}", provider.as_str());
        assert_eq!(turns[1].origin, TurnOrigin::Agent);
        let turn_msgs = |tid: &str| -> Vec<String> {
            turn_items
                .iter()
                .filter(|i| i.turn_id.as_deref() == Some(tid))
                .filter_map(|i| match &i.kind {
                    TurnItemKind::AssistantMessage { text, .. } => Some(text.clone()),
                    _ => None,
                })
                .collect()
        };
        assert_eq!(
            turn_msgs(&turns[1].id),
            vec![expected_auto.to_string()],
            "provider={} autonomous message mismatch",
            provider.as_str()
        );
        assert_eq!(
            turn_msgs(&turns[2].id),
            vec![expected_t2.to_string()],
            "provider={} replayed burst was not deduplicated",
            provider.as_str()
        );
    }

    rt.shutdown().await;
}
