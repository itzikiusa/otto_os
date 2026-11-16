//! Round 5 regression tests for custom provider `claudework` parity across
//! `ProviderRegistry`, PTY CLI flags, `SessionManager::prepare_runtime_spawn`,
//! `tool_probe`, `"gemini"` in `MODEL_FAMILIES`, and `"pi"` word-boundary
//! inference.

#![allow(clippy::disallowed_methods)]

use std::os::unix::fs::PermissionsExt;

use chrono::Utc;
use otto_core::agent_runtime::RuntimeProvider;
use otto_core::domain::{Session, SessionKind, SessionStatus, Workspace};
use otto_pty::CommandSpec;
use otto_sessions::manager::RuntimeMcpDelivery;
use otto_sessions::provider_models::invalidate_caches_after_login;
use otto_sessions::providers::{infer_model_family, MODEL_FAMILIES};
use otto_sessions::tool_probe::{
    extract_custom_cmd_executable, probe_custom_command_available, DETECTED_TOOLS,
};
use otto_sessions::{ProviderRegistry, SessionManager};
use otto_state::SessionsRepo;
use serde_json::json;
use tokio::sync::broadcast;

fn make_session(id: &str, provider: &str, cwd: &str, meta: serde_json::Value) -> Session {
    Session {
        id: id.into(),
        workspace_id: "ws-1".into(),
        kind: SessionKind::Agent,
        provider: provider.into(),
        title: "Test".into(),
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

fn make_workspace(root: &str) -> Workspace {
    Workspace {
        id: "ws-1".into(),
        name: "W".into(),
        root_path: root.into(),
        settings: json!({}),
        archived: false,
        created_at: Utc::now(),
    }
}

#[test]
fn gemini_is_in_model_families_and_pi_uses_word_boundary_matching() {
    assert!(MODEL_FAMILIES.contains(&"gemini"));
    let reg = ProviderRegistry::new(Some(&json!({
        "geminiwork": { "cmd": "gemini-work" }
    })));
    assert!(reg.supports_model("gemini"));
    assert!(reg.supports_model("geminiwork"));
    assert_eq!(
        reg.model_family_for("geminiwork").as_deref(),
        Some("gemini")
    );
    assert_eq!(
        reg.runtime_provider_for("geminiwork"),
        Some(RuntimeProvider::Gemini)
    );

    // Short family name `"pi"` must NOT match substrings like `"api-agent"`,
    // `"pipeline"`, or `"copilot-cli"`.
    assert_eq!(infer_model_family(None, "api-agent", "api-agent"), None);
    assert_eq!(
        infer_model_family(None, "pipeline", "pipeline-runner"),
        None
    );
    assert_eq!(infer_model_family(None, "pilot", "pilot"), None);

    // Valid word-boundary / basename tokens for `"pi"` MUST match.
    assert_eq!(infer_model_family(None, "pi", "custom"), Some("pi".into()));
    assert_eq!(
        infer_model_family(None, "/usr/local/bin/pi", "my-agent"),
        Some("pi".into())
    );
    assert_eq!(
        infer_model_family(None, "wrapper", "pi-work"),
        Some("pi".into())
    );
    assert_eq!(
        infer_model_family(None, "wrapper", "my-pi"),
        Some("pi".into())
    );
}

#[test]
fn claudework_resolves_runtime_provider_and_pty_session_flags() {
    let reg = ProviderRegistry::new(Some(&json!({
        "claudework": { "cmd": "claude-work" }
    })));
    assert_eq!(
        reg.runtime_provider_for("claudework"),
        Some(RuntimeProvider::Claude)
    );
    assert_eq!(
        reg.model_family_for("claudework").as_deref(),
        Some("claude")
    );

    // Fresh PTY build auto-injects `--session-id <sid>`; resume injects `--resume <sid>`.
    let fresh = reg
        .build_spec("claudework", "sid-123", "/tmp/w", false)
        .unwrap();
    assert!(fresh
        .args
        .windows(2)
        .any(|w| w == ["--session-id", "sid-123"]));

    let resumed = reg
        .build_spec("claudework", "sid-123", "/tmp/w", true)
        .unwrap();
    assert!(resumed
        .args
        .windows(2)
        .any(|w| w == ["--resume", "sid-123"]));

    // Disabling `claudework` prevents `runtime_provider_for` from resolving it.
    reg.set_disabled(&["claudework"]);
    assert_eq!(reg.runtime_provider_for("claudework"), None);
}

#[tokio::test]
async fn prepare_runtime_spawn_rewrites_claudework_respects_access_mode_and_wraps_shell_alias() {
    let dir = tempfile::tempdir().unwrap();
    let ws_dir = dir.path().join("project");
    std::fs::create_dir_all(&ws_dir).unwrap();
    let ws_str = ws_dir.to_string_lossy().into_owned();

    // Create a real executable script for direct-binary `claudework-bin`.
    let fake_bin = dir.path().join("claude-work-bin");
    std::fs::write(&fake_bin, "#!/bin/sh\nexec echo \"$@\"\n").unwrap();
    std::fs::set_permissions(&fake_bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    let providers = ProviderRegistry::new(Some(&json!({
        "claudework": {
            "cmd": fake_bin.to_string_lossy(),
            "args": ["--dangerously-skip-permissions", "--append-system-prompt", "cwd={cwd} sid={sid}"]
        },
        "claudealias": {
            "cmd": "claude-work-shell-fn",
            "model_family": "claude"
        }
    })));
    let pool = otto_state::open(&dir.path().join("state.db"))
        .await
        .unwrap();
    let (events, _rx) = broadcast::channel(16);
    let mgr = SessionManager::new(SessionsRepo::new(pool), events, providers)
        .with_data_dir(dir.path().join("data"));
    let ws = make_workspace(&ws_str);

    // 1. ApprovalRequired / Plan mode: driver `spec.args` does NOT have
    //    `--dangerously-skip-permissions`, so `prepare_runtime_spawn` MUST strip it
    //    from `claudework`'s custom `args` while expanding `{cwd}` and `{sid}`!
    let sess = make_session(
        "rt-sess-1",
        "claudework",
        &ws_str,
        json!({"runtime": "structured", "provider_slug": "claudework"}),
    );
    let mut approval_spec = CommandSpec {
        program: "claude".into(),
        args: vec![
            "--output-format".into(),
            "stream-json".into(),
            "--permission-mode".into(),
            "default".into(),
        ],
        cwd: Some(ws_str.clone()),
        env: Vec::new(),
    };
    mgr.prepare_runtime_spawn(
        &ws,
        &sess,
        &mut approval_spec,
        Vec::new(),
        RuntimeMcpDelivery::Protocol,
    )
    .await
    .unwrap();
    assert_eq!(approval_spec.program, fake_bin.to_string_lossy());
    assert!(
        !approval_spec
            .args
            .iter()
            .any(|a| a == "--dangerously-skip-permissions"),
        "ApprovalRequired must strip --dangerously-skip-permissions: {:?}",
        approval_spec.args
    );
    assert!(
        !approval_spec
            .args
            .iter()
            .any(|a| a == "--session-id" || a == "--resume"),
        "PTY --session-id must not duplicate structured driver flags: {:?}",
        approval_spec.args
    );
    assert!(approval_spec
        .args
        .iter()
        .any(|a| a == &format!("cwd={ws_str} sid=rt-sess-1")));

    // 2. FullAccess mode: driver `spec.args` carries `--dangerously-skip-permissions`,
    //    so it remains present once without duplication.
    let mut full_spec = CommandSpec {
        program: "claude".into(),
        args: vec![
            "--output-format".into(),
            "stream-json".into(),
            "--dangerously-skip-permissions".into(),
        ],
        cwd: Some(ws_str.clone()),
        env: Vec::new(),
    };
    mgr.prepare_runtime_spawn(
        &ws,
        &sess,
        &mut full_spec,
        Vec::new(),
        RuntimeMcpDelivery::Protocol,
    )
    .await
    .unwrap();
    let skip_count = full_spec
        .args
        .iter()
        .filter(|a| a.as_str() == "--dangerously-skip-permissions")
        .count();
    assert_eq!(skip_count, 1, "{:?}", full_spec.args);

    // 3. Non-PATH shell function/alias (`claude-work-shell-fn`): wrapped via login
    //    shell (`-lic`) so zsh/bash aliases launch cleanly in Structured Runtime.
    let alias_sess = make_session(
        "rt-sess-2",
        "claudealias",
        &ws_str,
        json!({"runtime": "structured", "provider_slug": "claudealias"}),
    );
    let mut alias_spec = CommandSpec {
        program: "claude".into(),
        args: vec!["--output-format".into(), "stream-json".into()],
        cwd: Some(ws_str.clone()),
        env: Vec::new(),
    };
    mgr.prepare_runtime_spawn(
        &ws,
        &alias_sess,
        &mut alias_spec,
        Vec::new(),
        RuntimeMcpDelivery::Protocol,
    )
    .await
    .unwrap();
    assert!(
        alias_spec.args.iter().any(|a| a == "-ilc" || a == "-lic"),
        "non-PATH alias must wrap through interactive login shell: program={} args={:?}",
        alias_spec.program,
        alias_spec.args
    );
    assert!(alias_spec.args.iter().any(|a| a == "claude-work-shell-fn"));
}

#[tokio::test]
async fn custom_command_probe_strips_templates_and_checks_login_shell() {
    assert_eq!(
        extract_custom_cmd_executable("FOO=bar claude-work --cwd {cwd} --session-id {sid}"),
        Some("claude-work".into())
    );
    assert_eq!(extract_custom_cmd_executable("{cwd}/bin/agent"), None);

    // Shell builtin (`printf`) is resolved via login-shell `command -v`.
    assert_eq!(
        probe_custom_command_available("ENV_A=1 printf --arg {sid}").await,
        Some(true)
    );
    assert_eq!(
        probe_custom_command_available("otto_nonexistent_cmd_round5_xyz --cwd {cwd}").await,
        Some(false)
    );

    // Post-login invalidation clears cold provisional caches cleanly.
    invalidate_caches_after_login(Some("agy"));
}

#[tokio::test]
async fn wrapper_claude_config_dir_cli_family_non_claude_resume_and_seatbelt_writable_root() {
    assert!(DETECTED_TOOLS.contains(&"opencode"));
    assert!(DETECTED_TOOLS.contains(&"pi"));

    let dir = tempfile::tempdir().unwrap();
    let fake_home = dir.path().join("fake-home");
    let custom_cfg = dir.path().join("claude-work-cfg");
    std::fs::create_dir_all(fake_home.join(".claude-conv")).unwrap();
    std::fs::create_dir_all(&custom_cfg).unwrap();

    // 1. Wrapper script with `export CLAUDE_CONFIG_DIR=...` + spaced path + duplicate skip flag.
    let spaced_dir = dir.path().join("My Tools");
    std::fs::create_dir_all(&spaced_dir).unwrap();
    let wrapper_bin = spaced_dir.join("claude-work");
    std::fs::write(
        &wrapper_bin,
        format!(
            "#!/bin/sh\nexport CLAUDE_CONFIG_DIR=\"{}\"\nexec echo \"$@\"\n",
            custom_cfg.display()
        ),
    )
    .unwrap();
    std::fs::set_permissions(&wrapper_bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    let codex_bin = spaced_dir.join("codex-work");
    std::fs::write(&codex_bin, "#!/bin/sh\nexec echo \"$@\"\n").unwrap();
    std::fs::set_permissions(&codex_bin, std::fs::Permissions::from_mode(0o755)).unwrap();

    let reg = ProviderRegistry::new(Some(&json!({
        "claudework": {
            "cmd": format!("\"{}\" --dangerously-skip-permissions", wrapper_bin.display()),
            "args": ["--dangerously-skip-permissions"]
        },
        "claudeconv": {
            "cmd": "claude-conv --dangerously-skip-permissions"
        },
        "codexwork": {
            "cmd": format!("\"{}\"", codex_bin.display()),
            "model_family": "codex"
        },
        "agywork": {
            "cmd": "agy-work"
        },
        "piwork": {
            "cmd": "pi-work"
        }
    })));

    // Wrapper `CLAUDE_CONFIG_DIR` discovery & conventional `~/.claude-<suffix>` fallback.
    assert_eq!(
        reg.claude_config_dir_for("claudework", &fake_home),
        Some(custom_cfg.clone())
    );
    assert_eq!(
        reg.claude_config_dir_for("claudeconv", &fake_home),
        Some(fake_home.join(".claude-conv"))
    );

    // Spaced executable path preserved & `--dangerously-skip-permissions` deduplicated.
    let pty_spec = reg
        .build_spec("claudework", "sid-space", "/tmp/w", false)
        .unwrap();
    assert_eq!(pty_spec.program, wrapper_bin.to_string_lossy());
    assert_eq!(
        pty_spec
            .args
            .iter()
            .filter(|a| a.as_str() == "--dangerously-skip-permissions")
            .count(),
        1,
        "{:?}",
        pty_spec.args
    );

    // Non-Claude custom providers synthesize default `resume_args` + `captures_session_id`.
    assert!(reg.supports_resume("codexwork"));
    assert!(reg.captures_session_id("codexwork"));
    assert!(reg.supports_resume("agywork"));
    assert!(reg.captures_session_id("agywork"));
    assert!(reg.supports_resume("piwork"));
    assert!(!reg.captures_session_id("piwork"));
    let codex_res = reg
        .build_spec("codexwork", "cx-99", "/tmp/w", true)
        .unwrap();
    assert_eq!(codex_res.args.first().map(String::as_str), Some("resume"));
    assert_eq!(codex_res.args.last().map(String::as_str), Some("cx-99"));
    let agy_res = reg.build_spec("agywork", "ag-99", "/tmp/w", true).unwrap();
    assert!(agy_res
        .args
        .windows(2)
        .any(|w| w == ["--conversation", "ag-99"]));

    // `SessionManager::cli_family` + Seatbelt `writable_roots` on `read_only` spawn.
    let ws_dir = dir.path().join("project");
    std::fs::create_dir_all(&ws_dir).unwrap();
    let ws_str = ws_dir.to_string_lossy().into_owned();
    let pool = otto_state::open(&dir.path().join("state.db"))
        .await
        .unwrap();
    let (events, _rx) = broadcast::channel(16);
    let mgr = SessionManager::new(SessionsRepo::new(pool), events, reg)
        .with_data_dir(dir.path().join("data"));

    assert_eq!(mgr.cli_family("claudework"), "claude");
    assert_eq!(mgr.cli_family("codexwork"), "codex");
    assert_eq!(mgr.cli_family("agywork"), "agy");
    assert_eq!(mgr.cli_family("piwork"), "pi");

    if otto_sandbox::is_supported() {
        let ws = make_workspace(&ws_str);
        let ro_sess = make_session(
            "rt-ro-1",
            "claudework",
            &ws_str,
            json!({"runtime": "structured", "provider_slug": "claudework", "read_only": true}),
        );
        let mut ro_spec = CommandSpec {
            program: "claude".into(),
            args: vec!["--output-format".into(), "stream-json".into()],
            cwd: Some(ws_str.clone()),
            env: Vec::new(),
        };
        mgr.prepare_runtime_spawn(
            &ws,
            &ro_sess,
            &mut ro_spec,
            Vec::new(),
            RuntimeMcpDelivery::Protocol,
        )
        .await
        .unwrap();
        assert_eq!(ro_spec.program, "/usr/bin/sandbox-exec");
        let sbpl = &ro_spec.args[1];
        let canon_cfg = std::fs::canonicalize(&custom_cfg).unwrap_or(custom_cfg);
        let cfg_str = canon_cfg.to_string_lossy();
        assert!(
            sbpl.contains(&format!("(subpath \"{cfg_str}\")")),
            "Seatbelt profile must allow writes to custom CLAUDE_CONFIG_DIR: {sbpl}"
        );
        assert!(
            sbpl.contains(&format!("{cfg_str}/settings.json")),
            "Seatbelt profile must still guard settings.json inside custom CLAUDE_CONFIG_DIR: {sbpl}"
        );
    }
}
