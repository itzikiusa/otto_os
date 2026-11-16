use std::collections::HashMap;

use otto_core::agent_runtime::RuntimeProvider;

use super::*;

fn args_for(reg: &ProviderRegistry, provider: &str, resume: bool) -> Vec<String> {
    reg.build_spec(provider, "SID", "/tmp/cwd", resume)
        .expect("build_spec")
        .args
}

/// Custom providers wrapping a known CLI (an alias, a `--settings` variant,
/// grok) list that CLI's models and take `--model`; an explicit empty
/// template still opts out, and an unknown CLI gets no catalog.
#[test]
fn custom_providers_inherit_their_cli_family() {
    let reg = ProviderRegistry::new(Some(&serde_json::json!({
        "claude-advisor": {"cmd": "claude", "args": ["--settings", "x.json"]},
        "claudework": {"cmd": "claude-work"},
        "gemini-pro": {"cmd": "gemini-work"},
        "grok": {"cmd": "grok"},
        "mine": {"cmd": "/opt/bin/aider"},
        "api-agent": {"cmd": "pipeline"},
        "pi-fast": {"cmd": "/usr/local/bin/pi"},
        "pinned": {"cmd": "my-wrapper", "model_family": "codex"},
        "noflag": {"cmd": "claude", "model_args": []}
    })));
    let fams: HashMap<_, _> = reg.model_families().into_iter().collect();
    assert_eq!(fams["claude-advisor"], "claude");
    assert_eq!(fams["claudework"], "claude");
    assert_eq!(fams["gemini-pro"], "gemini");
    assert_eq!(fams["grok"], "grok");
    assert_eq!(fams["pi-fast"], "pi");
    assert_eq!(fams["pinned"], "codex");
    assert_eq!(fams["claude"], "claude");
    assert!(!fams.contains_key("mine"));
    assert!(
        !fams.contains_key("api-agent"),
        "api-agent / pipeline must not false-match short family 'pi'"
    );
    assert!(!fams.contains_key("shell"));
    assert!(reg.supports_model("gemini"));
    assert!(reg.supports_model("claudework"));
    assert!(reg.supports_model("gemini-pro"));
    assert!(reg.supports_model("grok"));
    assert!(reg.supports_model("pi-fast"));
    assert!(!reg.supports_model("mine"));
    assert!(!reg.supports_model("api-agent"));
    assert!(!reg.supports_model("noflag"));
}

#[test]
fn runtime_provider_for_resolves_builtins_custom_families_and_disabled() {
    let reg = ProviderRegistry::new(Some(&serde_json::json!({
        "claudework": {"cmd": "claude-work", "args": ["--dangerously-skip-permissions"]},
        "geminiwork": {"cmd": "gemini-work"},
        "api-agent": {"cmd": "pipeline"},
    })));
    assert_eq!(
        reg.runtime_provider_for("claude"),
        Some(RuntimeProvider::Claude)
    );
    assert_eq!(
        reg.runtime_provider_for("gemini"),
        Some(RuntimeProvider::Gemini)
    );
    assert_eq!(
        reg.runtime_provider_for("claudework"),
        Some(RuntimeProvider::Claude)
    );
    assert_eq!(
        reg.runtime_provider_for("geminiwork"),
        Some(RuntimeProvider::Gemini)
    );
    assert_eq!(reg.runtime_provider_for("api-agent"), None);
    assert_eq!(reg.runtime_provider_for("shell"), None);
    assert_eq!(reg.runtime_provider_for("unknown"), None);

    reg.set_disabled(&["claudework", "gemini"]);
    assert_eq!(reg.runtime_provider_for("claudework"), None);
    assert_eq!(reg.runtime_provider_for("gemini"), None);
}

#[test]
fn claude_family_custom_provider_gets_default_session_and_resume_flags() {
    let reg = ProviderRegistry::new(Some(&serde_json::json!({
        "claudework": {"cmd": "/bin/sh", "model_family": "claude", "args": ["--dangerously-skip-permissions"]},
        "claude-custom-resume": {
            "cmd": "/bin/sh",
            "model_family": "claude",
            "args": ["--session-id", "fixed"],
            "resume_args": ["--resume", "custom-{sid}"]
        }
    })));
    assert_eq!(
        args_for(&reg, "claudework", false),
        ["--dangerously-skip-permissions", "--session-id", "SID"]
    );
    assert_eq!(
        args_for(&reg, "claudework", true),
        ["--dangerously-skip-permissions", "--resume", "SID"]
    );
    assert_eq!(
        args_for(&reg, "claude-custom-resume", false),
        ["--session-id", "fixed"]
    );
    assert_eq!(
        args_for(&reg, "claude-custom-resume", true),
        ["--resume", "custom-SID"]
    );
}

#[test]
fn path_resolution_distinguishes_executables_from_aliases() {
    assert!(resolves_on_path("/bin/sh"));
    assert!(resolves_on_path("sh"));
    assert!(!resolves_on_path("otto-no-such-alias-xyz"));
    assert!(!resolves_on_path("/no/such/bin"));
}

/// A custom provider whose command is a shell alias (not on PATH) launches
/// through the user's interactive login shell, with its args (and anything
/// appended later) passed positionally rather than spliced into the script.
#[test]
fn unresolved_custom_command_runs_through_login_shell() {
    let reg = ProviderRegistry::new(Some(&serde_json::json!({
        "claudework": {"cmd": "otto-no-such-alias-xyz", "args": ["--session-id", "{sid}"]}
    })));
    let spec = reg
        .build_spec("claudework", "SID", "/tmp/cwd", false)
        .expect("build_spec");
    assert_eq!(
        &spec.args[..3],
        &[
            "-ilc",
            "otto-no-such-alias-xyz \"$@\"",
            "otto-no-such-alias-xyz"
        ]
    );
    assert_eq!(&spec.args[3..], &["--session-id", "SID"]);
    let resolvable = ProviderRegistry::new(Some(&serde_json::json!({
        "plain": {"cmd": "/bin/sh", "args": ["-c", "true"]}
    })));
    let spec = resolvable.build_spec("plain", "S", "/tmp", false).unwrap();
    assert_eq!(spec.program, "/bin/sh");
    assert_eq!(spec.args, ["-c", "true"]);
}

/// Default (skip on): every built-in agent CLI carries its skip-permissions
/// flag on both launch and resume.
#[test]
fn skip_permissions_on_by_default() {
    let reg = ProviderRegistry::new(None);
    assert!(reg.skip_permissions());
    assert!(args_for(&reg, "claude", false).contains(&"--dangerously-skip-permissions".into()));
    assert!(args_for(&reg, "claude", true).contains(&"--dangerously-skip-permissions".into()));
    assert!(args_for(&reg, "codex", false)
        .contains(&"--dangerously-bypass-approvals-and-sandbox".into()));
    assert!(args_for(&reg, "codex", true)
        .contains(&"--dangerously-bypass-approvals-and-sandbox".into()));
    assert!(args_for(&reg, "agy", false).contains(&"--dangerously-skip-permissions".into()));
}

/// Opted out: the skip/bypass flag is gone from every provider (launch + resume)
/// while the rest of the launch line is preserved.
#[test]
fn skip_permissions_off_drops_the_flag_only() {
    let reg = ProviderRegistry::new(None);
    reg.set_skip_permissions(false, None);
    assert!(!reg.skip_permissions());

    let claude = args_for(&reg, "claude", false);
    assert!(!claude.iter().any(|a| a.contains("dangerously")));
    assert!(claude.contains(&"--session-id".into()));
    assert!(!args_for(&reg, "claude", true)
        .iter()
        .any(|a| a.contains("dangerously")));

    let codex = args_for(&reg, "codex", false);
    assert!(!codex.iter().any(|a| a.contains("dangerously")));
    assert!(codex.contains(&"--search".into()));
    assert!(!args_for(&reg, "codex", true)
        .iter()
        .any(|a| a.contains("dangerously")));

    let agy = args_for(&reg, "agy", false);
    assert!(!agy.iter().any(|a| a.contains("dangerously")));
    assert!(agy.iter().any(|a| a.starts_with("--add-dir=")));
}

/// Disabling a provider hides it from `names()` (the picker/meta list) but
/// keeps its launch spec, so an existing session on it still resumes. A
/// reload or skip-permissions change preserves the disabled set.
#[test]
fn disabled_providers_hidden_from_names_but_still_spawnable() {
    let reg = ProviderRegistry::new(None);
    assert!(reg.names().contains(&"agy".to_string()));

    reg.set_disabled(&["agy"]);
    assert!(!reg.names().contains(&"agy".to_string()));
    assert!(reg.names().contains(&"claude".to_string()));
    assert_eq!(reg.disabled_names(), vec!["agy".to_string()]);
    assert!(reg.build_spec("agy", "SID", "/tmp", false).is_ok());

    reg.reload(None);
    assert!(!reg.names().contains(&"agy".to_string()));
    reg.set_skip_permissions(false, None);
    assert!(!reg.names().contains(&"agy".to_string()));

    reg.set_disabled::<String>(&[]);
    assert!(reg.names().contains(&"agy".to_string()));
}

/// `resolve_default` never returns a DISABLED provider.
#[test]
fn resolve_default_skips_disabled_providers() {
    let reg = ProviderRegistry::new(None);
    assert_eq!(reg.resolve_default(&["", "codex"]), "codex");
    assert_eq!(reg.resolve_default(&["", ""]), "claude");

    reg.set_disabled(&["codex"]);
    assert_eq!(reg.resolve_default(&["", "codex"]), "claude");

    reg.set_disabled(&["codex", "claude"]);
    let d = reg.resolve_default(&["", "codex"]);
    assert!(d != "codex" && d != "claude", "got disabled provider: {d}");
    assert!(reg.names().contains(&d));
}

#[test]
fn builtin_model_templates() {
    let reg = ProviderRegistry::new(None);
    for p in ["claude", "codex", "agy"] {
        assert_eq!(
            reg.model_args_template(p),
            Some(vec!["--model".to_string(), "{model}".to_string()]),
            "{p} should carry the --model template"
        );
        assert!(reg.supports_model(p));
    }
    assert_eq!(reg.model_args_template("shell"), None);
    assert!(!reg.supports_model("shell"));
    assert!(
        !reg.supports_model("nope"),
        "unknown provider has no template"
    );
}

#[test]
fn custom_model_template_from_override() {
    let overrides = serde_json::json!({
        "grok": { "cmd": "grok", "model_args": ["-m", "{model}"] },
        "kilo": { "cmd": "kilo" },
        "opencode": { "cmd": "opencode", "model_args": [] },
    });
    let reg = ProviderRegistry::new(Some(&overrides));
    assert_eq!(
        reg.model_args_template("grok"),
        Some(vec!["-m".to_string(), "{model}".to_string()])
    );
    assert!(reg.supports_model("grok"));
    assert_eq!(reg.model_args_template("kilo"), None);
    assert!(!reg.supports_model("kilo"));
    assert_eq!(reg.model_args_template("opencode"), None);
}

#[test]
fn builtin_override_keeps_family_model_flag_unless_emptied() {
    let reg = ProviderRegistry::new(Some(
        &serde_json::json!({ "claude": { "cmd": "my-claude" } }),
    ));
    assert_eq!(
        reg.model_args_template("claude"),
        Some(vec!["--model".to_string(), "{model}".to_string()])
    );
    let reg = ProviderRegistry::new(Some(
        &serde_json::json!({ "claude": { "cmd": "my-claude", "model_args": [] } }),
    ));
    assert_eq!(reg.model_args_template("claude"), None);
}

#[test]
fn reload_preserves_skip_mode() {
    let reg = ProviderRegistry::new(None);
    reg.set_skip_permissions(false, None);
    reg.reload(None);
    assert!(!reg.skip_permissions());
    assert!(!args_for(&reg, "claude", false)
        .iter()
        .any(|a| a.contains("dangerously")));
}

/// "Update CLIs" (and the auto-update pass) never runs a disabled provider's
/// update command, nor one whose CLI is not installed: the user may not want
/// that CLI installed at all.
#[test]
fn update_commands_skip_disabled_and_missing_providers() {
    let reg = ProviderRegistry::new(None);
    let names = |r: &ProviderRegistry, installed: &dyn Fn(&str) -> bool| {
        r.update_commands_where(installed)
            .into_iter()
            .map(|(n, _)| n)
            .collect::<Vec<_>>()
    };
    let all = names(&reg, &|_| true);
    assert!(all.contains(&"pi".to_string()));
    assert!(all.contains(&"gemini".to_string()));

    let only_claude = names(&reg, &|cmd| cmd == "claude");
    assert_eq!(only_claude, ["claude"]);

    reg.set_disabled(&["pi", "opencode"]);
    let left = names(&reg, &|_| true);
    assert!(!left.contains(&"pi".to_string()));
    assert!(!left.contains(&"opencode".to_string()));
    assert!(left.contains(&"claude".to_string()));
}

/// Gemini CLI and Grok Build start a session with Otto's id and reopen it by
/// that id; each approves every tool only while permission prompts are
/// skipped.
#[test]
fn gemini_and_grok_start_and_resume_by_otto_id() {
    let reg = ProviderRegistry::new(None);
    for (provider, skip) in [("gemini", "--yolo"), ("grok", "--always-approve")] {
        assert_eq!(
            args_for(&reg, provider, false),
            [skip, "--session-id", "SID"],
            "{provider}"
        );
        assert_eq!(
            args_for(&reg, provider, true),
            [skip, "--resume", "SID"],
            "{provider}"
        );
    }
    reg.set_skip_permissions(false, None);
    for provider in ["gemini", "grok"] {
        assert_eq!(args_for(&reg, provider, false), ["--session-id", "SID"]);
        assert_eq!(args_for(&reg, provider, true), ["--resume", "SID"]);
    }
}

/// Muse Code is built in: `--trust-workspace` always, `--yolo` only while
/// permission prompts are skipped, no resume argv (Threads resumes it over
/// ACP), the launcher's update check off, the launcher's own refresh as its
/// update command, and the Muse runtime driver.
#[test]
fn muse_is_a_builtin_terminal_and_runtime_provider() {
    let reg = ProviderRegistry::new(None);
    assert_eq!(
        args_for(&reg, "muse", false),
        ["--trust-workspace", "--yolo"]
    );
    let spec = reg.build_spec("muse", "SID", "/tmp/cwd", false).unwrap();
    assert_eq!(spec.program, "muse");
    assert!(spec
        .env
        .contains(&("MUSE_NO_AUTO_UPDATE".to_string(), "1".to_string())));
    assert!(reg.build_spec("muse", "SID", "/tmp/cwd", true).is_err());
    assert_eq!(
        reg.model_args_template("muse"),
        Some(vec!["--model".to_string(), "{model}".to_string()])
    );
    assert!(reg
        .update_commands_where(|_| true)
        .contains(&("muse".to_string(), muse_update_command("muse"))));
    assert_eq!(
        reg.runtime_provider_for("muse"),
        Some(RuntimeProvider::Muse)
    );
    let fams: HashMap<_, _> = reg.model_families().into_iter().collect();
    assert_eq!(fams["muse"], "muse");
    reg.set_skip_permissions(false, None);
    assert_eq!(args_for(&reg, "muse", false), ["--trust-workspace"]);
    // A custom wrapper of the `muse` command rides the Muse family.
    assert_eq!(
        infer_model_family(None, "/opt/muse/bin/muse", "metamuse"),
        Some("muse".into())
    );
}

fn session_on(provider: &str, psid: Option<&str>) -> otto_core::domain::Session {
    otto_core::domain::Session {
        id: otto_core::new_id(),
        workspace_id: otto_core::new_id(),
        kind: otto_core::domain::SessionKind::Agent,
        provider: provider.into(),
        title: "t".into(),
        status: otto_core::domain::SessionStatus::Idle,
        cwd: "/tmp".into(),
        provider_session_id: psid.map(str::to_string),
        connection_id: None,
        created_by: otto_core::new_id(),
        created_at: chrono::Utc::now(),
        last_active_at: chrono::Utc::now(),
        archived: false,
        meta: serde_json::json!({}),
    }
}

/// I-02: the one "would a restart keep the conversation?" predicate that
/// gates every restart Otto makes on the person's behalf (the nightly CLI
/// update's reload) and picks the argv in `restart_locked`. Muse and opencode
/// have no resume, a custom CLI without `resume_args` neither, and a codex
/// session whose id was never captured would start over too.
#[test]
fn resumes_session_only_when_the_conversation_survives_a_restart() {
    let reg = ProviderRegistry::new(Some(&serde_json::json!({
        "aider": {"cmd": "/opt/bin/aider"}
    })));
    assert!(reg.resumes_session(&session_on("claude", Some("u-1"))));
    assert!(reg.resumes_session(&session_on("codex", Some("019f"))));
    assert!(reg.resumes_session(&session_on("pi", Some("u-2"))));
    assert!(!reg.resumes_session(&session_on("muse", Some("m-1"))));
    assert!(!reg.resumes_session(&session_on("opencode", Some("o-1"))));
    assert!(!reg.resumes_session(&session_on("aider", Some("a-1"))));
    assert!(!reg.resumes_session(&session_on("codex", None)));
    assert!(!reg.resumes_session(&session_on("claude", Some(""))));
    let mut conn = session_on("claude", Some("u-1"));
    conn.kind = otto_core::domain::SessionKind::Connection;
    assert!(!reg.resumes_session(&conn));
}

/// A Muse-family custom provider refreshes ITS OWN binary the Muse way (the
/// launcher has no `update` subcommand), never a hard-coded `muse`.
#[test]
fn muse_family_custom_provider_updates_its_own_binary() {
    let reg = ProviderRegistry::new(Some(&serde_json::json!({
        "musework": {"cmd": "/opt/muse-beta/bin/muse"}
    })));
    let cmds: HashMap<_, _> = reg.update_commands_where(|_| true).into_iter().collect();
    assert_eq!(
        cmds["musework"],
        "MUSE_NO_AUTO_UPDATE=0 MUSE_SYNC_UPDATE=1 MUSE_UPDATE_INTERVAL_SECONDS=0 \
         '/opt/muse-beta/bin/muse' --version"
    );
    assert!(cmds["muse"].ends_with(" muse --version"));
}

/// I-04/I-05: every provider family Otto launches has an entry in the ONE
/// sandbox path table (its state dirs, home config denies and read-only
/// project denies). Adding a family to `MODEL_FAMILIES` without one fails here.
#[test]
fn every_model_family_has_a_sandbox_entry() {
    for fam in MODEL_FAMILIES {
        assert!(
            otto_sandbox::providers::covers(fam),
            "provider family {fam} has no entry in otto_sandbox::providers::PROVIDERS"
        );
    }
    for p in RuntimeProvider::ALL {
        assert!(
            otto_sandbox::providers::covers(p.as_str()),
            "runtime provider {} has no sandbox entry",
            p.as_str()
        );
    }
}

/// I-07: a provider turned off in Settings › Providers can't start a NEW
/// session (every new-session path builds through `build_new_spec`), while an
/// existing session on it still resumes through `build_spec`.
#[test]
fn disabled_provider_refuses_new_sessions_but_still_resumes() {
    let reg = ProviderRegistry::new(None);
    assert!(reg.build_new_spec("muse", "SID", "/tmp/cwd").is_ok());
    reg.set_disabled(&["muse", "claude"]);
    let err = reg
        .build_new_spec("muse", "SID", "/tmp/cwd")
        .expect_err("disabled");
    assert!(
        err.to_string()
            .contains("muse is turned off in Settings › Providers"),
        "{err}"
    );
    assert!(reg.ensure_enabled("claude").is_err());
    assert!(reg.ensure_enabled("codex").is_ok());
    assert!(reg.build_spec("claude", "SID", "/tmp/cwd", true).is_ok());
}

/// I-09: family inference never guesses from a bare prefix (`museum-cli` is
/// not Muse), while the `<family>work` and `<family>-…` spellings still ride
/// their family; `cli_family` resolves registered slugs through the registry
/// (an explicit `model_family` wins over the name) and leaves unknown CLIs as
/// themselves.
#[test]
fn family_resolution_is_registry_first_and_never_prefix_guessed() {
    assert_eq!(infer_model_family(None, "", "museum-cli"), None);
    assert_eq!(infer_model_family(None, "", "codexify"), None);
    assert_eq!(
        infer_model_family(None, "", "claudework"),
        Some("claude".into())
    );
    assert_eq!(
        infer_model_family(None, "", "muse-beta"),
        Some("muse".into())
    );
    assert_eq!(
        infer_model_family(None, "", "claude2"),
        Some("claude".into())
    );
    let reg = ProviderRegistry::new(Some(&serde_json::json!({
        "work": {"cmd": "/opt/wrap", "model_family": "claude"},
        "museum-cli": {"cmd": "/opt/museum"}
    })));
    assert_eq!(reg.cli_family("work"), "claude");
    assert_eq!(reg.cli_family("museum-cli"), "museum-cli");
    assert_eq!(reg.cli_family("muse"), "muse");
    assert_eq!(reg.cli_family("unknown-cli"), "unknown-cli");
}

/// `/meta.provider_resumable`: Muse and opencode can't reopen a terminal
/// conversation; a disabled provider is not listed.
#[test]
fn resume_flags_list_enabled_providers() {
    let reg = ProviderRegistry::new(None);
    reg.set_disabled(&["gemini"]);
    let f = reg.resume_flags();
    assert_eq!(f.get("claude"), Some(&true));
    assert_eq!(f.get("pi"), Some(&true));
    assert_eq!(f.get("muse"), Some(&false));
    assert_eq!(f.get("opencode"), Some(&false));
    assert!(!f.contains_key("gemini"));
}
