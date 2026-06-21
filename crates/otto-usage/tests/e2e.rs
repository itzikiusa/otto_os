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

use std::time::Duration;

use otto_usage::{
    AttributionDimension, ClickHouse, ForecastReq, MetricsSampler, UsageConfig, UsageEngine,
    UsageEvent,
};

fn event(provider: &str, session: &str, model: &str, kind: &str, inp: u64, out: u64, cost: f64) -> UsageEvent {
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

#[tokio::test]
async fn usage_engine_end_to_end() {
    if ClickHouse::locate(None).is_none() {
        eprintln!("SKIP: no `clickhouse` binary found on this machine");
        return;
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    let data_dir = tmp.path().to_path_buf();

    let config = UsageConfig {
        enabled: true,
        retention_days: 180,
        metrics_interval_secs: 60,
        clickhouse_path: None,
    };

    let engine = UsageEngine::start(config, data_dir.clone()).await;
    assert!(engine.available(), "engine should be available with a real clickhouse");

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
    engine.record(event("claude", "s2", "claude-sonnet-4", "completion", 0, 300, 0.01));
    // The background writer flushes on a ~2s timer; give it room.
    tokio::time::sleep(Duration::from_millis(2500)).await;

    // ── Provider rollup ─────────────────────────────────────────────────────
    let providers = engine.provider_usage(30, false).await.expect("provider usage");
    assert_eq!(providers.len(), 2, "two providers recorded");
    let claude = providers.iter().find(|p| p.provider == "claude").expect("claude row");
    // 3 claude events: (1000+500) + (800) + (400+200) + buffered (300) = 4 events total now.
    assert_eq!(claude.events, 4, "claude events (incl. buffered)");
    assert_eq!(claude.input_tokens, 1400, "claude input tokens (1000+0+400+0)");
    assert_eq!(claude.output_tokens, 1800, "claude output tokens (500+800+200+300)");
    assert_eq!(claude.total_tokens, 3200, "claude total tokens");
    assert!((claude.cost_usd - 0.07).abs() < 1e-9, "claude cost = 0.06 + 0.01");

    let codex = providers.iter().find(|p| p.provider == "codex").expect("codex row");
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

    let points = engine.metrics(60).await.expect("metrics query");
    assert_eq!(points.len(), 1, "one metric point stored");
    assert_eq!(points[0].active_sessions, 7);
    assert!(points[0].mem_total_mb > 0.0);

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
    assert!(engine2.available());
    let reopened = engine2.summary(30, false).await.expect("summary after reopen");
    assert_eq!(reopened.total_events, 6, "data persisted across restart");
    assert_eq!(reopened.total_tokens, 5300);
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
    engine.insert_events(&[event("claude", "s1", "m", "prompt", 1, 1, 0.0)]).await.unwrap();
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
    assert_eq!(back.pr_number, "");  // defaulted to empty on missing key
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

    let tmp = tempfile::tempdir().expect("tempdir");
    let engine = UsageEngine::start(
        UsageConfig {
            enabled: true,
            ..Default::default()
        },
        tmp.path().to_path_buf(),
    )
    .await;
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

    let product = rows.iter().find(|r| r.key == "product").expect("product row");
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
}

/// Forecast returns a no-data response when the engine has no history for the
/// requested feature/provider (and must not panic or error).
#[tokio::test]
async fn forecast_no_history_returns_zero() {
    if ClickHouse::locate(None).is_none() {
        eprintln!("SKIP: no `clickhouse` binary found on this machine");
        return;
    }

    let tmp = tempfile::tempdir().expect("tempdir");
    let engine = UsageEngine::start(
        UsageConfig {
            enabled: true,
            ..Default::default()
        },
        tmp.path().to_path_buf(),
    )
    .await;
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
    assert!(resp.projected_cost_usd > 0.0, "should produce a non-zero estimate");
    assert!(
        resp.basis.contains("2000"),
        "basis must mention token count: {}",
        resp.basis
    );
}
