use super::*;
use crate::submit::SubmitOpts;

/// architecture-4-01 / concurrency-async-4-01: the LEGACY submit
/// (`SubmitOpts::default()`, the one scheduled tasks, goal loops, product,
/// personal agents, insights and the channel bridge use) holds the session's
/// submit lock from paste through Enter and re-Enter, so a `submit_text`
/// racing into the same session lands as its own paste + Enter instead of
/// between the prompt's paste and its Enter (one merged prompt plus an empty
/// one). The verified path is covered by otto-server's
/// `submit_prompt_and_submit_text_do_not_interleave`.
#[tokio::test]
async fn scheduled_style_submit_and_submit_text_do_not_interleave() {
    let (mgr, repo, ws, owner) = test_manager().await;
    let id = seed_session(&repo, &ws, &owner, None).await;
    let tmp = tempfile::tempdir().unwrap();
    // The line discipline echoes what is typed while `cat` records exactly
    // what reached the PTY; each Enter ends one recorded line.
    let sink = tmp.path().join("pty-input");
    let handle = Arc::new(
        PtyHandle::spawn(&CommandSpec {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                format!("echo ready; exec cat > '{}'", sink.display()),
            ],
            cwd: Some(tmp.path().to_string_lossy().into()),
            env: vec![],
        })
        .unwrap(),
    );
    mgr.live.insert(id.clone(), Arc::clone(&handle));
    let prompt = tokio::spawn({
        let mgr = Arc::clone(&mgr);
        let id = id.clone();
        async move {
            mgr.submit_prompt(&id, "from-scheduled task", SubmitOpts::default())
                .await
        }
    });
    // Race in the moment the prompt's paste is on screen, before its Enter.
    let mut echoed = false;
    for _ in 0..400 {
        if String::from_utf8_lossy(&handle.scrollback(50)).contains("from-scheduled") {
            echoed = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(echoed, "the prompt's paste never echoed");
    mgr.submit_text(&id, "from-text").await.unwrap();
    assert!(
        prompt.await.unwrap(),
        "submit_prompt reported a dead session"
    );
    let mut lines = Vec::new();
    for _ in 0..100 {
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
    // A re-Enter (when the Enter's echo was missed) only adds an empty line.
    lines.sort();
    assert_eq!(
        lines,
        vec![
            "\x1b[200~from-scheduled task\x1b[201~".to_string(),
            "\x1b[200~from-text\x1b[201~".to_string(),
        ],
        "each submission must be its own paste + Enter"
    );
    let _ = handle.kill();
    mgr.remove(&id).await.unwrap();
}

/// architecture-4-01r: a run that has ended types nothing — the guard is
/// checked before the paste (and before every re-inject), so a stopped swarm
/// run's late submit never lands in a session it no longer owns. The same
/// session still takes a submit whose run is active.
#[tokio::test]
async fn ended_run_guard_blocks_the_paste() {
    use crate::submit::RunGuard;
    use std::sync::atomic::{AtomicBool, Ordering};
    let (mgr, repo, ws, owner) = test_manager().await;
    let id = seed_session(&repo, &ws, &owner, None).await;
    let tmp = tempfile::tempdir().unwrap();
    let sink = tmp.path().join("pty-input");
    let handle = Arc::new(
        PtyHandle::spawn(&CommandSpec {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                format!("echo ready; exec cat > '{}'", sink.display()),
            ],
            cwd: Some(tmp.path().to_string_lossy().into()),
            env: vec![],
        })
        .unwrap(),
    );
    mgr.live.insert(id.clone(), Arc::clone(&handle));
    let active = Arc::new(AtomicBool::new(false));
    // The async form the swarm uses (a DB read): each check is a future.
    let guard = RunGuard::new({
        let active = Arc::clone(&active);
        move || {
            let active = Arc::clone(&active);
            async move { active.load(Ordering::SeqCst) }
        }
    });
    let out = mgr
        .submit_prompt_while(&id, "late-run", SubmitOpts::default(), guard.clone())
        .await;
    assert!(out.is_none(), "an ended run must not report a submit");
    active.store(true, Ordering::SeqCst);
    assert!(mgr
        .submit_prompt_while(&id, "live-run", SubmitOpts::default(), guard)
        .await
        .is_some());
    let mut got = String::new();
    for _ in 0..100 {
        got = std::fs::read_to_string(&sink).unwrap_or_default();
        if got.contains("live-run") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    assert!(got.contains("live-run"), "{got:?}");
    assert!(!got.contains("late-run"), "the ended run typed: {got:?}");
    let _ = handle.kill();
    mgr.remove(&id).await.unwrap();
}

/// architecture-4-01r: a run stopped AFTER the TUI wait but before the paste
/// types nothing. The submit is parked on the session's submit lock (another
/// submitter holds it) with its TUI ready; the run ends; once the lock frees,
/// the guard — checked under the lock — refuses the paste, and the async
/// guard is what the swarm runs on (`engine::run_guard`).
#[tokio::test]
async fn a_run_stopped_between_the_wait_and_the_paste_pastes_nothing() {
    use crate::submit::RunGuard;
    use std::sync::atomic::{AtomicBool, Ordering};
    let (mgr, repo, ws, owner) = test_manager().await;
    let id = seed_session(&repo, &ws, &owner, None).await;
    let tmp = tempfile::tempdir().unwrap();
    let sink = tmp.path().join("pty-input");
    let handle = Arc::new(
        PtyHandle::spawn(&CommandSpec {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                format!("echo ready; exec cat > '{}'", sink.display()),
            ],
            cwd: Some(tmp.path().to_string_lossy().into()),
            env: vec![],
        })
        .unwrap(),
    );
    mgr.live.insert(id.clone(), Arc::clone(&handle));
    let active = Arc::new(AtomicBool::new(true));
    let checks = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let guard = RunGuard::new({
        let (active, checks) = (Arc::clone(&active), Arc::clone(&checks));
        move || {
            let (active, checks) = (Arc::clone(&active), Arc::clone(&checks));
            async move {
                checks.fetch_add(1, Ordering::SeqCst);
                active.load(Ordering::SeqCst)
            }
        }
    });
    let lock = mgr.submit_lock(&id);
    let held = lock.lock().await;
    let submit = tokio::spawn({
        let (mgr, id) = (Arc::clone(&mgr), id.clone());
        async move {
            mgr.submit_prompt_while(&id, "stopped-run", SubmitOpts::VERIFIED, guard)
                .await
        }
    });
    // The TUI wait passes (it checks the guard while polling), then the
    // submit queues on the held lock.
    for _ in 0..200 {
        if checks.load(Ordering::SeqCst) > 0
            && handle.last_output_at().elapsed() > Duration::from_secs(1)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert!(
        !submit.is_finished(),
        "the submit should be parked on the lock"
    );
    active.store(false, Ordering::SeqCst);
    drop(held);
    assert!(
        submit.await.unwrap().is_none(),
        "a stopped run reports no submit"
    );
    tokio::time::sleep(Duration::from_millis(300)).await;
    let got = std::fs::read_to_string(&sink).unwrap_or_default();
    assert!(
        !got.contains("stopped-run"),
        "the stopped run typed: {got:?}"
    );
    let _ = handle.kill();
    mgr.remove(&id).await.unwrap();
}

/// A session with no live PTY is reported as not submitted, without writing.
#[tokio::test]
async fn submit_prompt_into_a_dead_session_is_not_delivered() {
    let (mgr, repo, ws, owner) = test_manager().await;
    let id = seed_session(&repo, &ws, &owner, None).await;
    assert!(!mgr.submit_prompt(&id, "hello", SubmitOpts::default()).await);
    assert!(mgr
        .submit_prompt_at(&id, "hello", SubmitOpts::VERIFIED)
        .await
        .is_none());
}

/// test-quality-4-08: the per-provider dispatch in `sync_workspace_mcp`. An
/// agy spawn writes `<cwd>/.agents/mcp_config.json` (agy never reads
/// `.mcp.json`) listing `otto`, and records `otto` as delivered; a claude
/// spawn writes no agy file. Swapping the provider name in either arm, or
/// dropping the delivered arm, fails here.
#[tokio::test]
async fn sync_workspace_mcp_delivers_otto_to_agy_through_its_own_file() {
    let (mgr, repo, ws, user) = test_manager().await;
    for provider in ["agy", "claude"] {
        let cwd = tempfile::tempdir().unwrap();
        let session = repo
            .create(NewSession {
                workspace_id: ws.id.clone(),
                kind: SessionKind::Agent,
                provider: provider.into(),
                title: format!("{provider} mcp"),
                cwd: cwd.path().to_string_lossy().into_owned(),
                provider_session_id: None,
                connection_id: None,
                created_by: user.clone(),
                meta: serde_json::json!({}),
            })
            .await
            .unwrap();
        let otto = crate::mcp::OttoToolsServer {
            command: "/usr/bin/true".into(),
            args: vec![],
            env: Default::default(),
        };
        mgr.sync_workspace_mcp(&session, Some(otto), false).await;
        let agy_file = cwd.path().join(".agents").join("mcp_config.json");
        let delivered = repo.get(&session.id).await.unwrap().meta["mcp_servers"].clone();
        let has_otto = delivered
            .as_array()
            .is_some_and(|a| a.iter().any(|v| v == crate::mcp::OTTO_TOOLS_KEY));
        if provider == "agy" {
            let cfg: serde_json::Value =
                serde_json::from_str(&std::fs::read_to_string(&agy_file).unwrap()).unwrap();
            assert!(
                cfg["mcpServers"].get(crate::mcp::OTTO_TOOLS_KEY).is_some(),
                "{cfg}"
            );
            assert!(has_otto, "agy delivered set: {delivered}");
        } else {
            assert!(!agy_file.exists(), "claude never gets the agy file");
            assert!(has_otto, "claude delivered set (.mcp.json): {delivered}");
        }
    }
}

/// error-handling-6-06: an enabled user server the launcher could not attach
/// lands in the session meta as `mcp_skipped` (`{server, reason}`) for the
/// session's MCP menu — and a later spawn where everything attached clears it.
#[tokio::test]
async fn sync_workspace_mcp_records_skipped_user_servers() {
    struct Servers(std::sync::Mutex<Vec<otto_core::hooks::McpServerSpec>>);
    impl crate::mcp::McpServerSource for Servers {
        fn enabled_servers(
            &self,
            _ws: &str,
        ) -> std::result::Result<Vec<otto_core::hooks::McpServerSpec>, String> {
            Ok(self.0.lock().unwrap().clone())
        }
    }
    let spec = |name: &str, env: &[(&str, &str)]| otto_core::hooks::McpServerSpec {
        name: name.into(),
        command: "node".into(),
        args: vec![],
        env: env
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
    };
    let source = Arc::new(Servers(std::sync::Mutex::new(vec![
        spec("jira", &[]),
        spec("broken", &[("NOT-A-SHELL-NAME", "x")]),
    ])));
    let (mut mgr, repo, ws, user) = test_manager().await;
    Arc::get_mut(&mut mgr).unwrap().mcp_servers = Some(source.clone());
    let cwd = tempfile::tempdir().unwrap();
    let session = repo
        .create(NewSession {
            workspace_id: ws.id.clone(),
            kind: SessionKind::Agent,
            provider: "codex".into(),
            title: "codex mcp".into(),
            cwd: cwd.path().to_string_lossy().into_owned(),
            provider_session_id: None,
            connection_id: None,
            created_by: user.clone(),
            meta: serde_json::json!({}),
        })
        .await
        .unwrap();
    mgr.sync_workspace_mcp(&session, None, false).await;
    let meta = repo.get(&session.id).await.unwrap().meta;
    assert_eq!(meta["mcp_servers"], serde_json::json!(["jira"]));
    let skipped = meta["mcp_skipped"].as_array().expect("skipped list");
    assert_eq!(skipped.len(), 1, "{meta}");
    assert_eq!(skipped[0]["server"], "broken");
    assert!(
        skipped[0]["reason"]
            .as_str()
            .is_some_and(|r| r.contains("NOT-A-SHELL-NAME")),
        "{meta}"
    );
    // Fixed and respawned: the stale list is cleared, not left behind.
    *source.0.lock().unwrap() = vec![spec("jira", &[])];
    mgr.sync_workspace_mcp(&session, None, false).await;
    let meta = repo.get(&session.id).await.unwrap().meta;
    assert!(
        meta.get("mcp_skipped").is_none_or(|v| v.is_null()),
        "{meta}"
    );
}
