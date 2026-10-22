use super::*;
use crate::titles::{clean_provider_title, PROVIDER_TITLE_MAX};
use otto_core::domain::{SessionKind, Workspace};
use otto_core::new_id;

#[test]
fn resume_refuses_a_missing_folder_but_not_scratch_or_otto_dirs() {
    let dir = tempfile::tempdir().unwrap();
    let gone = dir.path().join("moved-away");
    let data = dir.path().join("otto-data");
    let mk = |ws: &str, cwd: &std::path::Path| Session {
        id: "s".into(),
        workspace_id: ws.into(),
        kind: SessionKind::Agent,
        provider: "claude".into(),
        title: "t".into(),
        status: SessionStatus::Exited,
        cwd: cwd.to_string_lossy().into_owned(),
        provider_session_id: Some("p".into()),
        connection_id: None,
        created_by: "u".into(),
        created_at: chrono::Utc::now(),
        last_active_at: chrono::Utc::now(),
        archived: false,
        meta: serde_json::json!({}),
    };
    let msg = missing_cwd_refusal_in(&mk("ws", &gone), &data).expect("missing folder refused");
    assert!(msg.contains("no longer exists"), "{msg}");
    assert!(!gone.exists(), "the refusal must not recreate the folder");
    assert!(missing_cwd_refusal_in(&mk("ws", dir.path()), &data).is_none());
    assert!(
        missing_cwd_refusal_in(&mk(otto_core::domain::SCRATCH_WORKSPACE_ID, &gone), &data)
            .is_none()
    );
    assert!(missing_cwd_refusal_in(&mk("ws", &data.join("db-assist/x")), &data).is_none());
}

/// S1-305: a deferred-adoption retry serializes with `ensure_live` on the
/// session's resume lock — while an open holds it, the retry waits rather
/// than deciding the holder underneath it — and re-reads the entry under
/// the lock, so a holder the open already decided is not touched again.
#[tokio::test]
async fn deferred_adoption_retry_waits_for_the_resume_lock() {
    let dir = tempfile::tempdir().unwrap();
    let pool = otto_state::open(&dir.path().join("t.db")).await.unwrap();
    let (events, _rx) = broadcast::channel(16);
    let mgr = Arc::new(SessionManager::new(
        SessionsRepo::new(pool),
        events,
        ProviderRegistry::new(None),
    ));
    let sid: Id = "deferred-1".into();
    mgr.deferred_holders
        .insert(sid.clone(), dir.path().join("gone.sock"));
    let lock = mgr.resume_lock(&sid);
    let guard = lock.lock().await;
    let retry = tokio::spawn({
        let mgr = Arc::clone(&mgr);
        async move { mgr.retry_deferred_adoptions().await }
    });
    tokio::time::sleep(Duration::from_millis(150)).await;
    assert!(!retry.is_finished(), "the retry must wait for the open");
    assert!(
        mgr.deferred_holders.contains_key(&sid),
        "nothing decided while the open holds the lock"
    );
    // The open decided it (e.g. adopted the holder) and dropped the entry.
    mgr.deferred_holders.remove(&sid);
    drop(guard);
    assert!(retry.await.unwrap().is_empty());
}

/// S1-306: the "Otto-managed, recreate on spawn" exemption follows the
/// CONFIGURED data dir — a daemon on `$OTTO_DATA_DIR` (e2e, dev, custom
/// installs) still recreates a cleaned workflow/db-assist scratch dir.
#[tokio::test]
async fn missing_cwd_exemption_uses_the_configured_data_dir() {
    let dir = tempfile::tempdir().unwrap();
    let pool = otto_state::open(&dir.path().join("t.db")).await.unwrap();
    let (events, _rx) = broadcast::channel(16);
    let custom = dir.path().join("custom-data");
    let mgr = SessionManager::new(SessionsRepo::new(pool), events, ProviderRegistry::new(None))
        .with_data_dir(custom.clone());
    let mk = |cwd: &std::path::Path| Session {
        id: "s".into(),
        workspace_id: "ws".into(),
        kind: SessionKind::Agent,
        provider: "claude".into(),
        title: "t".into(),
        status: SessionStatus::Exited,
        cwd: cwd.to_string_lossy().into_owned(),
        provider_session_id: Some("p".into()),
        connection_id: None,
        created_by: "u".into(),
        created_at: chrono::Utc::now(),
        last_active_at: chrono::Utc::now(),
        archived: false,
        meta: serde_json::json!({}),
    };
    assert!(
        mgr.missing_cwd_refusal(&mk(&custom.join("workflow-runs/r1")))
            .is_none(),
        "an Otto-managed dir under the configured data dir is recreated"
    );
    assert!(
        mgr.missing_cwd_refusal(&mk(&dir.path().join("user-repo")))
            .is_some(),
        "a user's moved repo is still refused"
    );
}
use otto_state::NewSession;

/// r3-01-09: the process table is only read when a sweep needs it, and
/// then once per sweep however many sessions consult it.
#[tokio::test]
async fn process_table_is_taken_lazily_and_once() {
    let mut t = LazyProcTable::default();
    assert!(!t.taken(), "a sweep with no candidate never runs ps");
    let first = t.get().await.len();
    assert!(t.taken());
    assert!(first > 0, "ps lists at least this test process");
    assert_eq!(t.get().await.len(), first, "same snapshot reused");
}

#[test]
fn capture_scan_backs_off_after_the_first_input() {
    assert_eq!(
        capture_scan_pause(Duration::from_secs(1)),
        Duration::from_secs(1)
    );
    assert_eq!(
        capture_scan_pause(Duration::from_secs(30)),
        Duration::from_secs(2)
    );
    assert_eq!(
        capture_scan_pause(Duration::from_secs(120)),
        Duration::from_secs(5)
    );
}

#[test]
fn cpu_verdict_is_the_pure_descendant_cpu_rule() {
    assert_eq!(
        cpu_verdict(Some(1_000), 1_500),
        CpuVerdict::Growing {
            prev: 1_000,
            now: 1_500
        }
    );
    assert_eq!(cpu_verdict(Some(1_000), 1_200), CpuVerdict::Idle);
    assert_eq!(cpu_verdict(None, 5), CpuVerdict::Baselining);
    assert_eq!(cpu_verdict(None, 0), CpuVerdict::Idle);
}

/// Review S1-07: the cap pass re-sampled the same cached table after the
/// idle pass had advanced the baseline, read `prev == now`, and suspended
/// a session the idle pass had just held for descendant CPU. A sweep's
/// verdict is now taken once and shared.
#[tokio::test]
async fn sweep_cpu_verdict_is_shared_between_the_idle_and_cap_passes() {
    let root = std::process::id();
    let id: Id = new_id();
    let baselines: DashMap<Id, u64> = DashMap::new();
    // A table where `root` has a busy child.
    let mut sweep = SweepCpu {
        table: LazyProcTable(Some(vec![(root, 1, 10), (root + 1, root, 5_000)])),
        ..Default::default()
    };
    baselines.insert(id.clone(), 1_000);
    let first = sweep.verdict(&baselines, &id, root).await;
    assert_eq!(
        first,
        CpuVerdict::Growing {
            prev: 1_000,
            now: 5_000
        }
    );
    // The baseline advanced, but the cap pass sees the SAME verdict.
    assert_eq!(baselines.get(&id).map(|v| *v), Some(5_000));
    assert_eq!(sweep.verdict(&baselines, &id, root).await, first);
    // The next sweep compares against the advanced baseline.
    let mut next = SweepCpu {
        table: LazyProcTable(Some(vec![(root, 1, 10), (root + 1, root, 5_050)])),
        ..Default::default()
    };
    assert_eq!(next.verdict(&baselines, &id, root).await, CpuVerdict::Idle);
}

#[test]
fn codex_creds_preserve_session_source_for_mcp_policy() {
    let session_id = new_id();
    let path = write_codex_creds(
        &session_id,
        "token",
        "http://127.0.0.1:7700",
        "workspace",
        Some("vault-docs-review"),
    )
    .unwrap();
    let value: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(value["source"], serde_json::json!("vault-docs-review"));
    let _ = std::fs::remove_file(path);
}

#[test]
fn env_based_mcp_providers_preserve_reviewer_source() {
    let mut session = Session {
        id: new_id(),
        workspace_id: new_id(),
        kind: SessionKind::Agent,
        provider: "claude".into(),
        title: "reviewer".into(),
        status: SessionStatus::Idle,
        cwd: "/tmp".into(),
        provider_session_id: None,
        connection_id: None,
        created_by: new_id(),
        created_at: chrono::Utc::now(),
        last_active_at: chrono::Utc::now(),
        archived: false,
        meta: serde_json::json!({"source": "vault-docs-review"}),
    };
    for provider in ["claude", "agy", "grok"] {
        session.provider = provider.into();
        let env = otto_tools_env(&session, "token", "http://127.0.0.1:7700");
        assert_eq!(
            env.get("OTTO_SESSION_SOURCE").map(String::as_str),
            Some("vault-docs-review"),
            "provider {provider} lost the reviewer MCP policy source"
        );
    }
    session.meta = serde_json::json!({"source": "vault-docs"});
    assert_eq!(
        otto_tools_env(&session, "token", "base")
            .get("OTTO_SESSION_SOURCE")
            .map(String::as_str),
        Some("vault-docs")
    );
}

fn agent_session(provider: &str, status: SessionStatus, meta: serde_json::Value) -> Session {
    Session {
        id: new_id(),
        workspace_id: new_id(),
        kind: SessionKind::Agent,
        provider: provider.into(),
        title: "Messi".into(),
        status,
        cwd: "/tmp".into(),
        provider_session_id: Some("019f-abc".into()),
        connection_id: None,
        created_by: new_id(),
        created_at: chrono::Utc::now(),
        last_active_at: chrono::Utc::now(),
        archived: false,
        meta,
    }
}

#[test]
fn clean_provider_title_strips_newlines_and_clips() {
    // Newlines/tabs collapse to single spaces; leading/trailing trimmed.
    assert_eq!(
        clean_provider_title("  fix the\n\tpty  redraw  bug\n"),
        Some("fix the pty redraw bug".into())
    );
    // Blank / whitespace-only yields None.
    assert_eq!(clean_provider_title("   \n\t "), None);
    assert_eq!(clean_provider_title(""), None);
    // Clipped to 60 chars + ellipsis, with no dangling space before it.
    let long = "a".repeat(200);
    let out = clean_provider_title(&long).unwrap();
    assert_eq!(out.chars().count(), PROVIDER_TITLE_MAX + 1);
    assert!(out.ends_with('…'));
    // A word boundary landing on the clip point doesn't leave "word …".
    let sentence = format!("{} zzzz", "word ".repeat(20));
    let clipped = clean_provider_title(&sentence).unwrap();
    assert!(
        !clipped.contains(" …"),
        "no space before ellipsis: {clipped:?}"
    );
}

#[test]
fn parse_claude_first_prompt_skips_meta_and_wrappers() {
    // Meta line, a slash-command wrapper, a tool_result-only turn, then the
    // real prompt (string content). The first genuine user turn wins.
    let jsonl = concat!(
        r#"{"type":"summary"}"#,
        "\n",
        r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"<system reminder>"}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"<command-name>/clear</command-name>"}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","content":"ok"}]}}"#,
        "\n",
        r#"{"type":"assistant","message":{"role":"assistant","content":"hi"}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"i have an issue\nwith the pty session"}}"#,
        "\n",
        r#"{"type":"user","message":{"role":"user","content":"a later prompt"}}"#,
    );
    assert_eq!(
        parse_claude_first_prompt(jsonl),
        Some("i have an issue with the pty session".into())
    );
}

#[test]
fn parse_claude_first_prompt_reads_array_text_parts() {
    let jsonl = r#"{"type":"user","message":{"role":"user","content":[{"type":"text","text":"refactor the parser"}]}}"#;
    assert_eq!(
        parse_claude_first_prompt(jsonl),
        Some("refactor the parser".into())
    );
}

/// The auto-namer reads only a capped head of the transcript: a prompt in
/// the head is found; a head with no prompt reports `capped` (so the probe
/// gives up for good) instead of re-reading a huge file every sweep.
#[test]
fn read_provider_title_reads_a_capped_head() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let noise = r#"{"type":"assistant","message":{"role":"assistant","content":"hi"}}"#;
    let prompt = r#"{"type":"user","message":{"role":"user","content":"name me"}}"#;
    // Prompt inside the head → found, short file → not capped.
    std::fs::write(&path, format!("{noise}\n{prompt}\n")).unwrap();
    let read = read_provider_title("claude", &path);
    assert_eq!(read.title.as_deref(), Some("name me"));
    assert!(!read.capped);
    // No prompt yet in a short file → not capped (retry later).
    std::fs::write(&path, format!("{noise}\n")).unwrap();
    let read = read_provider_title("claude", &path);
    assert!(read.title.is_none() && !read.capped);
    // Prompt only past the line cap → never found, capped → give up.
    let mut body = format!("{noise}\n").repeat(PROVIDER_TITLE_SCAN_LINES);
    body.push_str(prompt);
    body.push('\n');
    std::fs::write(&path, &body).unwrap();
    let read = read_provider_title("claude", &path);
    assert!(read.title.is_none() && read.capped);
    // One giant line past the byte cap (invalid UTF-8 included) → capped,
    // read lossily, never the whole file.
    let mut big = vec![b'x'; (PROVIDER_TITLE_SCAN_BYTES + 1024) as usize];
    big[10] = 0xff;
    std::fs::write(&path, &big).unwrap();
    let read = read_provider_title("claude", &path);
    assert!(read.title.is_none() && read.capped);
    // Unreadable file → nothing, not capped.
    let read = read_provider_title("claude", &dir.path().join("missing"));
    assert!(read.title.is_none() && !read.capped);
}

#[test]
fn parse_claude_first_prompt_none_when_no_user_turn() {
    let jsonl = r#"{"type":"assistant","message":{"role":"assistant","content":"hi"}}"#;
    assert_eq!(parse_claude_first_prompt(jsonl), None);
}

#[test]
fn parse_codex_first_prompt_prefers_user_message_event() {
    // session_meta, the injected AGENTS.md as a response_item user turn, then
    // the real typed prompt as an event_msg/user_message. The event wins.
    let jsonl = concat!(
        r#"{"type":"session_meta","payload":{"id":"019f","cwd":"/x"}}"#,
        "\n",
        r##"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions\nblah"}]}}"##,
        "\n",
        r#"{"type":"event_msg","payload":{"type":"user_message","message":"scan the repo into the vault"}}"#,
    );
    assert_eq!(
        parse_codex_first_prompt(jsonl),
        Some("scan the repo into the vault".into())
    );
}

#[test]
fn parse_codex_first_prompt_falls_back_to_input_text() {
    // No user_message event (older rollout): the first non-wrapper user
    // input_text is used, and the AGENTS.md wrapper before it is skipped.
    let jsonl = concat!(
        r#"{"type":"session_meta","payload":{"id":"019f","cwd":"/x"}}"#,
        "\n",
        r##"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"# AGENTS.md instructions"}]}}"##,
        "\n",
        r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"add a retry to the client"}]}}"#,
    );
    assert_eq!(
        parse_codex_first_prompt(jsonl),
        Some("add a retry to the client".into())
    );
}

#[test]
fn title_eligible_guards_ownership_and_lifecycle() {
    // Theme-named live claude session → eligible.
    assert!(title_eligible(&agent_session(
        "claude",
        SessionStatus::Idle,
        serde_json::json!({ "title_source": "theme" }),
    )));
    // "auto" (#N fallback) and absent title_source are also eligible.
    assert!(title_eligible(&agent_session(
        "codex",
        SessionStatus::Working,
        serde_json::json!({ "title_source": "auto" }),
    )));
    assert!(title_eligible(&agent_session(
        "claude",
        SessionStatus::Running,
        serde_json::json!({}),
    )));
    // User-owned → never touched.
    assert!(!title_eligible(&agent_session(
        "claude",
        SessionStatus::Idle,
        serde_json::json!({ "title_source": "user" }),
    )));
    // Already provider-named → nothing new to read.
    assert!(!title_eligible(&agent_session(
        "claude",
        SessionStatus::Idle,
        serde_json::json!({ "title_source": "provider" }),
    )));
    // Exited → skipped (a dead session gains no new prompt).
    assert!(!title_eligible(&agent_session(
        "claude",
        SessionStatus::Exited,
        serde_json::json!({ "title_source": "theme" }),
    )));
    // Provider without a human title source (shell/agy) → skipped.
    assert!(!title_eligible(&agent_session(
        "agy",
        SessionStatus::Idle,
        serde_json::json!({ "title_source": "theme" }),
    )));
    // Background (engine-owned) session → skipped.
    assert!(!title_eligible(&agent_session(
        "claude",
        SessionStatus::Idle,
        serde_json::json!({ "source": "review", "title_source": "theme" }),
    )));
    // No captured provider session id yet → skipped.
    let mut no_psid = agent_session("claude", SessionStatus::Idle, serde_json::json!({}));
    no_psid.provider_session_id = None;
    assert!(!title_eligible(&no_psid));
}

#[test]
fn ps_time_parses_all_shapes() {
    assert_eq!(parse_ps_time_ms("0:00.05"), Some(50));
    assert_eq!(parse_ps_time_ms("1:30.00"), Some(90_000));
    assert_eq!(
        parse_ps_time_ms("2:03:04"),
        Some(2 * 3_600_000 + 3 * 60_000 + 4_000)
    );
    assert_eq!(parse_ps_time_ms("1-00:00:01"), Some(86_400_000 + 1_000));
    assert_eq!(parse_ps_time_ms("garbage"), None);
}

#[test]
fn descendant_cpu_excludes_the_root_and_walks_depth() {
    // 100 (root, 9999ms) → 200 (10ms) → 300 (500ms); 400 unrelated.
    let table = vec![
        (100, 1, 9_999),
        (200, 100, 10),
        (300, 200, 500),
        (400, 1, 777),
    ];
    assert_eq!(descendant_cpu_ms(100, &table), 510);
    assert_eq!(descendant_cpu_ms(400, &table), 0);
    // A (bogus) cyclic table must not hang.
    let cyclic = vec![(2, 1, 5), (1, 2, 7)];
    assert_eq!(descendant_cpu_ms(1, &cyclic), 12);
}

/// A status-task tick that passed its `maps_handle` check before the
/// shutdown removed the handle must not overwrite the shutdown's `Exited`
/// (the macOS-only `persist_restart` failure: a killed engine session
/// came back `suspended` on the next boot).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_waits_for_an_in_flight_status_write() {
    let (manager, repo, workspace, user) = test_manager().await;
    let session = manager
        .create(
            &workspace,
            &user,
            CreateSessionReq {
                kind: SessionKind::Agent,
                provider: Some("shell".into()),
                title: Some("Shutdown race".into()),
                cwd: Some("/tmp".into()),
                connection_id: None,
                model: None,
                meta: None,
            },
            None,
        )
        .await
        .unwrap();
    let id = session.id.clone();
    // Stand in for the tick: hold the session lock as it does across its
    // check-then-write.
    let lock = manager.resume_lock(&id);
    let guard = lock.lock().await;
    let shutdown = tokio::spawn({
        let manager = Arc::clone(&manager);
        async move { manager.shutdown_all().await }
    });
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        !shutdown.is_finished(),
        "shutdown must wait for the in-flight status write"
    );
    repo.update_status(&id, SessionStatus::Working)
        .await
        .unwrap();
    drop(guard);
    assert_eq!(shutdown.await.unwrap(), 1);
    assert_eq!(repo.get(&id).await.unwrap().status, SessionStatus::Exited);
    assert_eq!(manager.live_count(), 0);
}

/// Review S1-15: resumes stuck holding their locks used to stall the
/// serial shutdown past the daemon's 12 s teardown budget, leaving later
/// sessions neither killed nor marked exited. The lock waits are bounded
/// now, and a restart after shutdown began refuses to spawn.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn shutdown_retires_every_session_within_its_budget_despite_held_locks() {
    let (manager, repo, workspace, user) = test_manager().await;
    let mut ids = Vec::new();
    for n in 0..4 {
        let s = manager
            .create(
                &workspace,
                &user,
                CreateSessionReq {
                    kind: SessionKind::Agent,
                    provider: Some("shell".into()),
                    title: Some(format!("Held {n}")),
                    cwd: Some("/tmp".into()),
                    connection_id: None,
                    model: None,
                    meta: None,
                },
                None,
            )
            .await
            .unwrap();
        ids.push(s.id);
    }
    // Every session looks mid-resume: its lock is held and never freed.
    let locks: Vec<_> = ids.iter().map(|id| manager.resume_lock(id)).collect();
    let mut guards = Vec::new();
    for lock in &locks {
        guards.push(lock.lock().await);
    }
    let started = std::time::Instant::now();
    assert_eq!(manager.shutdown_all().await, ids.len());
    assert!(
        started.elapsed() < SHUTDOWN_LOCK_BUDGET + Duration::from_secs(3),
        "shutdown took {:?}",
        started.elapsed()
    );
    for id in &ids {
        assert_eq!(repo.get(id).await.unwrap().status, SessionStatus::Exited);
    }
    // The app closing is not the daemon exiting: later resumes must work.
    assert!(
        !manager
            .shutting_down
            .load(std::sync::atomic::Ordering::SeqCst),
        "an app-close kill sweep must not leave spawns refused"
    );
    assert_eq!(manager.live_count(), 0);
    drop(guards);
    // A resume that reaches its spawn after the DAEMON's shutdown began
    // must not spawn.
    let _ = manager.shutdown_for_restart().await;
    let spec = CommandSpec {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), "exec /bin/sleep 60".into()],
        cwd: Some("/tmp".into()),
        env: vec![],
    };
    let err = manager
        .restart(&ids[0], Some(spec))
        .await
        .expect_err("shutting down");
    assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    assert_eq!(manager.live_count(), 0);
}

#[path = "agent_cwd.rs"]
mod agent_cwd;
#[path = "archive.rs"]
mod archive;
#[path = "grid.rs"]
mod grid;
#[path = "submit.rs"]
mod submit;

async fn test_manager() -> (Arc<SessionManager>, SessionsRepo, Workspace, Id) {
    // A migrated on-disk sqlite (in a tempdir) via otto-state's opener.
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("test.db");
    let pool = otto_state::open(&db).await.unwrap();
    // Keep the tempdir alive for the whole process (leak is fine in tests).
    std::mem::forget(dir);

    let user = new_id();
    let ws_id = new_id();
    let now = chrono::Utc::now().to_rfc3339();
    sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, ?, ?, ?, 0, ?)")
        .bind(&user).bind("u").bind("x").bind("U").bind(&now)
        .execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)")
        .bind(&ws_id)
        .bind("w")
        .bind("/tmp")
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();

    let auth = AuthRepo::new(pool.clone());
    let repo = SessionsRepo::new(pool);
    let (events, _rx) = broadcast::channel(16);
    let providers = ProviderRegistry::new(None);
    let mgr = Arc::new(SessionManager::new(repo.clone(), events, providers).with_auth_repo(auth));
    let ws = Workspace {
        id: ws_id,
        name: "w".into(),
        root_path: "/tmp".into(),
        settings: serde_json::json!({}),
        archived: false,
        created_at: chrono::Utc::now(),
    };
    (mgr, repo, ws, user)
}

#[tokio::test]
async fn capture_waiting_for_lifecycle_preserves_late_assigned_id() {
    let (mut manager, repo, workspace, user) = test_manager().await;
    let dir = tempfile::tempdir().unwrap();
    let profiles = otto_state::provider_accounts::ProviderAccountsRepo::new(repo.pool());
    let profile = profiles
        .create(&user, "codex", "Late capture fixture")
        .await
        .unwrap();
    Arc::get_mut(&mut manager).unwrap().provider_accounts =
        Some((profiles, dir.path().join("profiles")));
    let cwd = dir
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let session = repo
        .create(NewSession {
            workspace_id: workspace.id.clone(),
            kind: SessionKind::Agent,
            provider: "codex".into(),
            title: "Late capture fixture".into(),
            cwd: cwd.clone(),
            provider_session_id: None,
            connection_id: None,
            created_by: user,
            meta: serde_json::json!({"account_id": profile.id}),
        })
        .await
        .unwrap();
    let day = manager
        .codex_root_for(&session)
        .join(chrono::Utc::now().format("%Y/%m/%d").to_string());
    write_rollout(
        &day,
        "other.jsonl",
        "OTHER-CONVERSATION",
        &cwd,
        "codex-tui",
        "user",
    );
    let lifecycle = manager.resume_lock(&session.id);
    let held = lifecycle.lock().await;
    manager.spawn_session_id_capture(&session);
    {
        let mut probe = manager.capture_probes.get_mut(&session.id).unwrap();
        probe.first_at = Some(std::time::SystemTime::now());
        probe.raw = b"my original conversation".to_vec();
    }
    // Same operation as ensure_live's late-capture commit, under its
    // lifecycle lock. The old capture still owns the original probe.
    repo.set_provider_session(&session.id, "LATE-ASSIGNED")
        .await
        .unwrap();
    drop(held);
    tokio::time::timeout(Duration::from_secs(5), async {
        while manager.captures_in_flight.contains_key(&cwd) {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("capture retires after observing an existing claim");
    assert_eq!(
        repo.get(&session.id)
            .await
            .unwrap()
            .provider_session_id
            .as_deref(),
        Some("LATE-ASSIGNED")
    );
    assert!(!manager.capture_probes.contains_key(&session.id));
    manager.remove(&session.id).await.unwrap();
}

#[tokio::test]
async fn late_capture_serializes_cross_session_claim_and_fails_closed() {
    let (mut manager, repo, workspace, user) = test_manager().await;
    let dir = tempfile::tempdir().unwrap();
    let profiles = otto_state::provider_accounts::ProviderAccountsRepo::new(repo.pool());
    let profile = profiles
        .create(&user, "codex", "Late capture contention fixture")
        .await
        .unwrap();
    Arc::get_mut(&mut manager).unwrap().provider_accounts =
        Some((profiles, dir.path().join("profiles")));
    let cwd = dir
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let new_codex = |title: &str| NewSession {
        workspace_id: workspace.id.clone(),
        kind: SessionKind::Agent,
        provider: "codex".into(),
        title: title.into(),
        cwd: cwd.clone(),
        provider_session_id: None,
        connection_id: None,
        created_by: user.clone(),
        meta: serde_json::json!({"account_id": profile.id}),
    };
    let s1 = repo.create(new_codex("Late capture A")).await.unwrap();
    let s2 = repo.create(new_codex("Late capture B")).await.unwrap();
    let day = manager
        .codex_root_for(&s1)
        .join(chrono::Utc::now().format("%Y/%m/%d").to_string());
    write_rollout(
        &day,
        "rollout-2026-10-18T12-00-00-SOLE-ROLLOUT-1.jsonl",
        "SOLE-ROLLOUT-1",
        &cwd,
        "codex-tui",
        "user",
    );

    // Hold the global capture lock while both sessions acquire their own
    // resume_lock (mirroring ensure_live's resume_lock -> codex_capture_lock
    // order) so both contend concurrently for the sole rollout on release.
    let gate = manager.codex_capture_lock.lock().await;
    let t1 = tokio::spawn({
        let (mgr, s1) = (Arc::clone(&manager), s1.clone());
        async move {
            let lock = mgr.resume_lock(&s1.id);
            let _resume = lock.lock().await;
            mgr.late_capture_provider_id(&s1).await
        }
    });
    let t2 = tokio::spawn({
        let (mgr, s2) = (Arc::clone(&manager), s2.clone());
        async move {
            let lock = mgr.resume_lock(&s2.id);
            let _resume = lock.lock().await;
            mgr.late_capture_provider_id(&s2).await
        }
    });
    for _ in 0..8 {
        tokio::task::yield_now().await;
    }
    drop(gate);

    let (r1, r2) = tokio::join!(t1, t2);
    let (r1, r2) = (r1.unwrap(), r2.unwrap());
    assert_ne!(
        r1.is_some(),
        r2.is_some(),
        "exactly one session may claim the sole rollout: r1={r1:?}, r2={r2:?}"
    );
    assert_eq!(
        r1.as_deref().or(r2.as_deref()),
        Some("SOLE-ROLLOUT-1"),
        "winner must claim SOLE-ROLLOUT-1"
    );

    let row1 = repo.get(&s1.id).await.unwrap();
    let row2 = repo.get(&s2.id).await.unwrap();
    assert_eq!(row1.provider_session_id, r1);
    assert_eq!(row2.provider_session_id, r2);
    let claimed = repo.provider_session_ids().await.unwrap();
    assert_eq!(claimed, vec!["SOLE-ROLLOUT-1".to_string()]);

    manager.remove(&s1.id).await.unwrap();
    manager.remove(&s2.id).await.unwrap();

    // Fail-closed check: if claimed-ID lookup fails (e.g. closed DB pool),
    // late_capture_provider_id must return None rather than treating claimed
    // as empty via unwrap_or_default.
    let s3 = repo.create(new_codex("Late capture C")).await.unwrap();
    repo.pool().close().await;
    assert_eq!(
        manager.late_capture_provider_id(&s3).await,
        None,
        "claimed-id lookup error must fail closed"
    );
}

#[tokio::test]
async fn delayed_predecessor_grid_cannot_overwrite_replacement() {
    let (manager, repo, workspace, user) = test_manager().await;
    let id = seed_session(&repo, &workspace, &user, None).await;
    let spec = CommandSpec {
        program: "/bin/cat".into(),
        args: vec![],
        cwd: Some("/".into()),
        env: vec![],
    };
    let old = Arc::new(PtyHandle::spawn_sized(&spec, 90, 25).unwrap());
    manager.live.insert(id.clone(), old.clone());
    manager.persist_grid(&id, &old).await.unwrap();
    let replacement = Arc::new(PtyHandle::spawn_sized(&spec, 150, 45).unwrap());
    manager.live.insert(id.clone(), replacement.clone());
    manager.persist_grid(&id, &replacement).await.unwrap();
    // Force the exact scheduling order: old-generation persistence runs
    // only AFTER the replacement's saved size has committed.
    manager.persist_grid(&id, &old).await.unwrap();
    let meta = repo.get(&id).await.unwrap().meta;
    assert_eq!(meta["pty_cols"], 150);
    assert_eq!(meta["pty_rows"], 45);
    let _ = old.kill();
    manager.remove(&id).await.unwrap();
}

#[tokio::test]
async fn superseded_capture_task_retires_without_touching_successor() {
    let (manager, repo, workspace, user) = test_manager().await;
    let dir = tempfile::tempdir().unwrap();
    let session = repo
        .create(NewSession {
            workspace_id: workspace.id.clone(),
            kind: SessionKind::Agent,
            provider: "codex".into(),
            title: "Capture generation fixture".into(),
            cwd: dir.path().to_string_lossy().into_owned(),
            provider_session_id: None,
            connection_id: None,
            created_by: user,
            meta: serde_json::json!({}),
        })
        .await
        .unwrap();
    manager.spawn_session_id_capture(&session);
    let old = manager
        .capture_probes
        .get(&session.id)
        .unwrap()
        .generation
        .clone();
    manager.spawn_session_id_capture(&session);
    let new = manager
        .capture_probes
        .get(&session.id)
        .unwrap()
        .generation
        .clone();
    assert!(!Arc::ptr_eq(&old, &new));
    let cwd = std::fs::canonicalize(dir.path())
        .unwrap()
        .to_string_lossy()
        .into_owned();
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if manager
                .captures_in_flight
                .get(&cwd)
                .is_some_and(|n| *n == 1)
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .expect("superseded capture should stop, not consume the successor probe");
    assert!(Arc::ptr_eq(
        &manager.capture_probes.get(&session.id).unwrap().generation,
        &new
    ));
    assert!(repo
        .get(&session.id)
        .await
        .unwrap()
        .provider_session_id
        .is_none());
    manager.remove(&session.id).await.unwrap();
}

#[tokio::test]
async fn fresh_restart_replaces_expired_capture_probe() {
    let (mut manager, repo, workspace, user) = test_manager().await;
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let program = dir.path().join("fake-codex");
    std::fs::write(&program, "#!/bin/sh\nexec /bin/sleep 60\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let profiles = otto_state::provider_accounts::ProviderAccountsRepo::new(repo.pool());
    let profile = profiles
        .create(&user, "codex", "Capture fixture")
        .await
        .unwrap();
    Arc::get_mut(&mut manager).unwrap().provider_accounts =
        Some((profiles, dir.path().join("profiles")));
    let session = repo
        .create(NewSession {
            workspace_id: workspace.id.clone(),
            kind: SessionKind::Agent,
            provider: "codex".into(),
            title: "Capture restart fixture".into(),
            cwd: dir.path().to_string_lossy().into_owned(),
            provider_session_id: None,
            connection_id: None,
            created_by: user,
            meta: serde_json::json!({"account_id":profile.id}),
        })
        .await
        .unwrap();
    // A missed capture retains its first prompt for late capture. A fresh
    // process must start with an empty probe, not match that old prompt.
    let stale = CaptureProbe {
        first_at: Some(std::time::SystemTime::UNIX_EPOCH),
        raw: b"previous process prompt".to_vec(),
        ..Default::default()
    };
    let generation = Arc::clone(&stale.generation);
    manager.capture_probes.insert(session.id.clone(), stale);
    manager
        .restart(
            &session.id,
            Some(CommandSpec {
                program: program.to_string_lossy().into_owned(),
                args: vec![],
                cwd: Some(session.cwd.clone()),
                env: vec![],
            }),
        )
        .await
        .unwrap();
    {
        let probe = manager.capture_probes.get(&session.id).unwrap();
        assert!(!Arc::ptr_eq(&generation, &probe.generation));
        assert!(probe.first_at.is_none());
        assert!(probe.raw.is_empty());
    }
    manager.remove(&session.id).await.unwrap();
}

/// Exercise actual restart argv and persistence with a local fake Claude.
/// The first restart has no provider ID; the second must resume its UUID
/// while retaining model and lean-turn options on both launches.
#[tokio::test]
async fn fresh_restart_retains_uuid_and_launch_options() {
    let (mut manager, repo, workspace, user) = test_manager().await;
    let dir = tempfile::tempdir().unwrap();
    let profiles = otto_state::provider_accounts::ProviderAccountsRepo::new(repo.pool());
    let profile = profiles
        .create(&user, "claude", "Restart fixture")
        .await
        .unwrap();
    Arc::get_mut(&mut manager).unwrap().provider_accounts =
        Some((profiles, dir.path().join("profiles")));
    let output = dir.path().join("argv");
    let script = "printf '%s\\n' \"$@\" > argv; exec /bin/sleep 60";
    manager.providers.reload(Some(&serde_json::json!({
        "claude": {
            "cmd": "/bin/sh",
            "args": ["-c", script, "fake-claude", "--session-id", "{sid}"],
            "resume_args": ["-c", script, "fake-claude", "--resume", "{sid}"],
            "model_args": ["--model", "{model}"]
        }
    })));
    let session = repo.create(NewSession {
        workspace_id: workspace.id.clone(),
        kind: SessionKind::Agent,
        provider: "claude".into(),
        title: "Fresh restart fixture".into(),
        cwd: dir.path().to_string_lossy().into_owned(),
        provider_session_id: None,
        connection_id: None,
        created_by: user,
        meta: serde_json::json!({"lean_turn":true,"model":"fixture-model","account_id":profile.id}),
    }).await.unwrap();
    let mut expected_id = None;
    for flag in ["--session-id", "--resume"] {
        let restarted = manager.restart(&session.id, None).await.unwrap();
        let sid = restarted.provider_session_id.unwrap();
        assert!(
            uuid::Uuid::parse_str(&sid).is_ok(),
            "provider requires a UUID"
        );
        if let Some(expected) = &expected_id {
            assert_eq!(&sid, expected, "restart must retain the conversation");
        }
        expected_id = Some(sid.clone());
        let argv = tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                if let Ok(args) = std::fs::read_to_string(&output) {
                    if args.starts_with(flag) {
                        break args;
                    }
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .unwrap();
        let args: Vec<_> = argv.lines().collect();
        assert_eq!(args[0], flag);
        assert_eq!(args[1], sid);
        assert!(args.windows(2).any(|a| a == ["--model", "fixture-model"]));
        assert!(args.contains(&"--strict-mcp-config"));
        assert!(args.contains(&"--disallowed-tools"));
    }
    manager.remove(&session.id).await.unwrap();
}

#[tokio::test]
async fn workspace_context_refreshes_on_create_and_restart_including_reviews() {
    #[derive(Default)]
    struct RecordingHook(std::sync::Mutex<Vec<serde_json::Value>>);
    impl PreSpawnHook for RecordingHook {
        fn before_spawn(
            &self,
            ws: &Workspace,
            _cwd: &str,
            _provider: &str,
        ) -> otto_core::hooks::SpawnInjection {
            self.0.lock().unwrap().push(ws.settings.clone());
            otto_core::hooks::SpawnInjection::default()
        }
        fn resume_injection(
            &self,
            _cwd: &str,
            _provider: &str,
        ) -> otto_core::hooks::SpawnInjection {
            panic!("restart must refresh workspace context, not reuse a stale bundle")
        }
    }
    let (mut manager, repo, mut workspace, user) = test_manager().await;
    let dir = tempfile::tempdir().unwrap();
    workspace.root_path = dir.path().to_string_lossy().into_owned();
    let workspaces = otto_state::WorkspacesRepo::new(repo.pool());
    workspaces
        .update(&workspace.id, None, Some(&workspace.root_path), None, None)
        .await
        .unwrap();
    let first =
        serde_json::from_value(serde_json::json!({"context_version":0,"memory_md":"before"}))
            .unwrap();
    workspace = workspaces
        .update_context(&workspace.id, &first)
        .await
        .unwrap();
    let hook = Arc::new(RecordingHook::default());
    Arc::get_mut(&mut manager).unwrap().pre_spawn_hook = Some(hook.clone());
    let spec = || CommandSpec {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), "exec /bin/sleep 60".into()],
        cwd: Some(workspace.root_path.clone()),
        env: vec![],
    };
    let mut sessions = Vec::new();
    for meta in [
        serde_json::json!({}),
        serde_json::json!({"source":"review"}),
    ] {
        let request = CreateSessionReq {
            kind: SessionKind::Agent,
            provider: Some("shell".into()),
            title: Some("Context fixture".into()),
            cwd: Some(workspace.root_path.clone()),
            connection_id: None,
            model: None,
            meta: Some(meta),
        };
        sessions.push(
            manager
                .create(&workspace, &user, request, Some(spec()))
                .await
                .unwrap(),
        );
    }
    let next = serde_json::from_value(serde_json::json!({"context_version":1,"memory_md":"after"}))
        .unwrap();
    workspaces
        .update_context(&workspace.id, &next)
        .await
        .unwrap();
    for session in &sessions {
        manager.restart(&session.id, Some(spec())).await.unwrap();
        manager.remove(&session.id).await.unwrap();
    }
    let calls = hook.0.lock().unwrap();
    assert_eq!(calls.len(), 4);
    assert_eq!(calls[0]["context"]["memory_md"], "before");
    assert_eq!(calls[1]["context"]["memory_md"], "before");
    assert_eq!(calls[2]["context"]["memory_md"], "after");
    assert_eq!(calls[3]["context"]["memory_md"], "after");
}

#[tokio::test]
async fn network_prepare_failure_removes_new_row_and_preserves_existing_pty() {
    let (manager, repo, workspace, user) = test_manager().await;
    let pool = repo.pool();
    sqlx::query("UPDATE users SET is_root=1 WHERE id=?")
        .bind(&user)
        .execute(&pool)
        .await
        .unwrap();
    sqlx::query("INSERT INTO connections(id,workspace_id,name,kind,params_json,created_by,created_at) VALUES('network-fixture',?,'Invalid host fixture','ssh','{}',?,'2026-01-01T00:00:00Z')").bind(&workspace.id).bind(&user).execute(&pool).await.unwrap();
    let profile=otto_state::network_profiles::NetworkProfilesRepo::new(pool).create(&workspace.id,&user,
        serde_json::from_value(serde_json::json!({"name":"Broken network","ssh_connection_id":"network-fixture","endpoints":[{"name":"db","remote_host":"internal","remote_port":5432}]})).unwrap()).await.unwrap();
    let meta = serde_json::json!({"network_profile_id":profile.id});
    let request = CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some("shell".into()),
        title: Some("Network fixture".into()),
        cwd: Some("/tmp".into()),
        connection_id: None,
        model: None,
        meta: Some(meta.clone()),
    };
    let error = manager
        .create(&workspace, &user, request, None)
        .await
        .unwrap_err();
    assert!(error.to_string().contains("valid host"));
    assert!(repo
        .list_by_workspace(&workspace.id)
        .await
        .unwrap()
        .is_empty());
    assert_eq!(manager.live_count(), 0);
    let session = repo
        .create(NewSession {
            workspace_id: workspace.id.clone(),
            kind: SessionKind::Agent,
            provider: "shell".into(),
            title: "Existing fixture".into(),
            cwd: "/tmp".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: user,
            meta,
        })
        .await
        .unwrap();
    let handle = Arc::new(
        PtyHandle::spawn(&CommandSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 10".into()],
            cwd: Some("/tmp".into()),
            env: vec![],
        })
        .unwrap(),
    );
    manager.live.insert(session.id.clone(), handle.clone());
    assert!(manager.restart(&session.id, None).await.is_err());
    assert!(Arc::ptr_eq(
        &manager.live_handle(&session.id).unwrap(),
        &handle
    ));
    assert!(handle.on_exit().borrow().is_none());
    manager.kill_session(&session.id).await.unwrap();
}

/// A restart that retires the old process and then fails to spawn the new
/// one must not leave the row `running` with nothing live: it becomes
/// `Reconnectable` (stamped `meta.suspended`), and the old child is dead.
#[tokio::test]
async fn failed_respawn_after_retiring_marks_session_reconnectable() {
    let (manager, repo, workspace, user) = test_manager().await;
    let session = repo
        .create(NewSession {
            workspace_id: workspace.id.clone(),
            kind: SessionKind::Agent,
            provider: "shell".into(),
            title: "Respawn fixture".into(),
            cwd: "/tmp".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: user,
            meta: serde_json::json!({}),
        })
        .await
        .unwrap();
    let old = Arc::new(
        PtyHandle::spawn(&CommandSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "exec sleep 30".into()],
            cwd: Some("/tmp".into()),
            env: vec![],
        })
        .unwrap(),
    );
    manager.live.insert(session.id.clone(), Arc::clone(&old));
    repo.update_status(&session.id, SessionStatus::Running)
        .await
        .unwrap();
    let broken = CommandSpec {
        program: "/nonexistent/otto-respawn-fixture".into(),
        args: vec![],
        cwd: Some("/tmp".into()),
        env: vec![],
    };
    assert!(manager.restart(&session.id, Some(broken)).await.is_err());
    assert!(!manager.is_live(&session.id));
    let row = repo.get(&session.id).await.unwrap();
    assert_eq!(row.status, SessionStatus::Reconnectable);
    // Stamped as a failed restart with its cause — not "released to free
    // memory" (error-handling-4-05).
    let suspended = &row.meta["suspended"];
    assert_eq!(suspended["reason"], SUSPEND_REASON_RESPAWN_FAILED);
    assert!(
        suspended["error"].as_str().is_some_and(|e| !e.is_empty()),
        "{suspended}"
    );
    // The retired child was waited for (bounded) before the respawn.
    assert!(old.on_exit().borrow().is_some(), "old child must be dead");
}

#[tokio::test]
async fn lifecycle_mutations_wait_for_in_flight_restart() {
    for operation in ["archive", "kill", "remove", "suspend", "unarchive"] {
        let (mgr, repo, ws, user) = test_manager().await;
        let id = seed_session(&repo, &ws, &user, None).await;
        // A restart owns this lock through setup/spawn. No terminal
        // mutation may publish its outcome while that spawn is pending.
        let lock = mgr.resume_lock(&id);
        let guard = lock.lock().await;
        let mut mutation = Box::pin(async {
            match operation {
                "archive" => mgr.archive(&id).await.map(|_| ()),
                "kill" => mgr.kill_session(&id).await,
                "remove" => mgr.remove(&id).await,
                "suspend" => mgr.suspend(&id).await,
                _ => mgr.unarchive(&id).await.map(|_| ()),
            }
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(100), &mut mutation)
                .await
                .is_err(),
            "{operation} crossed an in-flight restart"
        );
        drop(guard);
        mutation.await.unwrap();
    }
}

/// The plaintext credential files a Codex spawn leaves on disk for `id`: the
/// MCP bearer-token file plus two user-server env files (Keychain secrets
/// included). Also writes ONE env file for a neighbour session whose id
/// extends `id` (`{id}0`), so a matcher that loses its `.` separator would
/// delete it too. Returns (`id`'s three files, the neighbour's file).
fn seed_credential_files(id: &Id) -> (Vec<std::path::PathBuf>, std::path::PathBuf) {
    let env = std::collections::BTreeMap::from([("API_KEY".to_string(), "s3cret".to_string())]);
    let creds = write_codex_creds(id, "tok", "http://127.0.0.1:7700", "ws", None).unwrap();
    let mut own = vec![creds];
    for server in ["a", "b"] {
        let path = codex_server_env_path(id, server);
        crate::mcp::write_env_file(&path, &env).unwrap();
        own.push(path);
    }
    let neighbour = codex_server_env_path(&format!("{id}0"), "a");
    crate::mcp::write_env_file(&neighbour, &env).unwrap();
    for f in own.iter().chain([&neighbour]) {
        assert!(f.exists(), "fixture {} written", f.display());
    }
    (own, neighbour)
}

/// `id`'s credential files are gone; the neighbour session's file survives
/// (then is cleaned up — it lives in the shared temp dir).
fn assert_credential_files_removed(
    op: &str,
    own: &[std::path::PathBuf],
    neighbour: &std::path::Path,
) {
    for f in own {
        assert!(!f.exists(), "{op} left {} on disk", f.display());
    }
    assert!(
        neighbour.exists(),
        "{op} deleted another session's env file {}",
        neighbour.display()
    );
    let _ = std::fs::remove_file(neighbour);
}

#[tokio::test]
async fn removing_session_revokes_persisted_tokens_without_in_memory_mapping() {
    let (mgr, repo, ws, user) = test_manager().await;
    let id = seed_session(&repo, &ws, &user, None).await;
    let auth = mgr.auth.as_ref().unwrap();
    let (token, _) = auth.issue_session_api_token(&user, &id).await.unwrap();
    let (own, neighbour) = seed_credential_files(&id);
    assert!(mgr.mcp_tokens.is_empty()); // As after a daemon restart.
    assert!(auth.authenticate(&token).await.is_ok());
    mgr.remove(&id).await.unwrap();
    assert!(auth.authenticate(&token).await.is_err());
    assert!(auth.list_api_tokens(&user).await.unwrap().is_empty());
    assert_credential_files_removed("remove", &own, &neighbour);
}

/// A retired process's MCP credential dies with it (it used to stay valid,
/// with the owner's full permissions, for its 10-year TTL while the row
/// lingered): archive / suspend / kill revoke it — and delete its plaintext
/// creds / env files, sparing other sessions' — the session row stays, and
/// a resume mints a fresh one.
#[tokio::test]
async fn retiring_a_session_revokes_its_managed_credentials() {
    for operation in ["archive", "suspend", "kill"] {
        let (mgr, repo, ws, user) = test_manager().await;
        let id = seed_session(&repo, &ws, &user, Some("sid-cred")).await;
        let auth = mgr.auth.as_ref().unwrap();
        let (token, _) = auth.issue_session_api_token(&user, &id).await.unwrap();
        let (own, neighbour) = seed_credential_files(&id);
        assert!(auth.authenticate(&token).await.is_ok());
        let outcome = match operation {
            "archive" => mgr.archive(&id).await.map(|_| ()),
            "suspend" => mgr.suspend(&id).await,
            _ => mgr.kill_session(&id).await,
        };
        outcome.unwrap();
        assert!(
            auth.authenticate(&token).await.is_err(),
            "{operation} left the session's MCP credential valid"
        );
        assert!(
            repo.get(&id).await.is_ok(),
            "{operation} must keep the session row"
        );
        assert_credential_files_removed(operation, &own, &neighbour);
    }
}

/// The boot sweep (`restore_all`) revokes credentials left valid by the
/// previous daemon run — no agent process survives a restart.
#[tokio::test]
async fn restore_all_revokes_leftover_session_credentials() {
    let (mgr, repo, ws, user) = test_manager().await;
    let id = seed_session(&repo, &ws, &user, Some("sid-boot")).await;
    let auth = mgr.auth.as_ref().unwrap();
    let (token, _) = auth.issue_session_api_token(&user, &id).await.unwrap();
    mgr.restore_all(&|_| None).await.unwrap();
    assert!(auth.authenticate(&token).await.is_err());
    assert_eq!(
        repo.get(&id).await.unwrap().status,
        SessionStatus::Reconnectable
    );
}

#[tokio::test]
async fn named_account_fake_cli_keeps_home_after_manager_restart_and_rejects_foreign_profiles() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let pool = otto_state::open(&dir.path().join("fixture.db"))
        .await
        .unwrap();
    let now = chrono::Utc::now().to_rfc3339();
    for owner in ["owner", "other"] {
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, ?, 'x', 'Fixture', 0, ?)")
            .bind(owner).bind(owner).bind(&now).execute(&pool).await.unwrap();
    }
    sqlx::query(
        "INSERT INTO workspaces (id, name, root_path, created_at) VALUES ('ws', 'Fixture', ?, ?)",
    )
    .bind(dir.path().to_str().unwrap())
    .bind(&now)
    .execute(&pool)
    .await
    .unwrap();
    let ws = Workspace {
        id: "ws".into(),
        name: "Fixture".into(),
        root_path: dir.path().to_str().unwrap().into(),
        settings: serde_json::json!({"otto_mcp_enabled":false}),
        archived: false,
        created_at: chrono::Utc::now(),
    };
    let profiles = otto_state::provider_accounts::ProviderAccountsRepo::new(pool.clone());
    let a = profiles
        .create(&"owner".into(), "claude", "A")
        .await
        .unwrap();
    let b = profiles
        .create(&"owner".into(), "claude", "B")
        .await
        .unwrap();
    let foreign = profiles
        .create(&"other".into(), "claude", "Foreign")
        .await
        .unwrap();
    let wrong_provider = profiles
        .create(&"owner".into(), "codex", "Codex")
        .await
        .unwrap();
    let program = dir.path().join("fake-cli");
    std::fs::write(&program, "#!/bin/sh\nprintf '%s\\n' \"$CLAUDE_CONFIG_DIR\" >> \"$CLAUDE_CONFIG_DIR/observed\"\nprintf 'fixture ready\\n'\nexec /bin/sleep 60\n").unwrap();
    std::fs::set_permissions(&program, std::fs::Permissions::from_mode(0o700)).unwrap();
    let config = serde_json::json!({"claude":{"cmd":program,"args":[],"resume_args":["--resume","{session_id}"]}});
    let make_manager = || {
        let (events, _) = broadcast::channel(16);
        SessionManager::new(
            SessionsRepo::new(pool.clone()),
            events,
            ProviderRegistry::new(Some(&config)),
        )
        .with_auth_repo(AuthRepo::new(pool.clone()))
        .with_provider_accounts(profiles.clone(), dir.path().join("accounts"))
    };
    let request = |id: &str| CreateSessionReq {
        kind: SessionKind::Agent,
        provider: Some("claude".into()),
        title: Some("Fixture".into()),
        cwd: Some(ws.root_path.clone()),
        connection_id: None,
        model: None,
        meta: Some(serde_json::json!({"account_id":id})),
    };
    let manager = make_manager();
    for id in [&foreign.id, &wrong_provider.id, &"missing".to_string()] {
        assert!(manager
            .create(&ws, &"owner".into(), request(id), None)
            .await
            .is_err());
    }
    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 0, "invalid accounts must not create a session");
    let sa = manager
        .create(&ws, &"owner".into(), request(&a.id), None)
        .await
        .unwrap();
    let sb = manager
        .create(&ws, &"owner".into(), request(&b.id), None)
        .await
        .unwrap();
    async fn wait_lines(path: &std::path::Path, count: usize) -> String {
        for _ in 0..100 {
            if let Ok(text) = std::fs::read_to_string(path) {
                if text.lines().count() >= count {
                    return text;
                }
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("fake CLI did not produce expected profile observation");
    }
    let ahome = dir.path().join("accounts").join(&a.id);
    let bhome = dir.path().join("accounts").join(&b.id);
    assert_eq!(
        wait_lines(&ahome.join("observed"), 1).await.trim(),
        ahome.to_str().unwrap()
    );
    assert_eq!(
        wait_lines(&bhome.join("observed"), 1).await.trim(),
        bhome.to_str().unwrap()
    );
    manager.kill_session(&sa.id).await.unwrap();
    manager.kill_session(&sb.id).await.unwrap();
    let restarted = make_manager();
    restarted.restart(&sa.id, None).await.unwrap();
    assert!(wait_lines(&ahome.join("observed"), 2)
        .await
        .lines()
        .all(|line| line == ahome.to_str().unwrap()));
    assert_eq!(
        std::fs::read_to_string(bhome.join("observed"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    restarted.remove(&sa.id).await.unwrap();
    restarted.remove(&sb.id).await.unwrap();

    // Failure after token minting must release the durable credential and row.
    let mut token_ws = ws.clone();
    token_ws.settings = serde_json::json!({"otto_mcp_enabled":true});
    let conflicting_spec = CommandSpec {
        program: program.to_string_lossy().into_owned(),
        args: vec![],
        cwd: Some(ws.root_path.clone()),
        env: vec![("CLAUDE_CODE_USE_BEDROCK".into(), "1".into())],
    };
    assert!(restarted
        .create(
            &token_ws,
            &"owner".into(),
            request(&a.id),
            Some(conflicting_spec)
        )
        .await
        .is_err());
    let rejected_tokens: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM auth_sessions WHERE session_scope IS NOT NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    let rejected_sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(
        (rejected_tokens, rejected_sessions),
        (0, 0),
        "policy mismatch must leave no session or credential"
    );
    let bad_spec = CommandSpec {
        program: program.to_string_lossy().into_owned(),
        args: vec!["\0".into()],
        cwd: Some(ws.root_path.clone()),
        env: vec![],
    };
    assert!(restarted
        .create(&token_ws, &"owner".into(), request(&a.id), Some(bad_spec))
        .await
        .is_err());
    let tokens: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM auth_sessions WHERE session_scope IS NOT NULL")
            .fetch_one(&pool)
            .await
            .unwrap();
    let sessions: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!((tokens, sessions), (0, 0));
}

/// RAII: set an env var for one test and restore the previous value on drop
/// (panic-safe). The environment is process-global and tests run in
/// parallel, so a `set_var` that outlives its test leaks to every test
/// scheduled after it — a leaked HOME pointing at a dropped tempdir made
/// unrelated PTY spawns fail with ENOENT (portable-pty uses $HOME as the
/// child cwd when none is given). Declare the guard AFTER the tempdir it
/// points at, so reverse drop order restores the var before the dir vanishes.
struct EnvVarGuard {
    key: &'static str,
    prev: Option<std::ffi::OsString>,
}

impl EnvVarGuard {
    fn set(key: &'static str, value: impl AsRef<std::ffi::OsStr>) -> Self {
        let prev = std::env::var_os(key);
        std::env::set_var(key, value);
        Self { key, prev }
    }
}

impl Drop for EnvVarGuard {
    fn drop(&mut self) {
        match &self.prev {
            Some(v) => std::env::set_var(self.key, v),
            None => std::env::remove_var(self.key),
        }
    }
}

/// Write a codex rollout file with a `session_meta` first line, returning its
/// path. `thread_source`/`originator` let tests forge subagent / non-codex rows.
fn write_rollout(
    dir: &std::path::Path,
    name: &str,
    session_id: &str,
    cwd: &str,
    originator: &str,
    thread_source: &str,
) -> std::path::PathBuf {
    std::fs::create_dir_all(dir).unwrap();
    let path = dir.join(name);
    let meta = serde_json::json!({
        "type": "session_meta",
        "payload": {
            "session_id": session_id,
            "id": session_id,
            "cwd": cwd,
            "originator": originator,
            "thread_source": thread_source,
        }
    });
    std::fs::write(&path, format!("{meta}\n")).unwrap();
    path
}

#[test]
fn codex_rollout_path_finds_by_uuid_suffix() {
    let root = tempfile::tempdir().unwrap();
    let day = root.path().join("2026").join("07").join("12");
    std::fs::create_dir_all(&day).unwrap();
    let psid = "019ed94a-994a-7010-b01f-9b840c5b7068";
    let f = day.join(format!("rollout-2026-07-12T20-00-00-{psid}.jsonl"));
    std::fs::write(&f, b"{}").unwrap();
    std::fs::write(
        day.join("rollout-2026-07-12T21-00-00-other-uuid-here.jsonl"),
        b"{}",
    )
    .unwrap();
    assert_eq!(codex_rollout_path_under(root.path(), psid), Some(f));
    assert_eq!(codex_rollout_path_under(root.path(), "nope"), None);
    assert_eq!(codex_rollout_path_under(root.path(), ""), None);
}

#[test]
fn codex_rollout_match_filters_to_top_level_in_cwd() {
    let tmp = tempfile::tempdir().unwrap();
    let day = tmp.path().join("2026/06/25");
    let top = write_rollout(&day, "a.jsonl", "AAA", "/work/proj", "codex-tui", "user");
    let sub = write_rollout(
        &day,
        "b.jsonl",
        "BBB",
        "/work/proj",
        "codex-tui",
        "subagent",
    );
    let other = write_rollout(&day, "c.jsonl", "CCC", "/elsewhere", "codex-tui", "user");

    assert_eq!(
        codex_rollout_match(&top, "/work/proj").map(|(s, _)| s),
        Some("AAA".to_string())
    );
    // Subagent thread and wrong-cwd rollouts are not matched.
    assert_eq!(
        codex_rollout_match(&sub, "/work/proj").map(|(s, _)| s),
        None
    );
    assert_eq!(
        codex_rollout_match(&other, "/work/proj").map(|(s, _)| s),
        None
    );
}

/// Append a `user_message` event line to a rollout, as codex does when the
/// first prompt is submitted (before that the file holds only the meta line).
fn append_user_message(path: &std::path::Path, text: &str) {
    use std::io::Write;
    let line = serde_json::json!({
        "timestamp": "2026-07-21T05:05:02.536Z",
        "type": "event_msg",
        "payload": { "type": "user_message", "message": text }
    });
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    writeln!(f, "{line}").unwrap();
}

#[test]
fn normalize_pty_input_strips_wrapping_and_collapses() {
    // Bracketed-paste markers, CSI sequences, CR/LF and whitespace runs all
    // reduce to the plain text codex records as the user_message.
    let raw = b"\x1b[200~Scan the  repository koala-vivo-go\rline2\x1b[201~\r";
    assert_eq!(
        normalize_pty_input(raw),
        "Scan the repository koala-vivo-go line2"
    );
    assert_eq!(
        normalize_pty_input(b"  hi \r\n there \x1b[A\x7f"),
        "hi there"
    );
    assert_eq!(normalize_pty_input(b"\x1b[200~\x1b[201~\r"), "");
}

#[test]
fn rollout_first_user_message_read() {
    let tmp = tempfile::tempdir().unwrap();
    let day = tmp.path().join("2026/07/21");
    let p = write_rollout(&day, "a.jsonl", "AAA", "/work/proj", "codex-tui", "user");
    // Meta-only rollout (pre-first-prompt): no message yet.
    assert_eq!(codex_rollout_first_user_message(&p), None);
    append_user_message(&p, "Scan the repository koala-vivo-go into the Vault");
    assert_eq!(
        codex_rollout_first_user_message(&p).as_deref(),
        Some("Scan the repository koala-vivo-go into the Vault")
    );
}

/// THE 2026-07-21 incident regression: two concurrent same-cwd spawns; the
/// rollout whose first user_message matches THIS session's typed input must
/// win, regardless of which rollout is older on disk. The old
/// oldest-mtime-unclaimed heuristic claimed another session's conversation.
#[test]
fn pick_claims_content_match_over_older_rollout() {
    use std::collections::HashSet;
    let tmp = tempfile::tempdir().unwrap();
    let day = tmp.path().join("2026/07/21");
    let a = write_rollout(&day, "a.jsonl", "AAA", "/work/proj", "codex-tui", "user");
    append_user_message(&a, "Scan the repository koala-turbogames-go into the Vault");
    std::thread::sleep(std::time::Duration::from_millis(30));
    let b = write_rollout(&day, "b.jsonl", "BBB", "/work/proj", "codex-tui", "user");
    append_user_message(&b, "Scan the repository koala-vivo-go into the Vault");

    let none: HashSet<&str> = HashSet::new();
    let floor = std::time::SystemTime::UNIX_EPOCH;
    assert_eq!(
        pick_codex_rollout(
            tmp.path(),
            "/work/proj",
            floor,
            &none,
            Some("Scan the repository koala-vivo-go into the Vault"),
        ),
        RolloutPick::Claim("BBB".to_string())
    );
}

/// Multiple same-cwd candidates and no content evidence to tell them apart:
/// never guess — report ambiguity so the caller keeps waiting / gives up.
#[test]
fn pick_refuses_ambiguous_without_content() {
    use std::collections::HashSet;
    let tmp = tempfile::tempdir().unwrap();
    let day = tmp.path().join("2026/07/21");
    write_rollout(&day, "a.jsonl", "AAA", "/work/proj", "codex-tui", "user");
    write_rollout(&day, "b.jsonl", "BBB", "/work/proj", "codex-tui", "user");

    let none: HashSet<&str> = HashSet::new();
    let floor = std::time::SystemTime::UNIX_EPOCH;
    // No probe at all → ambiguous.
    assert_eq!(
        pick_codex_rollout(tmp.path(), "/work/proj", floor, &none, None),
        RolloutPick::Ambiguous
    );
    // A too-short probe can't discriminate either.
    assert_eq!(
        pick_codex_rollout(tmp.path(), "/work/proj", floor, &none, Some("hi")),
        RolloutPick::Ambiguous
    );
}

/// The common single-spawn case must keep working without any probe: one
/// candidate in the cwd → claim it; already claimed → nothing.
#[test]
fn pick_sole_candidate_claims_without_probe() {
    use std::collections::HashSet;
    let tmp = tempfile::tempdir().unwrap();
    let day = tmp.path().join("2026/07/21");
    write_rollout(&day, "a.jsonl", "AAA", "/work/proj", "codex-tui", "user");

    let none: HashSet<&str> = HashSet::new();
    let floor = std::time::SystemTime::UNIX_EPOCH;
    assert_eq!(
        pick_codex_rollout(tmp.path(), "/work/proj", floor, &none, None),
        RolloutPick::Claim("AAA".to_string())
    );
    let claimed: HashSet<&str> = ["AAA"].into_iter().collect();
    assert_eq!(
        pick_codex_rollout(tmp.path(), "/work/proj", floor, &claimed, None),
        RolloutPick::Nothing
    );
}

/// A sole candidate whose recorded first message CONTRADICTS what this
/// session's PTY received is someone else's conversation — never claim it.
#[test]
fn pick_never_claims_contradicting_content() {
    use std::collections::HashSet;
    let tmp = tempfile::tempdir().unwrap();
    let day = tmp.path().join("2026/07/21");
    let a = write_rollout(&day, "a.jsonl", "AAA", "/work/proj", "codex-tui", "user");
    append_user_message(&a, "Scan the repository koala-turbogames-go into the Vault");

    let none: HashSet<&str> = HashSet::new();
    let floor = std::time::SystemTime::UNIX_EPOCH;
    assert_eq!(
        pick_codex_rollout(
            tmp.path(),
            "/work/proj",
            floor,
            &none,
            Some("Scan the repository koala-vivo-go into the Vault"),
        ),
        RolloutPick::Nothing
    );
}

/// `input()` must feed the capture probe (first-input time + normalized
/// text) for sessions with a pending provider-id capture.
#[tokio::test]
async fn input_records_capture_probe() {
    let (mgr, repo, ws, user) = test_manager().await;
    let id = seed_session(&repo, &ws, &user, None).await;
    let spec = CommandSpec {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), "exec sleep 30".into()],
        // Explicit cwd: under a parallel workspace run another test can
        // drop the tempdir the process cwd points at, and inheriting a
        // deleted cwd makes spawn fail with ENOENT.
        cwd: Some("/".into()),
        env: vec![],
    };
    mgr.live
        .insert(id.clone(), Arc::new(PtyHandle::spawn(&spec).unwrap()));
    // A pending capture registers an empty probe at spawn.
    mgr.capture_probes
        .insert(id.clone(), CaptureProbe::default());

    mgr.input(&id, b"\x1b[200~hello world\x1b[201~\r")
        .await
        .unwrap();

    {
        let probe = mgr.capture_probes.get(&id).unwrap();
        assert!(probe.first_at.is_some(), "first input moment recorded");
        assert_eq!(normalize_pty_input(&probe.raw), "hello world");
    }
    // A retained read guard deadlocks the next input when its session ID
    // hashes to this shard. Check that shard without risking a hung test.
    assert!(
        mgr.capture_probes.try_get_mut(&id).is_present(),
        "capture-probe inspection must release its shard before more input"
    );
    // Sessions WITHOUT a pending capture don't accumulate probes.
    let other = seed_session(&repo, &ws, &user, Some("sid")).await;
    mgr.live
        .insert(other.clone(), Arc::new(PtyHandle::spawn(&spec).unwrap()));
    mgr.input(&other, b"x").await.unwrap();
    assert!(mgr.capture_probes.get(&other).is_none());
}

/// The sweep's reap of an unresumable session re-checks the idle guards
/// under the resume lock: an engine turn (or viewer) that appeared after
/// the sweep's slow reads must win over the kill.
#[tokio::test]
async fn kill_if_idle_rechecks_guards_under_the_lock() {
    let (mgr, repo, ws, owner) = test_manager().await;
    let id = seed_session(&repo, &ws, &owner, None).await;
    let spec = CommandSpec {
        program: "/bin/cat".into(),
        args: vec![],
        cwd: Some("/".into()),
        env: vec![],
    };
    let handle = Arc::new(PtyHandle::spawn(&spec).unwrap());
    mgr.live.insert(id.clone(), Arc::clone(&handle));
    let turn = mgr.hold_for_turn(&id);
    assert!(!mgr.kill_if_idle(&id, Duration::ZERO).await.unwrap());
    assert!(
        handle.on_exit().borrow().is_none(),
        "held session not killed"
    );
    drop(turn);
    // Too-recent output (a huge grace) also holds it.
    assert!(!mgr
        .kill_if_idle(&id, Duration::from_secs(3600))
        .await
        .unwrap());
    assert!(mgr.kill_if_idle(&id, Duration::ZERO).await.unwrap());
    assert_eq!(repo.get(&id).await.unwrap().status, SessionStatus::Exited);
}

/// Rapid resizes persist the LATEST grid: each write stores the handle's
/// size at its turn, serialized, so an older write can't land last.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn saved_grid_is_the_latest_after_rapid_resizes() {
    let (mgr, repo, ws, owner) = test_manager().await;
    let id = seed_session(&repo, &ws, &owner, None).await;
    let spec = CommandSpec {
        program: "/bin/cat".into(),
        args: vec![],
        cwd: Some("/".into()),
        env: vec![],
    };
    let handle = Arc::new(PtyHandle::spawn(&spec).unwrap());
    mgr.live.insert(id.clone(), Arc::clone(&handle));
    for cols in 81..=120u16 {
        mgr.resize(&id, cols, 30).await.unwrap();
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    loop {
        let meta = repo.get(&id).await.unwrap().meta;
        if meta.get("pty_cols").and_then(|v| v.as_u64()) == Some(120) {
            // Give any straggler a chance to (wrongly) overwrite it.
            tokio::time::sleep(Duration::from_millis(200)).await;
            let meta = repo.get(&id).await.unwrap().meta;
            assert_eq!(meta["pty_cols"], 120);
            assert_eq!(meta["pty_rows"], 30);
            break;
        }
        assert!(std::time::Instant::now() < deadline, "grid never persisted");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let _ = handle.kill();
}

/// A viewer's zero/huge grid is `Invalid` and never reaches the PTY (a
/// 65535² grid aborted the daemon; rows=0 froze the session's output).
#[tokio::test]
async fn human_resize_rejects_out_of_range_grids() {
    let (mgr, repo, ws, owner) = test_manager().await;
    let id = seed_session(&repo, &ws, &owner, None).await;
    let spec = CommandSpec {
        program: "/bin/cat".into(),
        args: vec![],
        cwd: Some("/".into()),
        env: vec![],
    };
    let handle = Arc::new(PtyHandle::spawn_sized(&spec, 100, 30).unwrap());
    mgr.live.insert(id.clone(), Arc::clone(&handle));
    for (c, r) in [(0, 24), (80, 0), (65535, 65535)] {
        let err = mgr
            .human_resize(&id, &owner, false, c, r)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{c}x{r}: {err:?}");
        assert!(mgr.resize(&id, c, r).await.is_err());
    }
    assert_eq!(handle.size(), (100, 30));
    mgr.human_resize(&id, &owner, false, 90, 25).await.unwrap();
    assert_eq!(handle.size(), (90, 25));
    let _ = handle.kill();
}

#[tokio::test]
async fn room_authority_gates_all_human_input_and_process_replacement() {
    let (mgr, repo, ws, owner) = test_manager().await;
    let id = seed_session(&repo, &ws, &owner, None).await;
    let spec = CommandSpec {
        program: "/bin/cat".into(),
        args: vec![],
        cwd: Some("/".into()),
        env: vec![],
    };
    mgr.live
        .insert(id.clone(), Arc::new(PtyHandle::spawn(&spec).unwrap()));
    mgr.begin_room_authority(&id, "room", &owner, "host")
        .await
        .unwrap();
    let epoch = mgr
        .set_room_driver(&id, "room", Some("guest"))
        .await
        .unwrap();
    mgr.room_input(&id, "room", "guest", epoch, b"guest\n")
        .await
        .unwrap();
    assert!(mgr
        .human_input(&id, &owner, true, true, b"scoped\n")
        .await
        .is_err());
    assert!(mgr
        .human_input(&id, &Id::from("admin"), false, true, b"admin\n")
        .await
        .is_err());
    assert!(mgr
        .human_input(&id, &owner, false, false, b"response")
        .await
        .is_err());
    assert!(mgr.human_resize(&id, &owner, false, 80, 24).await.is_err());
    assert_eq!(mgr.room_authority_snapshot(&id).await.unwrap().epoch, epoch);
    mgr.human_input(&id, &owner, false, true, b"host\n")
        .await
        .unwrap();
    assert!(mgr
        .room_input(&id, "room", "guest", epoch, b"stale\n")
        .await
        .is_err());
    let new_epoch = mgr
        .set_room_driver(&id, "room", Some("guest"))
        .await
        .unwrap();
    assert!(new_epoch > epoch);
    assert!(mgr
        .room_input(&id, "room", "guest", epoch, b"stale\n")
        .await
        .is_err());
    let old = mgr
        .live
        .insert(id.clone(), Arc::new(PtyHandle::spawn(&spec).unwrap()));
    assert!(mgr
        .room_input(&id, "room", "guest", new_epoch, b"wrong process\n")
        .await
        .is_err());
    mgr.end_room_authority(&id, "room").await;
    assert!(
        mgr.live_handle(&id).is_some(),
        "ending a room must leave the process alive"
    );
    mgr.human_input(&id, &owner, false, true, b"normal\n")
        .await
        .unwrap();
    drop(old);
    mgr.live.remove(&id);
}
#[tokio::test]
async fn room_authority_delayed_submit_keeps_its_original_epoch() {
    let (mgr, repo, ws, owner) = test_manager().await;
    let id = seed_session(&repo, &ws, &owner, None).await;
    let spec = CommandSpec {
        program: "/bin/cat".into(),
        args: vec![],
        cwd: Some("/".into()),
        env: vec![],
    };
    mgr.live
        .insert(id.clone(), Arc::new(PtyHandle::spawn(&spec).unwrap()));
    mgr.begin_room_authority(&id, "room", &owner, "host")
        .await
        .unwrap();
    let result = mgr
        .human_submit_text_checked(&id, &owner, false, "draft", || async {
            mgr.set_room_driver(&id, "room", Some("guest")).await?;
            mgr.set_room_driver(&id, "room", None).await?;
            Ok(())
        })
        .await;
    assert!(
        matches!(result, Err(Error::Forbidden(_))),
        "a delayed Enter must not acquire a fresh epoch"
    );
    assert_eq!(mgr.room_authority_snapshot(&id).await.unwrap().driver, None);
    mgr.end_room_authority(&id, "room").await;
    mgr.live.remove(&id);
}
/// concurrency-async-1-02: two submissions racing into one session (an
/// agent→agent message and a human's message within the 200 ms paste delay)
/// must each arrive as paste + Enter — never paste A, paste B, Enter, Enter,
/// which the TUI submits as ONE merged prompt plus an empty line.
#[tokio::test]
async fn concurrent_submits_do_not_interleave() {
    let (mgr, repo, ws, owner) = test_manager().await;
    let id = seed_session(&repo, &ws, &owner, None).await;
    let dir = tempfile::tempdir().unwrap();
    let sink = dir.path().join("pty-input");
    // `cat` records exactly what reached the PTY; each `\r` (or the `\n` the
    // line discipline maps it to) ends one submission.
    let spec = CommandSpec {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), format!("exec cat > '{}'", sink.display())],
        cwd: Some("/".into()),
        env: vec![],
    };
    mgr.live
        .insert(id.clone(), Arc::new(PtyHandle::spawn(&spec).unwrap()));
    let (a, b) = tokio::join!(
        mgr.submit_text(&id, "from-agent"),
        mgr.human_submit_text(&id, &owner, false, "from-human"),
    );
    a.unwrap();
    b.unwrap();
    let mut lines = Vec::new();
    for _ in 0..60 {
        let got = std::fs::read_to_string(&sink).unwrap_or_default();
        lines = got
            .split(['\r', '\n'])
            .filter(|l| !l.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        if lines.len() >= 2 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    lines.sort();
    assert_eq!(
        lines,
        vec![
            "\x1b[200~from-agent\x1b[201~".to_string(),
            "\x1b[200~from-human\x1b[201~".to_string(),
        ],
        "each submission must be its own paste + Enter"
    );
    mgr.live.remove(&id);
}

/// Resume-fork guard: a rollout that another live process is appending to
/// must be detected so ensure_live refuses to `codex resume` a fork of it.
#[tokio::test]
async fn rollout_actively_written_detects_writer() {
    let tmp = tempfile::tempdir().unwrap();
    let day = tmp.path().join("2026/07/21");
    let p = write_rollout(
        &day,
        "rollout-2026-07-21T08-03-46-PSID1.jsonl",
        "PSID1",
        "/work/proj",
        "codex-tui",
        "user",
    );
    // Static file → not actively written.
    assert!(!rollout_actively_written(tmp.path(), "PSID1", Duration::from_millis(150)).await);
    // A concurrent writer appending during the settle window → detected.
    // It appends until told to stop (not a fixed 6×40 ms burst): on a
    // slow CI runner the burst could finish before the walk + first stat
    // even ran, leaving a static file inside the window.
    // The writer must really be running before the window opens, and the
    // window is wide (1 s vs a 20 ms cadence) so a starved writer thread
    // on a loaded macOS runner can't leave the file static inside it.
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let writes = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let writer = std::thread::spawn({
        let (p, stop, writes) = (p.clone(), Arc::clone(&stop), Arc::clone(&writes));
        move || {
            while !stop.load(Ordering::Relaxed) {
                append_user_message(&p, "more output");
                writes.fetch_add(1, Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        }
    });
    while writes.load(Ordering::Relaxed) == 0 {
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    let detected = rollout_actively_written(tmp.path(), "PSID1", Duration::from_secs(1)).await;
    stop.store(true, Ordering::Relaxed);
    writer.join().unwrap();
    assert!(detected);
    // Unknown psid → never blocks a resume.
    assert!(!rollout_actively_written(tmp.path(), "NOPE", Duration::from_millis(50)).await);
}

/// SG-04: a nested agent that never writes a transcript is probed at the
/// loop's cadence for the first misses, then with a doubling (capped) gap;
/// a capture or a new launch in the terminal resets it.
#[test]
fn nested_probe_backs_off_after_misses_and_resets() {
    let id: Id = "nested-backoff-test".into();
    nested_probe_forget(&id);
    for _ in 0..NESTED_FREE_MISSES {
        assert!(nested_probe_due(&id, 42));
        nested_probe_missed(&id, 42);
    }
    assert!(nested_probe_due(&id, 42), "free misses keep the cadence");
    nested_probe_missed(&id, 42);
    assert!(!nested_probe_due(&id, 42), "backing off now");
    {
        let m = nested_misses().lock().unwrap();
        let (n, at) = m[&(id.clone(), 42)];
        assert_eq!(n, NESTED_FREE_MISSES + 1);
        let gap = at.saturating_duration_since(std::time::Instant::now());
        assert!(gap > Duration::from_secs(50) && gap <= Duration::from_secs(60));
    }
    for _ in 0..20 {
        nested_probe_missed(&id, 42);
    }
    {
        let m = nested_misses().lock().unwrap();
        let gap = m[&(id.clone(), 42)]
            .1
            .saturating_duration_since(std::time::Instant::now());
        assert!(gap <= NESTED_MAX_BACKOFF);
    }
    // A new launch (other pid) in the same terminal starts from zero.
    assert!(nested_probe_due(&id, 43));
    nested_probe_missed(&id, 43);
    assert!(nested_probe_due(&id, 42), "the old launch's entry is gone");
    nested_probe_forget(&id);
    assert!(nested_probe_due(&id, 43));
}

#[test]
fn scan_agy_conversation_matches_fresh_unclaimed_cwd() {
    use std::collections::HashSet;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join("cache")).unwrap();
    std::fs::create_dir_all(root.join("conversations")).unwrap();
    // cwd -> most-recent conversation id (agy's last_conversations cache).
    std::fs::write(
        root.join("cache/last_conversations.json"),
        r#"{"/work/proj":"AAA","/other":"BBB"}"#,
    )
    .unwrap();
    // Only AAA has a conversation file on disk (fresh).
    std::fs::write(root.join("conversations/AAA.db"), b"x").unwrap();
    let floor = std::time::SystemTime::UNIX_EPOCH;
    let none: HashSet<&str> = HashSet::new();

    // The cwd maps to AAA and its file is fresh + unclaimed → captured.
    assert_eq!(
        scan_agy_conversation(root, "/work/proj", floor, &none),
        Some("AAA".to_string())
    );
    // /other maps to BBB but there is no conversation file → not captured.
    assert_eq!(scan_agy_conversation(root, "/other", floor, &none), None);
    // Already claimed by another session → skip.
    let claimed: HashSet<&str> = ["AAA"].into_iter().collect();
    assert_eq!(
        scan_agy_conversation(root, "/work/proj", floor, &claimed),
        None
    );
    // Unknown cwd → nothing.
    assert_eq!(scan_agy_conversation(root, "/nope", floor, &none), None);
}

async fn seed_session(repo: &SessionsRepo, ws: &Workspace, user: &Id, psid: Option<&str>) -> Id {
    let s = repo
        .create(NewSession {
            workspace_id: ws.id.clone(),
            kind: SessionKind::Agent,
            provider: "claude".into(),
            title: "t".into(),
            cwd: "/tmp".into(),
            provider_session_id: psid.map(|s| s.to_string()),
            connection_id: None,
            created_by: user.clone(),
            meta: serde_json::json!({}),
        })
        .await
        .unwrap();
    s.id
}

/// Root-cause regression for the orphaned-process leak: a superseded handle's
/// status-task exit must NOT evict a freshly-respawned handle from `live`.
/// With the old unconditional `live.remove(id)` the replacement was evicted
/// (untracked → never killed by suspend/archive → orphan). `evict_if_same`
/// makes the stale exit a no-op against the replacement, while the
/// replacement's own exit still evicts it.
#[tokio::test]
async fn evict_if_same_ignores_superseded_handle() {
    let spec = CommandSpec {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), "exec sleep 30".into()],
        cwd: None,
        env: vec![],
    };
    let live: DashMap<Id, Arc<PtyHandle>> = DashMap::new();
    let id: Id = "S1".into();
    let h1 = Arc::new(PtyHandle::spawn(&spec).expect("spawn h1")); // old, superseded
    let h2 = Arc::new(PtyHandle::spawn(&spec).expect("spawn h2")); // the respawn

    // After a restart, `live` maps the NEW handle.
    live.insert(id.clone(), Arc::clone(&h2));

    // The OLD handle's status task fires on its exit → must be a no-op here.
    assert!(
        !evict_if_same(&live, &id, &h1),
        "a superseded handle must not remove the replacement entry"
    );
    assert!(live.contains_key(&id), "replacement must stay tracked");
    assert!(Arc::ptr_eq(live.get(&id).unwrap().value(), &h2));

    // The replacement's OWN exit does evict it.
    assert!(evict_if_same(&live, &id, &h2));
    assert!(!live.contains_key(&id));

    // Drop kills both children (RAII); no orphan left behind.
}

#[tokio::test]
async fn attach_guard_counts_and_releases() {
    let (mgr, repo, ws, user) = test_manager().await;
    let id = seed_session(&repo, &ws, &user, Some("sid")).await;

    assert_eq!(mgr.attached_count(&id), 0);
    assert!(!mgr.is_attached(&id));

    let g1 = mgr.attach(&id);
    assert_eq!(mgr.attached_count(&id), 1);
    assert!(mgr.is_attached(&id));

    let g2 = mgr.attach(&id);
    assert_eq!(mgr.attached_count(&id), 2);

    drop(g1);
    assert_eq!(mgr.attached_count(&id), 1);
    assert!(mgr.is_attached(&id));

    drop(g2);
    assert_eq!(mgr.attached_count(&id), 0);
    assert!(!mgr.is_attached(&id));
}

#[tokio::test]
async fn size_authority_typing_owner_blocks_passive_resizers() {
    let (mgr, repo, ws, user) = test_manager().await;
    let id = seed_session(&repo, &ws, &user, Some("sid-size")).await;

    let pane = mgr.attach(&id); // the user's open pane
    let tile = mgr.attach(&id); // a passive tiled-overview tile

    // Nobody typed yet: first-come resize is allowed for both.
    assert!(mgr.may_resize(&id, pane.conn_id()));
    assert!(mgr.may_resize(&id, tile.conn_id()));

    // Typing in the pane claims authority; the tile may no longer resize.
    mgr.note_input_authority(&id, pane.conn_id());
    assert!(mgr.may_resize(&id, pane.conn_id()));
    assert!(!mgr.may_resize(&id, tile.conn_id()));

    // A focus claim moves authority (e.g. user clicks the other viewer).
    mgr.note_input_authority(&id, tile.conn_id());
    assert!(!mgr.may_resize(&id, pane.conn_id()));
    assert!(mgr.may_resize(&id, tile.conn_id()));

    // Authority is STICKY: the owner detaching does NOT hand the size to
    // survivors — a passive pane/tile still may not resize (agent output
    // printed while the user is away must keep the owner's width)…
    drop(tile);
    assert!(!mgr.may_resize(&id, pane.conn_id()));
    // …until the survivor claims (pane re-attach claims on open).
    mgr.note_input_authority(&id, pane.conn_id());
    assert!(mgr.may_resize(&id, pane.conn_id()));
}

/// Perf 01 N7: the socket claims authority when a keystroke is QUEUED and
/// reverts only its own claim when the write fails.
#[tokio::test]
async fn optimistic_input_claim_reverts_only_its_own_claim() {
    let (mgr, repo, ws, user) = test_manager().await;
    let id = seed_session(&repo, &ws, &user, Some("sid-claim")).await;
    let pane = mgr.attach(&id);
    let tile = mgr.attach(&id);

    // Unclaimed → claim → failed write: back to "nobody owns it".
    assert_eq!(mgr.claim_input_authority(&id, pane.conn_id()), None);
    assert!(!mgr.may_resize(&id, tile.conn_id()));
    mgr.revert_input_authority(&id, pane.conn_id(), None);
    assert!(mgr.may_resize(&id, tile.conn_id()));

    // Tile owned it; the pane's failed claim hands it back to the tile.
    mgr.note_input_authority(&id, tile.conn_id());
    let prev = mgr.claim_input_authority(&id, pane.conn_id());
    assert_eq!(prev, Some(tile.conn_id()));
    mgr.revert_input_authority(&id, pane.conn_id(), prev);
    assert!(mgr.may_resize(&id, tile.conn_id()));
    assert!(!mgr.may_resize(&id, pane.conn_id()));

    // A newer claim by someone else stands through a stale revert.
    let prev = mgr.claim_input_authority(&id, pane.conn_id());
    mgr.note_input_authority(&id, tile.conn_id());
    mgr.revert_input_authority(&id, pane.conn_id(), prev);
    assert!(mgr.may_resize(&id, tile.conn_id()));
}

#[tokio::test]
async fn suspend_marks_reconnectable_and_keeps_row() {
    let (mgr, repo, ws, user) = test_manager().await;
    let id = seed_session(&repo, &ws, &user, Some("sid-keep")).await;

    // No live PTY in this test; suspend still drives the DB-side outcome.
    mgr.suspend(&id).await.unwrap();

    let s = repo.get(&id).await.unwrap();
    assert_eq!(s.status, SessionStatus::Reconnectable);
    // The session is NOT lost — row and resume id are preserved.
    assert_eq!(s.provider_session_id.as_deref(), Some("sid-keep"));
}

/// Wait (bounded) for `handle`'s child to exit, then give the status task
/// a beat to run its exit arm.
async fn wait_child_exit_and_settle(handle: &Arc<PtyHandle>) {
    let mut exit = handle.on_exit();
    tokio::time::timeout(Duration::from_secs(10), wait_exit_code(&mut exit))
        .await
        .expect("child exited in time");
    tokio::time::sleep(Duration::from_millis(400)).await;
}

/// A child that takes a while to die after SIGHUP, like the claude CLI
/// (~0.8 s): the late exit is what used to overwrite the suspend.
fn slow_hup_spec() -> CommandSpec {
    CommandSpec {
        program: "/bin/sh".into(),
        args: vec![
            "-c".into(),
            "trap 'sleep 0.5; exit 0' HUP; while :; do sleep 0.1; done".into(),
        ],
        cwd: None,
        env: vec![],
    }
}

/// Root-cause regression (A1, 334 of 337 idle suspends ended `exited`):
/// the status task's exit arm fires AFTER `suspend()` returned, and must
/// leave the session `Reconnectable` — resumable by the chat view and
/// reusable by the channel bridge — not flip it to `Exited`.
#[tokio::test]
async fn suspend_survives_late_exit_of_slow_dying_child() {
    let (mgr, repo, ws, user) = test_manager().await;
    let id = seed_session(&repo, &ws, &user, Some("sid-late")).await;
    let handle = Arc::new(PtyHandle::spawn(&slow_hup_spec()).expect("spawn"));
    mgr.live.insert(id.clone(), Arc::clone(&handle));
    mgr.start_status_task(
        id.clone(),
        ws.id.clone(),
        "claude".into(),
        Arc::clone(&handle),
        false,
    );
    tokio::time::sleep(Duration::from_millis(200)).await;

    mgr.suspend(&id).await.unwrap();
    assert_eq!(
        repo.get(&id).await.unwrap().status,
        SessionStatus::Reconnectable
    );

    wait_child_exit_and_settle(&handle).await;
    assert_eq!(
        repo.get(&id).await.unwrap().status,
        SessionStatus::Reconnectable,
        "the suspended child's late exit overwrote Reconnectable"
    );
    assert!(!mgr.is_live(&id));
}

/// A2: the exit of a SUPERSEDED process (restart / ensure_live replaced
/// it) must not mark the new live incarnation `Exited`.
#[tokio::test]
async fn superseded_exit_does_not_mark_new_process_exited() {
    let (mgr, repo, ws, user) = test_manager().await;
    let id = seed_session(&repo, &ws, &user, Some("sid-respawn")).await;
    let old = Arc::new(PtyHandle::spawn(&slow_hup_spec()).expect("spawn old"));
    mgr.live.insert(id.clone(), Arc::clone(&old));
    mgr.start_status_task(
        id.clone(),
        ws.id.clone(),
        "claude".into(),
        Arc::clone(&old),
        false,
    );
    tokio::time::sleep(Duration::from_millis(200)).await;

    // What `restart_locked` does: untrack + kill the old, track the new,
    // write Running.
    let fresh = Arc::new(PtyHandle::spawn(&slow_hup_spec()).expect("spawn new"));
    mgr.live.remove(&id);
    let _ = old.kill();
    mgr.live.insert(id.clone(), Arc::clone(&fresh));
    repo.update_status(&id, SessionStatus::Running)
        .await
        .unwrap();

    wait_child_exit_and_settle(&old).await;
    assert_eq!(
        repo.get(&id).await.unwrap().status,
        SessionStatus::Running,
        "the old process's exit clobbered the respawned session"
    );
    assert!(Arc::ptr_eq(&mgr.live_handle(&id).unwrap(), &fresh));
}

/// The current process exiting on its own is still a real exit.
#[tokio::test]
async fn natural_exit_of_current_process_marks_exited() {
    let (mgr, repo, ws, user) = test_manager().await;
    let id = seed_session(&repo, &ws, &user, Some("sid-natural")).await;
    let spec = CommandSpec {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), "sleep 0.3".into()],
        cwd: None,
        env: vec![],
    };
    let handle = Arc::new(PtyHandle::spawn(&spec).expect("spawn"));
    mgr.live.insert(id.clone(), Arc::clone(&handle));
    mgr.start_status_task(
        id.clone(),
        ws.id.clone(),
        "claude".into(),
        Arc::clone(&handle),
        false,
    );

    wait_child_exit_and_settle(&handle).await;
    assert_eq!(repo.get(&id).await.unwrap().status, SessionStatus::Exited);
    assert!(!mgr.is_live(&id));
}

/// Root-cause regression for the dead-on-reopen codex session
/// (2026-07-16): a session whose spawn-time id capture timed out (codex
/// wrote its rollout ~20s after launch, past the old 12s window) stayed
/// non-resumable forever, and `ensure_live` silently no-opped — the
/// session showed in the UI but could never be reopened. The late capture
/// must claim the on-disk rollout born after the session's creation, and
/// must NOT steal an id another session already claimed.
#[tokio::test]
async fn late_capture_claims_rollout_written_after_spawn_window() {
    let (mgr, repo, ws, user) = test_manager().await;
    let cwd_dir = tempfile::tempdir().unwrap();
    // Canonical form up front: the capture compares cwd by exact string.
    let cwd = cwd_dir
        .path()
        .canonicalize()
        .unwrap()
        .to_string_lossy()
        .into_owned();
    let new_codex = |title: &str| NewSession {
        workspace_id: ws.id.clone(),
        kind: SessionKind::Agent,
        provider: "codex".into(),
        title: title.into(),
        cwd: cwd.clone(),
        provider_session_id: None,
        connection_id: None,
        created_by: user.clone(),
        meta: serde_json::json!({}),
    };
    let s = repo.create(new_codex("t")).await.unwrap();

    // The rollout lands only now — after `created_at`, i.e. after the
    // spawn-time window would have expired. CODEX_HOME points the scan at
    // it (safe here: every other test passes its scan root explicitly).
    let codex_home = tempfile::tempdir().unwrap();
    let day = codex_home.path().join("sessions").join("2026/07/16");
    write_rollout(&day, "r.jsonl", "LATE-1", &cwd, "codex-tui", "user");
    let _codex_home = EnvVarGuard::set("CODEX_HOME", codex_home.path());

    assert_eq!(
        mgr.late_capture_provider_id(&s).await.as_deref(),
        Some("LATE-1"),
        "late capture must find the rollout the spawn-time window missed"
    );

    // Claim it for `s`; a second non-resumable session in the same cwd
    // must not capture the same conversation.
    repo.set_provider_session(&s.id, "LATE-1").await.unwrap();
    let s2 = repo.create(new_codex("t2")).await.unwrap();
    assert_eq!(
        mgr.late_capture_provider_id(&s2).await,
        None,
        "a claimed rollout id must never be re-claimed by another session"
    );
}

/// The reopen contract for a terminal the user ran an agent in: the
/// captured conversation comes back verbatim, and a `cd` is emitted ONLY
/// when the agent was launched somewhere other than the shell's own cwd
/// (claude/codex/agy all resolve a conversation relative to the directory
/// they run in, so resuming from the wrong one finds nothing).
#[test]
fn nested_resume_command_reflects_the_captured_launch() {
    let mk = |psid: Option<&str>, meta: serde_json::Value| Session {
        id: "s".into(),
        workspace_id: "ws".into(),
        kind: SessionKind::Agent,
        provider: "shell".into(),
        title: "t".into(),
        status: SessionStatus::Reconnectable,
        cwd: "/Users/dev/project".into(),
        provider_session_id: psid.map(str::to_string),
        connection_id: None,
        created_by: "u".into(),
        created_at: chrono::Utc::now(),
        last_active_at: chrono::Utc::now(),
        archived: false,
        meta,
    };
    let same_dir = serde_json::json!({
        "nested_provider": "claude", "nested_cwd": "/Users/dev/project", "nested_pid": 42,
    });
    assert_eq!(
        nested_resume_command(&mk(Some("abc-123"), same_dir.clone())).as_deref(),
        Some("claude --resume 'abc-123'"),
    );
    // Launched after a `cd` → the resume has to go back there first.
    let other_dir = serde_json::json!({
        "nested_provider": "codex", "nested_cwd": "/Users/dev/other", "nested_pid": 42,
    });
    assert_eq!(
        nested_resume_command(&mk(Some("abc"), other_dir)).as_deref(),
        Some("cd '/Users/dev/other' && codex resume 'abc'"),
    );
    // A plain terminal — nothing was ever captured — just respawns empty.
    assert_eq!(
        nested_resume_command(&mk(None, serde_json::json!({}))),
        None
    );
    // Half a capture (id but no provider, or the reverse) is never guessed at.
    assert_eq!(
        nested_resume_command(&mk(Some("abc"), serde_json::json!({}))),
        None
    );
    assert_eq!(nested_resume_command(&mk(None, same_dir)), None);
}

/// The sweep's kill branch (fd-leak fix): a live agent session that can
/// never be suspended (no resume id) used to hold its PTY, agent process
/// and MCP sidecar forever — 60 leaked review agents took the daemon over
/// launchd's 256-fd cap. Only engine-owned (background) agent sessions
/// past the reap grace qualify; the user's own sessions and connection
/// terminals never do.
#[test]
fn reap_decision_targets_only_idle_background_agents() {
    let mk = |kind: SessionKind, meta: serde_json::Value| Session {
        id: "s".into(),
        workspace_id: "ws".into(),
        kind,
        provider: "codex".into(),
        title: "t".into(),
        status: SessionStatus::Idle,
        cwd: "/tmp".into(),
        provider_session_id: None,
        connection_id: None,
        created_by: "u".into(),
        created_at: chrono::Utc::now(),
        last_active_at: chrono::Utc::now(),
        archived: false,
        meta,
    };
    let past = REAP_UNRESUMABLE_GRACE;
    let under = REAP_UNRESUMABLE_GRACE - Duration::from_secs(1);
    let bg = serde_json::json!({ "source": "review" });

    // Engine-owned, idle past the grace → reap.
    assert!(should_reap_unresumable(
        &mk(SessionKind::Agent, bg.clone()),
        past
    ));
    // Not yet past the (long) grace → keep.
    assert!(!should_reap_unresumable(
        &mk(SessionKind::Agent, bg.clone()),
        under
    ));
    // Foreground (no source / unknown source) → NEVER killed.
    assert!(!should_reap_unresumable(
        &mk(SessionKind::Agent, serde_json::json!({})),
        past
    ));
    assert!(!should_reap_unresumable(
        &mk(
            SessionKind::Agent,
            serde_json::json!({ "source": "someday-new" })
        ),
        past
    ));
    // Connection terminals (ssh/db) are not agent sessions → never killed.
    assert!(!should_reap_unresumable(
        &mk(SessionKind::Connection, bg),
        past
    ));
}

/// Build a bare session row for the pure sweep-guard decisions.
fn guard_session(meta: serde_json::Value) -> Session {
    Session {
        id: "s".into(),
        workspace_id: "ws".into(),
        kind: SessionKind::Agent,
        provider: "claude".into(),
        title: "t".into(),
        status: SessionStatus::Idle,
        cwd: "/tmp".into(),
        provider_session_id: Some("psid".into()),
        connection_id: None,
        created_by: "u".into(),
        created_at: chrono::Utc::now(),
        last_active_at: chrono::Utc::now(),
        archived: false,
        meta,
    }
}

/// THE BUG: the sweep suspended the user's own interactive claude session
/// three times in one afternoon while it was mid-turn, waiting on quiet
/// background watchers. A session the user started from the Agents page is
/// recognised as such and gets the long manual grace (see
/// `manual_sessions_are_suspended_after_the_manual_grace`).
#[test]
fn manual_origin_sessions_are_recognised() {
    // The plain-create path stamps `work.origin = "manual"`.
    assert!(is_user_started(&guard_session(
        serde_json::json!({ "work": { "origin": "manual" } })
    )));
    // Older rows pre-date the stamp entirely — still the user's own.
    assert!(is_user_started(&guard_session(serde_json::json!({}))));
    // A work ref without an origin (repo/branch only) — still the user's.
    assert!(is_user_started(&guard_session(
        serde_json::json!({ "work": { "repo_id": "r1", "branch": "main" } })
    )));
    // A pinned manual session is doubly held (keep_alive guard + this one).
    assert!(is_user_started(&guard_session(
        serde_json::json!({ "keep_alive": true, "work": { "origin": "manual" } })
    )));
}

/// A user-started session that printed something a minute ago, inside the
/// default manual grace, and that somebody typed into.
const RECENT: ManualIdle = ManualIdle {
    quiet_for: Duration::from_secs(60),
    grace: Some(MANUAL_IDLE_SUSPEND),
    passive_resume: false,
};

/// r3-05-01: the manual hold is a long GRACE now, not forever. Inside it
/// the session is held whatever the transcript says; past it only the
/// open-turn guard (fresh transcript) can keep it; `0` in the setting
/// (`grace: None`) restores the old never-suspend rule.
#[test]
fn manual_sessions_are_suspended_after_the_manual_grace() {
    let manual = guard_session(serde_json::json!({ "work": { "origin": "manual" } }));
    let fresh = Some(Duration::from_secs(30));
    let past = ManualIdle {
        quiet_for: MANUAL_IDLE_SUSPEND,
        ..RECENT
    };
    // Inside the grace: held, even with a closed turn.
    assert_eq!(
        sweep_hold(&manual, Some(false), fresh, RECENT),
        Some("origin=manual")
    );
    // Just inside the boundary still holds.
    let edge = ManualIdle {
        quiet_for: MANUAL_IDLE_SUSPEND - Duration::from_secs(1),
        ..RECENT
    };
    assert_eq!(sweep_hold(&manual, None, None, edge), Some("origin=manual"));
    // Past it: reclaimable once the turn is closed / unknown…
    assert_eq!(sweep_hold(&manual, None, None, past), None);
    assert_eq!(sweep_hold(&manual, Some(false), fresh, past), None);
    // …but never mid-turn (a quiet agent `sleep`-polling a watcher).
    assert_eq!(
        sweep_hold(&manual, Some(true), fresh, past),
        Some("turn open")
    );
    // A stuck "open" tail does not pin it forever.
    assert_eq!(
        sweep_hold(&manual, Some(true), Some(REAP_UNRESUMABLE_GRACE), past),
        None
    );
    // Pinned beats everything.
    let pinned = guard_session(serde_json::json!({
        "keep_alive": true, "work": { "origin": "manual" }
    }));
    assert_eq!(sweep_hold(&pinned, None, None, past), Some("keep_alive"));
    // Setting = 0 → never auto-suspend a manual session (the old rule).
    let never = ManualIdle {
        quiet_for: Duration::from_secs(365 * 24 * 3600),
        grace: None,
        passive_resume: false,
    };
    assert_eq!(
        sweep_hold(&manual, None, None, never),
        Some("origin=manual")
    );
}

/// r3-05-01 root cause: opening a suspended session (terminal attach or
/// chat ping → `ensure_live`) spawned `claude --resume` and the manual
/// hold then kept that process forever. A process nobody has typed into
/// since the resume gets NO manual hold — the engine grace applies.
#[test]
fn passive_resume_earns_no_manual_hold() {
    let manual = guard_session(serde_json::json!({ "work": { "origin": "manual" } }));
    let passive = ManualIdle {
        passive_resume: true,
        ..RECENT
    };
    assert_eq!(sweep_hold(&manual, None, None, passive), None);
    assert_eq!(
        sweep_hold(&manual, Some(false), Some(Duration::from_secs(5)), passive),
        None
    );
    // Even the "never" setting does not pin a passive resume.
    let never = ManualIdle {
        grace: None,
        ..passive
    };
    assert_eq!(sweep_hold(&manual, None, None, never), None);
    // Pins and open turns still hold it.
    let pinned = guard_session(serde_json::json!({
        "keep_alive": true, "work": { "origin": "manual" }
    }));
    assert_eq!(sweep_hold(&pinned, None, None, passive), Some("keep_alive"));
    assert_eq!(
        sweep_hold(&manual, Some(true), Some(Duration::from_secs(5)), passive),
        Some("turn open")
    );
}

#[test]
fn live_cap_holds_and_excess() {
    let s = |meta: serde_json::Value, status: SessionStatus| {
        let mut s = guard_session(meta);
        s.status = status;
        s
    };
    let idle = s(
        serde_json::json!({ "work": { "origin": "manual" } }),
        SessionStatus::Idle,
    );
    let fresh = Some(Duration::from_secs(5));
    // The cap overrides the manual grace: an idle manual session is fair game.
    assert_eq!(cap_hold(&idle, true, Some(false), fresh), None);
    assert_eq!(cap_hold(&idle, true, None, None), None);
    // Never mid-turn, working, pinned, or unresumable.
    assert_eq!(cap_hold(&idle, true, Some(true), fresh), Some("turn open"));
    let working = s(serde_json::json!({}), SessionStatus::Working);
    assert_eq!(cap_hold(&working, true, None, None), Some("working"));
    let pinned = s(
        serde_json::json!({ "keep_alive": true }),
        SessionStatus::Idle,
    );
    assert_eq!(cap_hold(&pinned, true, None, None), Some("keep_alive"));
    assert_eq!(cap_hold(&idle, false, None, None), Some("not resumable"));
    // Only agent CLIs count toward the cap.
    assert!(counts_toward_cap(&idle));
    let mut shell = idle.clone();
    shell.provider = "shell".into();
    assert!(!counts_toward_cap(&shell));
    let mut conn = idle.clone();
    conn.kind = SessionKind::Connection;
    assert!(!counts_toward_cap(&conn));
    // Excess arithmetic; 0 = no cap.
    assert_eq!(cap_excess(14, 12), 2);
    assert_eq!(cap_excess(12, 12), 0);
    assert_eq!(cap_excess(3, 12), 0);
    assert_eq!(cap_excess(500, 0), 0);
}

/// End to end over real (sleeping) PTYs: with the cap at 2 and four live
/// user-started sessions, the sweep suspends the two LEAST-recently-used
/// ones — never the attached one, even though it is the oldest — and the
/// manual grace (disabled here) does not protect them from the cap.
#[tokio::test]
async fn live_cap_suspends_least_recently_used_unwatched_sessions() {
    let (mgr0, repo, ws, user) = test_manager().await;
    let settings = otto_state::SettingsRepo::new(repo.pool());
    settings
        .put("idle_suspend_grace_secs", &serde_json::json!(0))
        .await
        .unwrap();
    settings
        .put("manual_idle_suspend_secs", &serde_json::json!(0))
        .await
        .unwrap();
    settings
        .put("max_live_agent_sessions", &serde_json::json!(2))
        .await
        .unwrap();
    let mgr = Arc::new(
        Arc::try_unwrap(mgr0)
            .ok()
            .expect("sole owner")
            .with_settings_repo(settings),
    );
    let spec = CommandSpec {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), "exec sleep 30".into()],
        cwd: None,
        env: vec![],
    };
    let mut ids = Vec::new();
    for n in 0..4 {
        let id = seed_session(&repo, &ws, &user, Some(&format!("psid-cap-{n}"))).await;
        mgr.live
            .insert(id.clone(), Arc::new(PtyHandle::spawn(&spec).unwrap()));
        tokio::time::sleep(Duration::from_millis(20)).await;
        ids.push(id);
    }
    // Use order, oldest first: ids[3] (never touched since spawn),
    // ids[0] (attached — a viewer is on it right now), ids[1], ids[2].
    let _viewer = mgr.attach(&ids[0]);
    tokio::time::sleep(Duration::from_millis(10)).await;
    mgr.touch(&ids[1]);
    tokio::time::sleep(Duration::from_millis(10)).await;
    mgr.touch(&ids[2]);

    let n = mgr.suspend_idle_unattached().await;
    assert_eq!(n, 2, "exactly the excess is freed");
    assert!(!mgr.is_live(&ids[3]), "least recently used goes first");
    assert!(mgr.is_live(&ids[0]), "an attached session is never capped");
    assert!(!mgr.is_live(&ids[1]), "next LRU after the skipped viewer");
    assert!(mgr.is_live(&ids[2]), "most recently used survives");
    for id in [&ids[1], &ids[3]] {
        assert_eq!(
            repo.get(id).await.unwrap().status,
            SessionStatus::Reconnectable
        );
    }
    // At the cap now: a second sweep frees nothing.
    assert_eq!(mgr.suspend_idle_unattached().await, 0);
    for id in [&ids[0], &ids[2]] {
        if let Some((_, h)) = mgr.live.remove(id) {
            let _ = h.kill();
        }
    }
}

/// The passive-resume mark is cleared by real input only: an emulator
/// reply from a freshly attached terminal is not the user typing.
#[tokio::test]
async fn passive_resume_is_cleared_by_input_not_by_emulator_replies() {
    let (mgr, repo, ws, user) = test_manager().await;
    let id = seed_session(&repo, &ws, &user, Some("psid-passive")).await;
    let spec = CommandSpec {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), "exec cat >/dev/null".into()],
        cwd: None,
        env: vec![],
    };
    mgr.live
        .insert(id.clone(), Arc::new(PtyHandle::spawn(&spec).unwrap()));
    mgr.passive_resume.insert(id.clone(), ());
    // A DA reply the attach's emulator produced on its own.
    mgr.human_input(&id, &user, false, false, b"\x1b[?1;2c")
        .await
        .unwrap();
    assert!(
        mgr.is_passive_resume(&id),
        "emulator reply is not engagement"
    );
    mgr.human_input(&id, &user, false, true, b"hi")
        .await
        .unwrap();
    assert!(!mgr.is_passive_resume(&id), "typing engages the session");
    // Automation input engages too; teardown forgets the mark.
    mgr.passive_resume.insert(id.clone(), ());
    mgr.input(&id, b"x").await.unwrap();
    assert!(!mgr.is_passive_resume(&id));
    mgr.passive_resume.insert(id.clone(), ());
    mgr.kill_session(&id).await.unwrap();
    assert!(!mgr.is_passive_resume(&id));
}

/// …while engine-owned sessions keep the old behaviour: their owner has
/// already consumed the turn output, so reclaiming their RAM loses nothing.
#[test]
fn background_origin_sessions_still_suspend_when_idle() {
    for origin in ["workflow", "channel", "review", "swarm", "delegation"] {
        assert!(
            !is_user_started(&guard_session(
                serde_json::json!({ "work": { "origin": origin } })
            )),
            "origin={origin} must stay auto-suspendable"
        );
    }
    // A background `meta.source` marks an engine session even when no work
    // ref was stamped at all (most engine create paths set only `source`).
    assert!(!is_user_started(&guard_session(
        serde_json::json!({ "source": "review" })
    )));
    assert!(!is_user_started(&guard_session(
        serde_json::json!({ "source": "swarm", "work": { "origin": "manual" } })
    )));
    // `personal_agent` is deliberately OUTSIDE `BACKGROUND_SESSION_SOURCES`
    // (these stay listed in the Agents tab), so the engine stamps an
    // explicit `work.origin` — without it the sweep would read a personal
    // agent's run as the user's own and never reclaim it.
    assert!(!is_user_started(&guard_session(serde_json::json!({
        "source": "personal_agent",
        "work": { "origin": "personal_agent" }
    }))));
    // Connection terminals are not agent sessions → not "user-started"
    // agent work; the resumable check already excludes them.
    let mut conn = guard_session(serde_json::json!({}));
    conn.kind = SessionKind::Connection;
    assert!(!is_user_started(&conn));
}

/// An `end_turn` followed by a `<task-notification>` is a PENDING harness
/// wake-up: the parent is about to speak again, so the turn is still open
/// even though the transcript (and the PTY, and the CPU) have gone quiet.
#[test]
fn claude_open_turn_keeps_session_alive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let user = r#"{"type":"user","message":{"role":"user","content":"go"}}"#;
    let end_turn = r#"{"type":"assistant","message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"done"}]}}"#;
    let notif = r#"{"type":"user","message":{"role":"user","content":"<task-notification>id=1 status=completed</task-notification>"}}"#;
    let thinking = r#"{"type":"assistant","message":{"role":"assistant","stop_reason":"tool_use","content":[{"type":"tool_use","name":"Bash"}]}}"#;

    // Tail = a plain end_turn → the turn is closed.
    std::fs::write(&path, format!("{user}\n{end_turn}\n")).unwrap();
    assert_eq!(agent_turn_open("claude", &path), Some(false));

    // …but an end_turn followed by a wake-up line → still open.
    std::fs::write(&path, format!("{user}\n{end_turn}\n{notif}\n")).unwrap();
    assert_eq!(agent_turn_open("claude", &path), Some(true));

    // A non-end_turn tail (mid tool call) → open.
    std::fs::write(&path, format!("{user}\n{thinking}\n")).unwrap();
    assert_eq!(agent_turn_open("claude", &path), Some(true));

    // An `attachment` notification is stapled onto the user's NEXT prompt,
    // not a wake-up — it is the normal tail of a FINISHED session that ran
    // sub-agents, and reading it as "open" held engine sessions forever.
    let attach = r#"{"type":"attachment","content":{"text":"<task-notification>id=1 status=completed</task-notification>"}}"#;
    std::fs::write(&path, format!("{user}\n{end_turn}\n{attach}\n")).unwrap();
    assert_eq!(agent_turn_open("claude", &path), Some(false));

    // Sidechain (sub-agent) lines never decide the PARENT's turn.
    let side = r#"{"type":"assistant","isSidechain":true,"message":{"role":"assistant","stop_reason":"tool_use","content":[]}}"#;
    std::fs::write(&path, format!("{user}\n{end_turn}\n{side}\n")).unwrap();
    assert_eq!(agent_turn_open("claude", &path), Some(false));

    // A partially-written trailing line is skipped, not read as a verdict.
    std::fs::write(&path, format!("{user}\n{end_turn}\n{{\"type\":\"assi")).unwrap();
    assert_eq!(agent_turn_open("claude", &path), Some(false));

    // Only the TAIL is read (these files reach hundreds of MB): a transcript
    // far larger than the window still answers from its last lines.
    let filler = format!(
        "{}\n",
        r#"{"type":"filler","pad":"0123456789012345678901234567890123456789012345678901234567890123456789"}"#
    );
    let mut big = filler.repeat(4096); // ~400 KiB, well past TURN_TAIL_BYTES
    big.push_str(&format!("{end_turn}\n{notif}\n"));
    std::fs::write(&path, &big).unwrap();
    assert!(std::fs::metadata(&path).unwrap().len() > TURN_TAIL_BYTES);
    assert_eq!(agent_turn_open("claude", &path), Some(true));
}

/// codex: the latest `task_started` with no matching `task_complete` /
/// `turn_aborted` is an in-flight turn.
#[test]
fn codex_open_task_keeps_session_alive() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rollout-x-psid.jsonl");
    let started = |t: &str| {
        format!(
            r#"{{"type":"event_msg","ordinal":1,"payload":{{"type":"task_started","turn_id":"{t}"}}}}"#
        )
    };
    let complete = |t: &str| {
        format!(
            r#"{{"type":"event_msg","ordinal":2,"payload":{{"type":"task_complete","turn_id":"{t}","last_agent_message":"ok"}}}}"#
        )
    };
    let aborted = |t: &str| {
        format!(
            r#"{{"type":"event_msg","ordinal":3,"payload":{{"type":"turn_aborted","turn_id":"{t}"}}}}"#
        )
    };

    // Started, never completed → open.
    std::fs::write(&path, format!("{}\n", started("t1"))).unwrap();
    assert_eq!(agent_turn_open("codex", &path), Some(true));

    // Completed → closed.
    std::fs::write(&path, format!("{}\n{}\n", started("t1"), complete("t1"))).unwrap();
    assert_eq!(agent_turn_open("codex", &path), Some(false));

    // Aborted → closed.
    std::fs::write(&path, format!("{}\n{}\n", started("t1"), aborted("t1"))).unwrap();
    assert_eq!(agent_turn_open("codex", &path), Some(false));

    // A NEW turn after a completed one re-opens it…
    std::fs::write(
        &path,
        format!("{}\n{}\n{}\n", started("t1"), complete("t1"), started("t2")),
    )
    .unwrap();
    assert_eq!(agent_turn_open("codex", &path), Some(true));

    // …and a stale turn's completion does not close the live one.
    std::fs::write(
        &path,
        format!("{}\n{}\n{}\n", started("t1"), started("t2"), complete("t1")),
    )
    .unwrap();
    assert_eq!(agent_turn_open("codex", &path), Some(true));
}

/// Never suspend on error: an unreadable, empty or foreign artifact yields
/// `None` (unknown), and the sweep keeps its pre-existing decision.
#[test]
fn unreadable_transcript_falls_back_to_legacy_decision() {
    let dir = tempfile::tempdir().unwrap();
    let missing = dir.path().join("nope.jsonl");
    assert_eq!(agent_turn_open("claude", &missing), None);
    assert_eq!(agent_turn_open("codex", &missing), None);

    // Present but useless: empty, garbage, or message-less.
    let path = dir.path().join("t.jsonl");
    for body in ["", "not json at all\n", "{\"type\":\"summary\"}\n"] {
        std::fs::write(&path, body).unwrap();
        assert_eq!(agent_turn_open("claude", &path), None, "body={body:?}");
        assert_eq!(agent_turn_open("codex", &path), None, "body={body:?}");
    }

    // A provider with no transcript rule is always unknown.
    std::fs::write(&path, "{}\n").unwrap();
    assert_eq!(agent_turn_open("agy", &path), None);
    assert_eq!(agent_turn_open("shell", &path), None);

    // A directory is not readable as a file → unknown, not "idle".
    assert_eq!(agent_turn_open("claude", dir.path()), None);
}

/// r3-08-01: the nightly CLI reload must never re-exec a session mid-turn.
#[test]
fn restart_would_interrupt_guards_every_live_turn_signal() {
    let quiet = RESTART_QUIET + Duration::from_secs(1);
    let fresh = Some(Duration::from_secs(10));
    let stale = Some(REAP_UNRESUMABLE_GRACE + Duration::from_secs(1));
    // Idle, quiet, closed turn → safe to restart.
    assert!(!restart_would_interrupt(
        SessionStatus::Idle,
        false,
        quiet,
        Some(false),
        fresh
    ));
    assert!(!restart_would_interrupt(
        SessionStatus::Idle,
        false,
        quiet,
        None,
        None
    ));
    // Each guard alone holds it.
    assert!(restart_would_interrupt(
        SessionStatus::Working,
        false,
        quiet,
        None,
        None
    ));
    assert!(restart_would_interrupt(
        SessionStatus::Idle,
        true,
        quiet,
        None,
        None
    ));
    assert!(restart_would_interrupt(
        SessionStatus::Idle,
        false,
        Duration::from_secs(2),
        None,
        None
    ));
    assert!(restart_would_interrupt(
        SessionStatus::Idle,
        false,
        quiet,
        Some(true),
        fresh
    ));
    // A stuck (stale) open turn does not pin the session forever.
    assert!(!restart_would_interrupt(
        SessionStatus::Idle,
        false,
        quiet,
        Some(true),
        stale
    ));
}

/// The sweep's whole per-session hold decision, in one place: delete a
/// guard from `sweep_hold` and this fails. (The sweep loop itself walks
/// `self.live`, which needs real child processes, so it cannot be driven
/// from a unit test — hence the pure predicate.)
#[test]
fn sweep_hold_names_the_guard_that_keeps_a_session_alive() {
    let fresh = Some(Duration::from_secs(30));
    let bg = || guard_session(serde_json::json!({ "source": "review" }));

    // No guard applies → the sweep reclaims it.
    assert_eq!(sweep_hold(&bg(), Some(false), fresh, RECENT), None);
    assert_eq!(sweep_hold(&bg(), None, None, RECENT), None);

    // Pinned by the user — checked before everything, and without any
    // transcript I/O (the sweep passes `None, None` on the first pass).
    let pinned = guard_session(serde_json::json!({
        "source": "review", "keep_alive": true
    }));
    assert_eq!(sweep_hold(&pinned, None, None, RECENT), Some("keep_alive"));
    assert_eq!(
        sweep_hold(&pinned, Some(false), fresh, RECENT),
        Some("keep_alive")
    );

    // Started from the Agents page — never auto-suspended, whatever the
    // transcript says.
    let manual = guard_session(serde_json::json!({ "work": { "origin": "manual" } }));
    assert_eq!(
        sweep_hold(&manual, None, None, RECENT),
        Some("origin=manual")
    );
    assert_eq!(
        sweep_hold(&manual, Some(false), fresh, RECENT),
        Some("origin=manual")
    );

    // An engine session mid-turn, on a transcript that is still moving.
    assert_eq!(
        sweep_hold(&bg(), Some(true), fresh, RECENT),
        Some("turn open")
    );

    // …but an "open" turn on a transcript that has not moved for the full
    // reap grace is STUCK, not live: it must not hold the session, or a
    // single un-answered notification tail leaks the PTY + agent process +
    // MCP sidecar forever.
    let stale = Some(REAP_UNRESUMABLE_GRACE);
    assert_eq!(sweep_hold(&bg(), Some(true), stale, RECENT), None);
    assert_eq!(
        sweep_hold(&bg(), Some(true), Some(REAP_UNRESUMABLE_GRACE * 4), RECENT),
        None
    );
    // Just inside the bound still holds.
    assert_eq!(
        sweep_hold(
            &bg(),
            Some(true),
            Some(REAP_UNRESUMABLE_GRACE - Duration::from_secs(1)),
            RECENT
        ),
        Some("turn open")
    );
    // Unknown age (unreadable artifact) never holds on the turn guard.
    assert_eq!(sweep_hold(&bg(), Some(true), None, RECENT), None);
}

/// End to end over a real file, the way the sweep reads it: a claude tail
/// that LOOKS open (`end_turn` + a `<task-notification>` user line) but
/// whose transcript has not moved for 31 minutes is a stuck turn and must
/// not keep the session alive.
#[test]
fn stale_open_turn_does_not_hold_a_session() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("t.jsonl");
    let end_turn = r#"{"type":"assistant","message":{"role":"assistant","stop_reason":"end_turn","content":[{"type":"text","text":"done"}]}}"#;
    let notif = r#"{"type":"user","message":{"role":"user","content":"<task-notification>id=1 status=completed</task-notification>"}}"#;
    std::fs::write(&path, format!("{end_turn}\n{notif}\n")).unwrap();

    // The verdict itself is "open" either way — the bound is what caps it.
    assert_eq!(agent_turn_open("claude", &path), Some(true));

    // Age read exactly as the sweep reads it.
    let age = |p: &std::path::Path| {
        std::fs::metadata(p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
    };
    let session = guard_session(serde_json::json!({ "source": "review" }));

    // Fresh → held.
    assert_eq!(
        sweep_hold(
            &session,
            agent_turn_open("claude", &path),
            age(&path),
            RECENT
        ),
        Some("turn open")
    );

    // Backdate the transcript 31 minutes (past REAP_UNRESUMABLE_GRACE).
    let old = std::time::SystemTime::now() - Duration::from_secs(31 * 60);
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_modified(old)
        .unwrap();
    assert!(age(&path).unwrap() >= REAP_UNRESUMABLE_GRACE);
    assert_eq!(
        sweep_hold(
            &session,
            agent_turn_open("claude", &path),
            age(&path),
            RECENT
        ),
        None,
        "a 31-minute-old 'open turn' tail must not hold the session"
    );
}

#[tokio::test]
async fn idle_suspend_skips_attached_sessions() {
    let (mgr, repo, ws, user) = test_manager().await;
    let id = seed_session(&repo, &ws, &user, Some("sid")).await;
    // Mark it as a (fake) live session so the sweep considers it, but with
    // an attachment so it must be skipped. We can't spawn a real PTY here,
    // so assert the guard semantics the sweep relies on: an attached
    // session reports is_attached == true.
    let _g = mgr.attach(&id);
    assert!(mgr.is_attached(&id));
    // The sweep over live sessions is a no-op (no live PTYs), and the
    // attachment registry it consults is correct.
    assert_eq!(mgr.suspend_idle_unattached().await, 0);
}

#[tokio::test]
async fn idle_suspend_skips_recently_viewed_sessions() {
    let (mgr, repo, ws, user) = test_manager().await;
    let id = seed_session(&repo, &ws, &user, Some("sid")).await;
    // An open CHAT mounts no terminal, so it never `attach()`es — it pings
    // `POST …/transcript/touch` once a minute instead. That ping must hold
    // the session exactly like a WS viewer does, or the sweep suspends a
    // conversation the user is looking at (observed live: both open chats
    // suspended 5 min after the last output, composer dead until reload).
    assert!(!mgr.is_watched(&id));
    mgr.note_view(&id);
    assert!(mgr.is_watched(&id));
    assert!(mgr.viewed_within(&id, VIEW_HOLD));
    // A stale view (older than the hold) no longer counts.
    assert!(!mgr.viewed_within(&id, Duration::ZERO));
    assert_eq!(mgr.suspend_idle_unattached().await, 0);
}

#[tokio::test]
async fn idle_suspend_skips_sessions_an_engine_turn_is_watching() {
    let (mgr, repo, ws, user) = test_manager().await;
    let id = seed_session(&repo, &ws, &user, Some("sid")).await;
    // A workflow step's driver polls the transcript with no terminal and
    // no chat view; the oracle's linger holds outlast the idle grace.
    // Observed live: the sweep suspended the step's session 8 min into a
    // 15-min background-task linger and the engine re-ran the step.
    assert!(!mgr.engine_turn_open(&id));
    assert!(!mgr.is_watched(&id));
    let hold = mgr.hold_for_turn(&id);
    assert!(mgr.engine_turn_open(&id));
    assert!(mgr.is_watched(&id));
    // Nested drivers (a retry overlapping a slow teardown) count separately.
    let hold2 = mgr.hold_for_turn(&id);
    drop(hold);
    assert!(mgr.engine_turn_open(&id));
    assert_eq!(mgr.suspend_idle_unattached().await, 0);
    // The last guard dropping — on ANY return path — releases the session.
    drop(hold2);
    assert!(!mgr.engine_turn_open(&id));
    assert!(!mgr.is_watched(&id));
    assert!(mgr.engine_turns.get(&id).is_none());
}

#[tokio::test]
async fn evict_signal_fires_to_subscribers() {
    let (mgr, repo, ws, user) = test_manager().await;
    let id = seed_session(&repo, &ws, &user, Some("sid")).await;

    // Two attached viewers each subscribe to the per-session disconnect
    // signal (broadcast so every viewer is dropped, not just one).
    let mut rx1 = mgr.evict_signal(&id);
    let mut rx2 = mgr.evict_signal(&id);

    // Nothing fired yet.
    assert!(rx1.try_recv().is_err());

    // Firing the signal yields a unit to every subscriber.
    mgr.evict(&id);
    assert!(rx1.recv().await.is_ok(), "subscriber 1 must receive evict");
    assert!(rx2.recv().await.is_ok(), "subscriber 2 must receive evict");
}

#[tokio::test]
async fn evict_without_subscribers_is_noop() {
    let (mgr, repo, ws, user) = test_manager().await;
    let id = seed_session(&repo, &ws, &user, None).await;
    // No receivers exist; evict must not panic or error (no-op).
    mgr.evict(&id);
    // A subscriber created afterwards does not see the earlier (lost) send.
    let mut rx = mgr.evict_signal(&id);
    assert!(rx.try_recv().is_err());
}

#[tokio::test]
async fn prune_keeps_session_with_existing_transcript() {
    let (mgr, repo, ws, user) = test_manager().await;
    // Point HOME at a tempdir holding a matching transcript.
    let home = tempfile::tempdir().unwrap();
    let cwd = "/tmp";
    let psid = "exists-1111";
    let proj = home
        .path()
        .join(".claude")
        .join("projects")
        .join(crate::lifecycle::claude_project_dir_name(cwd));
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join(format!("{psid}.jsonl")), b"{}").unwrap();

    let id = seed_session(&repo, &ws, &user, Some(psid)).await;
    repo.update_status(&id, SessionStatus::Reconnectable)
        .await
        .unwrap();

    // Scoped: restored on drop, BEFORE the tempdir is deleted.
    let _home = EnvVarGuard::set("HOME", home.path());
    let pruned = mgr.prune_dead_sessions().await;
    assert_eq!(pruned, 0, "existing transcript must be kept");
    assert!(repo.get(&id).await.is_ok());
}

// ── Grid-size resolution tests ───────────────────────────────────────────

/// `resolve_grid` returns the clamped values when both fall in-range.
#[test]
fn resolve_grid_in_range() {
    let (c, r) = resolve_grid(Some(132), Some(50));
    assert_eq!(c, 132);
    assert_eq!(r, 50);
}

/// Out-of-range cols fall back to the default; rows are accepted when valid.
#[test]
fn resolve_grid_cols_out_of_range_falls_back() {
    // cols = 0 is below MIN_COLS (20) → default 80
    let (c, r) = resolve_grid(Some(0), Some(40));
    assert_eq!(c, otto_pty::DEFAULT_COLS, "zero cols should yield default");
    assert_eq!(r, 40);

    // cols = 501 is above MAX_COLS (500) → default 80
    let (c, _) = resolve_grid(Some(501), Some(24));
    assert_eq!(
        c,
        otto_pty::DEFAULT_COLS,
        "oversized cols should yield default"
    );
}

/// Rows out-of-range fall back to the default.
#[test]
fn resolve_grid_rows_out_of_range_falls_back() {
    let (_, r) = resolve_grid(Some(80), Some(1));
    assert_eq!(
        r,
        otto_pty::DEFAULT_ROWS,
        "rows below MIN_ROWS should yield default"
    );

    let (_, r) = resolve_grid(Some(80), Some(201));
    assert_eq!(
        r,
        otto_pty::DEFAULT_ROWS,
        "rows above MAX_ROWS should yield default"
    );
}

/// `None` values yield the defaults.
#[test]
fn resolve_grid_none_yields_defaults() {
    let (c, r) = resolve_grid(None, None);
    assert_eq!(c, otto_pty::DEFAULT_COLS);
    assert_eq!(r, otto_pty::DEFAULT_ROWS);
}

// ── model_args tests ────────────────────────────────────────────────────

/// A read-only claude session loses its shell and sub-agents.
#[test]
fn read_only_args_strip_the_shell_for_read_only_claude_sessions() {
    let on = serde_json::json!({ "read_only": true });
    let args = read_only_args("claude", &on);
    assert_eq!(args[0], "--disallowed-tools");
    assert!(args[1].split(' ').any(|t| t == "Bash"));
    assert!(args[1].split(' ').any(|t| t == "Task"));
    assert!(read_only_args("claude", &serde_json::json!({})).is_empty());
    assert!(read_only_args("claude", &serde_json::json!({ "read_only": "yes" })).is_empty());
    assert!(read_only_args("codex", &on).is_empty());
    // No project settings / MCP / web lockdown unless the caller marks the
    // checkout untrusted (a personal agent keeps its MCP + web tools).
    assert!(!args.iter().any(|a| a == "--setting-sources"));
    assert!(!args.iter().any(|a| a == "--strict-mcp-config"));
    assert!(!args[1].split(' ').any(|t| t == "WebFetch"));
}

/// A review session: shell gone, sub-agents kept for orchestrator lenses,
/// and the checkout's own `.claude/settings*.json` (hooks) never loaded.
#[test]
fn read_only_review_args_keep_subagents_and_drop_project_settings() {
    let meta = serde_json::json!({
        "read_only": true, "allow_subagents": true, "project_settings": false
    });
    let args = read_only_args("claude", &meta);
    assert_eq!(args[0], "--disallowed-tools");
    assert!(args[1].split(' ').any(|t| t == "Bash"));
    assert!(!args[1].split(' ').any(|t| t == "Task"));
    let at = args.iter().position(|a| a == "--setting-sources").unwrap();
    assert_eq!(args[at + 1], "user");
    // S2-301(b): no web tools and no MCP server at all on an untrusted checkout.
    assert!(args[1].split(' ').any(|t| t == "WebFetch"));
    assert!(args[1].split(' ').any(|t| t == "WebSearch"));
    assert!(args.iter().any(|a| a == "--strict-mcp-config"));
    assert!(!args.iter().any(|a| a == "--mcp-config"));
    // project_settings without read_only does nothing (never a silent opt-in).
    assert!(read_only_args("claude", &serde_json::json!({ "project_settings": false })).is_empty());
}

/// S2-301(a)(d): the full spawn argv a REVIEWER gets, per provider. codex
/// loses live web search and never loads the checkout's AGENTS.md as
/// instructions; claude loses shell + web tools + every MCP server.
#[test]
fn review_argv_per_provider_is_locked_down() {
    let reg = crate::providers::ProviderRegistry::new(None);
    let meta = serde_json::json!({
        "source": "review", "read_only": true, "project_settings": false
    });
    let argv = |provider: &str| {
        let mut spec = reg.build_spec(provider, "sid-1", "/tmp/co", false).unwrap();
        read_only_strip(provider, &meta, &mut spec.args);
        spec.args.extend(read_only_args(provider, &meta));
        spec.args
    };
    let codex = argv("codex");
    assert!(!codex.iter().any(|a| a == "--search"), "{codex:?}");
    let at = codex
        .iter()
        .position(|a| a == "-c")
        .expect("project-doc override");
    assert_eq!(codex[at + 1], "project_doc_max_bytes=0");
    let claude = argv("claude");
    let at = claude
        .iter()
        .position(|a| a == "--disallowed-tools")
        .unwrap();
    for tool in ["Bash", "Task", "WebFetch", "WebSearch", "NotebookEdit"] {
        assert!(
            claude[at + 1].split(' ').any(|t| t == tool),
            "{tool}: {claude:?}"
        );
    }
    assert!(claude.iter().any(|a| a == "--strict-mcp-config"));
    assert!(!claude.iter().any(|a| a == "--mcp-config"));

    // A read-only personal agent (trusted folder) keeps codex web search
    // stripped but its project docs; a normal codex session keeps --search.
    let personal = serde_json::json!({ "read_only": true });
    let mut spec = reg.build_spec("codex", "sid-2", "/tmp/co", false).unwrap();
    read_only_strip("codex", &personal, &mut spec.args);
    assert!(!spec.args.iter().any(|a| a == "--search"));
    assert!(read_only_args("codex", &personal).is_empty());
    let mut spec = reg.build_spec("codex", "sid-3", "/tmp/co", false).unwrap();
    read_only_strip("codex", &serde_json::json!({}), &mut spec.args);
    assert!(spec.args.iter().any(|a| a == "--search"));
}

/// claude with a model set → ["--model", name].
#[test]
fn lean_turn_args_only_for_opted_in_claude() {
    let on = serde_json::json!({ "lean_turn": true });
    let args = lean_turn_args("claude", &on);
    assert_eq!(args[0], "--strict-mcp-config");
    assert_eq!(args[1], "--disallowed-tools");
    // Read/Grep/Glob stay allowed — a truncated diff still needs an escape hatch.
    assert!(
        !args[2].contains("Read"),
        "read-only tools must stay allowed"
    );
    assert!(args[2].contains("Bash"));

    // Off / absent / wrong provider ⇒ no flags at all.
    assert!(lean_turn_args("claude", &serde_json::json!({ "lean_turn": false })).is_empty());
    assert!(lean_turn_args("claude", &serde_json::json!({})).is_empty());
    assert!(
        lean_turn_args("codex", &on).is_empty(),
        "codex takes neither flag"
    );
}

/// The built-in `--model {model}` template (claude/codex/agy) expands to
/// `["--model", <name>]`.
#[test]
fn model_args_builtin_template_with_model() {
    let tpl = vec!["--model".to_string(), "{model}".to_string()];
    let meta = serde_json::json!({ "model": "claude-opus-4-8" });
    let args = model_args(Some(&tpl), &meta);
    assert_eq!(args, vec!["--model", "claude-opus-4-8"]);
}

/// A custom provider's template (`-m {model}`) is honored verbatim, with
/// `{model}` substituted wherever it appears.
#[test]
fn model_args_custom_template() {
    let tpl = vec!["-m".to_string(), "{model}".to_string()];
    let meta = serde_json::json!({ "model": "grok-4" });
    let args = model_args(Some(&tpl), &meta);
    assert_eq!(args, vec!["-m", "grok-4"]);

    // `{model}` embedded inside a larger element substitutes too.
    let tpl = vec!["--model={model}".to_string()];
    assert_eq!(args_join(model_args(Some(&tpl), &meta)), "--model=grok-4");
}

/// No template (shell, template-less custom provider) → the pinned model
/// is dropped regardless of meta. Pickers hide the control via
/// `/meta.model_flags`, so this is never a surprise.
#[test]
fn model_args_no_template_drops_model() {
    let meta = serde_json::json!({ "model": "some-model" });
    assert!(model_args(None, &meta).is_empty());
}

/// No model in meta → empty vec even with a template.
#[test]
fn model_args_absent_model_empty() {
    let tpl = vec!["--model".to_string(), "{model}".to_string()];
    let args = model_args(Some(&tpl), &serde_json::json!({}));
    assert!(args.is_empty(), "no model key should yield no args");
}

/// Whitespace-only model is silently skipped.
#[test]
fn model_args_blank_model_empty() {
    let tpl = vec!["--model".to_string(), "{model}".to_string()];
    let args = model_args(Some(&tpl), &serde_json::json!({ "model": "   " }));
    assert!(args.is_empty(), "blank model should yield no args");
}

/// Leading/trailing whitespace is trimmed from the model name.
#[test]
fn model_args_model_is_trimmed() {
    let tpl = vec!["--model".to_string(), "{model}".to_string()];
    let args = model_args(Some(&tpl), &serde_json::json!({ "model": "  opus  " }));
    assert_eq!(args, vec!["--model", "opus"]);
}

fn args_join(v: Vec<String>) -> String {
    v.join(" ")
}

/// `add_dir_args` is provider-agnostic: ANY non-shell provider handed
/// `extra_dirs` gets `--add-dir`. This is the contract that makes gating a
/// skill bundle to claude the CALLER's job (see
/// `otto_server::review_session::review_skills_extra_dirs`) — handing the
/// bundle to codex here would re-introduce the wrong-skill bug.
#[test]
fn add_dir_args_emits_for_any_non_shell_with_extra_dirs() {
    let meta = serde_json::json!({ "extra_dirs": ["/bundle"] });
    assert_eq!(add_dir_args("claude", &meta), vec!["--add-dir", "/bundle"]);
    assert_eq!(add_dir_args("codex", &meta), vec!["--add-dir", "/bundle"]);
    assert_eq!(add_dir_args("agy", &meta), vec!["--add-dir", "/bundle"]);
}

/// shell never gets `--add-dir`, and an absent/empty `extra_dirs` yields none.
#[test]
fn add_dir_args_empty_for_shell_or_no_dirs() {
    let meta = serde_json::json!({ "extra_dirs": ["/bundle"] });
    assert!(add_dir_args("shell", &meta).is_empty());
    assert!(add_dir_args("codex", &serde_json::json!({})).is_empty());
    // Empty-string entries are skipped.
    let empties = serde_json::json!({ "extra_dirs": ["", " "] });
    assert_eq!(add_dir_args("claude", &empties), vec!["--add-dir", " "]);
}

/// A session spawned with saved grid meta reports that size via screen_size().
#[tokio::test]
async fn spawn_sized_restores_grid() {
    use otto_pty::{CommandSpec, PtyHandle};
    let spec = CommandSpec {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), "exit 0".into()],
        cwd: None,
        env: vec![],
    };
    let handle = PtyHandle::spawn_sized(&spec, 132, 50).expect("spawn");
    let (rows, cols) = handle.screen_size();
    assert_eq!(cols, 132, "restored cols");
    assert_eq!(rows, 50, "restored rows");
}

/// S1-10: the sandbox protects the CONFIGURED data dir, not a hard-coded
/// default — a daemon on `$OTTO_DATA_DIR` must not leave its real
/// `secrets.json` / `otto.db` / `bin/ottod` open to the agent.
#[tokio::test]
async fn sandbox_protects_the_configured_data_dir() {
    let dir = tempfile::tempdir().unwrap();
    let pool = otto_state::open(&dir.path().join("t.db")).await.unwrap();
    let (events, _rx) = broadcast::channel(16);
    let home = std::path::Path::new("/nonexistent-otto-home/u");
    let custom = std::path::PathBuf::from("/nonexistent-otto-data/custom");
    let mgr = SessionManager::new(SessionsRepo::new(pool), events, ProviderRegistry::new(None))
        .with_data_dir(custom.clone());
    assert_eq!(mgr.sandbox_data_dir(home), custom);
    let sbpl = sandbox_policy(
        std::path::Path::new("/nonexistent-otto-work/p"),
        home,
        &mgr.sandbox_data_dir(home),
        &[],
        otto_sandbox::NetworkPolicy::Full,
        false,
    )
    .to_sbpl();
    assert!(sbpl.contains("(deny file-write* (subpath \"/nonexistent-otto-data/custom\"))"));
    assert!(sbpl.contains("(literal \"/nonexistent-otto-data/custom/secrets.json\")"));
    assert!(!sbpl.contains("Library/Application Support/Otto"));
}

/// S1-03(d): only read-only sessions lose write access to the project's
/// claude config.
#[test]
fn read_only_sandbox_denies_project_claude_config() {
    let p = |ro| {
        sandbox_policy(
            std::path::Path::new("/nonexistent-otto-work/p"),
            std::path::Path::new("/nonexistent-otto-home/u"),
            std::path::Path::new("/nonexistent-otto-data/d"),
            &[],
            otto_sandbox::NetworkPolicy::Full,
            ro,
        )
        .to_sbpl()
    };
    let needle = "(literal \"/nonexistent-otto-work/p/.mcp.json\")";
    assert!(p(true).contains(needle));
    assert!(!p(false).contains(needle));
}

/// S3-04: a scheduled shell task (`process_sandbox_policy("shell", …)`)
/// is gated exactly like a `shell` agent session: off when the setting is
/// off or names other providers, on by default when enabled.
#[test]
fn sandbox_decision_covers_scheduled_shell_tasks() {
    let off = serde_json::json!({ "enabled": false });
    let not_shell = serde_json::json!({ "enabled": true, "providers": ["claude"] });
    let on = serde_json::json!({ "enabled": true });
    assert_eq!(sandbox_decision(&off, SessionKind::Agent, "shell"), None);
    assert_eq!(
        sandbox_decision(&not_shell, SessionKind::Agent, "shell"),
        None
    );
    assert!(sandbox_decision(&on, SessionKind::Agent, "shell").is_some());
}

/// S11-309: with the sandbox on, a provider nobody listed (a user-added
/// CLI) is confined; only a person's explicit exemption (or the legacy
/// list leaving out a BUILT-IN provider) lets one run unconfined.
#[test]
fn sandbox_decision_confines_unlisted_providers_by_default() {
    let on = serde_json::json!({ "enabled": true });
    for p in ["opencode", "grok", "gemini", "my-cli"] {
        assert!(
            sandbox_decision(&on, SessionKind::Agent, p).is_some(),
            "{p}"
        );
    }
    let legacy = serde_json::json!({ "enabled": true, "providers": ["claude"] });
    assert!(sandbox_decision(&legacy, SessionKind::Agent, "opencode").is_some());
    assert!(sandbox_decision(&legacy, SessionKind::Agent, "claude").is_some());
    assert_eq!(sandbox_decision(&legacy, SessionKind::Agent, "codex"), None);
    let exempt = serde_json::json!({ "enabled": true, "exempt_providers": ["opencode"] });
    assert_eq!(
        sandbox_decision(&exempt, SessionKind::Agent, "opencode"),
        None
    );
    assert!(sandbox_decision(&exempt, SessionKind::Agent, "claude").is_some());
}

/// S11-302: no agent session at `/`, a parent of `$HOME`, or Otto's data
/// dir — refused before anything spawns; an ordinary folder is fine.
#[tokio::test]
async fn create_refuses_unsafe_session_folders() {
    let (mgr, _repo, ws, _user) = test_manager().await;
    let data = tempfile::tempdir().unwrap();
    let mgr = Arc::try_unwrap(mgr)
        .ok()
        .expect("sole owner")
        .with_data_dir(data.path());
    for cwd in ["/", data.path().to_str().unwrap()] {
        let err = mgr.check_session_cwd(&ws, cwd, true).unwrap_err();
        assert!(matches!(err, Error::Invalid(_)), "{cwd}: {err:?}");
    }
    let bin = data.path().join("bin");
    assert!(mgr
        .check_session_cwd(&ws, bin.to_str().unwrap(), true)
        .is_err());
    let run = data.path().join("workflow-runs/r1");
    assert!(mgr
        .check_session_cwd(&ws, run.to_str().unwrap(), false)
        .is_ok());
    assert!(mgr.check_session_cwd(&ws, "/tmp", false).is_ok());
    if let Ok(home) = std::env::var("HOME") {
        if std::path::Path::new(&home).is_dir() {
            assert!(mgr.check_session_cwd(&ws, &home, false).is_err(), "$HOME");
            // With no sandbox profile there is no grant to widen: the
            // creator may start an agent in ~ itself (not in its parents).
            assert!(mgr.check_session_cwd(&ws, &home, true).is_ok());
            if let Some(parent) = std::path::Path::new(&home).parent() {
                let parent = parent.to_str().unwrap();
                assert!(mgr.check_session_cwd(&ws, parent, true).is_err());
            }
            // …unless the workspace itself is rooted there.
            let mut home_ws = ws.clone();
            home_ws.root_path = home.clone();
            assert!(mgr.check_session_cwd(&home_ws, &home, false).is_ok());
        }
    }
}

#[test]
fn sandbox_decision_gates_correctly() {
    use otto_sandbox::NetworkPolicy;
    let on = serde_json::json!({"enabled": true, "network": "full"});
    // Agent + a default provider → sandbox with the configured network.
    assert_eq!(
        sandbox_decision(&on, SessionKind::Agent, "claude"),
        Some(NetworkPolicy::Full)
    );
    // Connection sessions are never sandboxed.
    assert_eq!(sandbox_decision(&on, SessionKind::Connection, "ssh"), None);
    // Disabled (or absent) → never.
    assert_eq!(
        sandbox_decision(
            &serde_json::json!({"enabled": false}),
            SessionKind::Agent,
            "claude"
        ),
        None
    );
    assert_eq!(
        sandbox_decision(&serde_json::json!({}), SessionKind::Agent, "claude"),
        None
    );
    // An explicit provider allowlist excludes others.
    let only_codex = serde_json::json!({"enabled": true, "providers": ["codex"]});
    assert_eq!(
        sandbox_decision(&only_codex, SessionKind::Agent, "claude"),
        None
    );
    assert_eq!(
        sandbox_decision(&only_codex, SessionKind::Agent, "codex"),
        Some(NetworkPolicy::Full)
    );
    // Network posture parsing (default = full).
    assert_eq!(
        sandbox_decision(
            &serde_json::json!({"enabled": true, "network": "loopback"}),
            SessionKind::Agent,
            "shell"
        ),
        Some(NetworkPolicy::LoopbackOnly)
    );
    assert_eq!(
        sandbox_decision(
            &serde_json::json!({"enabled": true, "network": "none"}),
            SessionKind::Agent,
            "shell"
        ),
        Some(NetworkPolicy::None)
    );
}

#[tokio::test]
async fn archive_and_restore_emit_authoritative_rows_to_every_subscriber() {
    let (mgr, repo, ws, user) = test_manager().await;
    let session = repo
        .create(NewSession {
            workspace_id: ws.id.clone(),
            kind: SessionKind::Agent,
            provider: "claude".into(),
            title: "archive test".into(),
            cwd: "/tmp".into(),
            provider_session_id: Some("provider-id".into()),
            connection_id: None,
            created_by: user,
            meta: serde_json::json!({}),
        })
        .await
        .unwrap();
    let mut a = mgr.events.subscribe();
    let mut b = mgr.events.subscribe();
    for archived in [true, false] {
        let row = if archived {
            mgr.archive(&session.id).await.unwrap()
        } else {
            mgr.unarchive(&session.id).await.unwrap()
        };
        for receiver in [&mut a, &mut b] {
            let event = loop {
                match receiver
                    .try_recv()
                    .expect("committed transition must emit a row")
                {
                    Event::SessionArchiveChanged { session } => break session,
                    _ => continue,
                }
            };
            assert_eq!(event.id, row.id);
            assert_eq!(event.workspace_id, row.workspace_id);
            assert_eq!(event.archived, archived);
            assert_eq!(event.status, row.status);
        }
    }
}

#[cfg(unix)]
#[tokio::test]
async fn prune_keeps_background_session_when_transcript_lookup_fails() {
    let (mgr, repo, ws, user) = test_manager().await;
    let home = tempfile::tempdir().unwrap();
    let session = repo
        .create(NewSession {
            workspace_id: ws.id,
            kind: SessionKind::Agent,
            provider: "claude".into(),
            title: "retain on IO error".into(),
            cwd: "/tmp/proj".into(),
            provider_session_id: Some("unreadable".into()),
            connection_id: None,
            created_by: user,
            meta: serde_json::json!({"source":"review"}),
        })
        .await
        .unwrap();
    repo.update_status(&session.id, SessionStatus::Exited)
        .await
        .unwrap();
    let path = crate::lifecycle::claude_transcript_path(home.path(), &session.cwd, "unreadable");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::os::unix::fs::symlink(&path, &path).unwrap();
    assert_eq!(mgr.prune_dead_sessions_with_home(home.path()).await, 0);
    assert!(repo.get(&session.id).await.is_ok());
}

/// A FOREGROUND (Agents-tab) session survives the pruner even when its
/// provider transcript is positively gone — only archive/delete may remove
/// it. Background sessions keep the existence-check prune.
#[tokio::test]
async fn prune_keeps_foreground_sessions_and_prunes_background_ones() {
    let (mgr, repo, ws, user) = test_manager().await;
    let home = tempfile::tempdir().unwrap();

    let mk = |title: &str, psid: &str, meta: serde_json::Value| NewSession {
        workspace_id: ws.id.clone(),
        kind: SessionKind::Agent,
        provider: "claude".into(),
        title: title.into(),
        cwd: "/tmp/proj".into(),
        provider_session_id: Some(psid.into()),
        connection_id: None,
        created_by: user.clone(),
        meta,
    };

    // Foreground (no meta.source), transcript GONE → must be KEPT.
    let fg = repo
        .create(mk("Messi", "psid-fg", serde_json::json!({})))
        .await
        .unwrap();
    repo.update_status(&fg.id, SessionStatus::Reconnectable)
        .await
        .unwrap();
    // Foreground with an unknown source, transcript GONE → also KEPT.
    let fg2 = repo
        .create(mk(
            "Ronaldo",
            "psid-fg2",
            serde_json::json!({"source": "someday-new"}),
        ))
        .await
        .unwrap();
    repo.update_status(&fg2.id, SessionStatus::Exited)
        .await
        .unwrap();
    // Background (channel ticket), transcript GONE → pruned as before.
    let bg = repo
        .create(mk(
            "ticket",
            "psid-bg",
            serde_json::json!({"source": "channel"}),
        ))
        .await
        .unwrap();
    repo.update_status(&bg.id, SessionStatus::Exited)
        .await
        .unwrap();
    // Background whose transcript still EXISTS → kept (unchanged behavior).
    let bg_live = repo
        .create(mk(
            "review",
            "psid-live",
            serde_json::json!({"source": "review"}),
        ))
        .await
        .unwrap();
    repo.update_status(&bg_live.id, SessionStatus::Exited)
        .await
        .unwrap();
    let proj = home
        .path()
        .join(".claude")
        .join("projects")
        .join("-tmp-proj");
    std::fs::create_dir_all(&proj).unwrap();
    std::fs::write(proj.join("psid-live.jsonl"), "{}\n").unwrap();

    let pruned = mgr.prune_dead_sessions_with_home(home.path()).await;
    assert_eq!(pruned, 1, "exactly the gone-transcript background session");
    assert!(
        repo.get(&fg.id).await.is_ok(),
        "foreground survives a gone transcript"
    );
    assert!(
        repo.get(&fg2.id).await.is_ok(),
        "unknown-source foreground survives too"
    );
    assert!(
        repo.get(&bg.id).await.is_err(),
        "channel session with gone transcript is pruned"
    );
    assert!(
        repo.get(&bg_live.id).await.is_ok(),
        "existing transcript keeps a background session"
    );
}

#[tokio::test]
async fn kill_retires_live_generation_before_returning_without_status_poller() {
    let (manager, repo, ws, user) = test_manager().await;
    let id = seed_session(&repo, &ws, &user, None).await;
    let handle = Arc::new(
        PtyHandle::spawn(&CommandSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "exec cat >/dev/null".into()],
            cwd: None,
            env: vec![],
        })
        .unwrap(),
    );
    let mut exited = handle.on_exit();
    manager.live.insert(id.clone(), handle);
    // No status task is installed: kill itself must retire the generation.
    manager.kill_session(&id).await.unwrap();
    assert!(!manager.is_live(&id));
    assert!(manager.retiring.contains_key(&id));
    assert_eq!(repo.get(&id).await.unwrap().status, SessionStatus::Exited);
    tokio::time::timeout(Duration::from_secs(2), async {
        while exited.borrow().is_none() {
            exited.changed().await.expect("child exit is reported");
        }
    })
    .await
    .expect("killed child exited");
}
