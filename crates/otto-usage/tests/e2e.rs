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

use otto_usage::{ClickHouse, MetricsSampler, UsageConfig, UsageEngine, UsageEvent};

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
