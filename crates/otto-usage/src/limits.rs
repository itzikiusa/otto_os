//! Remaining subscription allowance per provider login (`GET /usage/limits`).
//!
//! One row per login Otto knows: the CLI's own default home (`~/.codex`,
//! `~/.claude`, every `~/.claude-*`) for root, and each Otto subscription
//! profile (`<data_dir>/provider-accounts/<id>`) for its owner. Only the newest
//! snapshot is served — no history — and only from files the CLIs already
//! write:
//!   * Codex: the `rate_limits` of the newest `token_count` line
//!     (`otto_transcript::limits`), read on demand by [`scan`];
//!   * Claude Code: it writes no plan limits to disk. Its rows come from an
//!     in-memory store ([`claude`]) fed by the `rate_limit_event` frames of
//!     structured Claude sessions and, only for a login whose owner turned on
//!     "Check with Anthropic", by that login's own CLI answering `get_usage`
//!     ([`cli`], [`logins`]). Otto never reads a login token;
//!   * Antigravity and Grok: no local source, so Otto asks each installed CLI
//!     ([`probes`]: `agy -p /usage`, grok's ACP `_x.ai/billing`), cached for
//!     five minutes. [`refresh_cli_limits`] runs before the rows are built.
//!
//! A snapshot older than [`STALE_AFTER_SECS`] is `stale`, and a window whose
//! reset time has passed since the snapshot drops its percent (it reset; the
//! file says nothing about use after that), so an old percent is never shown
//! as current.

mod claude;
mod cli;
mod logins;
mod probes;
mod scan;

pub use claude::{claude_config_dir, record_claude_line, record_claude_rate_limit};
pub use logins::{
    run_claude_check, run_claude_check_filtered, set_claude_check, set_claude_check_filtered,
    ClaudeCheckReq, ClaudeRunReq,
};
pub use probes::refresh as refresh_cli_limits;

use std::collections::HashSet;
use std::path::Path;

use chrono::{DateTime, Utc};
use otto_core::provider_accounts::ProviderAccount;
use otto_core::Id;
use otto_state::provider_accounts::ProviderAccountsRepo;
use otto_state::DbPool;
use otto_transcript::limits::{LimitCredits, LimitSnapshot};
use otto_transcript::roots::{home_dir, RootKind, RootsEnv};
use serde::Serialize;

/// A snapshot older than this is reported `stale`: Codex refreshes it on
/// every turn, and use from elsewhere (another Mac, the web) is invisible
/// until the next local turn.
pub const STALE_AFTER_SECS: i64 = 30 * 60;

/// Why a Claude row has no numbers (check off, no session reported any).
pub const CLAUDE_REASON: &str = "Claude Code does not write its plan limits to disk. Otto shows them once a structured Claude session reports them, or when Check with Anthropic is on for this login.";
/// Why an Antigravity row has no numbers before agy first answered.
pub const AGY_REASON: &str = "Otto has not heard back from agy yet.";

/// `GET /usage/limits`.
#[derive(Debug, Clone, Serialize)]
pub struct LimitsResp {
    pub accounts: Vec<AccountLimits>,
    pub generated_at: DateTime<Utc>,
    pub stale_after_secs: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitStatus {
    /// A fresh snapshot.
    Ok,
    /// A snapshot older than [`STALE_AFTER_SECS`].
    Stale,
    /// No snapshot; `reason` says why.
    Unknown,
}

/// One provider login.
#[derive(Debug, Clone, Serialize)]
pub struct AccountLimits {
    /// Stable row key (`codex:default`, `codex:account:<id>`, `claude:~/.claude-work`).
    pub key: String,
    /// `codex` | `claude` | `agy` | `grok`.
    pub provider: String,
    /// The Otto subscription profile, when the row is one.
    pub account_id: Option<Id>,
    pub label: String,
    /// `default` (the CLI's own home) | `config_dir` (`~/.claude-*`) | `account`.
    pub source: String,
    pub status: LimitStatus,
    pub reason: Option<String>,
    /// As the CLI names it (`pro`, `plus`, …).
    pub plan: Option<String>,
    /// When the CLI saw these numbers.
    pub taken_at: Option<DateTime<Utc>>,
    pub windows: Vec<WindowLimit>,
    pub credits: Option<LimitCredits>,
    /// Set while a window is exhausted (as of `taken_at`).
    pub limit_reached: Option<String>,
    /// Claude rows only: the opt-in "Check with Anthropic" state.
    pub check: Option<ClaudeCheck>,
}

/// The opt-in active check of one Claude login.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ClaudeCheck {
    /// The owner turned it on (off by default).
    pub enabled: bool,
    /// A check is running now.
    pub checking: bool,
    /// When the last check finished.
    pub checked_at: Option<DateTime<Utc>>,
    /// Why the last check failed (while enabled).
    pub error: Option<String>,
}

/// One window of a login's allowance.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct WindowLimit {
    /// `primary` / `secondary`; `<limit_id>:<slot>` for a model-specific bucket.
    pub id: String,
    /// `5-hour`, `Weekly`, `Monthly`, … (prefixed with a bucket's name).
    pub name: String,
    pub window_minutes: Option<u64>,
    /// `None` once the window reset after the snapshot.
    pub used_percent: Option<f64>,
    pub left_percent: Option<f64>,
    pub resets_at: Option<DateTime<Utc>>,
    /// `resets_at` passed after the snapshot was taken.
    pub reset_since_snapshot: bool,
    /// `false` for a limit the provider lists but does not enforce now
    /// (Claude `limits[].is_active`); the page shows it dimmed.
    pub active: bool,
    /// The provider's own level (`normal`, …) when it sent one.
    pub severity: Option<String>,
}

/// Active provider snapshot from `ProviderRegistry` so `GET /usage/limits`
/// hides disabled providers (e.g. `grok` marked Hidden) and leftover
/// `~/.claude-*` directories whose custom provider was removed or hidden.
#[derive(Debug, Clone, Default)]
pub struct ActiveLimitProviders {
    enabled_slugs: HashSet<String>,
    enabled_families: HashSet<String>,
    claude_tokens: HashSet<String>,
    claude_dir_hints: HashSet<String>,
    explicit_claude_dirs: Vec<std::path::PathBuf>,
    claude_dir_to_slug: Vec<(std::path::PathBuf, String)>,
    claude_norm_to_slug: std::collections::HashMap<String, Option<String>>,
    installed_families: Option<HashSet<String>>,
}

fn record_unambiguous_slug(
    map: &mut std::collections::HashMap<String, Option<String>>,
    key: String,
    slug: &str,
) {
    if key.is_empty() || key == "claude" {
        return;
    }
    match map.get_mut(&key) {
        None => {
            map.insert(key, Some(slug.to_string()));
        }
        Some(Some(existing)) if existing == slug => {}
        Some(slot) => *slot = None,
    }
}

fn norm_alnum(s: &str) -> String {
    s.chars()
        .filter(|c| c.is_ascii_alphanumeric())
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

impl ActiveLimitProviders {
    /// Build from `(slug, cmd, args, model_family)` tuples of enabled providers.
    pub fn from_specs<'a>(
        specs: impl IntoIterator<Item = (&'a str, &'a str, &'a [String], Option<&'a str>)>,
    ) -> Self {
        Self::from_specs_with_env_in(
            specs
                .into_iter()
                .map(|(slug, cmd, args, family)| (slug, cmd, args, family, None)),
            &home_dir(),
        )
    }

    /// Build from `(slug, cmd, args, model_family, env)` tuples of enabled providers.
    #[allow(clippy::type_complexity)]
    pub fn from_specs_with_env<'a>(
        specs: impl IntoIterator<
            Item = (
                &'a str,
                &'a str,
                &'a [String],
                Option<&'a str>,
                Option<&'a std::collections::BTreeMap<String, String>>,
            ),
        >,
    ) -> Self {
        Self::from_specs_with_env_in(specs, &home_dir())
    }

    /// Testable variant of [`Self::from_specs_with_env`] with an explicit `home`.
    #[allow(clippy::type_complexity)]
    pub fn from_specs_with_env_in<'a>(
        specs: impl IntoIterator<
            Item = (
                &'a str,
                &'a str,
                &'a [String],
                Option<&'a str>,
                Option<&'a std::collections::BTreeMap<String, String>>,
            ),
        >,
        home: &Path,
    ) -> Self {
        let empty_env = std::collections::BTreeMap::new();
        let mut out = Self::default();
        for (slug, cmd, args, family, env_opt) in specs {
            let slug_lc = slug.trim().to_ascii_lowercase();
            if slug_lc.is_empty() {
                continue;
            }
            out.enabled_slugs.insert(slug_lc.clone());
            if let Some(fam) = family.map(str::trim).filter(|f| !f.is_empty()) {
                out.enabled_families.insert(fam.to_ascii_lowercase());
            }
            let cmd_base = cmd
                .split_whitespace()
                .find(|tok| !tok.contains('='))
                .unwrap_or("")
                .rsplit('/')
                .next()
                .unwrap_or("");
            let cmd_lc = cmd_base.to_ascii_lowercase();
            for (prefix, fam) in [
                ("claude", "claude"),
                ("codex", "codex"),
                ("agy", "agy"),
                ("gemini", "agy"),
                ("grok", "grok"),
            ] {
                if slug_lc.starts_with(prefix) || cmd_lc.starts_with(prefix) {
                    out.enabled_families.insert(fam.to_string());
                }
            }
            for tok in cmd
                .split_whitespace()
                .chain(args.iter().map(String::as_str))
            {
                for seg in tok.split(['/', '=', '"', '\'']) {
                    if seg.starts_with(".claude-") {
                        out.claude_dir_hints.insert(seg.to_string());
                        let norm = norm_alnum(seg.trim_start_matches('.'));
                        if !norm.is_empty() && norm != "claude" {
                            out.claude_tokens.insert(norm);
                        }
                    }
                }
            }
            let is_claude = family.is_some_and(|f| f.eq_ignore_ascii_case("claude"))
                || slug_lc.starts_with("claude")
                || cmd_lc.starts_with("claude");
            if is_claude || family.is_none() {
                let env_map = env_opt.unwrap_or(&empty_env);
                if let Some(resolved_dir) =
                    otto_transcript::claude_config::resolve_claude_config_dir(
                        slug,
                        cmd,
                        args,
                        env_map.iter().map(|(k, v)| (k.as_str(), v.as_str())),
                        home,
                    )
                {
                    out.enabled_families.insert("claude".to_string());
                    if let Some(name) = resolved_dir.file_name().and_then(|n| n.to_str()) {
                        if name.starts_with(".claude-") {
                            out.claude_dir_hints.insert(name.to_string());
                            let norm = norm_alnum(name.trim_start_matches('.'));
                            if !norm.is_empty() && norm != "claude" {
                                out.claude_tokens.insert(norm.clone());
                                if slug_lc != "claude" {
                                    record_unambiguous_slug(
                                        &mut out.claude_norm_to_slug,
                                        norm,
                                        &slug_lc,
                                    );
                                }
                            }
                        }
                    }
                    if slug_lc != "claude" {
                        out.claude_dir_to_slug
                            .push((resolved_dir.clone(), slug_lc.clone()));
                    }
                    if !out.explicit_claude_dirs.contains(&resolved_dir) {
                        out.explicit_claude_dirs.push(resolved_dir);
                    }
                }
            }
            if is_claude {
                if slug_lc != "claude" {
                    record_unambiguous_slug(
                        &mut out.claude_norm_to_slug,
                        norm_alnum(&slug_lc),
                        &slug_lc,
                    );
                }
                for candidate in [slug_lc.as_str(), cmd_base] {
                    let norm = norm_alnum(candidate);
                    if !norm.is_empty() && norm != "claude" {
                        out.claude_tokens.insert(norm);
                    }
                }
            }
        }
        out
    }

    /// Restrict CLI-probed providers (`agy`, `grok`) to families whose binary
    /// or shell alias actually resolves on this machine.
    pub fn with_installed_families<I, S>(mut self, families: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: Into<String>,
    {
        self.installed_families = Some(
            families
                .into_iter()
                .map(|s| s.into().trim().to_ascii_lowercase())
                .collect(),
        );
        self
    }

    /// Whether the exact provider slug (`"claude"`, `"claudework"`, …) is enabled.
    pub fn is_builtin_slug_enabled(&self, slug: &str) -> bool {
        self.enabled_slugs
            .contains(&slug.trim().to_ascii_lowercase())
    }

    /// Whether any provider in `family` (`"claude"`, `"codex"`, `"agy"`, `"grok"`) is enabled.
    pub fn is_family_enabled(&self, family: &str) -> bool {
        let lc = family.trim().to_ascii_lowercase();
        self.enabled_slugs.contains(&lc) || self.enabled_families.contains(&lc)
    }

    /// Whether `provider` (`claude`, `codex`, `agy`, `grok`, …) is enabled at the family level.
    pub fn is_enabled(&self, provider: &str) -> bool {
        self.is_family_enabled(provider)
    }

    /// Whether `family`'s CLI is installed/resolvable (defaults to `true` when unset).
    pub fn is_installed(&self, family: &str) -> bool {
        let lc = family.trim().to_ascii_lowercase();
        self.installed_families
            .as_ref()
            .is_none_or(|set| set.contains(&lc))
    }

    /// Whether the default `~/.claude` login should be shown: when built-in
    /// `"claude"` is enabled OR a custom provider explicitly points at `default_dir`.
    pub fn is_default_claude_enabled(&self, default_dir: &Path) -> bool {
        self.is_builtin_slug_enabled("claude")
            || self.explicit_claude_dirs.iter().any(|d| d == default_dir)
    }

    /// Explicit/inferred custom `CLAUDE_CONFIG_DIR` paths of enabled providers.
    pub fn explicit_claude_dirs(&self) -> &[std::path::PathBuf] {
        &self.explicit_claude_dirs
    }

    /// Whether a non-default Claude config dir (`~/.claude-*` or
    /// `~/.config/claude`) corresponds to an enabled provider.
    pub fn matches_claude_config_dir(&self, dir: &Path) -> bool {
        if self.explicit_claude_dirs.iter().any(|d| d == dir) {
            return true;
        }
        let Some(name) = dir.file_name().and_then(|n| n.to_str()) else {
            return false;
        };
        if !name.starts_with(".claude-") {
            return self.is_builtin_slug_enabled("claude");
        }
        if self.claude_dir_hints.contains(name) {
            return true;
        }
        let norm = norm_alnum(name.trim_start_matches('.'));
        !norm.is_empty() && norm != "claude" && self.claude_tokens.contains(&norm)
    }

    /// Resolve a recorded `usage_events.provider` label (`"claude-work"`,
    /// `"claude:~/.claude-work"`, `"grok"`, …) to its canonical active slug,
    /// or `None` when `filter_inactive` is set and the provider is disabled or
    /// belongs to an orphan `~/.claude-*` directory.
    pub fn resolve_usage_provider_in(
        &self,
        raw: &str,
        filter_inactive: bool,
        home: &Path,
    ) -> Option<String> {
        let trimmed = raw.trim();
        let lc = trimmed.to_ascii_lowercase();
        if self.enabled_slugs.contains(&lc) {
            return Some(lc);
        }
        if let Some(rest) = trimmed
            .strip_prefix("claude:")
            .or_else(|| trimmed.strip_prefix("CLAUDE:"))
            .map(str::trim)
        {
            if !rest.starts_with("account:") {
                let dir = if let Some(rel) = rest.strip_prefix("~/") {
                    home.join(rel)
                } else if rest == "~" {
                    home.to_path_buf()
                } else {
                    std::path::PathBuf::from(rest)
                };
                let mut matched_slug: Option<&str> = None;
                let mut ambiguous = false;
                for (cand, slug) in &self.claude_dir_to_slug {
                    if cand == &dir {
                        match matched_slug {
                            None => matched_slug = Some(slug.as_str()),
                            Some(prev) if prev == slug => {}
                            Some(_) => ambiguous = true,
                        }
                    }
                }
                if let Some(slug) = matched_slug.filter(|_| !ambiguous) {
                    return Some(slug.to_string());
                }
                if let Some(name) = dir.file_name().and_then(|n| n.to_str()) {
                    let norm = norm_alnum(name.trim_start_matches('.'));
                    if let Some(Some(slug)) = self.claude_norm_to_slug.get(&norm) {
                        return Some(slug.clone());
                    }
                }
                let default_dir = home.join(".claude");
                let active = if dir == default_dir {
                    self.is_default_claude_enabled(&default_dir)
                } else {
                    self.matches_claude_config_dir(&dir)
                };
                return (!filter_inactive || active).then_some(lc);
            }
        }
        if lc.starts_with("claude-") {
            let norm = norm_alnum(&lc);
            if let Some(Some(slug)) = self.claude_norm_to_slug.get(&norm) {
                return Some(slug.clone());
            }
            let active = self.claude_dir_hints.contains(&format!(".{lc}"))
                || self.claude_tokens.contains(&norm);
            return (!filter_inactive || active).then_some(lc);
        }
        if filter_inactive {
            match lc.as_str() {
                "claude" if !self.is_default_claude_enabled(&home.join(".claude")) => {
                    return None;
                }
                "codex" | "agy" | "grok" | "opencode" | "pi" if !self.is_family_enabled(&lc) => {
                    return None;
                }
                _ => {}
            }
        }
        Some(lc)
    }

    /// Canonicalize `claude-work` / `claude:<dir>` rows onto their matching
    /// custom provider slug (e.g. `claudework`) and, when `filter_inactive` is
    /// true (`otto_only = false`), hide disabled built-ins and orphan
    /// `~/.claude-*` directories from `summary`.
    pub fn canonicalize_summary(&self, summary: &mut crate::UsageSummary, filter_inactive: bool) {
        self.canonicalize_summary_in(summary, filter_inactive, &home_dir());
    }

    pub fn canonicalize_summary_in(
        &self,
        summary: &mut crate::UsageSummary,
        filter_inactive: bool,
        home: &Path,
    ) {
        let mut merged_providers: Vec<crate::ProviderUsage> =
            Vec::with_capacity(summary.providers.len());
        for mut row in std::mem::take(&mut summary.providers) {
            let Some(canon) = self.resolve_usage_provider_in(&row.provider, filter_inactive, home)
            else {
                summary.total_events = summary.total_events.saturating_sub(row.events);
                summary.total_input_tokens =
                    summary.total_input_tokens.saturating_sub(row.input_tokens);
                summary.total_output_tokens = summary
                    .total_output_tokens
                    .saturating_sub(row.output_tokens);
                summary.total_cache_read_tokens = summary
                    .total_cache_read_tokens
                    .saturating_sub(row.cache_read_tokens);
                summary.total_cache_write_tokens = summary
                    .total_cache_write_tokens
                    .saturating_sub(row.cache_write_tokens);
                summary.total_tokens = summary.total_tokens.saturating_sub(row.total_tokens);
                summary.total_cost_usd = (summary.total_cost_usd - row.cost_usd).max(0.0);
                continue;
            };
            row.provider = canon;
            if let Some(existing) = merged_providers
                .iter_mut()
                .find(|p| p.provider == row.provider)
            {
                existing.events += row.events;
                existing.input_tokens += row.input_tokens;
                existing.output_tokens += row.output_tokens;
                existing.cache_read_tokens += row.cache_read_tokens;
                existing.cache_write_tokens += row.cache_write_tokens;
                existing.total_tokens += row.total_tokens;
                existing.cost_usd += row.cost_usd;
            } else {
                merged_providers.push(row);
            }
        }
        merged_providers.sort_by_key(|a| std::cmp::Reverse(a.total_tokens));
        summary.providers = merged_providers;

        let mut merged_models: Vec<crate::ModelUsage> = Vec::with_capacity(summary.models.len());
        for mut m in std::mem::take(&mut summary.models) {
            let Some(canon) = self.resolve_usage_provider_in(&m.provider, filter_inactive, home)
            else {
                continue;
            };
            m.provider = canon;
            if let Some(existing) = merged_models
                .iter_mut()
                .find(|x| x.provider == m.provider && x.model == m.model)
            {
                existing.events += m.events;
                existing.input_tokens += m.input_tokens;
                existing.output_tokens += m.output_tokens;
                existing.cache_read_tokens += m.cache_read_tokens;
                existing.cache_write_tokens += m.cache_write_tokens;
                existing.total_tokens += m.total_tokens;
                existing.cost_usd += m.cost_usd;
            } else {
                merged_models.push(m);
            }
        }
        merged_models.sort_by_key(|a| std::cmp::Reverse(a.total_tokens));
        summary.models = merged_models;

        let mut merged_daily_models: Vec<crate::DailyModelUsage> =
            Vec::with_capacity(summary.daily_models.len());
        for mut dm in std::mem::take(&mut summary.daily_models) {
            let Some(canon) = self.resolve_usage_provider_in(&dm.provider, filter_inactive, home)
            else {
                continue;
            };
            dm.provider = canon;
            if let Some(existing) = merged_daily_models
                .iter_mut()
                .find(|x| x.day == dm.day && x.provider == dm.provider && x.model == dm.model)
            {
                existing.input_tokens += dm.input_tokens;
                existing.output_tokens += dm.output_tokens;
                existing.cache_read_tokens += dm.cache_read_tokens;
                existing.cache_write_tokens += dm.cache_write_tokens;
                existing.total_tokens += dm.total_tokens;
                existing.cost_usd += dm.cost_usd;
            } else {
                merged_daily_models.push(dm);
            }
        }
        summary.daily_models = merged_daily_models;
    }
}

/// Every login `user_id` may see, with its newest limits. Root also sees the
/// machine's default CLI homes. Filesystem work runs on the blocking pool.
pub async fn remaining_limits(
    pool: &DbPool,
    user_id: &Id,
    is_root: bool,
    data_dir: &Path,
) -> otto_core::Result<LimitsResp> {
    remaining_limits_filtered(pool, user_id, is_root, data_dir, None).await
}

/// [`remaining_limits`] scoped to the providers currently enabled in Settings.
pub async fn remaining_limits_filtered(
    pool: &DbPool,
    user_id: &Id,
    is_root: bool,
    data_dir: &Path,
    active: Option<&ActiveLimitProviders>,
) -> otto_core::Result<LimitsResp> {
    let accounts = ProviderAccountsRepo::new(pool.clone())
        .list(user_id)
        .await?;
    let checks = logins::load_checks(pool).await;
    let data_dir = data_dir.to_path_buf();
    let active = active.cloned();
    let readings = probes::readings();
    let now = Utc::now();
    let rows = tokio::task::spawn_blocking(move || {
        build_filtered(
            &accounts,
            is_root,
            &data_dir,
            &home_dir(),
            &RootsEnv::from_process(),
            &checks,
            &readings,
            now,
            active.as_ref(),
        )
    })
    .await
    .map_err(|e| otto_core::Error::Internal(format!("read provider limits: {e}")))?;
    Ok(LimitsResp {
        accounts: rows,
        generated_at: now,
        stale_after_secs: STALE_AFTER_SECS,
    })
}

/// The rows, in display order: Codex, Claude, Antigravity, Grok; within each
/// the default login, `~/.claude-*` dirs, then profiles by label.
#[cfg(test)]
#[allow(clippy::too_many_arguments)]
fn build(
    accounts: &[ProviderAccount],
    is_root: bool,
    data_dir: &Path,
    home: &Path,
    env: &RootsEnv,
    checks: &logins::Checks,
    readings: &probes::Readings,
    now: DateTime<Utc>,
) -> Vec<AccountLimits> {
    build_filtered(
        accounts, is_root, data_dir, home, env, checks, readings, now, None,
    )
}

#[allow(clippy::too_many_arguments)]
fn build_filtered(
    accounts: &[ProviderAccount],
    is_root: bool,
    data_dir: &Path,
    home: &Path,
    env: &RootsEnv,
    checks: &logins::Checks,
    readings: &probes::Readings,
    now: DateTime<Utc>,
    active: Option<&ActiveLimitProviders>,
) -> Vec<AccountLimits> {
    let mut rows: Vec<AccountLimits> =
        logins::claude_logins_filtered(accounts, is_root, data_dir, home, env, active)
            .iter()
            .map(|login| logins::claude_row(login, checks.contains(&login.key), now))
            .collect();
    if is_root {
        let roots = env.discover(home, data_dir);
        if active.is_none_or(|a| a.is_enabled("codex")) {
            let codex: Vec<_> = roots
                .codex
                .iter()
                .filter(|r| r.kind != RootKind::Account && r.dir.is_dir())
                .collect();
            for (i, root) in codex.iter().enumerate() {
                let key = if i == 0 {
                    "codex:default".to_string()
                } else {
                    format!("codex:{}", tilde(&root.dir, home))
                };
                let label = if i == 0 {
                    "Default Codex login".to_string()
                } else {
                    tilde(root.dir.parent().unwrap_or(&root.dir), home)
                };
                rows.push(codex_row(key, None, label, "default", &root.dir, now));
            }
        }
        if active.is_none_or(|a| {
            a.is_enabled("agy") && (readings.contains_key("agy") || a.is_installed("agy"))
        }) && roots
            .agy
            .iter()
            .any(|r| r.kind != RootKind::Account && r.dir.is_dir())
        {
            rows.push(match readings.get("agy") {
                Some(r) => probes::agy_row(r, now),
                None => unknown_row(
                    "agy:default".into(),
                    "agy",
                    None,
                    probes::AGY_LABEL.into(),
                    "default",
                    AGY_REASON,
                ),
            });
        }
        // A reading is shown only when `grok` is enabled in Settings (hiding
        // `grok` immediately drops any cached reading from earlier in the run).
        if active.is_none_or(|a| a.is_enabled("grok") && a.is_installed("grok")) {
            if let Some(r) = readings.get("grok") {
                rows.push(probes::grok_row(r, now));
            }
        }
    }
    if active.is_none_or(|a| a.is_enabled("codex")) {
        let homes = data_dir.join("provider-accounts");
        for account in accounts {
            let Some(id) = otto_core::paths::safe_component(&account.id) else {
                continue;
            };
            let key = format!("{}:account:{id}", account.provider);
            if account.provider == "codex" {
                rows.push(codex_row(
                    key,
                    Some(account.id.clone()),
                    account.label.clone(),
                    "account",
                    &homes.join(id).join("sessions"),
                    now,
                ));
            }
        }
    }
    rows.sort_by_key(|r| (provider_rank(&r.provider), source_rank(&r.source)));
    rows
}

fn provider_rank(p: &str) -> u8 {
    match p {
        "codex" => 0,
        "claude" => 1,
        "agy" => 2,
        _ => 3,
    }
}

fn source_rank(s: &str) -> u8 {
    match s {
        "default" => 0,
        "config_dir" => 1,
        _ => 2,
    }
}

/// `dir` with the home prefix shown as `~`.
pub(crate) fn tilde(dir: &Path, home: &Path) -> String {
    match dir.strip_prefix(home) {
        Ok(rest) => format!("~/{}", rest.display()),
        Err(_) => dir.display().to_string(),
    }
}

pub(crate) fn unknown_row(
    key: String,
    provider: &str,
    account_id: Option<Id>,
    label: String,
    source: &str,
    reason: &str,
) -> AccountLimits {
    AccountLimits {
        key,
        provider: provider.into(),
        account_id,
        label,
        source: source.into(),
        status: LimitStatus::Unknown,
        reason: Some(reason.into()),
        plan: None,
        taken_at: None,
        windows: Vec::new(),
        credits: None,
        limit_reached: None,
        check: None,
    }
}

fn codex_row(
    key: String,
    account_id: Option<Id>,
    label: String,
    source: &str,
    sessions: &Path,
    now: DateTime<Utc>,
) -> AccountLimits {
    let found = scan::latest_cached(sessions);
    let Some(main) = found.main.as_ref() else {
        let reason = if !found.any_rollout {
            "No Codex session has run on this login yet. Limits appear after its first turn."
        } else if found.saw_no_limits {
            "This login reports no plan limits (an API key, or a Codex CLI too old to write them)."
        } else {
            "No Codex turn on this login has reported limits yet."
        };
        return unknown_row(key, "codex", account_id, label, source, reason);
    };
    let mut windows = codex_windows(main, None, now);
    for extra in &found.extra {
        // A model bucket older than the main snapshot is from an earlier
        // session; showing it beside fresh numbers would mix two moments.
        if extra.taken_at >= main.taken_at - chrono::Duration::seconds(STALE_AFTER_SECS) {
            let name = extra
                .limit_name
                .clone()
                .or_else(|| extra.limit_id.clone())
                .unwrap_or_default();
            windows.extend(codex_windows(extra, Some(&name), now));
        }
    }
    let stale = (now - main.taken_at).num_seconds() > STALE_AFTER_SECS;
    AccountLimits {
        key,
        provider: "codex".into(),
        account_id,
        label,
        source: source.into(),
        status: if stale {
            LimitStatus::Stale
        } else {
            LimitStatus::Ok
        },
        reason: None,
        plan: main.plan.clone(),
        taken_at: Some(main.taken_at),
        windows,
        credits: main.credits.clone(),
        limit_reached: main.reached.clone(),
        check: None,
    }
}

/// The windows of one snapshot as served. `bucket`: a model-specific
/// bucket's name (prefixes the ids and names).
fn codex_windows(s: &LimitSnapshot, bucket: Option<&str>, now: DateTime<Utc>) -> Vec<WindowLimit> {
    let monthly_plan = matches!(s.plan.as_deref(), Some("free" | "go"));
    s.windows
        .iter()
        .map(|w| {
            // No length on record: paid plans use 5-hour + weekly, Free/Go a
            // monthly primary (the same fallback T3 Code applies).
            let name = match w.window_minutes {
                Some(m) => window_name(m),
                None if w.slot == "secondary" => "Weekly".into(),
                None if monthly_plan => "Monthly".into(),
                None => "5-hour".into(),
            };
            let reset = w.resets_at.is_some_and(|r| r <= now);
            let used = (!reset).then_some(w.used_percent);
            WindowLimit {
                id: match bucket {
                    Some(_) => format!("{}:{}", s.limit_id.as_deref().unwrap_or("bucket"), w.slot),
                    None => w.slot.clone(),
                },
                name: match bucket {
                    Some(b) => format!("{b} · {name}"),
                    None => name,
                },
                window_minutes: w.window_minutes,
                used_percent: used,
                left_percent: used.map(|u| (100.0 - u).clamp(0.0, 100.0)),
                resets_at: w.resets_at,
                reset_since_snapshot: reset,
                active: true,
                severity: None,
            }
        })
        .collect()
}

/// `300` → `5-hour`, `10080` → `Weekly`, `43200` → `Monthly`.
fn window_name(minutes: u64) -> String {
    const DAY: u64 = 24 * 60;
    const WEEK: u64 = 7 * DAY;
    match minutes {
        m if m >= 28 * DAY => "Monthly".into(),
        WEEK => "Weekly".into(),
        DAY => "Daily".into(),
        m if m.is_multiple_of(DAY) => format!("{}-day", m / DAY),
        m if m.is_multiple_of(60) => format!("{}-hour", m / 60),
        m => format!("{m}-minute"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(ts: &str, used: f64, resets_at: i64, plan: &str) -> String {
        format!(
            r#"{{"timestamp":"{ts}","type":"event_msg","payload":{{"type":"token_count","info":null,"rate_limits":{{"limit_id":"codex","limit_name":null,"primary":{{"used_percent":{used},"window_minutes":300,"resets_at":{resets_at}}},"secondary":{{"used_percent":40.0,"window_minutes":10080,"resets_at":{resets_at}}},"credits":{{"has_credits":true,"unlimited":false,"balance":"62500.0"}},"plan_type":"{plan}","rate_limit_reached_type":null}}}}}}"#
        )
    }

    fn rollout(sessions: &Path, text: &str) {
        let day = sessions.join("2026/11/12");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("rollout-2026-11-12T10-00-00-x.jsonl"), text).unwrap();
    }

    fn at(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s).unwrap().with_timezone(&Utc)
    }

    fn env_for(claude: &Path, codex: &Path) -> RootsEnv {
        RootsEnv {
            explicit: Some(format!("{}:{}", claude.display(), codex.display())),
            isolated: false,
            codex_home: None,
        }
    }

    fn account(id: &str, provider: &str, label: &str) -> ProviderAccount {
        ProviderAccount {
            id: id.into(),
            provider: provider.into(),
            label: label.into(),
            created_at: Utc::now(),
        }
    }

    #[test]
    fn fresh_codex_snapshot_reports_used_and_left_per_window() {
        let dir = tempfile::tempdir().unwrap();
        let (claude, codex) = (dir.path().join("claude"), dir.path().join("codex"));
        std::fs::create_dir_all(&claude).unwrap();
        let reset = at("2026-11-12T15:00:00Z").timestamp();
        rollout(&codex, &line("2026-11-12T10:00:00Z", 13.0, reset, "pro"));
        let now = at("2026-11-12T10:10:00Z");
        let rows = build(
            &[],
            true,
            dir.path(),
            dir.path(),
            &env_for(&claude, &codex),
            &Default::default(),
            &Default::default(),
            now,
        );
        let row = rows.iter().find(|r| r.key == "codex:default").unwrap();
        assert_eq!(row.status, LimitStatus::Ok);
        assert_eq!(row.plan.as_deref(), Some("pro"));
        assert_eq!(row.taken_at, Some(at("2026-11-12T10:00:00Z")));
        assert_eq!(row.windows.len(), 2);
        assert_eq!(row.windows[0].name, "5-hour");
        assert_eq!(row.windows[0].used_percent, Some(13.0));
        assert_eq!(row.windows[0].left_percent, Some(87.0));
        assert_eq!(row.windows[1].name, "Weekly");
        assert_eq!(
            row.credits.as_ref().unwrap().balance.as_deref(),
            Some("62500.0")
        );
        // Claude never has numbers: unknown, with the reason.
        let claude_row = rows.iter().find(|r| r.provider == "claude").unwrap();
        assert_eq!(claude_row.status, LimitStatus::Unknown);
        assert_eq!(claude_row.reason.as_deref(), Some(CLAUDE_REASON));
        assert!(claude_row.windows.is_empty());
        assert_eq!(rows[0].provider, "codex", "codex rows first");
    }

    #[test]
    fn an_old_snapshot_is_stale_and_a_passed_reset_drops_the_percent() {
        let dir = tempfile::tempdir().unwrap();
        let (claude, codex) = (dir.path().join("claude"), dir.path().join("codex"));
        let reset = at("2026-11-12T15:00:00Z").timestamp();
        rollout(&codex, &line("2026-11-12T10:00:00Z", 13.0, reset, "pro"));
        let now = at("2026-11-12T16:00:00Z");
        let rows = build(
            &[],
            true,
            dir.path(),
            dir.path(),
            &env_for(&claude, &codex),
            &Default::default(),
            &Default::default(),
            now,
        );
        let row = &rows[0];
        assert_eq!(row.status, LimitStatus::Stale);
        let w = &row.windows[0];
        assert!(w.reset_since_snapshot);
        assert_eq!(w.used_percent, None);
        assert_eq!(w.left_percent, None);
    }

    #[test]
    fn profiles_are_keyed_by_account_and_explain_missing_data() {
        let dir = tempfile::tempdir().unwrap();
        let (claude, codex) = (dir.path().join("claude"), dir.path().join("codex"));
        let sessions = dir.path().join("provider-accounts/acc1/sessions");
        rollout(
            &sessions,
            r#"{"timestamp":"2026-11-12T10:00:00Z","type":"event_msg","payload":{"type":"token_count","info":{},"rate_limits":null}}"#,
        );
        let accounts = [
            account("acc1", "codex", "Work"),
            account("acc2", "codex", "Personal"),
            account("acc3", "claude", "Max"),
            account("../evil", "codex", "Bad"),
        ];
        let now = at("2026-11-12T10:10:00Z");
        // A non-root user sees only their own profiles.
        let rows = build(
            &accounts,
            false,
            dir.path(),
            dir.path(),
            &env_for(&claude, &codex),
            &Default::default(),
            &Default::default(),
            now,
        );
        let keys: Vec<_> = rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "codex:account:acc1",
                "codex:account:acc2",
                "claude:account:acc3"
            ]
        );
        assert!(rows.iter().all(|r| r.status == LimitStatus::Unknown));
        assert!(rows[0].reason.as_deref().unwrap().contains("API key"));
        assert!(rows[1]
            .reason
            .as_deref()
            .unwrap()
            .contains("No Codex session"));
        assert_eq!(rows[0].account_id.as_deref(), Some("acc1"));
        assert_eq!(rows[0].label, "Work");
    }

    #[test]
    fn a_claude_session_event_turns_the_claude_row_into_numbers() {
        let dir = tempfile::tempdir().unwrap();
        let (claude, codex) = (dir.path().join("claude"), dir.path().join("codex"));
        std::fs::create_dir_all(&claude).unwrap();
        let now = Utc::now();
        // The explicit Claude root's config dir is its parent (the temp dir).
        let ev = serde_json::json!({"type":"rate_limit_event","rate_limit_info":{"status":"allowed","rateLimitType":"five_hour","utilization":0.25,"resetsAt":(now + chrono::Duration::hours(2)).timestamp()}});
        assert!(record_claude_rate_limit(Some(dir.path()), &ev, now));
        let rows = build(
            &[],
            true,
            dir.path(),
            dir.path(),
            &env_for(&claude, &codex),
            &Default::default(),
            &Default::default(),
            now,
        );
        let row = rows.iter().find(|r| r.provider == "claude").unwrap();
        assert_eq!(row.status, LimitStatus::Ok);
        assert_eq!(row.reason, None);
        assert_eq!(row.windows[0].name, "5-hour");
        assert_eq!(row.windows[0].left_percent, Some(75.0));
        let check = row.check.as_ref().unwrap();
        assert!(!check.enabled && !check.checking);
    }

    #[test]
    fn agy_and_grok_rows_come_from_the_cli_readings() {
        let dir = tempfile::tempdir().unwrap();
        let agy = dir.path().join("agy");
        std::fs::create_dir_all(&agy).unwrap();
        let (claude, codex) = (dir.path().join("claude"), dir.path().join("codex"));
        let env = RootsEnv {
            explicit: Some(format!(
                "{}:{}:{}",
                claude.display(),
                codex.display(),
                agy.display()
            )),
            isolated: false,
            codex_home: None,
        };
        let now = at("2026-11-13T13:05:00Z");
        let build_with = |readings: &probes::Readings| {
            build(
                &[],
                true,
                dir.path(),
                dir.path(),
                &env,
                &Default::default(),
                readings,
                now,
            )
        };
        // agy has not answered yet: a row with the reason; no grok provider: no row.
        let rows = build_with(&Default::default());
        let keys: Vec<_> = rows.iter().map(|r| r.key.as_str()).collect();
        assert!(keys.contains(&"agy:default"), "{keys:?}");
        assert!(!keys.contains(&"grok:default"));
        let reading = |answer| probes::Reading {
            taken_at: at("2026-11-13T13:00:00Z"),
            answer,
        };
        let agy_data = serde_json::json!({"groups":[{"name":"Gemini Models","buckets":[{"id":"gemini-5h","window":"5h","remaining_fraction":0.66,"reset_time":"2026-11-13T15:24:17Z"}]}]});
        let readings = probes::Readings::from([
            ("agy", reading(Ok(agy_data))),
            ("grok", reading(Err("grok exited before answering.".into()))),
        ]);
        let rows = build_with(&readings);
        let agy_row = rows.iter().find(|r| r.key == "agy:default").unwrap();
        assert_eq!(agy_row.status, LimitStatus::Ok);
        assert_eq!(agy_row.windows[0].left_percent, Some(66.0));
        let grok = rows.last().unwrap();
        assert_eq!(
            (grok.key.as_str(), grok.status),
            ("grok:default", LimitStatus::Unknown)
        );
    }

    #[test]
    fn window_names_follow_their_length() {
        assert_eq!(window_name(300), "5-hour");
        assert_eq!(window_name(10080), "Weekly");
        assert_eq!(window_name(43200), "Monthly");
        assert_eq!(window_name(1440), "Daily");
        assert_eq!(window_name(2880), "2-day");
        assert_eq!(window_name(45), "45-minute");
    }

    #[test]
    fn serialized_shape_matches_the_contract() {
        let row = unknown_row(
            "claude:~/.claude".into(),
            "claude",
            None,
            "Default Claude login".into(),
            "default",
            CLAUDE_REASON,
        );
        let v = serde_json::to_value(&row).unwrap();
        assert_eq!(v["status"], "unknown");
        for key in [
            "key",
            "provider",
            "account_id",
            "label",
            "source",
            "status",
            "reason",
            "plan",
            "taken_at",
            "windows",
            "credits",
            "limit_reached",
            "check",
        ] {
            assert!(v.get(key).is_some(), "{key}");
        }
    }

    #[test]
    fn inactive_claude_sub_and_hidden_grok_are_excluded_from_remaining_limits() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".claude/projects")).unwrap();
        std::fs::create_dir_all(home.join(".claude-work/projects")).unwrap();
        std::fs::create_dir_all(home.join(".claude-sub/projects")).unwrap();
        std::fs::create_dir_all(home.join(".codex/sessions")).unwrap();
        std::fs::create_dir_all(home.join(".gemini/antigravity-cli/brain")).unwrap();
        let env = RootsEnv {
            explicit: None,
            isolated: false,
            codex_home: None,
        };
        let now = at("2026-11-15T20:00:00Z");
        let readings = probes::Readings::from([(
            "grok",
            probes::Reading {
                taken_at: now,
                answer: Err("cached before hiding".into()),
            },
        )]);
        let args = vec!["--dangerously-skip-permissions".to_string()];
        let active = ActiveLimitProviders::from_specs_with_env_in(
            [
                ("codex", "codex", &[][..], Some("codex"), None),
                ("claude", "claude", args.as_slice(), Some("claude"), None),
                (
                    "claudework",
                    "claude-work",
                    args.as_slice(),
                    Some("claude"),
                    None,
                ),
                ("agy", "agy", &[][..], Some("agy"), None),
            ],
            &home,
        );
        let rows = build_filtered(
            &[],
            true,
            dir.path(),
            &home,
            &env,
            &Default::default(),
            &readings,
            now,
            Some(&active),
        );
        let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(
            keys,
            [
                "codex:default",
                "claude:~/.claude",
                "claude:~/.claude-work",
                "agy:default",
            ]
        );
    }

    #[test]
    fn custom_only_claudework_hides_default_claude_uninstalled_agy_and_merges_summary_rows() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        std::fs::create_dir_all(home.join(".claude/projects")).unwrap();
        // Freshly logged-in `~/.claude-work` with `.credentials.json` before its first session.
        std::fs::create_dir_all(home.join(".claude-work")).unwrap();
        std::fs::write(home.join(".claude-work/.credentials.json"), "{}").unwrap();
        std::fs::create_dir_all(home.join(".claude-sub/projects")).unwrap();
        std::fs::create_dir_all(home.join(".codex/sessions")).unwrap();
        std::fs::create_dir_all(home.join(".gemini/antigravity-cli/brain")).unwrap();

        let active = ActiveLimitProviders::from_specs_with_env_in(
            [
                ("codex", "codex", &[][..], Some("codex"), None),
                ("claudework", "claude-work", &[][..], Some("claude"), None),
                ("agy", "agy", &[][..], Some("agy"), None),
            ],
            &home,
        )
        .with_installed_families(["codex", "claude"]);

        assert!(!active.is_builtin_slug_enabled("claude"));
        assert!(active.is_family_enabled("claude"));
        assert!(active.is_enabled("claude"));
        assert!(active.is_enabled("agy"));
        assert!(!active.is_installed("agy"));

        let env = RootsEnv {
            explicit: None,
            isolated: false,
            codex_home: None,
        };
        let rows = build_filtered(
            &[],
            true,
            dir.path(),
            &home,
            &env,
            &Default::default(),
            &Default::default(),
            at("2026-11-15T20:00:00Z"),
            Some(&active),
        );
        let keys: Vec<&str> = rows.iter().map(|r| r.key.as_str()).collect();
        assert_eq!(
            keys,
            ["codex:default", "claude:~/.claude-work"],
            "disabled default ~/.claude, orphan ~/.claude-sub, and uninstalled agy must stay hidden"
        );

        let mut summary = crate::UsageSummary {
            days: 7,
            total_events: 10,
            total_input_tokens: 1_000,
            total_output_tokens: 500,
            total_tokens: 1_500,
            total_cost_usd: 1.50,
            providers: vec![
                crate::ProviderUsage {
                    provider: "claudework".into(),
                    events: 3,
                    input_tokens: 300,
                    output_tokens: 150,
                    total_tokens: 450,
                    cost_usd: 0.45,
                    ..Default::default()
                },
                crate::ProviderUsage {
                    provider: "claude-work".into(),
                    events: 4,
                    input_tokens: 400,
                    output_tokens: 200,
                    total_tokens: 600,
                    cost_usd: 0.60,
                    ..Default::default()
                },
                crate::ProviderUsage {
                    provider: "claude-sub".into(),
                    events: 2,
                    input_tokens: 200,
                    output_tokens: 100,
                    total_tokens: 300,
                    cost_usd: 0.30,
                    ..Default::default()
                },
                crate::ProviderUsage {
                    provider: "grok".into(),
                    events: 1,
                    input_tokens: 100,
                    output_tokens: 50,
                    total_tokens: 150,
                    cost_usd: 0.15,
                    ..Default::default()
                },
            ],
            models: vec![
                crate::ModelUsage {
                    provider: "claudework".into(),
                    model: "claude-sonnet-4-5".into(),
                    events: 3,
                    total_tokens: 450,
                    cost_usd: 0.45,
                    ..Default::default()
                },
                crate::ModelUsage {
                    provider: "claude:~/.claude-work".into(),
                    model: "claude-sonnet-4-5".into(),
                    events: 4,
                    total_tokens: 600,
                    cost_usd: 0.60,
                    ..Default::default()
                },
                crate::ModelUsage {
                    provider: "claude-sub".into(),
                    model: "claude-sonnet-4-5".into(),
                    events: 2,
                    total_tokens: 300,
                    cost_usd: 0.30,
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        active.canonicalize_summary_in(&mut summary, true, &home);
        assert_eq!(summary.providers.len(), 1);
        assert_eq!(summary.providers[0].provider, "claudework");
        assert_eq!(summary.providers[0].events, 7);
        assert_eq!(summary.providers[0].total_tokens, 1_050);
        assert_eq!(summary.total_events, 7);
        assert_eq!(summary.total_tokens, 1_050);
        assert_eq!(summary.models.len(), 1);
        assert_eq!(summary.models[0].provider, "claudework");
        assert_eq!(summary.models[0].total_tokens, 1_050);
    }
}
