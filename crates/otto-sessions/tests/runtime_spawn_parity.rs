//! Spawn parity: a structured runtime thread gets what its terminal would —
//! no MCP servers at all for an untrusted read-only thread (a PR's reviewer),
//! the extra folders it may read, and no web search on a read-only Codex.

#![allow(clippy::disallowed_methods)]

use chrono::Utc;
use otto_core::domain::{Session, SessionKind, SessionStatus, Workspace};
use otto_pty::CommandSpec;
use otto_sessions::manager::RuntimeMcpDelivery;
use otto_sessions::{ProviderRegistry, SessionManager};
use otto_state::SessionsRepo;
use serde_json::json;
use tokio::sync::broadcast;

fn session(provider: &str, cwd: &str, meta: serde_json::Value) -> Session {
    Session {
        id: format!("rt-{provider}"),
        workspace_id: "ws-1".into(),
        kind: SessionKind::Agent,
        provider: provider.into(),
        title: "T".into(),
        status: SessionStatus::Running,
        cwd: cwd.into(),
        provider_session_id: None,
        connection_id: None,
        created_by: "u1".into(),
        created_at: Utc::now(),
        last_active_at: Utc::now(),
        archived: false,
        meta,
    }
}

async fn rig() -> (tempfile::TempDir, SessionManager, Workspace, String) {
    let dir = tempfile::tempdir().unwrap();
    let ws_dir = dir.path().join("project");
    std::fs::create_dir_all(&ws_dir).unwrap();
    let ws_str = ws_dir.to_string_lossy().into_owned();
    let pool = otto_state::open(&dir.path().join("state.db"))
        .await
        .unwrap();
    let (events, _rx) = broadcast::channel(16);
    let mgr = SessionManager::new(SessionsRepo::new(pool), events, ProviderRegistry::new(None))
        .with_data_dir(dir.path().join("data"));
    let ws = Workspace {
        id: "ws-1".into(),
        name: "W".into(),
        root_path: ws_str.clone(),
        settings: json!({}),
        archived: false,
        created_at: Utc::now(),
    };
    (dir, mgr, ws, ws_str)
}

fn spec(program: &str, args: &[&str]) -> CommandSpec {
    CommandSpec {
        program: program.into(),
        args: args.iter().map(|a| a.to_string()).collect(),
        cwd: None,
        env: Vec::new(),
    }
}

/// DR-001: the browser server (opted in by meta) stands for every server a
/// thread may get; an untrusted read-only thread gets none, on either path.
#[tokio::test]
async fn an_untrusted_read_only_thread_gets_no_mcp_servers() {
    let (_dir, mgr, ws, cwd) = rig().await;
    let meta = |untrusted: bool| {
        let mut m = json!({"runtime": "structured", "read_only": true, "browser": true});
        if untrusted {
            m["project_settings"] = json!(false);
        }
        m
    };
    // Over the protocol (ACP): a trusted read-only thread keeps its servers.
    let trusted = mgr
        .prepare_runtime_spawn(
            &ws,
            &session("gemini", &cwd, meta(false)),
            &mut spec("gemini", &["--acp"]),
            Vec::new(),
            RuntimeMcpDelivery::Protocol,
        )
        .await
        .unwrap();
    assert!(
        !trusted.is_empty(),
        "the trusted thread keeps its browser server"
    );
    let untrusted = mgr
        .prepare_runtime_spawn(
            &ws,
            &session("gemini", &cwd, meta(true)),
            &mut spec("gemini", &["--acp"]),
            Vec::new(),
            RuntimeMcpDelivery::Protocol,
        )
        .await
        .unwrap();
    assert!(
        untrusted.is_empty(),
        "{:?}",
        untrusted.iter().map(|s| &s.name).collect::<Vec<_>>()
    );
    // Claude's launcher path: no --mcp-config document, only --strict-mcp-config.
    let mut claude = spec("claude", &["--output-format", "stream-json"]);
    mgr.prepare_runtime_spawn(
        &ws,
        &session("claude", &cwd, meta(true)),
        &mut claude,
        Vec::new(),
        RuntimeMcpDelivery::Launcher,
    )
    .await
    .unwrap();
    assert!(
        !claude.args.iter().any(|a| a == "--mcp-config"),
        "{:?}",
        claude.args
    );
    assert!(
        claude.args.iter().any(|a| a == "--strict-mcp-config"),
        "{:?}",
        claude.args
    );
}

/// DR-030: the extra folders reach a runtime Claude spawn as `--add-dir`, and
/// a read-only Codex thread loses `--search`.
#[tokio::test]
async fn extra_dirs_and_the_read_only_strip_reach_the_runtime_argv() {
    let (dir, mgr, ws, cwd) = rig().await;
    let extra = dir.path().join("docs").to_string_lossy().into_owned();
    let mut claude = spec("claude", &["--output-format", "stream-json"]);
    mgr.prepare_runtime_spawn(
        &ws,
        &session(
            "claude",
            &cwd,
            json!({"runtime": "structured", "extra_dirs": [extra]}),
        ),
        &mut claude,
        Vec::new(),
        RuntimeMcpDelivery::Launcher,
    )
    .await
    .unwrap();
    assert!(
        claude
            .args
            .windows(2)
            .any(|w| w == ["--add-dir".to_string(), extra.clone()]),
        "{:?}",
        claude.args
    );
    let mut codex = spec("codex", &["app-server", "--search"]);
    mgr.prepare_runtime_spawn(
        &ws,
        &session(
            "codex",
            &cwd,
            json!({"runtime": "structured", "read_only": true}),
        ),
        &mut codex,
        Vec::new(),
        RuntimeMcpDelivery::Launcher,
    )
    .await
    .unwrap();
    assert!(
        !codex.args.iter().any(|a| a == "--search"),
        "{:?}",
        codex.args
    );
}
