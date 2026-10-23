//! The usage-budget service (architecture-3-03): the persisted opt-in spend
//! caps, spend-vs-cap status over the budget window, and the admission check
//! every gated surface consults (goal loops, reviews, swarm turns, product
//! runs, the workflow `budget_gate` node). It used to live in the
//! `routes/usage.rs` handler file behind `*_pub` shims, so the Mission Control
//! card and the budget sampler in `monitor.rs` imported a route module to
//! reach it. The HTTP faces (`GET`/`PUT /usage/budgets`) stay in
//! `routes/usage.rs`; every gate caller imports `crate::budget` directly.

use otto_core::api::{BudgetStatusRow, UsageBudgetConfig, UsageBudgetStatus};
use otto_state::SettingsRepo;

use crate::state::ServerCtx;

/// `settings` key the usage-budget config is persisted under.
pub(crate) const BUDGETS_KEY: &str = "usage_budgets";

/// The budget service bound to one daemon context. Cheap to build per call.
pub struct BudgetGate<'a> {
    ctx: &'a ServerCtx,
}

impl<'a> BudgetGate<'a> {
    pub fn new(ctx: &'a ServerCtx) -> Self {
        Self { ctx }
    }

    /// The persisted config, `None` when none was ever saved. A settings read
    /// error or a stored value that no longer parses is an ERROR, never "no
    /// budgets" (error-handling-5-07): the page would show an empty editable
    /// form whose save overwrites the real caps, and admission would fail open.
    pub async fn read_config(&self) -> otto_core::Result<Option<UsageBudgetConfig>> {
        let raw = SettingsRepo::new(self.ctx.pool.clone())
            .get(BUDGETS_KEY)
            .await?;
        raw.map(serde_json::from_value).transpose().map_err(|e| {
            otto_core::Error::Internal(format!("stored usage budgets don't parse: {e}"))
        })
    }

    /// Best-effort [`Self::read_config`] for read-only summaries (Mission
    /// Control's budget card): an unreadable config shows no rows (logged).
    pub async fn stored_config(&self) -> Option<UsageBudgetConfig> {
        self.read_config().await.unwrap_or_else(|error| {
            tracing::warn!(%error, "usage budgets unreadable");
            None
        })
    }

    /// The persisted config (defaults: enforcement off), or the read error.
    pub async fn try_config(&self) -> otto_core::Result<UsageBudgetConfig> {
        Ok(self.read_config().await?.unwrap_or_default())
    }

    /// Best-effort [`Self::try_config`] for the BACKGROUND budget sampler
    /// (`monitor.rs`): an unreadable config samples nothing (logged). The
    /// budgets route and admission use `try_config` and surface the error.
    pub async fn config(&self) -> UsageBudgetConfig {
        self.try_config().await.unwrap_or_else(|error| {
            tracing::warn!(%error, "usage budgets unreadable; sampler skips this tick");
            UsageBudgetConfig::default()
        })
    }

    /// Replace the persisted config.
    pub async fn save_config(&self, cfg: &UsageBudgetConfig) -> otto_core::Result<()> {
        let value = serde_json::to_value(cfg)
            .map_err(|e| otto_core::Error::Internal(format!("serialize budgets: {e}")))?;
        SettingsRepo::new(self.ctx.pool.clone())
            .put(BUDGETS_KEY, &value)
            .await
    }

    /// Spend vs cap for every configured budget — a FOREGROUND read (the
    /// budgets route, Mission Control's budget card): it may wake ClickHouse.
    pub async fn status(&self, cfg: UsageBudgetConfig) -> UsageBudgetStatus {
        budget_status(self.ctx, cfg).await
    }

    /// [`Self::status`] for BACKGROUND checks (the budget sampler): never
    /// resets ClickHouse's idle clock nor wakes a parked server; `None` when
    /// there is no spend to judge yet.
    pub async fn status_background(&self, cfg: UsageBudgetConfig) -> Option<UsageBudgetStatus> {
        budget_status_background(self.ctx, cfg).await
    }

    /// The admission verdict for a `(workspace, provider)` — see [`check_budget`].
    pub async fn check(&self, workspace_id: &str, provider: &str) -> BudgetVerdict {
        check_with(self.ctx, workspace_id, provider).await
    }
}

/// [`budget_status`] for BACKGROUND checks — the budget sampler in
/// `monitor.rs` and [`check_budget`] (S9-303). Reads spend without resetting
/// ClickHouse's idle clock and without waking a parked server (it serves the
/// last spend it saw); `None` when there is no spend to judge yet.
async fn budget_status_background(
    ctx: &ServerCtx,
    mut cfg: UsageBudgetConfig,
) -> Option<UsageBudgetStatus> {
    let window_days = budget_window(&cfg);
    cfg.window_days = window_days;
    let totals = ctx
        .usage
        .session_totals_background(window_days, true)
        .await?;
    Some(fold_budget_status(ctx, cfg, &totals).await)
}

/// The configured budget window in days (0 = the 30-day default).
pub(crate) fn budget_window(cfg: &UsageBudgetConfig) -> u32 {
    if cfg.window_days == 0 {
        30
    } else {
        cfg.window_days.clamp(1, 3650)
    }
}

/// 80% of a cap is the "warning" line.
const BUDGET_WARN_FRACTION: f64 = 0.8;

/// Compute spend vs. cap for every configured budget over the window. On an
/// engine error the caps still render, with `spend_unavailable` set so the UI
/// says "Spend unavailable" rather than a measured $0 (error-handling-5-07).
async fn budget_status(ctx: &ServerCtx, mut cfg: UsageBudgetConfig) -> UsageBudgetStatus {
    let window_days = budget_window(&cfg);
    cfg.window_days = window_days;
    match ctx.usage.session_totals(window_days, true).await {
        Ok(totals) => fold_budget_status(ctx, cfg, &totals).await,
        Err(error) => {
            tracing::warn!(%error, "budget status: spend unavailable");
            let mut status = fold_budget_status(ctx, cfg, &[]).await;
            status.spend_unavailable = true;
            status
        }
    }
}

/// Fold per-session totals into spend-vs-cap rows (`cfg.window_days` already
/// normalised), labelling each workspace row with the workspace's name.
async fn fold_budget_status(
    ctx: &ServerCtx,
    cfg: UsageBudgetConfig,
    totals: &[otto_usage::SessionTotals],
) -> UsageBudgetStatus {
    let window_days = cfg.window_days;
    let mut rows = budget_rows(&cfg, totals);
    for row in rows.iter_mut().filter(|r| r.scope == "workspace") {
        row.label = ctx.workspaces.get(&row.key).await.ok().map(|w| w.name);
    }
    UsageBudgetStatus {
        config: cfg,
        window_days,
        rows,
        spend_unavailable: false,
    }
}

/// Spend vs cap for every configured (non-zero) cap: workspace rows first,
/// then provider rows, in config order. Pure — workspace rows carry no label
/// yet ([`fold_budget_status`] adds it).
fn budget_rows(
    cfg: &UsageBudgetConfig,
    totals: &[otto_usage::SessionTotals],
) -> Vec<BudgetStatusRow> {
    // One pass over per-session totals folds spend into both buckets.
    let mut by_ws: std::collections::HashMap<&str, f64> = std::collections::HashMap::new();
    let mut by_provider: std::collections::HashMap<&str, f64> = std::collections::HashMap::new();
    for t in totals {
        *by_ws.entry(t.workspace_id.as_str()).or_default() += t.cost_usd;
        *by_provider.entry(t.provider.as_str()).or_default() += t.cost_usd;
    }

    let mut rows = Vec::new();
    for b in &cfg.workspaces {
        if b.monthly_usd <= 0.0 {
            continue;
        }
        let spent = by_ws.get(b.workspace_id.as_str()).copied().unwrap_or(0.0);
        rows.push(make_row(
            "workspace",
            &b.workspace_id,
            None,
            b.monthly_usd,
            spent,
        ));
    }
    for b in &cfg.providers {
        if b.monthly_usd <= 0.0 {
            continue;
        }
        let spent = by_provider.get(b.provider.as_str()).copied().unwrap_or(0.0);
        rows.push(make_row(
            "provider",
            &b.provider,
            Some(b.provider.clone()),
            b.monthly_usd,
            spent,
        ));
    }
    rows
}

fn make_row(
    scope: &str,
    key: &str,
    label: Option<String>,
    limit: f64,
    spent: f64,
) -> BudgetStatusRow {
    let used = if limit > 0.0 { spent / limit } else { 0.0 };
    BudgetStatusRow {
        scope: scope.to_string(),
        key: key.to_string(),
        label,
        limit_usd: limit,
        spent_usd: spent,
        used_fraction: used,
        warning: used >= BUDGET_WARN_FRACTION,
        exceeded: used >= 1.0,
    }
}

/// Outcome of a daemon-side budget consultation for a workspace + provider.
#[derive(Debug, Clone, Default)]
pub struct BudgetVerdict {
    /// True when enforcement is on AND `block_on_exceed` AND a relevant cap is
    /// exceeded or its spend cannot be read. Callers that gate work check this.
    pub blocked: bool,
    /// True when enforcement is on and a relevant cap is exceeded (whether or not
    /// blocking is enabled). Callers can use this to warn prominently.
    pub exceeded: bool,
    /// Human-readable reason when exceeded/blocked (which cap, spend vs limit).
    pub reason: Option<String>,
}

/// Daemon-consultable budget check for a `(workspace, provider)`. Returns a
/// no-op verdict when enforcement is off (the default), so callers can wire this
/// in safely without changing behaviour until a root user opts in. An unreadable
/// relevant cap blocks only when blocking enforcement is explicitly enabled;
/// unknown spend is never reported as a measured exceedance.
pub async fn check_budget(ctx: &ServerCtx, workspace_id: &str, provider: &str) -> BudgetVerdict {
    BudgetGate::new(ctx).check(workspace_id, provider).await
}

async fn check_with(ctx: &ServerCtx, workspace_id: &str, provider: &str) -> BudgetVerdict {
    // An unreadable config is NOT "no budgets" (error-handling-5-07): the
    // enforce / block flags are unknown, so admission fails closed exactly
    // like unavailable spend, with a reason the caller shows.
    let cfg = match BudgetGate::new(ctx).try_config().await {
        Ok(cfg) => cfg,
        Err(error) => {
            tracing::warn!(%error, workspace_id, provider, "budget config unavailable");
            return config_unavailable_verdict();
        }
    };
    if !cfg.enforce {
        return BudgetVerdict::default();
    }
    let relevant_cap = cfg
        .workspaces
        .iter()
        .any(|cap| cap.workspace_id == workspace_id && cap.monthly_usd > 0.0)
        || cfg
            .providers
            .iter()
            .any(|cap| cap.provider == provider && cap.monthly_usd > 0.0);
    if !relevant_cap {
        return BudgetVerdict::default();
    }
    let mut cfg = cfg;
    cfg.window_days = budget_window(&cfg);
    // Admission must include accepted pending spend and retain read/flush errors.
    // The strict accessor preserves background idle semantics without treating a
    // failed flush or an unavailable engine as a successful zero-spend reading.
    let totals = match ctx
        .usage
        .session_totals_for_budget(cfg.window_days, true)
        .await
    {
        Ok(totals) => totals,
        Err(error) => {
            tracing::warn!(%error, workspace_id, provider, "budget spend unavailable");
            return BudgetVerdict {
                blocked: cfg.block_on_exceed,
                exceeded: false,
                reason: Some(
                    "Budget spend unavailable; retry after usage accounting recovers".into(),
                ),
            };
        }
    };
    verdict_from(&cfg, &totals, workspace_id, provider)
}

/// The admission verdict for a `(workspace, provider)` given the window's
/// measured spend — pure, so every gate decision is unit-tested. Enforcement
/// off never blocks; only a cap on THIS workspace or THIS provider counts; a
/// cap is exceeded at 100% of it (exactly at the cap included) and blocks
/// only with `block_on_exceed`.
pub(crate) fn verdict_from(
    cfg: &UsageBudgetConfig,
    totals: &[otto_usage::SessionTotals],
    workspace_id: &str,
    provider: &str,
) -> BudgetVerdict {
    if !cfg.enforce {
        return BudgetVerdict::default();
    }
    let window_days = budget_window(cfg);
    for row in budget_rows(cfg, totals) {
        let relevant = (row.scope == "workspace" && row.key == workspace_id)
            || (row.scope == "provider" && row.key == provider);
        if relevant && row.exceeded {
            let reason = format!(
                "{} budget exceeded: ${:.2} spent of ${:.2} cap over {}d",
                row.scope, row.spent_usd, row.limit_usd, window_days
            );
            return BudgetVerdict {
                blocked: cfg.block_on_exceed,
                exceeded: true,
                reason: Some(reason),
            };
        }
    }
    BudgetVerdict::default()
}

/// The verdict when the stored budget config can't be read or parsed.
fn config_unavailable_verdict() -> BudgetVerdict {
    BudgetVerdict {
        blocked: true,
        exceeded: false,
        reason: Some(
            "Budget settings unavailable; fix or re-save them under Usage → Budgets and retry"
                .into(),
        ),
    }
}

#[cfg(test)]
mod budget_admission_tests {
    use super::*;
    use otto_core::api::{ProviderBudget, WorkspaceBudget};
    use otto_state::SettingsRepo;

    /// An unavailable engine must not become a successful zero-spend reading.
    /// Exercise the real settings lookup and admission function, without a
    /// ClickHouse installation or any access to the user's accounting data.
    #[tokio::test]
    async fn unavailable_spend_respects_only_explicit_relevant_blocking_caps() {
        use crate::routes::browser::tests::{mem_pool, seed_workspace, test_ctx};
        let tmp = tempfile::TempDir::new().unwrap();
        let pool = mem_pool().await;
        seed_workspace(&pool, "budget-ws").await;
        let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
        let settings = SettingsRepo::new(pool);
        let mut cfg = UsageBudgetConfig {
            enforce: true,
            block_on_exceed: true,
            workspaces: vec![WorkspaceBudget {
                workspace_id: "budget-ws".into(),
                monthly_usd: 10.0,
            }],
            ..Default::default()
        };
        settings
            .put(BUDGETS_KEY, &serde_json::to_value(&cfg).unwrap())
            .await
            .unwrap();
        let verdict = check_budget(&ctx, "budget-ws", "codex").await;
        assert!(
            verdict.blocked,
            "unreadable spend cannot satisfy an explicit blocking cap"
        );
        assert!(
            !verdict.exceeded,
            "unknown spend is not proof that the cap was exceeded"
        );
        assert!(verdict.reason.as_deref().unwrap().contains("unavailable"));
        assert!(!check_budget(&ctx, "other-ws", "codex").await.blocked);

        cfg.block_on_exceed = false;
        settings
            .put(BUDGETS_KEY, &serde_json::to_value(&cfg).unwrap())
            .await
            .unwrap();
        assert!(!check_budget(&ctx, "budget-ws", "codex").await.blocked);
        cfg.block_on_exceed = true;
        cfg.enforce = false;
        settings
            .put(BUDGETS_KEY, &serde_json::to_value(&cfg).unwrap())
            .await
            .unwrap();
        assert!(!check_budget(&ctx, "budget-ws", "codex").await.blocked);

        cfg.enforce = true;
        cfg.workspaces[0].monthly_usd = 0.0;
        settings
            .put(BUDGETS_KEY, &serde_json::to_value(&cfg).unwrap())
            .await
            .unwrap();
        assert!(!check_budget(&ctx, "budget-ws", "codex").await.blocked);
        cfg.providers.push(ProviderBudget {
            provider: "codex".into(),
            monthly_usd: 10.0,
        });
        settings
            .put(BUDGETS_KEY, &serde_json::to_value(&cfg).unwrap())
            .await
            .unwrap();
        assert!(check_budget(&ctx, "other-ws", "codex").await.blocked);
        assert!(!check_budget(&ctx, "other-ws", "grok").await.blocked);
    }

    fn spend(ws: &str, provider: &str, usd: f64) -> otto_usage::SessionTotals {
        serde_json::from_value(serde_json::json!({
            "session_id": format!("{ws}-{provider}"), "workspace_id": ws, "provider": provider,
            "events": 1, "total_tokens": 1, "cost_usd": usd,
        }))
        .unwrap()
    }

    fn ws_cap(ws: &str, usd: f64, block: bool) -> UsageBudgetConfig {
        UsageBudgetConfig {
            enforce: true,
            block_on_exceed: block,
            workspaces: vec![WorkspaceBudget {
                workspace_id: ws.into(),
                monthly_usd: usd,
            }],
            ..Default::default()
        }
    }

    /// test-quality-6-03: the measured path. Spend exactly at the cap is
    /// exceeded, and blocks iff `block_on_exceed`.
    #[test]
    fn spend_exactly_at_the_cap_is_exceeded_and_blocks_only_when_asked() {
        let totals = [spend("a", "codex", 6.0), spend("a", "grok", 4.0)];
        let v = verdict_from(&ws_cap("a", 10.0, true), &totals, "a", "codex");
        assert!(v.exceeded && v.blocked);
        assert_eq!(
            v.reason.as_deref(),
            Some("workspace budget exceeded: $10.00 spent of $10.00 cap over 30d")
        );
        let warn_only = verdict_from(&ws_cap("a", 10.0, false), &totals, "a", "codex");
        assert!(warn_only.exceeded && !warn_only.blocked);
        let under = verdict_from(&ws_cap("a", 10.01, true), &totals, "a", "codex");
        assert!(!under.exceeded && !under.blocked);
    }

    #[test]
    fn the_warning_line_is_eighty_percent() {
        let cfg = ws_cap("a", 100.0, true);
        let at = |usd: f64| budget_rows(&cfg, &[spend("a", "codex", usd)])[0].clone();
        assert!(!at(79.0).warning);
        assert!(at(80.0).warning && !at(80.0).exceeded);
    }

    #[test]
    fn another_workspaces_overspend_leaves_this_one_unblocked() {
        let mut cfg = ws_cap("a", 10.0, true);
        cfg.workspaces.push(WorkspaceBudget {
            workspace_id: "b".into(),
            monthly_usd: 10.0,
        });
        let totals = [spend("a", "codex", 50.0), spend("b", "codex", 1.0)];
        assert!(verdict_from(&cfg, &totals, "a", "codex").blocked);
        let b = verdict_from(&cfg, &totals, "b", "codex");
        assert!(!b.blocked && !b.exceeded);
    }

    #[test]
    fn a_provider_cap_blocks_that_provider_only() {
        let cfg = UsageBudgetConfig {
            enforce: true,
            block_on_exceed: true,
            providers: vec![ProviderBudget {
                provider: "codex".into(),
                monthly_usd: 5.0,
            }],
            ..Default::default()
        };
        // Spend lands across workspaces: the provider cap sums all of it.
        let totals = [spend("a", "codex", 3.0), spend("b", "codex", 2.0)];
        let codex = verdict_from(&cfg, &totals, "a", "codex");
        assert!(codex.blocked);
        assert!(codex
            .reason
            .unwrap()
            .starts_with("provider budget exceeded"));
        assert!(!verdict_from(&cfg, &totals, "a", "grok").blocked);
    }

    #[test]
    fn enforcement_off_never_blocks() {
        let mut cfg = ws_cap("a", 1.0, true);
        cfg.enforce = false;
        let v = verdict_from(&cfg, &[spend("a", "codex", 999.0)], "a", "codex");
        assert!(!v.blocked && !v.exceeded && v.reason.is_none());
    }

    /// error-handling-5-07: a stored config that no longer parses is an
    /// error for the budgets route (`try_config`) and a fail-closed verdict
    /// for admission — never "no budgets" (enforcement off, admit).
    #[tokio::test]
    async fn an_unparsable_config_is_an_error_and_admission_fails_closed() {
        use crate::routes::browser::tests::{mem_pool, seed_workspace, test_ctx};
        let tmp = tempfile::TempDir::new().unwrap();
        let pool = mem_pool().await;
        seed_workspace(&pool, "budget-ws").await;
        let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
        let gate = BudgetGate::new(&ctx);
        assert!(gate.read_config().await.unwrap().is_none(), "never saved");
        assert!(!check_budget(&ctx, "budget-ws", "codex").await.blocked);
        SettingsRepo::new(pool)
            .put(BUDGETS_KEY, &serde_json::json!({"enforce": "yes please"}))
            .await
            .unwrap();
        assert!(gate.try_config().await.is_err());
        let verdict = check_budget(&ctx, "budget-ws", "codex").await;
        assert!(verdict.blocked && !verdict.exceeded);
        assert!(verdict
            .reason
            .unwrap()
            .contains("Budget settings unavailable"));
    }
}
