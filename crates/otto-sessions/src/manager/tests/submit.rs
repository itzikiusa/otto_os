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
