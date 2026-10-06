//! End-to-end test of the embedded ClickHouse usage engine against a *real*
//! local `clickhouse` binary.
//!
//! Exercises the full path the daemon uses: locate the binary → create the
//! schema → record usage events (buffered + synchronous) → sample & store
//! system metrics → run every dashboard aggregate → verify retention TTL
//! changes apply → verify data survives an engine restart (on-disk
//! persistence).
//!
//! Skips (does not fail) when no `clickhouse` binary is present, so it's safe
//! in CI that lacks one.

// Test harness: blocking std calls (ps, kill, read_dir) are fine here.
#![allow(clippy::disallowed_methods)]

use std::time::Duration;

use otto_usage::{
    AttributionDimension, ClickHouse, ForecastReq, MetricsSampler, ReportOptions, UsageConfig,
    UsageEngine, UsageEvent, UsageScope,
};

fn event(
    provider: &str,
    session: &str,
    model: &str,
    kind: &str,
    inp: u64,
    out: u64,
    cost: f64,
) -> UsageEvent {
    UsageEvent {
        workspace_id: "ws1".into(),
        session_id: session.into(),
        provider: provider.into(),
        model: model.into(),
        kind: kind.into(),
        input_tokens: inp,
        output_tokens: out,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        cost_usd: cost,
        duration_ms: 1200,
        ..Default::default()
    }
}

/// Build a usage event with work-graph dims populated (B1).
fn event_with_dims(
    provider: &str,
    session: &str,
    cost: f64,
    origin: &str,
    repo_id: &str,
    branch: &str,
) -> UsageEvent {
    UsageEvent {
        workspace_id: "ws1".into(),
        session_id: session.into(),
        provider: provider.into(),
        model: "claude-opus-4".into(),
        kind: "completion".into(),
        input_tokens: 100,
        output_tokens: 200,
        cache_read_tokens: 0,
        cache_write_tokens: 0,
        cost_usd: cost,
        duration_ms: 500,
        origin: origin.to_string(),
        repo_id: repo_id.to_string(),
        branch: branch.to_string(),
        ..Default::default()
    }
}

/// Tests that boot a *real* ClickHouse server are serialized: several servers
/// starting at once (install checks, dir adoption, port hunting) slow each
/// other past the `wait_ready` windows and flake.
static SERIAL: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

async fn serial() -> tokio::sync::MutexGuard<'static, ()> {
    SERIAL.lock().await
}

#[tokio::test]
async fn usage_engine_end_to_end() {
    if ClickHouse::locate(None).is_none() {
        eprintln!("SKIP: no `clickhouse` binary found on this machine");
        return;
    }
    let _serial = serial().await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let data_dir = tmp.path().to_path_buf();

    let config = UsageConfig {
        enabled: true,
        retention_days: 180,
        metrics_interval_secs: 60,
        clickhouse_path: None,
    };

    let engine = UsageEngine::start(config, data_dir.clone()).await;
    engine.wait_ready(Duration::from_secs(30)).await;
    assert!(
        engine.available(),
        "engine should be available with a real clickhouse"
    );

    // ── Record usage (synchronous insert for determinism) ──────────────────
    engine
        .insert_events(&[
            event("claude", "s1", "claude-opus-4", "prompt", 1000, 500, 0.0),
            event("claude", "s1", "claude-opus-4", "completion", 0, 800, 0.06),
            event("claude", "s2", "claude-sonnet-4", "prompt", 400, 200, 0.0),
            event("codex", "s3", "gpt-5-codex", "prompt", 1200, 900, 0.02),
            event("codex", "s3", "gpt-5-codex", "tool", 0, 0, 0.0),
        ])
        .await
        .expect("insert events");

    // ── Also exercise the buffered fire-and-forget path ─────────────────────
    engine.record(event(
        "claude",
        "s2",
        "claude-sonnet-4",
        "completion",
        0,
        300,
        0.01,
    ));
    // The background writer flushes FLUSH_INTERVAL after the first buffered
    // event, or as soon as a read pokes it (each `summary` below does): wait
    // for the row to land (condition, not a fixed sleep).
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    while engine
        .summary(30, false)
        .await
        .expect("summary")
        .total_events
        < 6
    {
        assert!(
            std::time::Instant::now() < deadline,
            "buffered event never flushed"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // ── Provider rollup ─────────────────────────────────────────────────────
    let providers = engine
        .provider_usage(30, false)
        .await
        .expect("provider usage");
    assert_eq!(providers.len(), 2, "two providers recorded");
    let claude = providers
        .iter()
        .find(|p| p.provider == "claude")
        .expect("claude row");
    // 3 claude events: (1000+500) + (800) + (400+200) + buffered (300) = 4 events total now.
    assert_eq!(claude.events, 4, "claude events (incl. buffered)");
    assert_eq!(
        claude.input_tokens, 1400,
        "claude input tokens (1000+0+400+0)"
    );
    assert_eq!(
        claude.output_tokens, 1800,
        "claude output tokens (500+800+200+300)"
    );
    assert_eq!(claude.total_tokens, 3200, "claude total tokens");
    assert!(
        (claude.cost_usd - 0.07).abs() < 1e-9,
        "claude cost = 0.06 + 0.01"
    );

    let codex = providers
        .iter()
        .find(|p| p.provider == "codex")
        .expect("codex row");
    assert_eq!(codex.events, 2);
    assert_eq!(codex.total_tokens, 2100);

    // ── Summary totals ────────────────────────────────────────────────────────
    let summary = engine.summary(30, false).await.expect("summary");
    assert_eq!(summary.total_events, 6);
    assert_eq!(summary.total_tokens, 5300);
    assert!((summary.total_cost_usd - 0.09).abs() < 1e-9);
    assert_eq!(summary.providers.len(), 2);

    // ── Daily rollup (everything lands on today) ──────────────────────────────
    let daily = engine.daily_usage(30, false).await.expect("daily");
    assert_eq!(daily.len(), 1, "all events are from today");
    assert_eq!(daily[0].total_tokens, 5300);

    // ── Session leaderboard ───────────────────────────────────────────────────
    let sessions = engine.session_usage(30, 50, false).await.expect("sessions");
    assert_eq!(sessions.len(), 3, "three distinct sessions");
    let top = &sessions[0];
    assert_eq!(top.session_id, "s1", "s1 has the most tokens (2300)");
    assert_eq!(top.total_tokens, 2300);

    // ── System metrics ────────────────────────────────────────────────────────
    let metric = tokio::task::spawn_blocking(|| MetricsSampler::new().sample(7))
        .await
        .expect("sample");
    assert!(metric.mem_total_mb > 0.0, "should read total memory");
    assert_eq!(metric.active_sessions, 7);
    engine.store_metric(&metric).await.expect("store metric");

    // Buffered (R1a): served from memory before the 5-minute batch insert…
    let points = engine.metrics(60).await.expect("metrics query");
    assert_eq!(points.len(), 1, "one metric point stored");
    assert_eq!(points[0].active_sessions, 7);
    assert!(points[0].mem_total_mb > 0.0);
    // …and exactly once after it (no duplicate of the flushed tail).
    assert_eq!(engine.flush_metrics().await.expect("flush metrics"), 1);
    let points = engine.metrics(60).await.expect("metrics after flush");
    assert_eq!(points.len(), 1, "flushed sample not double-counted");
    assert_eq!(points[0].active_sessions, 7);

    // ── Status ────────────────────────────────────────────────────────────────
    let status = engine.status().await;
    assert!(status.available);
    assert_eq!(status.usage_rows, 6);
    assert_eq!(status.metric_rows, 1);
    assert!(status.version.is_some());
    assert!(status.disk_bytes > 0);
    assert_eq!(status.retention_days, 180);

    // ── Retention change applies live ─────────────────────────────────────────
    engine.set_retention(90).await.expect("set retention");
    assert_eq!(engine.status().await.retention_days, 90);

    // ── Persistence: a fresh engine over the same dir sees prior data ──────────
    drop(engine);
    let engine2 = UsageEngine::start(
        UsageConfig {
            enabled: true,
            retention_days: 90,
            metrics_interval_secs: 60,
            clickhouse_path: None,
        },
        data_dir.clone(),
    )
    .await;
    engine2.wait_ready(Duration::from_secs(30)).await;
    assert!(engine2.available());
    let reopened = engine2
        .summary(30, false)
        .await
        .expect("summary after reopen");
    assert_eq!(reopened.total_events, 6, "data persisted across restart");
    assert_eq!(reopened.total_tokens, 5300);
    engine2.shutdown().await; // stop the server (no orphan when the test exits)
}

#[tokio::test]
async fn disabled_engine_is_a_noop() {
    let tmp = tempfile::tempdir().expect("tempdir");
    let engine = UsageEngine::start(
        UsageConfig {
            enabled: false,
            ..Default::default()
        },
        tmp.path().to_path_buf(),
    )
    .await;
    assert!(!engine.available());
    // Recording / querying a disabled engine must not error.
    engine.record(event("claude", "s1", "m", "prompt", 1, 1, 0.0));
    engine
        .insert_events(&[event("claude", "s1", "m", "prompt", 1, 1, 0.0)])
        .await
        .unwrap();
    assert_eq!(engine.summary(7, false).await.unwrap().total_events, 0);
    assert!(!engine.status().await.available);
}

// ---------------------------------------------------------------------------
// Work-ref attribution tests (B1)
// ---------------------------------------------------------------------------

/// Verify that a `UsageEvent` with work-graph dims round-trips through JSON
/// serialization and that absent dims are omitted (matching the column DEFAULT).
#[test]
fn usage_event_workref_round_trip() {
    let ev = UsageEvent {
        workspace_id: "ws1".into(),
        session_id: "s1".into(),
        provider: "claude".into(),
        model: "claude-opus-4".into(),
        kind: "completion".into(),
        input_tokens: 100,
        output_tokens: 200,
        cost_usd: 0.05,
        origin: "review".into(),
        repo_id: "repo-abc".into(),
        branch: "feature/b1".into(),
        // All other dims left as empty string (default) → omitted in JSON.
        ..Default::default()
    };

    let json = serde_json::to_string(&ev).expect("serialize");
    // Set dims must be present.
    assert!(json.contains("\"origin\":\"review\""), "origin present");
    assert!(json.contains("\"repo_id\":\"repo-abc\""), "repo_id present");
    assert!(json.contains("\"branch\":\"feature/b1\""), "branch present");
    // Unset dims must be absent (skip_serializing_if = is_empty).
    assert!(!json.contains("\"pr_number\""), "pr_number absent");
    assert!(!json.contains("\"story_id\""), "story_id absent");
    assert!(!json.contains("\"swarm_task_id\""), "swarm_task_id absent");

    // Round-trip.
    let back: UsageEvent = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back.origin, "review");
    assert_eq!(back.repo_id, "repo-abc");
    assert_eq!(back.branch, "feature/b1");
    assert_eq!(back.pr_number, ""); // defaulted to empty on missing key
    assert_eq!(back.story_id, "");
}

/// End-to-end attribution query: insert events with origin dims, then group by
/// origin and verify the aggregates.
#[tokio::test]
async fn attribution_groups_by_dimension() {
    if ClickHouse::locate(None).is_none() {
        eprintln!("SKIP: no `clickhouse` binary found on this machine");
        return;
    }
    let _serial = serial().await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let engine = UsageEngine::start(
        UsageConfig {
            enabled: true,
            ..Default::default()
        },
        tmp.path().to_path_buf(),
    )
    .await;
    engine.wait_ready(Duration::from_secs(30)).await;
    assert!(engine.available());

    // Insert 3 events across 2 origins.
    engine
        .insert_events(&[
            event_with_dims("claude", "s1", 0.05, "review", "repo-1", "main"),
            event_with_dims("claude", "s2", 0.03, "review", "repo-1", "feature"),
            event_with_dims("claude", "s3", 0.08, "product", "repo-2", "main"),
        ])
        .await
        .expect("insert with dims");

    // Group by origin.
    let rows = engine
        .attribution(&AttributionDimension::Origin, 30)
        .await
        .expect("attribution by origin");

    assert_eq!(rows.len(), 2, "two distinct origins");
    let review = rows.iter().find(|r| r.key == "review").expect("review row");
    assert_eq!(review.sessions, 2, "review: 2 distinct sessions");
    assert!(
        (review.cost_usd - 0.08).abs() < 1e-6,
        "review cost = 0.05 + 0.03 = {:.6}",
        review.cost_usd
    );

    let product = rows
        .iter()
        .find(|r| r.key == "product")
        .expect("product row");
    assert_eq!(product.sessions, 1, "product: 1 session");
    assert!((product.cost_usd - 0.08).abs() < 1e-6);

    // Group by repo_id.
    let by_repo = engine
        .attribution(&AttributionDimension::Repo, 30)
        .await
        .expect("attribution by repo");
    assert_eq!(by_repo.len(), 2, "two distinct repos");
    let repo1 = by_repo.iter().find(|r| r.key == "repo-1").expect("repo-1");
    assert_eq!(repo1.sessions, 2);

    // Empty dim (no events without origin) should not appear.
    assert!(
        by_repo.iter().all(|r| !r.key.is_empty()),
        "empty keys must be filtered"
    );
    engine.shutdown().await; // stop the server (no orphan when the test exits)
}

/// Forecast returns a no-data response when the engine has no history for the
/// requested feature/provider (and must not panic or error).
#[tokio::test]
async fn forecast_no_history_returns_zero() {
    if ClickHouse::locate(None).is_none() {
        eprintln!("SKIP: no `clickhouse` binary found on this machine");
        return;
    }
    let _serial = serial().await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let engine = UsageEngine::start(
        UsageConfig {
            enabled: true,
            ..Default::default()
        },
        tmp.path().to_path_buf(),
    )
    .await;
    engine.wait_ready(Duration::from_secs(30)).await;
    assert!(engine.available());

    let resp = engine
        .forecast(&ForecastReq {
            feature: "review".to_string(),
            provider: "claude".to_string(),
            est_tokens: None,
        })
        .await;

    assert_eq!(resp.projected_cost_usd, 0.0);
    assert!(
        resp.basis.contains("no recent"),
        "basis explains no history: {}",
        resp.basis
    );
    engine.shutdown().await; // stop the server (no orphan when the test exits)
}

/// Forecast with an explicit token estimate prices it directly (no ClickHouse needed).
#[tokio::test]
async fn forecast_with_est_tokens_prices_directly() {
    let tmp = tempfile::tempdir().expect("tempdir");
    // Engine can be disabled — forecast with est_tokens doesn't need ClickHouse.
    let engine = UsageEngine::start(
        UsageConfig {
            enabled: false,
            ..Default::default()
        },
        tmp.path().to_path_buf(),
    )
    .await;

    let resp = engine
        .forecast(&ForecastReq {
            feature: "agent".to_string(),
            provider: "claude".to_string(),
            est_tokens: Some(2_000),
        })
        .await;

    // Must price 1000 in + 1000 out at claude rates (whatever that comes to;
    // we just check it's non-zero and the basis explains it).
    assert!(
        resp.projected_cost_usd > 0.0,
        "should produce a non-zero estimate"
    );
    assert!(
        resp.basis.contains("2000"),
        "basis must mention token count: {}",
        resp.basis
    );
}

/// Explicit `ts` on an event must date it at that time (not "now"), and the
/// dedup-rebuild purge must remove exactly the tailer-shaped rows (claude +
/// completion + no work dims) on/after the given date — nothing else.
#[tokio::test]
async fn explicit_ts_inserts_and_tailer_purge() {
    if ClickHouse::locate(None).is_none() {
        eprintln!("SKIP: no `clickhouse` binary found on this machine");
        return;
    }
    let _serial = serial().await;

    let tmp = tempfile::tempdir().expect("tempdir");
    let engine = UsageEngine::start(
        UsageConfig {
            enabled: true,
            retention_days: 180,
            metrics_interval_secs: 60,
            clickhouse_path: None,
        },
        tmp.path().to_path_buf(),
    )
    .await;
    engine.wait_ready(Duration::from_secs(30)).await;
    assert!(engine.available());

    let day = |back: i64| (chrono::Utc::now() - chrono::Duration::days(back)).date_naive();
    let ts = |back: i64| Some(format!("{}T10:00:00.000Z", day(back)));

    // Tailer-shaped (claude + completion + dim-less): purged.
    let mut tailer_old = event("claude", "s1", "claude-opus-4", "completion", 100, 10, 0.01);
    tailer_old.ts = ts(5);
    let tailer_now = event("claude", "s1", "claude-opus-4", "completion", 200, 20, 0.02);
    // Same shape but dim-carrying (e.g. `/ingest/usage` stamps origin): kept.
    let mut ingest = event("claude", "s2", "claude-opus-4", "completion", 400, 40, 0.04);
    ingest.ts = ts(5);
    ingest.origin = "ingest".to_string();
    // Other provider / other kind: kept.
    let mut codex = event("codex", "s3", "gpt-5-codex", "completion", 800, 80, 0.08);
    codex.ts = ts(5);
    let prompt = event("claude", "s1", "claude-opus-4", "prompt", 1600, 160, 0.16);

    engine
        .insert_events(&[tailer_old, tailer_now, ingest, codex, prompt])
        .await
        .expect("insert events");

    // The explicit-ts rows must land on their historical date.
    let daily = engine.daily_usage(30, false).await.expect("daily");
    let hist = daily
        .iter()
        .find(|d| d.day == day(5).to_string())
        .expect("a bucket on the backdated day");
    assert_eq!(
        hist.total_tokens,
        110 + 440 + 880,
        "backdated rows on that day"
    );

    // Purge from 10 days back: both tailer-shaped rows go, everything else stays.
    engine
        .purge_claude_tailer_rows(&day(10).to_string())
        .await
        .expect("purge");
    let summary = engine.summary(30, false).await.expect("summary");
    assert_eq!(summary.total_events, 3, "ingest + codex + prompt survive");
    assert_eq!(summary.total_tokens, 440 + 880 + 1760);

    // A malformed date must error out, not reach SQL.
    assert!(engine.purge_claude_tailer_rows("junk';--").await.is_err());

    engine.shutdown().await;
}

/// Seed `n` usage rows spread over the last 20 days / 500 sessions / 4 models
/// in one server-side INSERT…SELECT (fast, no client payload).
async fn seed_rows(engine: &UsageEngine, n: u64) {
    engine
        .exec_sql(&format!(
            "INSERT INTO usage_events (ts, workspace_id, session_id, provider, model, kind, \
             input_tokens, output_tokens, cache_read_tokens, cache_write_tokens, cost_usd, duration_ms) \
             SELECT now64(3) - toIntervalDay(number % 20), 'ws1', concat('s', toString(number % 500)), \
             if(number % 3 = 0, 'codex', 'claude'), concat('m', toString(number % 4)), 'completion', \
             number % 100, number % 50, number % 7, 0, (number % 7) / 1000.0, 0 FROM numbers({n})"
        ))
        .await
        .expect("seed rows");
}

fn test_config() -> UsageConfig {
    UsageConfig {
        enabled: true,
        retention_days: 180,
        metrics_interval_secs: 60,
        clickhouse_path: None,
    }
}

/// Perf guard (R3/R4/R6): a dashboard summary is at most TWO ClickHouse
/// statements (the grouped scan + the memoised per-session totals) and a
/// report at most two, each reading no more than the table once; the
/// by-kind rollup and budgets that follow a summary reuse its totals (zero
/// statements). The report's re-aggregated tables match the old per-table
/// queries.
#[tokio::test]
async fn summary_and_report_query_budget() {
    if ClickHouse::locate(None).is_none() {
        eprintln!("SKIP: no `clickhouse` binary found on this machine");
        return;
    }
    let _serial = serial().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let engine = UsageEngine::start(test_config(), tmp.path().to_path_buf()).await;
    assert!(engine.wait_ready(Duration::from_secs(30)).await);
    const N: u64 = 50_000;
    seed_rows(&engine, N).await;
    let ch = engine.clickhouse().expect("clickhouse handle");
    let rows_cap = N + N / 10; // ≤ 1.1 × the table per statement

    let (q0, r0) = ch.query_stats();
    let summary = engine.summary(30, false).await.expect("summary");
    let (q1, r1) = ch.query_stats();
    assert_eq!(summary.total_events, N);
    assert_eq!(summary.sessions.len(), 50, "top-50 from the folded totals");
    assert!(
        q1 - q0 <= 2,
        "summary ran {} statements (budget 2)",
        q1 - q0
    );
    assert!(r1 > r0, "X-ClickHouse-Summary read_rows must be reported");
    assert!(
        r1 - r0 <= 2 * rows_cap,
        "summary read {} rows for a {N}-row table",
        r1 - r0
    );
    // By-kind + budgets right after: the single-flight memo serves both.
    let (fa, fb) = tokio::join!(
        engine.feature_usage(30, false, |_| "agent".to_string()),
        engine.session_totals(30, false)
    );
    assert_eq!(fa.expect("by kind")[0].events, N);
    assert_eq!(fb.expect("totals").len(), 500);
    assert_eq!(ch.query_stats().0, q1, "memoised totals: no new statements");

    // The page's slim report: two statements, capped sessions, no day×model.
    let (q2, r2) = ch.query_stats();
    let slim = engine
        .report_with(
            30,
            false,
            UsageScope::All,
            ReportOptions {
                sessions_limit: 100,
                daily_models: false,
            },
        )
        .await
        .expect("slim report");
    let (q3, r3) = ch.query_stats();
    assert!(q3 - q2 <= 2, "report ran {} statements (budget 2)", q3 - q2);
    assert!(r3 - r2 <= 2 * rows_cap, "report read {} rows", r3 - r2);
    assert_eq!(slim.sessions.len(), 100);
    assert!(slim.daily_models.is_empty());

    // Full (export) report == the old per-table queries.
    let full = engine
        .report(30, false, UsageScope::All)
        .await
        .expect("report");
    assert_eq!(full.sessions.len(), 500);
    let daily = engine.daily_usage(30, false).await.expect("daily");
    assert_eq!(full.daily.len(), daily.len());
    for (a, b) in full.daily.iter().zip(&daily) {
        assert_eq!(
            (&a.day, a.events, a.total_tokens),
            (&b.day, b.events, b.total_tokens)
        );
        assert!((a.cost_usd - b.cost_usd).abs() < 1e-6, "{} cost", a.day);
    }
    assert_eq!(full.totals.total_tokens, summary.total_tokens);
    let months = engine
        .query_rows(
            "SELECT formatDateTime(event_date, '%Y-%m') AS month, count() AS events, \
             sum(input_tokens + output_tokens + cache_read_tokens + cache_write_tokens) AS t \
             FROM usage_events WHERE event_date >= today() - 29 GROUP BY month ORDER BY month",
        )
        .await
        .expect("months");
    assert_eq!(full.monthly.len(), months.len());
    for (m, row) in full.monthly.iter().zip(&months) {
        assert_eq!(m.month, row["month"].as_str().unwrap());
        assert_eq!(m.events, row["events"].as_u64().unwrap());
        assert_eq!(m.total_tokens, row["t"].as_u64().unwrap());
    }
    let models_tokens: u64 = full.models.iter().map(|m| m.total_tokens).sum();
    assert_eq!(models_tokens, summary.total_tokens);
    assert_eq!(full.models.len(), 8, "2 providers × 4 models");
    assert!(!full.daily_models.is_empty());
    engine.shutdown().await;
}

/// R1d: an idle server is stopped (no process, no threads) and transparently
/// restarted by the next query or insert, data intact; while parked it
/// counts as alive (no self-heal reinit).
#[tokio::test]
async fn idle_stop_parks_and_wakes_on_demand() {
    if ClickHouse::locate(None).is_none() {
        eprintln!("SKIP: no `clickhouse` binary found on this machine");
        return;
    }
    let _serial = serial().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let engine = UsageEngine::start(test_config(), tmp.path().to_path_buf()).await;
    assert!(engine.wait_ready(Duration::from_secs(30)).await);
    engine
        .insert_events(&[event(
            "claude",
            "s1",
            "claude-opus-4",
            "prompt",
            10,
            5,
            0.01,
        )])
        .await
        .expect("insert");
    let ch = engine.clickhouse().expect("clickhouse handle");
    let pid = ch.server_pid().expect("server pid");

    engine.set_idle_stop(Some(Duration::from_secs(1)));
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !ch.is_parked() {
        assert!(std::time::Instant::now() < deadline, "never idle-stopped");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    engine.set_idle_stop(None);
    assert!(ch.server_pid().is_none());
    assert!(ch.server_alive(), "parked is not dead (no heal)");
    // `parked` flips before the SIGTERM shutdown finishes: wait for the exit.
    let alive = || {
        std::process::Command::new("kill")
            .args(["-0", &pid.to_string()])
            .status()
            .is_ok_and(|s| s.success())
    };
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while alive() {
        assert!(std::time::Instant::now() < deadline, "server never exited");
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    // A read wakes it; data survived.
    let started = std::time::Instant::now();
    let s = engine.summary(30, false).await.expect("summary after wake");
    eprintln!("wake + summary: {} ms", started.elapsed().as_millis());
    assert_eq!(s.total_events, 1);
    assert!(!ch.is_parked());
    assert_eq!(ch.park_stats(), (1, 1));
    // An insert while running lands normally.
    engine
        .insert_events(&[event("codex", "s2", "gpt-5", "prompt", 1, 1, 0.0)])
        .await
        .expect("insert after wake");
    assert_eq!(engine.summary(30, false).await.unwrap().total_events, 2);
    engine.shutdown().await;
}

/// perf3 N1/G2: background metrics writes must not keep the server up. With
/// samples stored + flushed every 300 ms (a live session's cadence, sped up)
/// and no real query, the server still idle-stops; samples taken while it is
/// parked stay buffered without waking it, and the next real request wakes it
/// and writes them — every sample lands exactly once.
#[tokio::test]
async fn idle_stop_fires_under_a_metrics_cadence_and_keeps_every_sample() {
    if ClickHouse::locate(None).is_none() {
        eprintln!("SKIP: no `clickhouse` binary found on this machine");
        return;
    }
    let _serial = serial().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let engine = UsageEngine::start(test_config(), tmp.path().to_path_buf()).await;
    assert!(engine.wait_ready(Duration::from_secs(30)).await);
    let gen0 = engine.usage_generation();
    engine
        .insert_events(&[event(
            "claude",
            "s1",
            "claude-opus-4",
            "prompt",
            10,
            5,
            0.01,
        )])
        .await
        .expect("insert");
    assert_eq!(
        engine.usage_generation(),
        gen0 + 1,
        "usage write bumps the generation"
    );
    let ch = engine.clickhouse().expect("clickhouse handle");

    let mut stored: u32 = 0;
    let store = |n: u32| {
        let engine = std::sync::Arc::clone(&engine);
        async move {
            let m = otto_usage::Metric {
                mem_total_mb: 1.0,
                active_sessions: n,
                ..Default::default()
            };
            engine.store_metric(&m).await.expect("store metric");
            engine.flush_metrics().await.expect("background flush")
        }
    };

    engine.set_idle_stop(Some(Duration::from_secs(2)));
    let mut written = 0;
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !ch.is_parked() {
        assert!(
            std::time::Instant::now() < deadline,
            "metrics writes kept clickhouse awake ({stored} samples, {written} written)"
        );
        written += store(stored).await;
        stored += 1;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert!(written > 0, "samples were written while the server ran");

    // Parked: more samples buffer without waking it.
    for _ in 0..5 {
        assert_eq!(store(stored).await, 0, "nothing written while parked");
        stored += 1;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }
    assert!(ch.is_parked(), "a metrics write woke the server");
    assert_eq!(ch.park_stats(), (1, 0));
    engine.set_idle_stop(None);

    // A real request wakes it; the held samples follow right after.
    let s = engine.summary(30, false).await.expect("summary after wake");
    assert_eq!(s.total_events, 1);
    assert!(!ch.is_parked());
    assert_eq!(ch.park_stats(), (1, 1));
    assert_eq!(
        engine.usage_generation(),
        gen0 + 1,
        "a read is not a usage write"
    );
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let n = engine
            .query_rows("SELECT count() AS n FROM system_metrics")
            .await
            .expect("count")[0]["n"]
            .as_u64()
            .unwrap();
        if n == u64::from(stored) {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "held samples never written after wake ({n}/{stored})"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut seen: Vec<u32> = engine
        .metrics(5)
        .await
        .expect("metrics after wake")
        .iter()
        .map(|p| p.active_sessions)
        .collect();
    seen.sort_unstable();
    assert_eq!(
        seen,
        (0..stored).collect::<Vec<_>>(),
        "every sample exactly once"
    );
    engine.shutdown().await;
}

/// S9-02: usage events recorded while the server is idle-stopped are held
/// by the writer (a background write) instead of waking it — an always-on
/// agent no longer keeps ClickHouse up 24/7. The next foreground read wakes
/// it and the held events land exactly once.
#[tokio::test]
async fn usage_writes_do_not_wake_a_parked_server() {
    if ClickHouse::locate(None).is_none() {
        eprintln!("SKIP: no `clickhouse` binary found on this machine");
        return;
    }
    let _serial = serial().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    let engine = UsageEngine::start(test_config(), tmp.path().to_path_buf()).await;
    assert!(engine.wait_ready(Duration::from_secs(30)).await);
    let ch = engine.clickhouse().expect("clickhouse handle");

    engine.set_idle_stop(Some(Duration::from_secs(1)));
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while !ch.is_parked() {
        assert!(std::time::Instant::now() < deadline, "never idle-stopped");
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    engine.set_idle_stop(None);

    for i in 0..3 {
        engine.record(event("claude", "s1", "claude-opus-4", "prompt", i, 1, 0.0));
    }
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert!(ch.is_parked(), "a usage write woke the idle-stopped server");
    assert_eq!(ch.park_stats(), (1, 0));

    // A read wakes it and pokes the writer; the held rows follow.
    let deadline = std::time::Instant::now() + Duration::from_secs(15);
    loop {
        let n = engine
            .summary(30, false)
            .await
            .expect("summary")
            .total_events;
        if n == 3 {
            break;
        }
        assert!(n < 3, "held events written more than once ({n})");
        assert!(
            std::time::Instant::now() < deadline,
            "held usage events never written after wake ({n}/3)"
        );
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(ch.park_stats(), (1, 1));
    engine.shutdown().await;
}

/// `OTTO_PERF=1` budget (R6): the embedded server's idle thread count with
/// Otto's config. Measured 58 on macOS with the bundled build (70 before the
/// R1c pool cut); budget 62, override with `OTTO_PERF_CH_THREADS`. Prints
/// idle CPU over the window for the perf report.
#[tokio::test]
async fn budget_clickhouse_idle_threads() {
    if std::env::var("OTTO_PERF").ok().as_deref() != Some("1") {
        eprintln!("SKIP: set OTTO_PERF=1 to run the ClickHouse idle budget");
        return;
    }
    if ClickHouse::locate(None).is_none() {
        eprintln!("SKIP: no `clickhouse` binary found on this machine");
        return;
    }
    let _serial = serial().await;
    let tmp = tempfile::tempdir().expect("tempdir");
    // The real bring-up (schema, partitioning, writer) with idle-stop off.
    let engine = UsageEngine::start(test_config(), tmp.path().to_path_buf()).await;
    assert!(engine.wait_ready(Duration::from_secs(30)).await);
    engine.set_idle_stop(None);
    let ch = engine.clickhouse().expect("clickhouse handle");
    let pid = ch.server_pid().expect("pid");
    let cpu_secs = |pid: u32| -> f64 {
        let out = std::process::Command::new("ps")
            .args(["-o", "time=", "-p", &pid.to_string()])
            .output()
            .expect("ps");
        let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
        // [[dd-]hh:]mm:ss[.cc]
        t.rsplit(['-', ':'])
            .zip([1.0, 60.0, 3600.0, 86400.0])
            .map(|(v, m)| v.parse::<f64>().unwrap_or(0.0) * m)
            .sum()
    };
    tokio::time::sleep(Duration::from_secs(10)).await; // settle after boot
    let c0 = cpu_secs(pid);
    tokio::time::sleep(Duration::from_secs(20)).await;
    let c1 = cpu_secs(pid);
    let threads = if cfg!(target_os = "linux") {
        std::fs::read_dir(format!("/proc/{pid}/task"))
            .map(|d| d.count())
            .unwrap_or(0)
    } else {
        let out = std::process::Command::new("ps")
            .args(["-M", "-p", &pid.to_string()])
            .output()
            .expect("ps -M");
        String::from_utf8_lossy(&out.stdout)
            .lines()
            .count()
            .saturating_sub(1)
    };
    let budget: usize = std::env::var("OTTO_PERF_CH_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(62);
    eprintln!(
        "clickhouse idle: {threads} threads (budget {budget}), cpu {:.2}% over 20 s",
        (c1 - c0) / 20.0 * 100.0
    );
    assert!(threads > 0, "could not count server threads");
    assert!(
        threads <= budget,
        "{threads} idle threads > budget {budget}"
    );
    engine.shutdown().await;
}
