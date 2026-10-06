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
    let found = analysis::rank(
        &[fast, slow],
        &Default::default(),
        &TelemetryConfig::default(),
        1,
    );
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
    let suggestions = analysis::rank(&[op], &Default::default(), &TelemetryConfig::default(), 42);
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
    service.request_flush();
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
    // Between flushes telemetry holds no lease: the engine may idle-stop.
    let ch = usage.clickhouse().unwrap();
    let start = Instant::now();
    while !ch.maybe_park(Duration::ZERO).await {
        assert!(
            start.elapsed() < Duration::from_secs(40),
            "telemetry must release ClickHouse between flushes"
        );
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    let delivered = |name: &'static str| {
        let usage = usage.clone();
        async move {
            usage
                .query_rows(&format!(
                    "SELECT count() n FROM otto_telemetry.otel_traces WHERE SpanName='{name}'"
                ))
                .await
                .is_ok_and(|rows| number(&rows[0], "n") > 0.0)
        }
    };
    // Sampling a parked engine never wakes it; a requested flush does, once.
    assert!(service.record(SpanRecord::new("after.park", "server", 120.0)));
    tokio::time::sleep(Duration::from_secs(3)).await;
    assert!(ch.is_parked(), "buffering must not wake ClickHouse");
    service.request_flush();
    let start = Instant::now();
    while !(service.status().last_error.is_none() && delivered("after.park").await) {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "{:?}",
            service.status()
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    // Replacing the shared engine changes its HTTP endpoint. The next flush
    // must lease the replacement and deliver to the recovered engine.
    ch.shutdown().await;
    assert!(service.record(SpanRecord::new("after.recovery", "server", 120.0)));
    service.request_flush();
    let start = Instant::now();
    while !delivered("after.recovery").await {
        assert!(
            start.elapsed() < Duration::from_secs(120),
            "ClickHouse replacement did not recover telemetry: {:?}",
            service.status()
        );
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
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
    // Retention DDL runs with the next flush (sampling keeps data buffered).
    service.request_flush();
    tokio::time::sleep(Duration::from_secs(30)).await;
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
        sql.find("HAVING count >= 3 AND p95_ms >= 100").unwrap() < sql.find("LIMIT 500").unwrap()
    );
    assert!(!sql.contains("LIMIT 1000"));
}
#[test]
fn ranking_uses_self_time_and_collapses_shared_traces() {
    let op = |name: &str, total: f64, trace: &str| OperationSummary {
        component: "git".into(),
        name: name.into(),
        count: 10,
        p95_ms: 500.0,
        max_ms: 900.0,
        total_ms: total,
        trace_id: trace.into(),
        ..Default::default()
    };
    // The handler only waits on fetch (same slowest trace); the status call is
    // an independent, smaller cost with its own trace.
    let handler = op("http.post.repos.fetch", 9000.0, "a");
    let fetch = op("git.fetch", 8800.0, "a");
    let status = op("git.status", 3000.0, "b");
    let self_ms: std::collections::HashMap<_, _> = [
        (("git".into(), "http.post.repos.fetch".into()), 200.0),
        (("git".into(), "git.fetch".into()), 8800.0),
        (("git".into(), "git.status".into()), 3000.0),
    ]
    .into();
    let found = analysis::rank(
        &[handler, fetch, status],
        &self_ms,
        &TelemetryConfig::default(),
        1,
    );
    let names: Vec<_> = found.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["git.fetch", "git.status"]);
    assert_eq!(found[0].self_ms, Some(8800.0));
    // Without self times, inclusive ranking still collapses the shared trace.
    let fallback = analysis::rank(
        &[op("parent", 9000.0, "a"), op("child", 8800.0, "a")],
        &Default::default(),
        &TelemetryConfig::default(),
        1,
    );
    assert_eq!(fallback.len(), 1);
    assert_eq!(fallback[0].name, "parent");
}
#[test]
fn trace_drilldown_is_bounded_by_the_trace_id_lookup() {
    let id = "0123456789abcdef0123456789abcdef";
    let sql = schema::trace(id, 1);
    assert!(sql.contains("FROM otto_telemetry.otel_traces_trace_id_ts WHERE TraceId="));
    assert!(sql.contains("Timestamp>=(SELECT min(Start)"));
    assert!(sql.contains("Timestamp<=(SELECT max(End)+1"));
}
#[test]
fn flush_waits_for_the_interval_and_wakes_only_when_urgent() {
    let minute = Duration::from_secs(60);
    // Nothing buffered: never flush.
    assert!(!flush_decision(false, false, None, 0).0);
    // First flush after enabling validates the pipeline and may wake.
    assert_eq!(flush_decision(true, false, None, 1), (true, true));
    // Within the interval: keep buffering.
    assert!(!flush_decision(true, false, Some(minute), 10).0);
    // Interval elapsed: flush, but do not wake an idle-stopped engine.
    assert_eq!(
        flush_decision(true, false, Some(6 * minute), 10),
        (true, false)
    );
    // Deferred too long, or queue pressure, or an explicit request: wake.
    assert_eq!(
        flush_decision(true, false, Some(31 * minute), 10),
        (true, true)
    );
    assert_eq!(
        flush_decision(true, false, Some(minute), QUEUE_LIMIT * 3 / 4),
        (true, true)
    );
    assert_eq!(flush_decision(true, true, Some(minute), 1), (true, true));
}
#[test]
fn resource_samples_collapse_to_bounded_per_minute_maxima() {
    let dropped = AtomicU64::new(0);
    let mut buffer = BTreeMap::new();
    let point = |t: i64, cpu: f64, rss: Option<f64>| ResourcePoint {
        timestamp: t,
        process: "daemon".into(),
        cpu_percent: Some(cpu),
        rss_mb: rss,
        host_load: None,
    };
    merge_point(&mut buffer, &point(120, 5.0, Some(100.0)), &dropped);
    merge_point(&mut buffer, &point(150, 40.0, None), &dropped);
    merge_point(&mut buffer, &point(179, 10.0, Some(90.0)), &dropped);
    merge_point(&mut buffer, &point(180, 1.0, Some(1.0)), &dropped);
    assert_eq!(buffer.len(), 2);
    let first = &buffer[&(120, "daemon".to_string())];
    assert_eq!(first.cpu_percent, Some(40.0));
    assert_eq!(first.rss_mb, Some(100.0));
    for minute in 0..(POINT_LIMIT as i64 + 5) {
        merge_point(
            &mut buffer,
            &point(10_000 + minute * 60, 1.0, None),
            &dropped,
        );
    }
    assert_eq!(buffer.len(), POINT_LIMIT);
    assert_eq!(dropped.load(Ordering::Relaxed), 7);
    assert!(
        !buffer.contains_key(&(120, "daemon".to_string())),
        "oldest evicted first"
    );
}
#[test]
fn collector_exporter_stats_parse_failures_and_queue_depth() {
    let text = "# HELP otelcol_exporter_queue_size Current size\n\
otelcol_exporter_queue_size{exporter=\"clickhouse/traces\",data_type=\"traces\"} 12\n\
otelcol_exporter_queue_size{exporter=\"clickhouse/logs\",data_type=\"logs\"} 3\n\
otelcol_exporter_send_failed_spans_total{exporter=\"clickhouse/traces\"} 7\n\
otelcol_exporter_send_failed_log_records_total{exporter=\"clickhouse/logs\"} 1\n\
otelcol_exporter_sent_spans_total{exporter=\"clickhouse/traces\"} 900\n";
    assert_eq!(
        collector::parse_exporter_stats(text),
        Some(collector::ExporterStats {
            send_failed: 8,
            queue_size: 15
        })
    );
    assert_eq!(collector::parse_exporter_stats("# nothing yet\n"), None);
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
    let started = service
        .persisted
        .read()
        .unwrap()
        .schedule_started_at
        .unwrap();
    assert_eq!(
        deadline - started,
        3600,
        "first analysis runs an hour after enabling, not a day"
    );
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

/// S9-03: a flush is only confirmed when the exporter counters were read and
/// every queue drained. Records still queued at the deadline (a slow / waking
/// ClickHouse) or unreadable counters are a failure: nothing counts as
/// exported and status carries an error.
#[test]
fn drain_settlement_never_claims_unconfirmed_exports() {
    use collector::ExporterStats;
    let stalled = settle(
        Some(ExporterStats {
            send_failed: 0,
            queue_size: 40,
        }),
        40,
    );
    assert_eq!(
        stalled.exported, 0,
        "queued records were counted as exported"
    );
    assert_eq!(stalled.failed, 40);
    assert!(stalled.error.as_deref().is_some_and(|e| e.contains("40")));

    let unknown = settle(None, 12);
    assert_eq!((unknown.exported, unknown.failed), (0, 12));
    assert!(
        unknown.error.is_some(),
        "missing counters must not read as success"
    );

    let partial = settle(
        Some(ExporterStats {
            send_failed: 3,
            queue_size: 0,
        }),
        10,
    );
    assert_eq!((partial.exported, partial.failed), (7, 3));
    assert!(partial.error.is_some());

    let clean = settle(Some(ExporterStats::default()), 10);
    assert_eq!(
        clean,
        Settlement {
            exported: 10,
            failed: 0,
            error: None
        }
    );
}

/// S9-05: cancelled spans are filed under their own operation name (so they
/// neither feed the real operation's quantiles nor vanish) and keep their
/// status through the drill-down; `Unset` is not shown as "ok".
#[test]
fn cancelled_and_unset_spans_stay_distinguishable() {
    let mut s = SpanRecord::new("GET /api/v1/x", "server", 900.0);
    s.status = "cancelled".into();
    let span = &otlp::traces(&[s])["resourceSpans"][0]["scopeSpans"][0]["spans"][0];
    assert_eq!(span["name"], "GET /api/v1/x [cancelled]");
    assert_eq!(span["status"]["code"], 0);
    assert_eq!(span["status"]["message"], "cancelled");
    assert!(span["attributes"]
        .as_array()
        .unwrap()
        .iter()
        .any(|a| a["key"] == "otto.status" && a["value"]["stringValue"] == "cancelled"));
    assert_eq!(span_status("Unset", Some("cancelled")), "cancelled");
    assert_eq!(span_status("Unset", None), "unset");
    assert_eq!(span_status("Error", None), "error");
    assert_eq!(span_status("Ok", None), "ok");
}

/// S9-08: resource points taken for an export that is cancelled (dropped
/// mid-send by a config change) go back into the buffer instead of vanishing.
#[tokio::test]
async fn cancelled_export_requeues_unsent_points() {
    let buffer = Arc::new(Mutex::new(BTreeMap::new()));
    let dropped = Arc::new(AtomicU64::new(0));
    let point = |t: i64| ResourcePoint {
        timestamp: t,
        process: "daemon".into(),
        cpu_percent: Some(1.0),
        rss_mb: None,
        host_load: None,
    };
    let (b, d) = (buffer.clone(), dropped.clone());
    let task = tokio::spawn(async move {
        let mut pending = PendingPoints {
            buffer: &b,
            dropped: &d,
            points: vec![point(60), point(120), point(180)],
            sent: 0,
        };
        pending.sent = 1; // first chunk acknowledged
        std::future::pending::<()>().await;
    });
    tokio::task::yield_now().await;
    task.abort();
    let _ = task.await;
    let keys: Vec<i64> = buffer.lock().unwrap().keys().map(|k| k.0).collect();
    assert_eq!(keys, vec![120, 180], "unsent points were lost on cancel");
    assert_eq!(dropped.load(Ordering::Relaxed), 0);
}

/// S9-04: the collector binary's verified digest is reused only for the
/// exact same file identity; any rewrite (new ctime/size) re-hashes.
#[tokio::test]
async fn verified_digest_cache_is_keyed_by_file_identity() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("bin");
    std::fs::write(&path, b"one").unwrap();
    let key = collector::file_key(&path).await;
    assert!(key.is_some());
    collector::remember_digest(key, "d1".into());
    assert_eq!(collector::verified_digest(key).as_deref(), Some("d1"));
    std::thread::sleep(Duration::from_millis(20));
    std::fs::write(&path, b"other").unwrap();
    let changed = collector::file_key(&path).await;
    assert_ne!(changed, key);
    assert_eq!(collector::verified_digest(changed), None);
    assert_eq!(collector::verified_digest(None), None);
}
