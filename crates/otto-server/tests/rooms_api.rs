//! Real HTTP, membership WebSocket and PTY boundary integration, isolated data.
use axum::Router;
use base64::{engine::general_purpose::STANDARD, Engine};
use futures_util::{SinkExt, StreamExt};
use otto_core::{secrets::SecretStore, Error, Id, Result};
use otto_rbac::RbacRoleChecker;
use otto_server::ServerCtx;
use otto_sessions::{ProviderRegistry, SessionManager};
use otto_state::{
    ConnectionSectionsRepo, ConnectionsRepo, DbExplorerRepo, DbPool, GitStore, IntegrationsRepo,
    IssuesRepo, ProductRepo, ReviewsRepo, SessionsRepo, SkillEvalsRepo, SwarmRepo, WorkspacesRepo,
};
use serde_json::{json, Value};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use std::{path::PathBuf, sync::Arc};
use tokio::sync::broadcast;
use tokio_tungstenite::{
    tungstenite::{client::IntoClientRequest, Message},
    MaybeTlsStream, WebSocketStream,
};
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;
#[derive(Default)]
struct NoopSecrets(std::sync::Mutex<std::collections::HashMap<String, String>>);
impl SecretStore for NoopSecrets {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        self.0.lock().unwrap().insert(key.into(), value.into());
        Ok(())
    }
    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }
    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

struct NoopSpawner;
impl otto_connections::Spawner for NoopSpawner {
    fn spawn_connection<'a>(
        &'a self,
        _ws_id: &'a Id,
        _user_id: &'a Id,
        _conn: &'a otto_core::domain::Connection,
        _spec: otto_pty::CommandSpec,
        _first_command: Option<String>,
        _title: Option<String>,
    ) -> otto_core::auth::BoxFuture<'a, Result<otto_core::domain::Session>> {
        Box::pin(async { Err(Error::Internal("noop spawner".into())) })
    }
}

async fn mem_pool() -> DbPool {
    let opts = SqliteConnectOptions::new()
        .in_memory(true)
        .foreign_keys(true);
    let pool = SqlitePoolOptions::new()
        .max_connections(1)
        .connect_with(opts)
        .await
        .expect("connect in-memory sqlite");
    sqlx::migrate!("../otto-state/migrations")
        .run(&pool)
        .await
        .expect("run migrations");
    pool.into()
}

async fn test_ctx(pool: &DbPool, data_dir: PathBuf) -> ServerCtx {
    let (events, _rx) = broadcast::channel(64);
    let secrets: Arc<dyn SecretStore> = Arc::new(NoopSecrets::default());
    let roles = Arc::new(RbacRoleChecker::new(pool.clone()));
    let repo = SessionsRepo::new(pool.clone());
    let providers = ProviderRegistry::new(None);
    let manager = Arc::new(SessionManager::new(repo, events.clone(), providers));
    let orchestrator = Arc::new(otto_orchestrator::Orchestrator::new("claude"));
    let improve_engine = Arc::new(otto_improve::ImprovementEngine {
        improvements: otto_state::ImprovementsRepo::new(pool.clone()),
        sessions: SessionsRepo::new(pool.clone()),
        workspaces: WorkspacesRepo::new(pool.clone()),
        producer: Arc::new(otto_improve::RealProposalProducer::new(
            orchestrator.clone(),
        )),
        events: events.clone(),
        library_root: data_dir.join("library"),
    });
    let connections = Arc::new(otto_connections::ConnectionsService::new(
        ConnectionsRepo::new(pool.clone()),
        ConnectionSectionsRepo::new(pool.clone()),
        secrets.clone(),
    ));
    let db_explorer = Arc::new(otto_dbviewer::DbViewerService::new(
        ConnectionsRepo::new(pool.clone()),
        secrets.clone(),
        DbExplorerRepo::new(pool.clone()),
    ));
    let brokers = Arc::new(otto_brokers::BrokersService::new(
        otto_state::BrokerClustersRepo::new(pool.clone()),
        secrets.clone(),
        None,
    ));
    let mcp = Arc::new(otto_mcp::McpService::new(pool.clone(), secrets.clone()));
    let swarm_repo = SwarmRepo::new(pool.clone());
    let swarm = Arc::new(otto_swarm::SwarmService::new(swarm_repo.clone()));
    let product_repo = ProductRepo::new(pool.clone());
    let product = Arc::new(otto_product::ProductService::new(
        product_repo.clone(),
        IssuesRepo::new(pool.clone()),
        secrets.clone(),
    ));
    let usage = otto_usage::UsageEngine::start(
        otto_usage::UsageConfig {
            enabled: false, // This fixture does not exercise metrics or start ClickHouse.
            ..Default::default()
        },
        data_dir.join("usage"),
    )
    .await;
    let context_library = otto_context::Library::new(data_dir.join("context"));

    ServerCtx {
        pool: pool.clone(),
        secrets,
        events: events.clone(),
        authenticator: Arc::new(otto_rbac::RbacAuthenticator::new(pool.clone())),
        roles,
        auth_cache: otto_rbac::AuthCache::new(),
        version: "test".into(),
        base_url: "http://127.0.0.1:0".into(),
        data_dir: data_dir.clone(),
        plugins: Arc::new(otto_server::plugins::PluginManager::new(
            otto_state::PluginsRepo::new(pool.clone()),
            data_dir.join("plugins"),
            data_dir.clone(),
            "http://127.0.0.1:7700/api/v1/plugin-host".into(),
        )),
        manager,
        workspaces: WorkspacesRepo::new(pool.clone()),
        connections,
        db_explorer,
        db_assist: otto_server::db_assist::new_registry(),
        transcript_cache: Default::default(),
        rooms: Default::default(),
        brokers,
        mcp,
        spawner: Arc::new(NoopSpawner),
        git_store: GitStore::new(pool.clone()),
        issues_store: IssuesRepo::new(pool.clone()),
        integrations_store: IntegrationsRepo::new(pool.clone()),
        channel_bridge: None,
        wf_skip_current: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        reviews_store: ReviewsRepo::new(pool.clone()),
        findings_store: otto_state::ReviewFindingsRepo::new(pool.clone()),
        finding_events_store: otto_state::FindingEventsRepo::new(pool.clone()),
        repo_rules_store: otto_state::RepoRulesRepo::new(pool.clone()),
        proof_packs_store: otto_state::ReviewProofPacksRepo::new(pool.clone()),
        skill_evals_store: SkillEvalsRepo::new(pool.clone()),
        golden_tasks_store: otto_state::GoldenTasksRepo::new(pool.clone()),
        eval_matrices_store: otto_state::EvalMatricesRepo::new(pool.clone()),
        skill_eval_cancels: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        skill_reviews_store: otto_state::SkillReviewsRepo::new(pool.clone()),
        skill_review_cancels: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        review_agent_cancels: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        review_cancels: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        orchestrator,
        improve_engine,
        context_library,
        usage,
        product,
        product_repo,
        attachment_repo: otto_state::ProductAttachmentRepo::new(pool.clone()),
        discovery_repo: otto_state::ProductDiscoveryRepo::new(pool.clone()),
        refinement_repo: otto_state::ProductRefinementRepo::new(pool.clone()),
        mockup_repo: otto_state::ProductMockupRepo::new(pool.clone()),
        discovery_chat_repo: otto_state::DiscoveryChatRepo::new(pool.clone()),
        canvas_repo: otto_state::CanvasRepo::new(pool.clone()),
        product_agent_cancels: otto_server::product_run::new_cancel_registry(),
        design_jobs: otto_server::design_blender::new_job_registry(),
        memory: Arc::new(otto_memory::MemoryService::with_defaults(pool.clone())),
        vault: Arc::new(otto_vault::VaultEngine::new(pool.clone())),
        vault_docs_runs: otto_server::vault_docs_agent::new_run_registry(),
        vault_docs_refine: otto_server::vault_docs_agent::new_refine_registry(),
        swarm,
        swarm_repo,
        swarm_coords: otto_server::swarm_runtime::new_registry(),
        swarm_run_cancels: otto_server::swarm_run::new_cancel_registry(),
        goal_loops_repo: otto_state::GoalLoopsRepo::new(pool.clone()),
        goal_loops: otto_server::goal_loop::new_registry(),
        workgraph: Arc::new(otto_workgraph::WorkGraphService::new(
            otto_state::WorkGraphRepo::new(pool.clone()),
            events.clone(),
        )),
        scheduled_tasks: otto_state::ScheduledTasksRepo::new(pool.clone()),
        proof_repo: otto_state::ProofRepo::new(pool.clone()),
        proof_locks: otto_server::proof::new_locks(),
        runs: otto_state::RunsRepo::new(pool.clone()),
        runs_engine: otto_server::run_engine::RunEngine::new(),
        browser_tabs: otto_state::BrowserTabsRepo::new(pool.clone()),
        browser_annotations: otto_state::BrowserAnnotationsRepo::new(pool.clone()),
        browser_credentials: otto_state::BrowserCredentialsRepo::new(pool.clone()),
        ui_bridge: Default::default(),
        browser: Arc::new(otto_server::routes::browser::BrowserEngineHandle::new(
            None, data_dir,
        )),
    }
}

async fn connect(
    origin: &str,
    path: &str,
    token: &str,
) -> std::result::Result<Socket, Box<tokio_tungstenite::tungstenite::Error>> {
    let mut request = format!("{}{path}", origin.replacen("http:", "ws:", 1))
        .into_client_request()
        .unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        format!("otto-room, {token}").parse().unwrap(),
    );
    tokio_tungstenite::connect_async(request)
        .await
        .map(|(socket, _)| socket)
        .map_err(Box::new)
}
async fn send(socket: &mut Socket, value: Value) {
    socket.send(Message::Text(value.to_string())).await.unwrap();
}
async fn event(socket: &mut Socket) -> Value {
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match socket.next().await.unwrap().unwrap() {
                Message::Text(text) => return serde_json::from_str(&text).unwrap(),
                Message::Ping(data) => socket.send(Message::Pong(data)).await.unwrap(),
                _ => {}
            }
        }
    })
    .await
    .expect("room event deadline")
}
async fn until(socket: &mut Socket, predicate: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..40 {
        let value = event(socket).await;
        if predicate(&value) {
            return value;
        }
    }
    panic!("expected event not received")
}
#[tokio::test]
async fn room_http_auth_admission_driver_and_terminal_process_are_isolated() {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
    let users = otto_state::UsersRepo::new(pool.clone());
    let owner = users.create("owner", "unused", "Host", true).await.unwrap();
    let other = users
        .create("other", "unused", "Other", true)
        .await
        .unwrap();
    let auth = otto_rbac::AuthRepo::new(pool.clone());
    let owner_token = auth.issue(&owner.id).await.unwrap();
    let other_token = auth.issue(&other.id).await.unwrap();
    let mcp_token = auth.issue_mcp_token(&owner.id, None).await.unwrap();
    let workspace = ctx
        .workspaces
        .create("Room test", tmp.path().to_str().unwrap(), &owner.id)
        .await
        .unwrap();
    let session = ctx
        .manager
        .create(
            &workspace,
            &owner.id,
            otto_core::api::CreateSessionReq {
                kind: otto_core::domain::SessionKind::Agent,
                provider: Some("shell".into()),
                title: Some("Isolated room test".into()),
                cwd: None,
                connection_id: None,
                meta: None,
                model: None,
            },
            Some(otto_pty::CommandSpec {
                program: "/bin/cat".into(),
                args: vec![],
                cwd: Some(tmp.path().to_string_lossy().into()),
                env: vec![],
            }),
        )
        .await
        .unwrap();
    let app: Router = otto_server::build_router(ctx.clone(), vec![], vec![]);
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let origin = format!("http://{}", listener.local_addr().unwrap());
    let server = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
    let client = reqwest::Client::new();
    let settings:Value=client.put(format!("{origin}/api/v1/room-settings")).bearer_auth(&owner_token).json(&json!({"public_origin":"https://rooms.example.test","stun_urls":["stun:relay.example.test:3478"],"turn_urls":["turn:relay.example.test:3478"],"relay_only":true,"turn_secret":"fixture-shared-secret"})).send().await.unwrap().json().await.unwrap();
    assert_eq!(settings["turn_secret_configured"], true);
    assert!(settings.get("turn_secret").is_none());
    let persisted = otto_state::SettingsRepo::new(pool.clone())
        .get("room_ice")
        .await
        .unwrap()
        .unwrap()
        .to_string();
    assert!(!persisted.contains("fixture-shared-secret"));
    let create = format!("{origin}/api/v1/sessions/{}/room", session.id);
    assert_eq!(
        client
            .post(&create)
            .json(&json!({"name":"Host"}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let share_token = auth
        .issue_share_token(
            &owner.id,
            &session.id,
            otto_core::domain::WorkspaceRole::Editor,
            600,
            None,
        )
        .await
        .unwrap()
        .0;
    let managed_token = auth
        .issue_session_api_token(&owner.id, &session.id)
        .await
        .unwrap()
        .0;
    let impersonation = auth
        .issue_impersonation_token(&other.id, &owner.id, chrono::Duration::minutes(5))
        .await
        .unwrap();
    for token in [
        &other_token,
        &mcp_token,
        &share_token,
        &managed_token,
        &impersonation,
    ] {
        assert_eq!(
            client
                .post(&create)
                .bearer_auth(token)
                .json(&json!({"name":"Host"}))
                .send()
                .await
                .unwrap()
                .status(),
            403
        );
    }
    let response = client
        .post(&create)
        .bearer_auth(&owner_token)
        .json(&json!({"name":"Host"}))
        .send()
        .await
        .unwrap();
    assert!(
        response.status().is_success(),
        "{}",
        response.text().await.unwrap()
    );
    // Create above response was consumed only on failure by the assertion.
    let host: otto_core::api::RoomCredential = response.json().await.unwrap();
    let room_path = format!("/ws/rooms/{}", host.room_id);
    let term_path = format!("{room_path}/terminal");
    assert_eq!(
        client
            .get(format!("{origin}/api/v1/workspaces"))
            .bearer_auth(&host.token)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    assert!(connect(&origin, &room_path, &owner_token).await.is_err());
    assert!(
        connect(&origin, &format!("{room_path}?token=x"), &host.token)
            .await
            .is_err()
    );
    let invite: Value = client
        .post(format!("{origin}/api/v1/rooms/{}/invites", host.room_id))
        .bearer_auth(&owner_token)
        .json(&json!({"role":"viewer"}))
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(invite["url"].as_str().unwrap().ends_with(&format!(
        "/#/room/{}/{}",
        host.room_id,
        invite["invite"].as_str().unwrap()
    )));
    let join = json!({"room_id":host.room_id,"invite":invite["invite"],"name":"Guest"});
    let guest: otto_core::api::RoomCredential = client
        .post(format!("{origin}/api/v1/room-join"))
        .json(&join)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert_eq!(
        client
            .post(format!("{origin}/api/v1/room-join"))
            .json(&join)
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    let mut guest_ws = connect(&origin, &room_path, &guest.token).await.unwrap();
    let pending = until(&mut guest_ws, |v| v["type"] == "snapshot").await;
    assert_eq!(pending["room"].as_object().unwrap().len(), 3);
    assert!(connect(&origin, &term_path, &guest.token).await.is_err());
    let mut host_ws = connect(&origin, &room_path, &host.token).await.unwrap();
    until(&mut host_ws, |v| v["type"] == "snapshot").await;
    send(
        &mut host_ws,
        json!({"type":"admit","member_id":guest.member_id,"role":"viewer"}),
    )
    .await;
    let admitted = until(&mut guest_ws, |v| {
        v["type"] == "snapshot" && v["room"]["admission"] == "admitted"
    })
    .await;
    send(&mut guest_ws, json!({"type":"request_ice"})).await;
    let ice = until(&mut guest_ws, |v| v["type"] == "ice").await;
    assert_eq!(ice["relay_configured"], true);
    assert_eq!(ice["relay_only"], true);
    assert_ne!(ice["ice_servers"][1]["credential"], "fixture-shared-secret");
    assert!(ice["ice_servers"][1]["username"]
        .as_str()
        .unwrap()
        .ends_with(&guest.member_id));
    let mut terminal = connect(&origin, &term_path, &guest.token).await.unwrap();
    until(&mut terminal, |v| v["type"] == "scrollback").await;
    // A viewer may pause its own output without acquiring terminal control.
    let authority = ctx
        .manager
        .room_authority_snapshot(&session.id)
        .await
        .unwrap();
    send(&mut terminal, json!({"type":"pause"})).await;
    // The ordered snapshot response proves the pause was handled first.
    send(&mut terminal, json!({"type":"snapshot"})).await;
    let ack = until(&mut terminal, |v| {
        v["type"] == "scrollback" || v["type"] == "error"
    })
    .await;
    assert_eq!(
        ack["type"], "scrollback",
        "viewer flow frames must be accepted"
    );
    ctx.manager
        .human_input(&session.id, &owner.id, false, true, b"PAUSED-ROOM-OUTPUT\n")
        .await
        .unwrap();
    let handle = ctx.manager.live_handle(&session.id).unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        while !String::from_utf8_lossy(&handle.snapshot_with_history(100))
            .contains("PAUSED-ROOM-OUTPUT")
        {
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), terminal.next())
            .await
            .is_err(),
        "paused viewers must not receive live output"
    );
    // The renderer discarded its backlog: resync must answer even though a
    // trailing resume clears the old pause in the same WebSocket batch.
    send(&mut terminal, json!({"type":"resync","lines":1000})).await;
    send(&mut terminal, json!({"type":"resume"})).await;
    let resync = until(&mut terminal, |v| v["type"] == "scrollback").await;
    assert!(
        String::from_utf8_lossy(&STANDARD.decode(resync["data"].as_str().unwrap()).unwrap())
            .contains("PAUSED-ROOM-OUTPUT")
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), terminal.next())
            .await
            .is_err(),
        "trailing resume must not duplicate the recovery snapshot"
    );
    // Nothing new was produced. Recovery still answers, with a burst retained
    // as one pending snapshot instead of rejected by the history rate limiter.
    for _ in 0..3 {
        send(&mut terminal, json!({"type":"resync","lines":1000})).await;
    }
    let idle = until(&mut terminal, |v| {
        v["type"] == "scrollback" || v["type"] == "error"
    })
    .await;
    assert_eq!(
        idle["type"], "scrollback",
        "idle recovery must not be rate-rejected"
    );
    assert!(
        String::from_utf8_lossy(&STANDARD.decode(idle["data"].as_str().unwrap()).unwrap())
            .contains("PAUSED-ROOM-OUTPUT")
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(100), terminal.next())
            .await
            .is_err(),
        "coalesced recovery must answer once"
    );
    ctx.manager
        .human_input(&session.id, &owner.id, false, true, b"AFTER-ROOM-RESYNC\n")
        .await
        .unwrap();
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut bytes = vec![];
        loop {
            match terminal.next().await.unwrap().unwrap() {
                Message::Binary(chunk) => {
                    bytes.extend_from_slice(&chunk);
                    if String::from_utf8_lossy(&bytes).contains("AFTER-ROOM-RESYNC") {
                        break;
                    }
                }
                other => panic!("expected ordered live output after recovery, got {other:?}"),
            }
        }
    })
    .await
    .unwrap();
    assert_eq!(
        ctx.manager
            .room_authority_snapshot(&session.id)
            .await
            .unwrap(),
        authority
    );
    // Credit flow control (r3-10-06): a viewer opts in without a driver grant,
    // gets the clamped window back, and live output keeps flowing under it;
    // acks are accepted (they have their own rate budget).
    send(&mut terminal, json!({"type":"credit","window":65536})).await;
    let grant = until(&mut terminal, |v| {
        v["type"] == "credit" || v["type"] == "error"
    })
    .await;
    assert_eq!(
        grant["type"], "credit",
        "viewer credit frames must be accepted"
    );
    assert_eq!(grant["window"], 65536);
    ctx.manager
        .human_input(
            &session.id,
            &owner.id,
            false,
            true,
            b"CREDITED-ROOM-OUTPUT\n",
        )
        .await
        .unwrap();
    let credited = tokio::time::timeout(std::time::Duration::from_secs(2), async {
        let mut bytes = vec![];
        loop {
            match terminal.next().await.unwrap().unwrap() {
                Message::Binary(chunk) => {
                    bytes.extend_from_slice(&chunk);
                    if String::from_utf8_lossy(&bytes).contains("CREDITED-ROOM-OUTPUT") {
                        break bytes.len();
                    }
                }
                other => panic!("expected credited live output, got {other:?}"),
            }
        }
    })
    .await
    .unwrap();
    for _ in 0..200 {
        send(&mut terminal, json!({"type":"ack","bytes":credited})).await;
    }
    // Beyond the general 60/s frame limit, yet the socket stays up and
    // answers no ack with an error (trailing live output may still arrive).
    while let Ok(next) =
        tokio::time::timeout(std::time::Duration::from_millis(150), terminal.next()).await
    {
        match next.unwrap().unwrap() {
            Message::Binary(_) => {}
            other => panic!("an ack burst must be neither rejected nor answered, got {other:?}"),
        }
    }
    assert_eq!(
        ctx.manager
            .room_authority_snapshot(&session.id)
            .await
            .unwrap(),
        authority
    );
    send(&mut terminal,json!({"type":"input","data":STANDARD.encode(b"VIEWER-MUST-NOT-WRITE\n"),"grant_epoch":admitted["room"]["grant_epoch"]})).await;
    until(&mut terminal, |v| v["type"] == "error").await;
    send(
        &mut host_ws,
        json!({"type":"role","member_id":guest.member_id,"role":"editor"}),
    )
    .await;
    send(
        &mut host_ws,
        json!({"type":"grant_control","member_id":guest.member_id}),
    )
    .await;
    let driver = until(&mut guest_ws, |v| {
        v["type"] == "snapshot" && v["room"]["driver_member_id"] == guest.member_id
    })
    .await;
    let epoch = driver["room"]["grant_epoch"].as_u64().unwrap();
    send(
        &mut terminal,
        json!({"type":"input","data":STANDARD.encode(b"ROOM-PTY-ROUNDTRIP\n"),"grant_epoch":epoch}),
    )
    .await;
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        let mut bytes = vec![];
        loop {
            if let Message::Binary(chunk) = terminal.next().await.unwrap().unwrap() {
                bytes.extend_from_slice(&chunk);
                if String::from_utf8_lossy(&bytes).contains("ROOM-PTY-ROUNDTRIP") {
                    break;
                }
            }
        }
    })
    .await
    .unwrap();
    send(&mut host_ws, json!({"type":"release_control"})).await;
    until(&mut guest_ws, |v| {
        v["type"] == "snapshot" && v["room"]["grant_epoch"].as_u64().is_some_and(|v| v > epoch)
    })
    .await;
    send(&mut terminal,json!({"type":"input","data":STANDARD.encode(b"STALE-MUST-NOT-WRITE\n"),"grant_epoch":epoch})).await;
    until(&mut terminal, |v| v["type"] == "error").await;
    let recap_url = format!("{origin}/api/v1/rooms/{}/recaps", host.room_id);
    assert_eq!(
        client
            .post(&recap_url)
            .bearer_auth(&other_token)
            .json(&json!({"allow_unavailable_speech":true}))
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    assert_eq!(
        client
            .post(&recap_url)
            .bearer_auth(&guest.token)
            .json(&json!({"allow_unavailable_speech":true}))
            .send()
            .await
            .unwrap()
            .status(),
        401
    );
    for denied in [&mcp_token, &share_token, &managed_token] {
        assert!(!client
            .post(&recap_url)
            .bearer_auth(denied)
            .json(&json!({"allow_unavailable_speech":true}))
            .send()
            .await
            .unwrap()
            .status()
            .is_success());
    }
    let response = client
        .post(&recap_url)
        .bearer_auth(&owner_token)
        .json(&json!({"allow_unavailable_speech":true}))
        .send()
        .await
        .unwrap();
    assert_eq!(response.status(), 200);
    let recap: Value = response.json().await.unwrap();
    let recap_id = recap["id"].as_str().unwrap();
    send(&mut host_ws, json!({"type":"recap_start","epoch":1})).await;
    until(&mut host_ws, |v| v["type"] == "error").await;
    send(
        &mut host_ws,
        json!({"type":"recap_consent","epoch":1,"allow":true}),
    )
    .await;
    send(
        &mut guest_ws,
        json!({"type":"recap_consent","epoch":1,"allow":true}),
    )
    .await;
    until(&mut host_ws, |v| {
        v["type"] == "snapshot" && v["room"]["recap"]["state"] == "ready"
    })
    .await;
    send(&mut host_ws, json!({"type":"recap_start","epoch":1})).await;
    until(&mut host_ws, |v| {
        v["type"] == "snapshot" && v["room"]["recap"]["state"] == "capturing"
    })
    .await;
    send(
        &mut guest_ws,
        json!({"type":"chat","text":"accepted recap chat","nonce":"recap-chat"}),
    )
    .await;
    until(&mut host_ws, |v| {
        v["type"] == "snapshot"
            && v["room"]["messages"]
                .as_array()
                .is_some_and(|ms| ms.iter().any(|m| m["nonce"] == "recap-chat"))
    })
    .await;
    let detail_url = format!("{origin}/api/v1/room-recaps/{recap_id}");
    assert_eq!(
        client
            .get(&detail_url)
            .bearer_auth(&other_token)
            .send()
            .await
            .unwrap()
            .status(),
        403
    );
    let mut host_terminal = connect(&origin, &term_path, &host.token).await.unwrap();
    until(&mut host_terminal, |v| v["type"] == "status").await;
    let current_epoch = ctx
        .rooms
        .snapshot(&host.room_id, &host.member_id)
        .await
        .unwrap()
        .grant_epoch
        .unwrap();
    send(&mut host_terminal,json!({"type":"input","data":STANDARD.encode(b"RECAP-ACTIVE-PTY\n"),"grant_epoch":current_epoch})).await;
    let archived = tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            let v: Value = client
                .get(&detail_url)
                .bearer_auth(&owner_token)
                .send()
                .await
                .unwrap()
                .json()
                .await
                .unwrap();
            if v["events"].as_array().is_some_and(|es| {
                es.iter().any(|e| e["payload"]["type"] == "chat")
                    && es.iter().any(|e| {
                        e["payload"]["type"] == "terminal"
                            && e["payload"]["data_base64"].as_str().is_some_and(|b| {
                                STANDARD.decode(b).is_ok_and(|b| {
                                    String::from_utf8_lossy(&b).contains("RECAP-ACTIVE-PTY")
                                })
                            })
                    })
            }) {
                break v;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    assert!(!archived["events"]
        .as_array()
        .unwrap()
        .iter()
        .any(|e| e["payload"]["type"] == "terminal"
            && e["payload"]["data_base64"]
                .as_str()
                .is_some_and(|b| STANDARD
                    .decode(b)
                    .is_ok_and(|b| String::from_utf8_lossy(&b).contains("ROOM-PTY-ROUNDTRIP")))));
    send(
        &mut guest_ws,
        json!({"type":"recap_consent","epoch":1,"allow":false}),
    )
    .await;
    until(&mut host_ws, |v| {
        v["type"] == "snapshot" && v["room"]["recap"]["state"] == "paused"
    })
    .await;
    assert_eq!(client.post(format!("{detail_url}/gap")).bearer_auth(&owner_token).json(&json!({"capture_epoch":1,"kind":"speech","member_id":null,"source_id":null,"reason":"stale upload"})).send().await.unwrap().status(),403);
    let recognizer = tmp.path().join("fake-whisper");
    std::fs::write(&recognizer,r#"#!/bin/sh
while [ "$#" -gt 0 ]; do
 if [ "$1" = "--output-file" ]; then shift; out="$1"; fi
 shift
done
/bin/sleep 1
/usr/bin/printf '%s' '{"transcription":[{"text":"Final accepted words","offsets":{"from":0,"to":100}}]}' > "$out.json"
"#).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&recognizer, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let model = tmp.path().join("fake-model.bin");
    std::fs::write(&model, b"fixture").unwrap();
    assert_eq!(client.put(format!("{origin}/api/v1/room-recap-settings")).bearer_auth(&owner_token).json(&json!({"whisper_executable":recognizer,"whisper_model":model,"language":"en","threads":1})).send().await.unwrap().status(),200);
    for socket in [&mut host_ws, &mut guest_ws] {
        send(
            socket,
            json!({"type":"recap_consent","epoch":2,"allow":true}),
        )
        .await;
    }
    until(&mut host_ws, |v| {
        v["type"] == "snapshot" && v["room"]["recap"]["state"] == "ready"
    })
    .await;
    send(&mut host_ws, json!({"type":"recap_start","epoch":2})).await;
    until(&mut host_ws, |v| {
        v["type"] == "snapshot" && v["room"]["recap"]["state"] == "capturing"
    })
    .await;
    send(
        &mut host_ws,
        json!({"type":"audio","joined":true,"muted":false}),
    )
    .await;
    let audio_state = until(&mut host_ws, |v| {
        v["type"] == "snapshot"
            && v["room"]["members"].as_array().is_some_and(|ms| {
                ms.iter().any(|m| {
                    m["id"] == host.member_id && m["audio_joined"] == true && m["muted"] == false
                })
            })
    })
    .await;
    send(
        &mut host_ws,
        json!({"type":"audio_applied","epoch":audio_state["room"]["audio_epoch"]}),
    )
    .await;
    let generation = audio_state["room"]["members"]
        .as_array()
        .unwrap()
        .iter()
        .find(|m| m["id"] == host.member_id)
        .unwrap()["generation"]
        .clone();
    let mut wav = vec![];
    wav.extend(b"RIFF");
    wav.extend(3236u32.to_le_bytes());
    wav.extend(b"WAVEfmt ");
    wav.extend(16u32.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(1u16.to_le_bytes());
    wav.extend(16000u32.to_le_bytes());
    wav.extend(32000u32.to_le_bytes());
    wav.extend(2u16.to_le_bytes());
    wav.extend(16u16.to_le_bytes());
    wav.extend(b"data");
    wav.extend(3200u32.to_le_bytes());
    wav.resize(3244, 0);
    let response=client.post(format!("{detail_url}/audio")).bearer_auth(&owner_token).json(&json!({"capture_epoch":2,"member_id":host.member_id,"member_generation":generation,"sequence":1,"offset_ms":0,"duration_ms":100,"wav_base64":STANDARD.encode(wav)})).send().await.unwrap();
    assert_eq!(response.status(), 200);
    send(&mut host_ws, json!({"type":"recap_stop"})).await;
    let finalizing = until(&mut host_ws, |v| {
        v["type"] == "snapshot" && v["room"]["recap"]["state"] == "finalizing"
    })
    .await;
    assert!(
        finalizing["room"]["recap"]["pending_jobs"]
            .as_u64()
            .unwrap()
            > 0
    );
    assert_eq!(client.post(format!("{detail_url}/gap")).bearer_auth(&owner_token).json(&json!({"capture_epoch":2,"kind":"speech","member_id":null,"source_id":null,"reason":"new capture during finalization"})).send().await.unwrap().status(),403);
    until(&mut host_ws, |v| {
        v["type"] == "snapshot" && v["room"]["recap"]["state"] == "stopped"
    })
    .await;
    let finished: Value = client
        .get(&detail_url)
        .bearer_auth(&owner_token)
        .send()
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    assert!(
        finished["events"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["payload"]["type"] == "speech"
                && e["payload"]["segments"][0]["text"] == "Final accepted words"),
        "accepted final audio must survive explicit Stop: {finished}"
    );
    assert_eq!(
        client
            .delete(format!("{origin}/api/v1/rooms/{}", host.room_id))
            .bearer_auth(&owner_token)
            .send()
            .await
            .unwrap()
            .status(),
        204
    );
    assert_eq!(
        client
            .get(&detail_url)
            .bearer_auth(&owner_token)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert_eq!(
        client
            .get(format!("{detail_url}/export"))
            .bearer_auth(&owner_token)
            .send()
            .await
            .unwrap()
            .status(),
        200
    );
    assert!(
        ctx.manager.is_live(&session.id),
        "Room end must leave PTY alive"
    );
    assert!(connect(&origin, &term_path, &guest.token).await.is_err());
    let output = ctx
        .manager
        .live_handle(&session.id)
        .unwrap()
        .snapshot_with_history(100);
    assert!(!String::from_utf8_lossy(&output).contains("VIEWER-MUST-NOT-WRITE"));
    assert!(!String::from_utf8_lossy(&output).contains("STALE-MUST-NOT-WRITE"));
    ctx.manager.kill_session(&session.id).await.unwrap();
    server.abort();
}
