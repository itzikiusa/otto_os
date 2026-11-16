//! Round 5 structured-runtime custom provider `claudework` parity, queued
//! image name recovery, sideband path confinement, and `runtime-holder`
//! disconnect/skew/persistence tests.

#![allow(clippy::disallowed_methods)]

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use chrono::Utc;
use otto_agent_protocol::acp::AcpDriver;
use otto_agent_protocol::claude::ClaudeDriver;
use otto_agent_protocol::codex::CodexDriver;
use otto_agent_protocol::driver::{ActiveTailKind, StructuredDriver};
use otto_agent_protocol::holder::{
    adopt_socket, list_holder_sockets, socket_path_for, HolderCheckpoint, HolderLauncher,
};
use otto_agent_protocol::host::as_agent_session;
use otto_agent_protocol::{
    validate_confined_tail_path, AgentRuntime, DriverFactory, LaunchSpec, McpDelivery,
    McpServerDef, RuntimeConfig, RuntimeHost,
};
use otto_core::agent_runtime::{
    AskChildReq, AskChildTarget, CouncilSeatReq, CreateCouncilReq, CreateRuntimeSessionBody,
    CreateRuntimeSessionReq, OpenPeerReq, PromptDelivery, PromptImage, ProviderSessionStatus,
    ProviderTurnStatus, RuntimeEvent, RuntimeEventBody, RuntimePromptReq, RuntimeProvider,
    RuntimeSession, SynthesizeCouncilReq, TurnTerminalStatus, PROVIDER_SLUG_META_KEY,
};
use otto_core::domain::Workspace;
use otto_core::{Id, Result};
use otto_state::{AgentRuntimeRepo, DbPool};
use serde_json::json;

macro_rules! fake_bin {
    ($name:literal) => {
        std::env::var(concat!("NEXTEST_BIN_EXE_", $name))
            .unwrap_or_else(|_| env!(concat!("CARGO_BIN_EXE_", $name)).to_string())
    };
}

const WAIT: Duration = Duration::from_secs(15);
const VALID_PNG_B64: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=";

struct ParityHost {
    root: String,
    persistence: AtomicBool,
    grok_enabled: AtomicBool,
    retired: AtomicUsize,
    events: Mutex<Vec<Arc<RuntimeEvent>>>,
    notify: tokio::sync::Notify,
}

#[async_trait::async_trait]
impl RuntimeHost for ParityHost {
    async fn workspace(&self, id: &Id) -> Result<Workspace> {
        Ok(Workspace {
            id: id.clone(),
            name: "W".into(),
            root_path: self.root.clone(),
            settings: json!({}),
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

    async fn retire(&self, _session: &RuntimeSession) {
        self.retired.fetch_add(1, Ordering::SeqCst);
        self.notify.notify_waiters();
    }

    fn resolve_custom_provider(&self, slug: &str) -> Option<RuntimeProvider> {
        (slug.trim() == "claudework").then_some(RuntimeProvider::Claude)
    }

    fn is_provider_enabled(&self, slug: &str) -> bool {
        if slug.trim().eq_ignore_ascii_case("grok") {
            return self.grok_enabled.load(Ordering::SeqCst);
        }
        true
    }

    async fn session_persistence_enabled(&self) -> bool {
        self.persistence.load(Ordering::SeqCst)
    }

    fn publish(&self, _s: &RuntimeSession, event: Arc<RuntimeEvent>) {
        self.events.lock().unwrap().push(event);
        self.notify.notify_waiters();
    }
}

impl ParityHost {
    fn new(root: &Path, persistence: bool) -> Arc<Self> {
        Arc::new(Self {
            root: root.to_string_lossy().into_owned(),
            persistence: AtomicBool::new(persistence),
            grok_enabled: AtomicBool::new(true),
            retired: AtomicUsize::new(0),
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
                panic!("timed out waiting for event; seen: {:#?}", self.all());
            }
        }
    }
}

fn fake_drivers() -> Arc<dyn DriverFactory> {
    Arc::new(|p: RuntimeProvider| -> Option<Box<dyn StructuredDriver>> {
        match p {
            RuntimeProvider::Claude => Some(Box::new(ClaudeDriver::with_program(fake_bin!(
                "otto-fake-claude"
            )))),
            RuntimeProvider::Codex => Some(Box::new(CodexDriver::with_program(fake_bin!(
                "otto-fake-codex"
            )))),
            RuntimeProvider::Gemini => {
                Some(Box::new(AcpDriver::gemini(&fake_bin!("otto-fake-agent"))))
            }
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

#[tokio::test]
async fn claudework_custom_provider_create_children_council_and_agent_session_parity() {
    let dir = tempfile::tempdir().unwrap();
    let pool = open_db(&dir.path().join("state.db")).await;
    let host = ParityHost::new(dir.path(), false);
    let rt = AgentRuntime::new(
        AgentRuntimeRepo::new(pool),
        Arc::clone(&host) as Arc<dyn RuntimeHost>,
        fake_drivers(),
        RuntimeConfig {
            attachments_dir: dir.path().join("attachments"),
            ..RuntimeConfig::default()
        },
    );

    let unknown: CreateRuntimeSessionBody =
        serde_json::from_value(json!({"provider": "nonexistent-custom"})).unwrap();
    assert!(unknown
        .into_req_with_resolver(|slug| rt.resolve_custom_provider(slug))
        .is_err());

    let body: CreateRuntimeSessionBody = serde_json::from_value(json!({
        "provider": "claudework",
        "cwd": dir.path().to_string_lossy(),
        "title": "Work Thread"
    }))
    .unwrap();
    let req = body
        .into_req_with_resolver(|slug| rt.resolve_custom_provider(slug))
        .unwrap();
    let s = rt.create(&"u1".into(), &"w1".into(), req).await.unwrap();
    assert_eq!(s.provider, RuntimeProvider::Claude);
    assert_eq!(s.provider_slug(), "claudework");
    assert_eq!(
        s.meta.as_ref().unwrap()[PROVIDER_SLUG_META_KEY],
        "claudework"
    );
    assert_eq!(as_agent_session(&s).provider, "claudework");

    let kids = rt
        .ask_children(
            &s.id,
            AskChildReq {
                prompt: "Check auth".into(),
                targets: vec![AskChildTarget {
                    provider: RuntimeProvider::Claude,
                    provider_slug: Some("claudework".into()),
                    model: None,
                    effort: None,
                }],
                ..AskChildReq::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(kids[0].provider_slug(), "claudework");
    assert_eq!(as_agent_session(&kids[0]).provider, "claudework");

    let seats = rt
        .create_council(
            &"u1".into(),
            &"w1".into(),
            CreateCouncilReq {
                prompt: "Compare designs".into(),
                title: Some("Design Council".into()),
                cwd: Some(dir.path().to_string_lossy().into_owned()),
                seats: vec![
                    CouncilSeatReq {
                        provider: RuntimeProvider::Claude,
                        provider_slug: Some("claudework".into()),
                        model: None,
                        effort: None,
                    },
                    CouncilSeatReq {
                        provider: RuntimeProvider::Codex,
                        provider_slug: None,
                        model: None,
                        effort: None,
                    },
                ],
                images: Vec::new(),
            },
        )
        .await
        .unwrap();
    assert_eq!(seats[0].provider_slug(), "claudework");
    for seat_sess in &seats {
        let sid = seat_sess.id.clone();
        host.wait(|e| {
            e.session_id == sid && matches!(e.body, RuntimeEventBody::TurnTerminal { .. })
        })
        .await;
    }
    let judge = rt
        .synthesize_council(
            &seats[0].id,
            SynthesizeCouncilReq {
                provider: RuntimeProvider::Claude,
                provider_slug: Some("claudework".into()),
                model: None,
                effort: None,
            },
        )
        .await
        .unwrap();
    assert_eq!(judge.provider_slug(), "claudework");

    // `OpenPeerReq` deserializes & round-trips custom slug `"claudework"`, and
    // `open_peer` preserves `provider_slug` on the spawned linked session.
    let peer_req: OpenPeerReq = serde_json::from_value(json!({
        "provider": "claudework",
        "prompt": "Pair on tests",
        "title": "Linked Work Peer"
    }))
    .unwrap();
    assert_eq!(peer_req.provider_slug(), "claudework");
    assert_eq!(
        serde_json::to_value(&peer_req).unwrap()["provider"],
        "claudework"
    );
    let peer = rt.open_peer(&s.id, peer_req).await.unwrap();
    assert_eq!(peer.provider_slug(), "claudework");
    assert_eq!(as_agent_session(&peer).provider, "claudework");

    // Disabling built-in `"grok"` rejects session creation, `open_peer`, and `ask_children`.
    host.grok_enabled.store(false, Ordering::SeqCst);
    let grok_body: CreateRuntimeSessionBody =
        serde_json::from_value(json!({"provider": "grok"})).unwrap();
    assert!(grok_body
        .into_req_with_resolver(|slug| rt.resolve_runtime_provider(slug))
        .is_err());
    let grok_peer: OpenPeerReq = serde_json::from_value(json!({"provider": "grok"})).unwrap();
    assert!(rt.open_peer(&s.id, grok_peer).await.is_err());
    assert!(rt
        .ask_children(
            &s.id,
            AskChildReq {
                prompt: "Should fail".into(),
                targets: vec![AskChildTarget {
                    provider: RuntimeProvider::Grok,
                    provider_slug: None,
                    model: None,
                    effort: None,
                }],
                ..AskChildReq::default()
            },
        )
        .await
        .is_err());

    rt.shutdown().await;
}

#[test]
fn sideband_path_confinement_blocks_secrets_traversals_and_symlinks() {
    let dir = tempfile::tempdir().unwrap();
    let cwd = dir.path().join("workspace");
    let brain = dir.path().join("agy-brain");
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(cwd.join(".git")).unwrap();
    std::fs::create_dir_all(&outside).unwrap();

    let conv_logs = brain
        .join("aaaa1111-bbbb-2222-cccc-333333333333")
        .join(".system_generated")
        .join("logs");
    std::fs::create_dir_all(&conv_logs).unwrap();
    let valid_transcript = conv_logs.join("transcript.jsonl");
    std::fs::write(&valid_transcript, "{}\n").unwrap();

    let valid_log = cwd.join("build.log");
    std::fs::write(&valid_log, "ok\n").unwrap();

    let otto_db = cwd.join("otto.db");
    std::fs::write(&otto_db, "sqlite").unwrap();
    let dotenv = cwd.join(".env");
    std::fs::write(&dotenv, "API_KEY=secret\n").unwrap();
    let dotenv_local = cwd.join(".env.local");
    std::fs::write(&dotenv_local, "TOKEN=secret\n").unwrap();
    let git_config = cwd.join(".git/config");
    std::fs::write(&git_config, "[remote \"origin\"]\n").unwrap();
    let outside_secret = outside.join("id_rsa");
    std::fs::write(&outside_secret, "PRIVATE KEY").unwrap();
    let symlink_escape = cwd.join("escape.log");
    std::os::unix::fs::symlink(&outside_secret, &symlink_escape).unwrap();

    assert!(validate_confined_tail_path(
        &valid_transcript,
        ActiveTailKind::AgySubagentTranscript,
        Some(&cwd),
        Some(&brain),
    )
    .is_some());
    assert!(validate_confined_tail_path(
        &valid_log,
        ActiveTailKind::CommandLog,
        Some(&cwd),
        Some(&brain),
    )
    .is_some());

    for forbidden in [
        Path::new("/Users/victim/.ssh/id_rsa"),
        Path::new("/Users/victim/.aws/credentials"),
        otto_db.as_path(),
        dotenv.as_path(),
        dotenv_local.as_path(),
        git_config.as_path(),
        outside_secret.as_path(),
        symlink_escape.as_path(),
        &cwd.join("../outside/id_rsa"),
    ] {
        assert!(
            validate_confined_tail_path(
                forbidden,
                ActiveTailKind::CommandLog,
                Some(&cwd),
                Some(&brain),
            )
            .is_none(),
            "must block {}",
            forbidden.display()
        );
    }
}

#[tokio::test]
async fn prompt_image_name_survives_queue_recover_and_unqueue() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let attach_dir = dir.path().join("attachments");

    let pool1 = open_db(&db_path).await;
    let host1 = ParityHost::new(dir.path(), false);
    let rt1 = AgentRuntime::new(
        AgentRuntimeRepo::new(pool1.clone()),
        Arc::clone(&host1) as Arc<dyn RuntimeHost>,
        fake_drivers(),
        RuntimeConfig {
            attachments_dir: attach_dir.clone(),
            ..RuntimeConfig::default()
        },
    );

    let s = rt1
        .create(
            &"u1".into(),
            &"w1".into(),
            CreateRuntimeSessionReq {
                provider: Some(RuntimeProvider::Claude),
                cwd: Some(dir.path().to_string_lossy().into_owned()),
                title: Some("Queue Image Name".into()),
                model: None,
                access_mode: None,
                meta: None,
            },
        )
        .await
        .unwrap();

    rt1.prompt(&s.id, "slow 10").await.unwrap();
    host1
        .wait(|e| {
            matches!(
                &e.body,
                RuntimeEventBody::ProviderTurnUpdated { provider_turn }
                    if provider_turn.status == ProviderTurnStatus::Running
            )
        })
        .await;

    let mut queued_req = RuntimePromptReq::of("follow-up with named screenshot");
    queued_req.delivery = Some(PromptDelivery::Queue);
    queued_req.images.push(PromptImage {
        mime: "image/png".into(),
        data_base64: VALID_PNG_B64.into(),
        name: Some("architecture-diagram.png".into()),
    });
    let q_resp = rt1.prompt_with(&s.id, &queued_req).await.unwrap();
    let qid = q_resp.queued_id.expect("queued_id");

    rt1.shutdown().await;
    drop(rt1);
    drop(pool1);

    let pool2 = open_db(&db_path).await;
    let host2 = ParityHost::new(dir.path(), false);
    let rt2 = AgentRuntime::new(
        AgentRuntimeRepo::new(pool2),
        Arc::clone(&host2) as Arc<dyn RuntimeHost>,
        fake_drivers(),
        RuntimeConfig {
            attachments_dir: attach_dir,
            ..RuntimeConfig::default()
        },
    );
    rt2.recover().await.unwrap();

    let restored = rt2
        .unqueue_prompt(&s.id, &qid)
        .await
        .unwrap()
        .expect("queued item survives recover");
    assert_eq!(restored.images.len(), 1);
    assert_eq!(
        restored.images[0].name.as_deref(),
        Some("architecture-diagram.png")
    );
    assert_eq!(restored.images[0].data_base64, VALID_PNG_B64);
    rt2.shutdown().await;
}

#[tokio::test]
async fn holder_disconnect_settles_and_respawns_plus_wire_skew_and_persistence_toggle() {
    let skewed = json!({
        "session_id": "s-skew",
        "provider": "claude",
        "acked_seq": 42,
        "streaming_items": [
            {
                "id": "good-1",
                "turn_id": "t1",
                "ordinal": 1,
                "status": "running",
                "title": null,
                "started_at": Utc::now().to_rfc3339(),
                "completed_at": null,
                "updated_at": Utc::now().to_rfc3339(),
                "error": null,
                "parent_item_id": null,
                "detail_omitted": false,
                "type": "assistant_message",
                "message_id": "m1",
                "text": "partial",
                "streaming": true
            },
            {
                "id": "future-2",
                "ordinal": 2,
                "status": "running",
                "updated_at": Utc::now().to_rfc3339(),
                "type": "future_unknown_variant",
                "foo": 123
            }
        ]
    });
    let cp = HolderCheckpoint::from_wire_value(skewed).expect("graceful checkpoint recovery");
    assert_eq!(cp.session_id, "s-skew");
    assert_eq!(cp.acked_seq, 42);
    assert_eq!(cp.streaming_items.len(), 1);
    assert_eq!(cp.streaming_items[0].id, "good-1");

    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("state.db");
    let holders_dir = PathBuf::from(format!("/tmp/otto-r5-{}", uuid::Uuid::new_v4().simple()));
    let pool = open_db(&db_path).await;
    let host = ParityHost::new(dir.path(), true);
    let rt = AgentRuntime::new(
        AgentRuntimeRepo::new(pool),
        Arc::clone(&host) as Arc<dyn RuntimeHost>,
        fake_drivers(),
        RuntimeConfig {
            attachments_dir: dir.path().join("attachments"),
            holders_dir: Some(holders_dir.clone()),
            holder_launcher: Some(HolderLauncher::for_bin(fake_bin!("otto-runtime-holder"))),
            ..RuntimeConfig::default()
        },
    );

    let s = rt
        .create(
            &"u1".into(),
            &"w1".into(),
            CreateRuntimeSessionReq {
                provider: Some(RuntimeProvider::Claude),
                cwd: Some(dir.path().to_string_lossy().into_owned()),
                title: Some("Crash Recovery".into()),
                model: None,
                access_mode: None,
                meta: None,
            },
        )
        .await
        .unwrap();
    host.wait(|e| {
        matches!(
            &e.body,
            RuntimeEventBody::ProviderSessionUpdated { provider_session }
                if provider_session.status == ProviderSessionStatus::Ready
        )
    })
    .await;

    let t1 = rt.prompt(&s.id, "slow 10").await.unwrap();
    host.wait(|e| {
        matches!(
            &e.body,
            RuntimeEventBody::ProviderTurnUpdated { provider_turn }
                if provider_turn.id == t1.turn_id
                    && provider_turn.status == ProviderTurnStatus::Running
        )
    })
    .await;

    let sock = socket_path_for(&holders_dir, &s.id).unwrap();
    let adopted = adopt_socket(&sock).await.expect("adopt live holder");
    unsafe {
        let _ = libc::kill(adopted.child_pid as libc::pid_t, libc::SIGKILL);
        let _ = libc::kill(adopted.holder_pid as libc::pid_t, libc::SIGKILL);
    }
    drop(adopted);

    host.wait(|e| {
        matches!(
            &e.body,
            RuntimeEventBody::TurnTerminal { turn_id, status, .. }
                if turn_id == &t1.turn_id && *status == TurnTerminalStatus::Failed
        )
    })
    .await;
    host.wait(|e| {
        matches!(
            &e.body,
            RuntimeEventBody::ProviderSessionUpdated { provider_session }
                if provider_session.status == ProviderSessionStatus::Error
        )
    })
    .await;
    assert!(host.retired.load(Ordering::SeqCst) >= 1);

    let t2 = rt.prompt(&s.id, "recovered turn").await.unwrap();
    host.wait(|e| {
        matches!(
            &e.body,
            RuntimeEventBody::TurnTerminal { turn_id, status, .. }
                if turn_id == &t2.turn_id && *status == TurnTerminalStatus::Completed
        )
    })
    .await;

    host.persistence.store(false, Ordering::SeqCst);
    rt.shutdown().await;
    let _ = std::fs::remove_dir_all(&holders_dir);

    let s_direct = rt
        .create(
            &"u1".into(),
            &"w1".into(),
            CreateRuntimeSessionReq {
                provider: Some(RuntimeProvider::Claude),
                cwd: Some(dir.path().to_string_lossy().into_owned()),
                title: Some("Ephemeral".into()),
                model: None,
                access_mode: None,
                meta: None,
            },
        )
        .await
        .unwrap();
    let td = rt.prompt(&s_direct.id, "ping").await.unwrap();
    host.wait(|e| {
        matches!(
            &e.body,
            RuntimeEventBody::TurnTerminal { turn_id, status, .. }
                if turn_id == &td.turn_id && *status == TurnTerminalStatus::Completed
        )
    })
    .await;
    assert!(list_holder_sockets(&holders_dir).is_empty());
    rt.shutdown_for_restart().await;
    assert!(list_holder_sockets(&holders_dir).is_empty());
    rt.shutdown().await;
    let _ = std::fs::remove_dir_all(&holders_dir);
}

#[test]
fn archived_queue_is_emptied_so_imported_prompts_never_send_themselves() {
    let settled =
        otto_agent_protocol::settle_archived_entity_json("queue", r#"[{"text":"deploy"}]"#);
    assert_eq!(settled.as_deref(), Some("[]"));
    assert_eq!(
        otto_agent_protocol::settle_archived_entity_json("queue", "[]"),
        None
    );
}
