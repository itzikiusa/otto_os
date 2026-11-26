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

/// 0.5.1 hotfix: spawning — or merely opening, resuming or viewing — a
/// Claude session wrote Otto's `otto` entry into the repository's
/// `.mcp.json`, and a plain (non-bypass) Claude terminal then stopped on
/// Claude Code 2.1.331's "New MCP server found in this project" dialog. Now
/// the set rides a per-session `--mcp-config` file outside the repository and
/// nothing under the cwd is touched. The stand-in CLI gates as the real one
/// does (bypass, `--strict-mcp-config` or an approval pass; any unapproved
/// project `.mcp.json` server shows the dialog): it blocks on the file the
/// old spawn wrote and starts with today's.
#[tokio::test]
async fn claude_spawn_writes_nothing_into_the_repo_and_no_dialog_blocks() {
    let (mgr, repo, ws, user) = test_manager().await;
    let cwd = tempfile::tempdir().unwrap();
    let session = repo
        .create(NewSession {
            workspace_id: ws.id.clone(),
            kind: SessionKind::Agent,
            provider: "claude".into(),
            title: "plain claude".into(),
            cwd: cwd.path().to_string_lossy().into_owned(),
            provider_session_id: None,
            connection_id: None,
            created_by: user.clone(),
            meta: serde_json::json!({ "interaction_mode": "accept-edits" }),
        })
        .await
        .unwrap();
    let otto = crate::mcp::OttoToolsServer {
        command: "/usr/bin/true".into(),
        args: vec![],
        env: Default::default(),
    };
    let cfg = crate::mcp::ManagedMcpConfig {
        otto_tools: Some(otto.clone()),
        ..Default::default()
    };
    let args = mgr.sync_workspace_mcp(&session, Some(otto), false).await;
    let left = tokio::fs::read_dir(cwd.path())
        .await
        .unwrap()
        .next_entry()
        .await
        .unwrap();
    assert!(left.is_none(), "wrote into the repository: {left:?}");
    assert_eq!(args.len(), 2, "{args:?}");
    assert_eq!(args[0], "--mcp-config");
    let config = std::path::Path::new(&args[1]);
    assert!(!config.starts_with(cwd.path()), "{config:?}");
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(config).unwrap()).unwrap();
    assert!(
        doc["mcpServers"].get(crate::mcp::OTTO_TOOLS_KEY).is_some(),
        "{doc}"
    );
    let bin = tempfile::tempdir().unwrap();
    let fake = bin.path().join("claude");
    std::fs::write(
        &fake,
        r#"#!/bin/sh
[ -f .mcp.json ] || { echo READY; exit 0; }
for a in "$@"; do
  case "$a" in --dangerously-skip-permissions|--strict-mcp-config) echo READY; exit 0 ;; esac
done
name=$(sed -n 's/^    "\([^"]*\)": {$/\1/p' .mcp.json | head -1)
[ -n "$name" ] && { echo "New MCP server found in this project: $name"; exit 3; }
echo READY
"#,
    )
    .unwrap();
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let run = |dir: std::path::PathBuf, args: Vec<String>| {
        let mut cmd = tokio::process::Command::new(&fake);
        cmd.args(args).current_dir(dir);
        async move {
            String::from_utf8_lossy(&cmd.output().await.unwrap().stdout)
                .trim()
                .to_string()
        }
    };
    assert_eq!(
        run(cwd.path().into(), args.clone()).await,
        "READY",
        "spawn args: {args:?}"
    );
    // What the old spawn did to the repository: the stand-in blocks on it.
    let old = tempfile::tempdir().unwrap();
    let old_dir = old.path().to_string_lossy().into_owned();
    crate::mcp::reconcile_managed_servers(&old_dir, &cfg).unwrap();
    assert_eq!(
        run(old.path().into(), vec![]).await,
        "New MCP server found in this project: otto"
    );
    assert_eq!(
        std::fs::metadata(config).unwrap().permissions().mode() & 0o777,
        0o600
    );
    mgr.retire_credentials(&session).await;
    assert!(!config.exists(), "removed with the session's credentials");
}

/// R1-089: a `--strict-mcp-config` Claude terminal (lean turn) takes no
/// servers, so its spawn writes no `--mcp-config` file nor any launcher env
/// file (secrets), and records no "couldn't write" skip for them. R2-003: nor
/// does a shell session, which serves no MCP at all.
#[tokio::test]
#[allow(clippy::disallowed_methods)] // a blocking dir listing in a test
async fn a_strict_claude_spawn_writes_no_mcp_files_and_skips_nothing() {
    struct Servers;
    impl crate::mcp::McpServerSource for Servers {
        fn enabled_servers(
            &self,
            _ws: &str,
        ) -> std::result::Result<Vec<otto_core::hooks::McpServerSpec>, String> {
            Ok(vec![otto_core::hooks::McpServerSpec {
                name: "jira".into(),
                command: "node".into(),
                args: vec![],
                env: [("TOKEN".to_string(), "secret".to_string())].into(),
            }])
        }
    }
    let (mut mgr, repo, ws, user) = test_manager().await;
    Arc::get_mut(&mut mgr).unwrap().mcp_servers = Some(Arc::new(Servers));
    for (provider, meta) in [
        ("claude", serde_json::json!({ "lean_turn": true })),
        ("shell", serde_json::json!({})),
        // An untrusted read-only structured thread (DR-001).
        (
            "claude",
            serde_json::json!({ "runtime": "structured", "read_only": true, "project_settings": false }),
        ),
    ] {
        let cwd = tempfile::tempdir().unwrap();
        let cwd_str = cwd.path().to_string_lossy().into_owned();
        let session = repo
            .create(NewSession {
                workspace_id: ws.id.clone(),
                kind: SessionKind::Agent,
                provider: provider.into(),
                title: "no servers".into(),
                cwd: cwd_str.clone(),
                provider_session_id: None,
                connection_id: None,
                created_by: user.clone(),
                meta,
            })
            .await
            .unwrap();
        let args = mgr.sync_workspace_mcp(&session, None, false).await;
        assert!(args.is_empty(), "{args:?}");
        assert!(!super::super::runtime_spawn::claude_mcp_config_path(&session.id).exists());
        let prefix = crate::mcp::launcher_env_prefix(&cwd_str);
        let env_files = std::fs::read_dir(std::env::temp_dir().join("otto-mcp"))
            .map(|d| {
                d.flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with(&prefix))
                    .count()
            })
            .unwrap_or(0);
        assert_eq!(env_files, 0, "wrote a launcher env file");
        let meta = repo.get(&session.id).await.unwrap().meta;
        assert!(
            meta.get("mcp_skipped").is_none_or(|v| v.is_null()),
            "{provider}: {meta}"
        );
    }
}

/// Opening an agy session no longer writes the shared `.mcp.json` (agy
/// reads only its own `.agents/mcp_config.json`).
#[tokio::test]
async fn an_agy_spawn_writes_no_mcp_json() {
    let (mgr, repo, ws, user) = test_manager().await;
    let cwd = tempfile::tempdir().unwrap();
    let session = repo
        .create(NewSession {
            workspace_id: ws.id.clone(),
            kind: SessionKind::Agent,
            provider: "agy".into(),
            title: "agy".into(),
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
    assert!(!cwd.path().join(".mcp.json").exists());
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

/// pi reads none of the launcher files: a pi spawn gets `-e <private
/// extension>` registering `otto` + the enabled user servers (a name pi
/// refuses lands in `mcp_skipped`), and the snapshot names what it was given.
/// The extension dies with the session's credentials.
#[tokio::test]
async fn sync_workspace_mcp_hands_pi_a_private_extension() {
    use std::os::unix::fs::PermissionsExt;
    struct Servers;
    impl crate::mcp::McpServerSource for Servers {
        fn enabled_servers(
            &self,
            _ws: &str,
        ) -> std::result::Result<Vec<otto_core::hooks::McpServerSpec>, String> {
            Ok(["jira", "bad name"]
                .map(|n| otto_core::hooks::McpServerSpec {
                    name: n.into(),
                    command: "node".into(),
                    args: vec!["srv.js".into()],
                    env: [("TOKEN".to_string(), "$ecret!".to_string())].into(),
                })
                .to_vec())
        }
    }
    let (mut mgr, repo, ws, user) = test_manager().await;
    Arc::get_mut(&mut mgr).unwrap().mcp_servers = Some(Arc::new(Servers));
    let cwd = tempfile::tempdir().unwrap();
    let session = repo
        .create(NewSession {
            workspace_id: ws.id.clone(),
            kind: SessionKind::Agent,
            provider: "pi".into(),
            title: "pi mcp".into(),
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
        args: vec!["mcp-tools".into()],
        env: [("OTTO_API_TOKEN".to_string(), "tok".to_string())].into(),
    };
    let args = mgr.sync_workspace_mcp(&session, Some(otto), false).await;
    let path = pi_mcp_extension_path(&session.id);
    assert_eq!(
        args,
        ["-e".to_string(), path.to_string_lossy().into_owned()]
    );
    let body = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o600
    );
    assert!(
        body.contains("pi.registerMcpServer(name, config)"),
        "{body}"
    );
    assert!(
        body.contains(r#""jira""#) && !body.contains("bad name"),
        "{body}"
    );
    // pi-escaped secret; the otto credential rides the process env only.
    assert!(body.contains(r#""TOKEN":"$$ecret$!""#), "{body}");
    assert!(!body.contains("tok\""), "{body}");
    let meta = repo.get(&session.id).await.unwrap().meta;
    assert_eq!(meta["mcp_servers"], serde_json::json!(["otto", "jira"]));
    assert_eq!(meta["mcp_skipped"][0]["server"], "bad name");
    // Not written into the work tree as a pi config.
    assert!(!cwd.path().join(".pi").exists());
    mgr.retire_credentials(&session).await;
    assert!(!path.exists(), "removed with the session's credentials");
}
