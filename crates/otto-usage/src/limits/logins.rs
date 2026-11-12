//! The Claude logins a caller may see, their rows, and the opt-in "Check with
//! Anthropic" switch (`PUT /usage/limits/claude-check`) and run (`POST
//! /usage/limits/claude-check/run`).
//!
//! The switch is per login and OFF by default. It lives in the daemon
//! `settings` table under [`CHECKS_KEY`] as `{ "<row key>": true }` — the same
//! key/value store the usage config and budgets use; no secret is stored.
//! A row key a caller cannot see (another user's profile, a machine login for
//! a non-root user) is refused, so the switch can only be flipped by the
//! login's owner.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{DateTime, Utc};
use otto_core::provider_accounts::ProviderAccount;
use otto_core::{Error, Id};
use otto_state::provider_accounts::ProviderAccountsRepo;
use otto_state::{DbPool, SettingsRepo};
use otto_transcript::roots::{home_dir, RootKind, RootsEnv};
use serde::Deserialize;

use super::{
    claude, cli, tilde, unknown_row, AccountLimits, ClaudeCheck, LimitsResp, CLAUDE_REASON,
};

/// `settings` key of the per-login switch map.
pub const CHECKS_KEY: &str = "usage_limits_claude_check";

/// Row keys whose owner turned the check on.
pub(crate) type Checks = HashSet<String>;

/// `PUT /usage/limits/claude-check`.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeCheckReq {
    /// A Claude row's `key` from `GET /usage/limits`.
    pub key: String,
    pub enabled: bool,
}

/// `POST /usage/limits/claude-check/run`.
#[derive(Debug, Clone, Deserialize)]
pub struct ClaudeRunReq {
    pub key: String,
}

/// One Claude login.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ClaudeLogin {
    pub key: String,
    pub account_id: Option<Id>,
    pub label: String,
    pub source: &'static str,
    /// Its config dir (what `CLAUDE_CONFIG_DIR` points at, or `~/.claude`).
    pub dir: PathBuf,
}

impl ClaudeLogin {
    /// The CLI's own default home: launched without `CLAUDE_CONFIG_DIR`.
    fn is_default(&self) -> bool {
        self.source == "default"
    }
}

/// Root: the machine's Claude config dirs that hold transcripts; everyone:
/// their own Claude subscription profiles.
pub(crate) fn claude_logins(
    accounts: &[ProviderAccount],
    is_root: bool,
    data_dir: &Path,
    home: &Path,
    env: &RootsEnv,
) -> Vec<ClaudeLogin> {
    let mut out = Vec::new();
    if is_root {
        let roots = env.discover(home, data_dir);
        for root in roots
            .claude
            .iter()
            .filter(|r| r.kind != RootKind::Account && r.dir.is_dir())
        {
            let dir = root.dir.parent().unwrap_or(&root.dir).to_path_buf();
            let (label, source) = if root.kind == RootKind::Default {
                ("Default Claude login".to_string(), "default")
            } else {
                (tilde(&dir, home), "config_dir")
            };
            out.push(ClaudeLogin {
                key: format!("claude:{}", tilde(&dir, home)),
                account_id: None,
                label,
                source,
                dir,
            });
        }
    }
    let homes = data_dir.join("provider-accounts");
    for account in accounts.iter().filter(|a| a.provider == "claude") {
        let Some(id) = otto_core::paths::safe_component(&account.id) else {
            continue;
        };
        out.push(ClaudeLogin {
            key: format!("claude:account:{id}"),
            account_id: Some(account.id.clone()),
            label: account.label.clone(),
            source: "account",
            dir: homes.join(id),
        });
    }
    out
}

/// The served row for one login: the newest readings (passive events or an
/// opt-in check), else `unknown` with the reason — never a 0 %.
pub(crate) fn claude_row(login: &ClaudeLogin, enabled: bool, now: DateTime<Utc>) -> AccountLimits {
    let e = claude::entry(&login.dir).unwrap_or_default();
    let check = ClaudeCheck {
        enabled,
        checking: cli::in_flight(&login.dir),
        checked_at: e.checked_at,
        error: e.check_error.clone().filter(|_| enabled),
    };
    match claude::served(&e, now) {
        Some((status, taken_at, windows)) => AccountLimits {
            key: login.key.clone(),
            provider: "claude".into(),
            account_id: login.account_id.clone(),
            label: login.label.clone(),
            source: login.source.into(),
            status,
            reason: None,
            plan: e.plan.clone(),
            taken_at: Some(taken_at),
            windows,
            credits: None,
            limit_reached: None,
            check: Some(check),
        },
        None => {
            let reason = match (enabled, &check.error) {
                (true, Some(why)) => format!("Couldn’t check with Anthropic: {why}"),
                (true, None) => "Not checked with Anthropic yet.".to_string(),
                (false, _) => CLAUDE_REASON.to_string(),
            };
            let mut row = unknown_row(
                login.key.clone(),
                "claude",
                login.account_id.clone(),
                login.label.clone(),
                login.source,
                &reason,
            );
            row.check = Some(check);
            row
        }
    }
}

/// The switch map (`{key: true}`); unreadable → nothing is on.
pub(crate) async fn load_checks(pool: &DbPool) -> Checks {
    let value = SettingsRepo::new(pool.clone())
        .get(CHECKS_KEY)
        .await
        .ok()
        .flatten();
    value
        .as_ref()
        .and_then(|v| v.as_object())
        .map(|m| {
            m.iter()
                .filter(|(_, on)| on.as_bool() == Some(true))
                .map(|(k, _)| k.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// The caller's login for `key`, or a 404 (another user's, or none).
async fn login_for(
    pool: &DbPool,
    user_id: &Id,
    is_root: bool,
    data_dir: &Path,
    env: &RootsEnv,
    home: &Path,
    key: &str,
) -> otto_core::Result<ClaudeLogin> {
    let accounts = ProviderAccountsRepo::new(pool.clone())
        .list(user_id)
        .await?;
    let (data_dir, env, home) = (data_dir.to_path_buf(), env.clone(), home.to_path_buf());
    let logins = tokio::task::spawn_blocking(move || {
        claude_logins(&accounts, is_root, &data_dir, &home, &env)
    })
    .await
    .map_err(|e| Error::Internal(format!("list Claude logins: {e}")))?;
    logins
        .into_iter()
        .find(|l| l.key == key)
        .ok_or_else(|| Error::NotFound("no Claude login with that key is visible to you".into()))
}

/// Turn "Check with Anthropic" on or off for one login; answers the page.
pub async fn set_claude_check(
    pool: &DbPool,
    user_id: &Id,
    is_root: bool,
    data_dir: &Path,
    req: &ClaudeCheckReq,
) -> otto_core::Result<LimitsResp> {
    let env = RootsEnv::from_process();
    let login = login_for(
        pool,
        user_id,
        is_root,
        data_dir,
        &env,
        &home_dir(),
        &req.key,
    )
    .await?;
    let repo = SettingsRepo::new(pool.clone());
    let mut map = repo
        .get(CHECKS_KEY)
        .await?
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    if req.enabled {
        map.insert(login.key, serde_json::Value::Bool(true));
    } else {
        map.remove(&login.key);
    }
    repo.put(CHECKS_KEY, &serde_json::Value::Object(map))
        .await?;
    super::remaining_limits(pool, user_id, is_root, data_dir).await
}

/// Run the check for one login now (cached 5 min, one at a time). Refused
/// while the switch is off — the CLI is never started for a login whose
/// owner did not opt in. `program`: the `claude` binary Otto launches
/// sessions with (`None`: not installed).
pub async fn run_claude_check(
    pool: &DbPool,
    user_id: &Id,
    is_root: bool,
    data_dir: &Path,
    key: &str,
    program: Option<String>,
) -> otto_core::Result<LimitsResp> {
    run_with(
        pool,
        user_id,
        is_root,
        data_dir,
        (&RootsEnv::from_process(), &home_dir()),
        key,
        program,
        cli::CHECK_TIMEOUT,
    )
    .await?;
    super::remaining_limits(pool, user_id, is_root, data_dir).await
}

#[allow(clippy::too_many_arguments)] // the pub entry point's inputs plus the injected env/timeout tests need
async fn run_with(
    pool: &DbPool,
    user_id: &Id,
    is_root: bool,
    data_dir: &Path,
    (env, home): (&RootsEnv, &Path),
    key: &str,
    program: Option<String>,
    timeout: Duration,
) -> otto_core::Result<()> {
    let login = login_for(pool, user_id, is_root, data_dir, env, home, key).await?;
    if !load_checks(pool).await.contains(&login.key) {
        return Err(Error::Invalid(
            "Turn on Check with Anthropic for this login first.".into(),
        ));
    }
    cli::check(
        program.as_deref(),
        &login.dir,
        login.is_default(),
        timeout,
        Utc::now,
    )
    .await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    struct Fixture {
        _dir: tempfile::TempDir,
        root: PathBuf,
        pool: DbPool,
        env: RootsEnv,
    }

    /// A root caller with one machine Claude login (`<root>/claude-home`).
    async fn fixture() -> Fixture {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().to_path_buf();
        let pool = otto_state::open(&root.join("t.db")).await.unwrap();
        let projects = root.join("claude-home").join("projects");
        std::fs::create_dir_all(&projects).unwrap();
        let env = RootsEnv {
            explicit: Some(format!(
                "{}:{}",
                projects.display(),
                root.join("codex").display()
            )),
            isolated: false,
            codex_home: None,
        };
        Fixture {
            _dir: dir,
            root,
            pool,
            env,
        }
    }

    /// A CLI that only records that it ran.
    fn marker_cli(root: &Path) -> String {
        let p = root.join("claude");
        std::fs::write(
            &p,
            format!("#!/bin/sh\ntouch '{}'\n", root.join("ran").display()),
        )
        .unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p.display().to_string()
    }

    #[tokio::test]
    async fn opted_out_logins_never_start_the_cli() {
        let f = fixture().await;
        let program = marker_cli(&f.root);
        let user: Id = "root".into();
        let key = format!("claude:{}", f.root.join("claude-home").display());
        let err = run_with(
            &f.pool,
            &user,
            true,
            &f.root,
            (&f.env, Path::new("/nonexistent-home")),
            &key,
            Some(program),
            Duration::from_secs(2),
        )
        .await
        .expect_err("refused while off");
        assert!(err.to_string().contains("Turn on Check with Anthropic"));
        assert!(!f.root.join("ran").exists(), "the CLI was started");
        // A key the caller cannot see is a 404, also without a spawn.
        let err = run_with(
            &f.pool,
            &user,
            false,
            &f.root,
            (&f.env, Path::new("/nonexistent-home")),
            &key,
            Some(marker_cli(&f.root)),
            Duration::from_secs(2),
        )
        .await
        .expect_err("not visible to a non-root caller");
        assert!(matches!(err, Error::NotFound(_)));
        assert!(!f.root.join("ran").exists());
    }

    #[tokio::test]
    async fn an_opted_in_failed_check_is_unknown_with_the_reason() {
        let f = fixture().await;
        let user: Id = "root".into();
        let key = format!("claude:{}", f.root.join("claude-home").display());
        SettingsRepo::new(f.pool.clone())
            .put(CHECKS_KEY, &{
                let mut on = serde_json::Map::new();
                on.insert(key.clone(), serde_json::Value::Bool(true));
                serde_json::Value::Object(on)
            })
            .await
            .unwrap();
        assert!(load_checks(&f.pool).await.contains(&key));
        // The marker CLI exits without answering: a failure, not a number.
        run_with(
            &f.pool,
            &user,
            true,
            &f.root,
            (&f.env, Path::new("/nonexistent-home")),
            &key,
            Some(marker_cli(&f.root)),
            Duration::from_secs(5),
        )
        .await
        .unwrap();
        assert!(f.root.join("ran").exists());
        let login = claude_logins(&[], true, &f.root, Path::new("/nonexistent-home"), &f.env)
            .into_iter()
            .find(|l| l.key == key)
            .unwrap();
        let row = claude_row(&login, true, Utc::now());
        assert_eq!(row.status, super::super::LimitStatus::Unknown);
        assert!(row.windows.is_empty());
        let reason = row.reason.unwrap();
        assert!(
            reason.starts_with("Couldn’t check with Anthropic"),
            "{reason}"
        );
        let check = row.check.unwrap();
        assert!(check.enabled && check.checked_at.is_some() && check.error.is_some());
        // Off again: the default reason, the old failure is not shown.
        let off = claude_row(&login, false, Utc::now());
        assert_eq!(off.reason.as_deref(), Some(CLAUDE_REASON));
        assert_eq!(off.check.unwrap().error, None);
    }

    #[test]
    fn profiles_and_machine_dirs_map_to_their_config_dirs() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let default = home.join(".claude").join("projects");
        std::fs::create_dir_all(&default).unwrap();
        let env = RootsEnv {
            explicit: Some(format!(
                "{}:{}",
                default.display(),
                home.join("codex").display()
            )),
            isolated: false,
            codex_home: None,
        };
        let accounts = [ProviderAccount {
            id: "acc1".into(),
            provider: "claude".into(),
            label: "Max".into(),
            created_at: Utc::now(),
        }];
        let logins = claude_logins(&accounts, true, dir.path(), &home, &env);
        assert_eq!(logins.len(), 2);
        assert_eq!(logins[0].key, "claude:~/.claude");
        assert_eq!(logins[0].dir, home.join(".claude"));
        assert!(logins[0].is_default());
        assert_eq!(logins[1].key, "claude:account:acc1");
        assert_eq!(
            logins[1].dir,
            dir.path().join("provider-accounts").join("acc1")
        );
        // A non-root caller sees only the profile.
        let own = claude_logins(&accounts, false, dir.path(), &home, &env);
        assert_eq!(own.len(), 1);
    }
}
