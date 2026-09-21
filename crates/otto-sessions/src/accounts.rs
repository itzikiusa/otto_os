//! Account-isolated provider launch configuration. No credentials are read here.

use otto_core::provider_accounts::ProviderAccount;
use otto_core::{Error, Id, Result};
use otto_pty::CommandSpec;
use std::path::{Path, PathBuf};

pub fn account_home(root: &Path, id: &Id) -> Result<PathBuf> {
    let id = otto_core::paths::safe_component(id)
        .ok_or_else(|| Error::Invalid("invalid provider account ID".into()))?;
    Ok(root.join(id))
}

pub fn prepare_home(home: &Path) -> Result<()> {
    if std::fs::symlink_metadata(home).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err(Error::Invalid(
            "provider account home must not be a symlink".into(),
        ));
    }
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder
        .create(home)
        .map_err(|e| Error::Internal(format!("create provider account home: {e}")))
}

pub fn login_args(provider: &str) -> Result<Vec<String>> {
    let args = match provider {
        "claude" => vec!["auth", "login", "--claudeai"],
        "codex" => vec!["login"],
        _ => {
            return Err(Error::Invalid(
                "subscription accounts support Claude and Codex".into(),
            ))
        }
    };
    Ok(args.into_iter().map(str::to_owned).collect())
}

/// Unset inherited alternative credentials without changing the daemon's own
/// environment. `/usr/bin/env` receives argv directly; no shell interpolation.
pub fn apply_account(spec: &mut CommandSpec, account: &ProviderAccount, home: &Path) -> Result<()> {
    validate_account(spec, account)?;
    const ALTERNATE_AUTH: &[&str] = &[
        "ANTHROPIC_API_KEY",
        "ANTHROPIC_AUTH_TOKEN",
        "CLAUDE_CODE_OAUTH_TOKEN",
        "OPENAI_API_KEY",
        "CODEX_API_KEY",
        "CODEX_ACCESS_TOKEN",
    ];
    spec.env.retain(|(key, _)| {
        !ALTERNATE_AUTH.contains(&key.as_str()) && key != "CLAUDE_CONFIG_DIR" && key != "CODEX_HOME"
    });
    let key = if account.provider == "claude" {
        "CLAUDE_CONFIG_DIR"
    } else {
        "CODEX_HOME"
    };
    spec.env
        .push((key.into(), home.to_string_lossy().into_owned()));
    let mut args: Vec<String> = ALTERNATE_AUTH
        .iter()
        .flat_map(|key| ["-u".into(), (*key).into()])
        .collect();
    args.push(spec.program.clone());
    if account.provider == "codex" {
        // The SAME canonical home is used by login and every session. A
        // per-cwd shadow home would select a different macOS Keychain entry.
        args.extend(
            [
                "-c",
                "forced_login_method=\"chatgpt\"",
                "-c",
                "cli_auth_credentials_store=\"keyring\"",
            ]
            .into_iter()
            .map(str::to_owned),
        );
    }
    args.append(&mut spec.args);
    spec.program = "/usr/bin/env".into();
    spec.args = args;
    Ok(())
}

pub fn validate_account(spec: &CommandSpec, account: &ProviderAccount) -> Result<()> {
    validate_subscription_routing(spec, |key| std::env::var(key).ok())?;
    if account.provider == "codex" {
        validate_codex_overrides(&spec.args)?;
    }
    Ok(())
}

fn validate_codex_overrides(args: &[String]) -> Result<()> {
    let mut iter = args.iter();
    while let Some(arg) = iter.next() {
        let config = if arg == "-c" || arg == "--config" {
            iter.next().map(String::as_str)
        } else {
            arg.strip_prefix("--config=").or_else(|| {
                arg.strip_prefix("-c")
                    .filter(|_| arg.starts_with("-c") && arg.len() > 2)
            })
        };
        if let Some((key, value)) = config.and_then(|c| c.split_once('=')) {
            let expected = match key.trim() {
                "forced_login_method" => Some("chatgpt"),
                "cli_auth_credentials_store" => Some("keyring"),
                "model_provider" => Some("openai"),
                _ => None,
            };
            if expected.is_some_and(|e| value.trim().trim_matches(['\"', '\'']) != e) {
                return Err(Error::Invalid(format!("subscription profile conflicts with Codex config {}; remove the conflicting provider override or use the default CLI account", key.trim())));
            }
        }
    }
    Ok(())
}

fn validate_subscription_routing(
    spec: &CommandSpec,
    inherited: impl Fn(&str) -> Option<String>,
) -> Result<()> {
    // Enterprise routing is policy, not an alternative credential to erase.
    // An explicit profile may never quietly bypass it.
    for key in [
        "CLAUDE_CODE_USE_BEDROCK",
        "CLAUDE_CODE_USE_VERTEX",
        "CLAUDE_CODE_USE_FOUNDRY",
        "ANTHROPIC_BASE_URL",
        "OPENAI_BASE_URL",
    ] {
        let value = spec
            .env
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
            .or_else(|| inherited(key));
        if value
            .is_some_and(|v| !v.trim().is_empty() && v != "0" && !v.eq_ignore_ascii_case("false"))
        {
            return Err(Error::Invalid(format!("subscription profile conflicts with {key}; use the default CLI account or resolve the provider routing policy before selecting this profile")));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::provider_accounts::ProviderAccount;
    use otto_pty::CommandSpec;

    #[test]
    fn account_launch_keeps_native_home_and_removes_alternate_credentials() {
        for provider in ["claude", "codex"] {
            let dir = tempfile::tempdir().unwrap();
            let a = ProviderAccount {
                id: otto_core::new_id(),
                provider: provider.into(),
                label: "Work".into(),
                created_at: chrono::Utc::now(),
            };
            let b = ProviderAccount {
                id: otto_core::new_id(),
                ..a.clone()
            };
            let home = account_home(dir.path(), &a.id).unwrap();
            assert_ne!(home, account_home(dir.path(), &b.id).unwrap());
            let mut spec = CommandSpec {
                program: provider.into(),
                args: vec!["--resume".into(), "session".into()],
                cwd: None,
                env: vec![
                    ("OPENAI_API_KEY".into(), "fixture-secret".into()),
                    ("CODEX_HOME".into(), "/wrong-home".into()),
                ],
            };
            apply_account(&mut spec, &a, &home).unwrap();
            assert_eq!(spec.program, "/usr/bin/env");
            assert!(!format!("{spec:?}").contains("fixture-secret"));
            let key = if provider == "claude" {
                "CLAUDE_CONFIG_DIR"
            } else {
                "CODEX_HOME"
            };
            assert_eq!(
                spec.env.iter().find(|(k, _)| k == key).unwrap().1,
                home.to_string_lossy()
            );
            assert!(spec.args.windows(2).any(|w| w == ["--resume", "session"]));
        }
    }

    #[test]
    fn later_cli_config_cannot_override_subscription_authentication() {
        for args in [
            vec!["-c", "forced_login_method=\"api\""],
            vec!["--config=cli_auth_credentials_store=\"file\""],
            vec!["-cmodel_provider=\"thirdparty\""],
        ] {
            assert!(validate_codex_overrides(
                &args.into_iter().map(str::to_owned).collect::<Vec<_>>()
            )
            .is_err());
        }
        assert!(
            validate_codex_overrides(&["-c".into(), "forced_login_method=\"chatgpt\"".into()])
                .is_ok()
        );
    }

    #[test]
    fn enterprise_routing_is_rejected_without_erasing_policy() {
        let spec = CommandSpec {
            program: "claude".into(),
            args: vec![],
            cwd: None,
            env: vec![],
        };
        assert!(
            validate_subscription_routing(&spec, |k| (k == "CLAUDE_CODE_USE_VERTEX")
                .then(|| "1".into()))
            .is_err()
        );
        assert!(validate_subscription_routing(&spec, |_| None).is_ok());
        let mut configured = spec;
        configured.env.push((
            "ANTHROPIC_BASE_URL".into(),
            "https://company.example".into(),
        ));
        assert!(validate_subscription_routing(&configured, |_| None).is_err());
        assert_eq!(configured.env.len(), 1);
    }

    #[test]
    fn profile_paths_reject_traversal_and_login_uses_subscription() {
        assert!(account_home(std::path::Path::new("/tmp/accounts"), &"../other".into()).is_err());
        assert_eq!(
            login_args("claude").unwrap(),
            ["auth", "login", "--claudeai"]
        );
        assert_eq!(login_args("codex").unwrap(), ["login"]);
        assert!(login_args("shell").is_err());
    }
}
