use super::*;
use serde_json::json;

#[test]
fn config_requires_finite_bounded_thresholds() {
    let mut c = TelemetryConfig::default();
    assert!(c.validate().is_ok());
    c.slow_threshold_ms = f64::NAN;
    assert!(c.validate().is_err());
    c.slow_threshold_ms = 0.01;
    assert!(c.validate().is_err());
    c = TelemetryConfig::default();
    c.analysis_window_hours = 25;
    assert!(c.validate().is_err());
}
#[test]
fn span_rejects_content_and_invalid_numbers() {
    let mut s = SpanRecord::new("http.server", "server", 12.0);
    assert!(s.validate().is_ok());
    s.attributes.insert("prompt".into(), json!("secret"));
    assert!(s.validate().is_err());
    s.attributes.clear();
    s.duration_ms = f64::INFINITY;
    assert!(s.validate().is_err());
    s.duration_ms = 1.0;
    s.trace_id = "0".repeat(32);
    assert!(s.validate().is_err());
}
#[test]
fn nanoseconds_accept_decimal_string() {
    let mut value = serde_json::to_value(SpanRecord::new("http.server", "server", 1.0)).unwrap();
    value["start_unix_nano"] = json!(now_nanos().to_string());
    assert!(serde_json::from_value::<SpanRecord>(value)
        .unwrap()
        .validate()
        .is_ok());
}
#[test]
fn ranking_excludes_fast_high_volume_operations() {
    let fast = OperationSummary {
        count: 1_000_000,
        p95_ms: 0.05,
        max_ms: 0.09,
        total_ms: 50000.0,
        ..Default::default()
    };
    let slow = OperationSummary {
        component: "database".into(),
        name: "http.server".into(),
        count: 10,
        p95_ms: 120.0,
        max_ms: 300.0,
        total_ms: 1800.0,
        ..Default::default()
    };
    let found = analysis::rank(&[fast, slow], &TelemetryConfig::default(), 1);
    assert_eq!(found.len(), 1);
    assert_eq!(found[0].component, "database");
}
#[test]
fn otlp_is_standard_json_and_does_not_export_error_bodies() {
    let mut s = SpanRecord::new("http.server", "server", 12.5);
    s.status = "error".into();
    let value = otlp::traces(&[s.clone()]);
    assert_eq!(
        value["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["startTimeUnixNano"],
        s.start_unix_nano.to_string()
    );
    assert_eq!(
        value["resourceSpans"][0]["scopeSpans"][0]["spans"][0]["status"]["code"],
        2
    );
    assert_eq!(
        otlp::errors(&[s])["resourceLogs"][0]["scopeLogs"][0]["logRecords"][0]["body"]
            ["stringValue"],
        "operation.error"
    );
}
#[test]
fn profile_redacts_paths_addresses_and_headers() {
    let text="Process: ottod [123]\nPath: /Users/private/otto\n    + 9 tokio::runtime::park (in ottod) + 10 [0xABC]\nBinary Images:\n /private/secrets";
    let frames = resource::sanitize_profile(text);
    assert_eq!(frames, vec!["    + 9 tokio::runtime::park"]);
    let weighted =
        resource::sanitize_profile("100 hot (in ottod)\n  + 1 cold (in ottod)\n1 hot (in ottod)");
    assert_eq!(weighted, vec!["100 hot", "  + 1 cold", "1 hot"]);
}

#[tokio::test]
async fn consent_disable_clears_bounded_queue_without_starting_collector() {
    let temp = tempfile::tempdir().unwrap();
    let usage = UsageEngine::start(
        otto_usage::UsageConfig {
            enabled: false,
            ..Default::default()
        },
        temp.path().join("usage"),
    )
    .await;
    let service = TelemetryService::start(
        usage.clone(),
        temp.path().join("telemetry"),
        TelemetryConfig::default(),
    )
    .await;
    tokio::task::yield_now().await;
    assert!(!service.record(SpanRecord::new("http.server", "server", 12.0)));
    assert!(!temp.path().join("telemetry").exists());
    // Enable only the ingestion gate to isolate the queue test from downloads.
    service.enabled.store(true, Ordering::SeqCst);
    for _ in 0..QUEUE_LIMIT {
        assert!(service.record(SpanRecord::new("http.server", "server", 12.0)));
    }
    assert!(!service.record(SpanRecord::new("http.server", "server", 12.0)));
    assert_eq!(service.status().dropped, 1);
    service.configure(TelemetryConfig::default()).await.unwrap();
    assert_eq!(service.status().queued, 0);
    assert!(!service.status().collector_ready);
    service.shutdown().await;
    usage.shutdown().await;
}

#[tokio::test]
async fn dismissed_suggestions_and_daily_schedule_survive_restart() {
    let temp = tempfile::tempdir().unwrap();
    let usage = UsageEngine::start(
        otto_usage::UsageConfig {
            enabled: false,
            ..Default::default()
        },
        temp.path().join("usage"),
    )
    .await;
    let service = TelemetryService::start(
        usage.clone(),
        temp.path().join("telemetry"),
        TelemetryConfig::default(),
    )
    .await;
    let op = OperationSummary {
        name: "http.server".into(),
        component: "server".into(),
        count: 10,
        p95_ms: 200.0,
        total_ms: 3000.0,
        ..Default::default()
    };
    let suggestions = analysis::rank(&[op], &TelemetryConfig::default(), 42);
    let id = suggestions[0].id.clone();
    {
        let mut state = service.persisted.write().unwrap();
        state.suggestions = suggestions;
        state.last_analysis_at = Some(42);
    }
    service.dismiss(&id).await.unwrap();
    // An operation disappearing from one analysis must not forget dismissal.
    service.persisted.write().unwrap().suggestions.clear();
    service.save().await.unwrap();
    service.shutdown().await;
    let restarted = TelemetryService::start(
        usage.clone(),
        temp.path().join("telemetry"),
        TelemetryConfig::default(),
    )
    .await;
    assert!(restarted.suggestions().is_empty());
    assert!(restarted
        .persisted
        .read()
        .unwrap()
        .dismissed_ids
        .contains(&id));
    assert_eq!(restarted.status().last_analysis_at, Some(42));
    restarted.shutdown().await;
    usage.shutdown().await;
}

#[tokio::test]
#[ignore = "downloads pinned collector and starts an isolated ClickHouse; run serially"]
async fn real_collector_delivers_all_signals_and_recovers() {
    let temp = tempfile::tempdir().unwrap();
    let usage = UsageEngine::start(
        otto_usage::UsageConfig::default(),
        temp.path().join("usage"),
    )
    .await;
    assert!(
        usage.wait_ready(Duration::from_secs(30)).await,
        "isolated ClickHouse must become ready"
    );
    let c = TelemetryConfig {
        enabled: true,
        sample_interval_secs: 2,
        ..Default::default()
    };
    let service =
        TelemetryService::start(usage.clone(), temp.path().join("telemetry"), c.clone()).await;
    let start = Instant::now();
    while !service.status().collector_ready && start.elapsed() < Duration::from_secs(360) {
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert!(service.status().collector_ready, "{:?}", service.status());
    let id = uuid::Uuid::new_v4().simple().to_string();
    for _ in 0..4 {
        let mut span = SpanRecord::new("http.server", "server", 250.0);
        span.trace_id = id.clone();
        span.status = "error".into();
        assert!(service.record(span));
    }
    let start = Instant::now();
    loop {
        let rows = usage
            .query_rows("SELECT count() n FROM otto_telemetry.otel_traces")
            .await
            .unwrap();
        if number(&rows[0], "n") >= 4.0 {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "traces not delivered"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    tokio::time::sleep(Duration::from_secs(4)).await;
    for table in [
        "otel_logs",
        "otel_metrics_gauge",
        "operation_minutes",
        "resource_minutes",
    ] {
        let rows = usage
            .query_rows(&format!("SELECT count() n FROM otto_telemetry.{table}"))
            .await
            .unwrap();
        assert!(
            number(&rows[0], "n") > 0.0,
            "empty signal or rollup: {table}"
        );
    }
    let trace = service.trace(&id).await.unwrap();
    assert_eq!(trace.len(), 4);
    let overview = service.overview(24).await.unwrap();
    assert_eq!(overview.operations[0].count, 4);
    assert_eq!(overview.operations[0].errors, 4);
    assert!(!overview.resources.is_empty());
    let recommendations = service.analyze().await.unwrap();
    assert_eq!(recommendations.len(), 1);
    assert_eq!(recommendations[0].count, 4);
    let recommendation_id = recommendations[0].id.clone();
    service.dismiss(&recommendation_id).await.unwrap();
    assert!(service.analyze().await.unwrap().is_empty());
    let ch = usage.clickhouse().unwrap();
    assert!(
        !ch.maybe_park(Duration::ZERO).await,
        "telemetry lease must prevent parking"
    );
    let old_pid = service
        .runtime
        .lock()
        .await
        .collector
        .as_ref()
        .unwrap()
        .pid();
    service
        .runtime
        .lock()
        .await
        .collector
        .as_mut()
        .unwrap()
        .stop()
        .await;
    let start = Instant::now();
    loop {
        let ready = {
            let runtime = service.runtime.lock().await;
            runtime
                .collector
                .as_ref()
                .is_some_and(|c| c.pid().is_some() && c.pid() != old_pid)
        };
        if ready {
            break;
        }
        assert!(start.elapsed() < Duration::from_secs(40));
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    // Replacing the shared engine changes its HTTP endpoint. The exporter must
    // acquire the replacement lease and deliver to the recovered engine.
    let old_endpoint = service
        .runtime
        .lock()
        .await
        .lease
        .as_ref()
        .unwrap()
        .endpoint
        .clone();
    ch.shutdown().await;
    let start = Instant::now();
    loop {
        let recovered = {
            let runtime = service.runtime.lock().await;
            runtime
                .lease
                .as_ref()
                .is_some_and(|lease| lease.endpoint != old_endpoint)
                && service.status().collector_ready
        };
        if recovered {
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(65),
            "ClickHouse replacement did not recover telemetry"
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    assert!(service.record(SpanRecord::new("after.recovery", "server", 120.0)));
    tokio::time::sleep(Duration::from_secs(4)).await;
    assert!(service
        .overview(24)
        .await
        .unwrap()
        .operations
        .iter()
        .any(|op| op.name == "after.recovery"));
    #[cfg(target_os = "macos")]
    {
        let mut profiling = c.clone();
        profiling.native_profiling = true;
        service.configure(profiling).await.unwrap();
        let profile = service.profile().await.unwrap();
        assert!(!profile.frames.is_empty());
        assert!(profile
            .frames
            .iter()
            .all(|frame| !frame.contains('/') && !frame.contains("0x")));
        assert_eq!(
            service.latest_profile().unwrap().captured_at,
            profile.captured_at
        );
        assert!(
            service.profile().await.is_err(),
            "profile cooldown must be enforced"
        );
    }
    let mut new_config = c.clone();
    new_config.traces_days = 2;
    new_config.logs_days = 3;
    new_config.metrics_days = 8;
    service.configure(new_config).await.unwrap();
    tokio::time::sleep(Duration::from_secs(6)).await;
    let rows=usage.query_rows("SELECT name,create_table_query FROM system.tables WHERE database='otto_telemetry' AND name IN ('otel_traces','otel_logs','otel_metrics_gauge','operation_minutes','resource_minutes')").await.unwrap();
    for row in rows {
        let expected = match string(&row, "name").as_str() {
            "otel_traces" | "operation_minutes" => "toIntervalDay(2)",
            "otel_logs" => "toIntervalDay(3)",
            _ => "toIntervalDay(8)",
        };
        assert!(
            string(&row, "create_table_query").contains(expected),
            "TTL mismatch: {row}"
        );
    }
    service.configure(TelemetryConfig::default()).await.unwrap();
    assert!(!service.status().collector_ready);
    assert!(
        usage.clickhouse().unwrap().maybe_park(Duration::ZERO).await,
        "disable must release lease"
    );
    service.shutdown().await;
    usage.shutdown().await;
    eprintln!("Verified OTLP traces/logs/gauges, minute MVs, merged latency quantiles, trace drilldown, persisted dismissal, collector recovery, TTL updates and parking lease in {}",temp.path().display());
}

#[test]
fn spikes_require_sustained_cpu_or_an_abrupt_memory_increase() {
    let mut detector = resource::SpikeDetector::default();
    let config = TelemetryConfig::default();
    let mut point = ResourcePoint {
        timestamp: 100,
        process: "daemon".into(),
        cpu_percent: Some(90.0),
        rss_mb: Some(100.0),
        host_load: None,
    };
    assert!(!detector.observe(&point, &config));
    point.timestamp = 105;
    assert!(detector.observe(&point, &config));
    point.timestamp = 110;
    assert!(!detector.observe(&point, &config));
    point.timestamp = 200;
    point.cpu_percent = Some(0.0);
    point.rss_mb = Some(400.0);
    assert!(detector.observe(&point, &config));
}
#[test]
fn slow_query_filters_before_limiting() {
    let sql = schema::slow_operations(&TelemetryConfig::default());
    assert!(
        sql.find("HAVING count >= 3 AND p95_ms >= 100").unwrap() < sql.find("LIMIT 100").unwrap()
    );
    assert!(!sql.contains("LIMIT 1000"));
}
#[tokio::test]
async fn enable_persists_a_stable_first_analysis_deadline() {
    let temp = tempfile::tempdir().unwrap();
    let usage = UsageEngine::start(
        otto_usage::UsageConfig {
            enabled: false,
            ..Default::default()
        },
        temp.path().join("usage"),
    )
    .await;
    let service = TelemetryService::start(
        usage.clone(),
        temp.path().to_path_buf(),
        TelemetryConfig::default(),
    )
    .await;
    assert!(service.overview(24).await.unwrap().operations.is_empty());
    let config = TelemetryConfig {
        enabled: true,
        ..Default::default()
    };
    tokio::time::timeout(Duration::from_secs(2), service.configure(config.clone()))
        .await
        .unwrap()
        .unwrap();
    let deadline = service.status().next_analysis_at.unwrap();
    service.shutdown().await;
    let restarted = TelemetryService::start(usage.clone(), temp.path().to_path_buf(), config).await;
    assert_eq!(restarted.status().next_analysis_at, Some(deadline));
    restarted.shutdown().await;
    usage.shutdown().await;
}

#[tokio::test]
#[cfg(target_os = "macos")]
async fn opt_out_during_profile_setup_cannot_launch_sample() {
    let temp = tempfile::tempdir().unwrap();
    let usage = UsageEngine::start(
        otto_usage::UsageConfig {
            enabled: false,
            ..Default::default()
        },
        temp.path().into(),
    )
    .await;
    let service = TelemetryService::start(
        usage.clone(),
        temp.path().into(),
        TelemetryConfig::default(),
    )
    .await;
    // Isolate the profile gate from collector installation.
    service.enabled.store(true, Ordering::SeqCst);
    service.config.write().unwrap().native_profiling = true;
    let arrived = Arc::new(Notify::new());
    let resume = Arc::new(Notify::new());
    *service.profile_setup_hook.lock().unwrap() = Some((arrived.clone(), resume.clone()));
    let worker = service.clone();
    let capture = tokio::spawn(async move { worker.profile().await });
    arrived.notified().await;
    service.configure(TelemetryConfig::default()).await.unwrap();
    resume.notify_one();
    let result = tokio::time::timeout(Duration::from_secs(1), capture)
        .await
        .expect("must reject before the three-second native sampler starts")
        .unwrap();
    assert!(result
        .unwrap_err()
        .to_string()
        .contains("disabled during setup"));
    assert!(service.latest_profile().is_none());
    service.shutdown().await;
    usage.shutdown().await;
}

#[tokio::test]
async fn canceled_pending_export_is_accounted_without_claiming_acceptance() {
    let discarded = Arc::new(AtomicU64::new(0));
    let count = discarded.clone();
    let task = tokio::spawn(async move {
        let _pending = PendingExport {
            discarded: &count,
            count: 17,
            accepted: false,
        };
        std::future::pending::<()>().await;
    });
    tokio::task::yield_now().await;
    task.abort();
    let _ = task.await;
    assert_eq!(discarded.load(Ordering::Relaxed), 17);
    {
        let _ack = PendingExport {
            discarded: &discarded,
            count: 8,
            accepted: true,
        };
    }
    assert_eq!(discarded.load(Ordering::Relaxed), 17);
}
