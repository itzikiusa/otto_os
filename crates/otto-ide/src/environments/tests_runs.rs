//! Unit & SQLite tests for scoped run configs, remote SSH runner commands,
//! process-group cleanup, and real-time Run SSE event streaming.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use otto_core::domain::User;
use tokio::process::Command;
use tokio::sync::{broadcast, watch};

use crate::target::TargetManager;
use crate::types::{IdeRunConfig, IdeRunStatus, IdeRunnerKind, IdeTargetKind, IdeTargetSpec};

use super::runner::{
    append_run_line, build_runner_command, evict_finished_runs, execute_run_task, finish_run,
    ActiveRunHandle, RunStateInner, MAX_FINISHED_RUNS, MAX_RUN_OUTPUT_BYTES, MAX_RUN_OUTPUT_LINES,
};
use super::EnvironmentManager;

fn sample_user(id: &str, is_root: bool) -> User {
    User {
        id: id.into(),
        username: id.into(),
        display_name: id.into(),
        is_root,
        disabled: false,
        created_at: chrono::Utc::now(),
    }
}

#[test]
fn finished_runs_evict_cleanly_past_max_finished_runs_preserving_running() {
    let mgr = EnvironmentManager::new(PathBuf::from("/tmp/otto-ide-test"));
    let total_finished = (MAX_FINISHED_RUNS + 8) as u64;
    for i in 0..total_finished {
        let (stop_tx, _) = watch::channel(false);
        let (events_tx, _) = broadcast::channel(16);
        let id = format!("run_fin_{i:02}");
        let handle = Arc::new(ActiveRunHandle {
            seq: i,
            owner_user_id: String::new(),
            state: Mutex::new(RunStateInner::new(
                id.clone(),
                "Finished".into(),
                IdeRunnerKind::Custom,
                format!("2026-01-01T00:00:{i:02}Z"),
            )),
            stop_tx,
            events_tx,
        });
        finish_run(
            &handle,
            IdeRunStatus::Passed,
            Some(0),
            Duration::from_millis(5),
            None,
        );
        mgr.runs.insert(id, handle);
        evict_finished_runs(&mgr.runs);
    }

    let (stop_tx, _) = watch::channel(false);
    let (events_tx, _) = broadcast::channel(16);
    mgr.runs.insert(
        "run_active".into(),
        Arc::new(ActiveRunHandle {
            seq: 100,
            owner_user_id: String::new(),
            state: Mutex::new(RunStateInner::new(
                "run_active".into(),
                "Active".into(),
                IdeRunnerKind::Custom,
                "2026-01-01T00:00:00Z".into(),
            )),
            stop_tx,
            events_tx,
        }),
    );
    evict_finished_runs(&mgr.runs);

    assert_eq!(mgr.runs.len(), MAX_FINISHED_RUNS + 1);
    assert!(mgr.get_run("run_active").is_ok());
    for i in 0..8 {
        assert!(mgr.get_run(&format!("run_fin_{i:02}")).is_err());
    }
    for i in 8..total_finished {
        assert!(mgr.get_run(&format!("run_fin_{i:02}")).is_ok());
    }
}

#[test]
fn streaming_10k_lines_ssh_runner_commands_and_maven_flags_work() {
    let (stop_tx, _stop_rx) = watch::channel(false);
    let (events_tx, _) = broadcast::channel(16);
    let handle = ActiveRunHandle {
        seq: 1,
        owner_user_id: String::new(),
        state: Mutex::new(RunStateInner::new(
            "run_perf".into(),
            "CargoTest".into(),
            IdeRunnerKind::CargoTest,
            "2026-01-01T00:00:00Z".into(),
        )),
        stop_tx,
        events_tx,
    };

    let started = Instant::now();
    for i in 0..10_000 {
        append_run_line(&handle, format!("stdout log line {i}"));
    }
    assert!(started.elapsed() < Duration::from_millis(500));
    assert_eq!(handle.snapshot().output.len(), MAX_RUN_OUTPUT_LINES);

    for _ in 0..200 {
        append_run_line(&handle, "x".repeat(8192));
    }
    assert!(handle.state.lock().unwrap().output_bytes() <= MAX_RUN_OUTPUT_BYTES);

    let mvn_cfg = IdeRunConfig {
        runner: IdeRunnerKind::Maven,
        args: vec!["spring-boot:run".into()],
        maven_profiles: vec!["dev".into(), "aws".into()],
        spring_profiles: vec!["local".into()],
        skip_tests: true,
        offline: true,
        ..Default::default()
    };
    let (prog, args) =
        build_runner_command(&mvn_cfg, IdeTargetKind::Local, "/tmp", &BTreeMap::new());
    assert_eq!(prog, "mvn");
    assert!(args.contains(&"-Pdev,aws".to_string()));
    assert!(args.contains(&"-Dspring.profiles.active=local".to_string()));
    assert!(args.contains(&"-Dspring-boot.run.profiles=local".to_string()));
    assert!(args.contains(&"-DskipTests".to_string()));
    assert!(args.contains(&"-o".to_string()));

    // Remote SSH Maven/Gradle/Pytest must never probe the local host filesystem.
    let (ssh_mvn_prog, ssh_mvn_args) = build_runner_command(
        &mvn_cfg,
        IdeTargetKind::Ssh,
        "/remote/app",
        &BTreeMap::new(),
    );
    assert_eq!(ssh_mvn_prog, "sh");
    assert!(ssh_mvn_args[1].contains("./mvnw"));
    assert!(ssh_mvn_args.contains(&"-Pdev,aws".to_string()));

    let gradle_cfg = IdeRunConfig {
        runner: IdeRunnerKind::Gradle,
        test_filter: Some("OrderServiceTest".into()),
        ..Default::default()
    };
    let (ssh_gr_prog, ssh_gr_args) = build_runner_command(
        &gradle_cfg,
        IdeTargetKind::Ssh,
        "/remote/app",
        &BTreeMap::new(),
    );
    assert_eq!(ssh_gr_prog, "sh");
    assert!(ssh_gr_args[1].contains("./gradlew"));
    assert!(ssh_gr_args.contains(&"OrderServiceTest".to_string()));

    let pytest_cfg = IdeRunConfig {
        runner: IdeRunnerKind::Pytest,
        test_filter: Some("test_checkout".into()),
        ..Default::default()
    };
    let (ssh_py_prog, ssh_py_args) = build_runner_command(
        &pytest_cfg,
        IdeTargetKind::Ssh,
        "/remote/app",
        &BTreeMap::new(),
    );
    assert_eq!(ssh_py_prog, "python3");
    assert_eq!(
        ssh_py_args,
        vec!["-m", "pytest", "-v", "-k", "test_checkout"]
    );
}

#[tokio::test]
#[cfg(unix)]
async fn stopping_run_kills_entire_unix_process_group_leaving_zero_orphans() {
    let tmp = tempfile::tempdir().unwrap();
    let pid_file = tmp.path().join("child.pid");
    let script = tmp.path().join("spawn_child.sh");
    std::fs::write(
        &script,
        format!(
            "#!/bin/sh\nsleep 60 &\necho $! > '{}'\nwait\n",
            pid_file.display()
        ),
    )
    .unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    let runs: Arc<DashMap<String, Arc<ActiveRunHandle>>> = Arc::new(DashMap::new());
    let (stop_tx, stop_rx) = watch::channel(false);
    let (events_tx, _) = broadcast::channel(32);
    let handle = Arc::new(ActiveRunHandle {
        seq: 1,
        owner_user_id: "u_test".into(),
        state: Mutex::new(RunStateInner::new(
            "run_pg".into(),
            "PG Test".into(),
            IdeRunnerKind::Shell,
            "2026-01-01T00:00:00Z".into(),
        )),
        stop_tx,
        events_tx,
    });
    runs.insert("run_pg".into(), Arc::clone(&handle));

    let target = IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned());
    let cfg = IdeRunConfig {
        id: "rc_pg".into(),
        name: "PG Test".into(),
        runner: IdeRunnerKind::Shell,
        target_path: Some("spawn_child.sh".into()),
        ..Default::default()
    };
    let runs_c = Arc::clone(&runs);
    let handle_c = Arc::clone(&handle);
    let task = tokio::spawn(async move {
        execute_run_task(
            runs_c,
            handle_c,
            TargetManager::new(),
            target,
            None,
            vec![(cfg, BTreeMap::new())],
            None,
            stop_rx,
        )
        .await;
    });

    for _ in 0..40 {
        if pid_file.is_file() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let grandchild_pid: u32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();

    handle.stop();
    task.await.unwrap();

    let alive = Command::new("/bin/kill")
        .args(["-0", &grandchild_pid.to_string()])
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false);
    assert!(
        !alive,
        "grandchild sleep process {grandchild_pid} leaked after stop_run!"
    );
}

#[tokio::test]
async fn saved_run_config_round_trips_and_blocks_cross_workspace_id_collision() {
    let tmp = tempfile::tempdir().unwrap();
    let ws_a = tmp.path().join("ws_a");
    let ws_b = tmp.path().join("ws_b");
    std::fs::create_dir_all(&ws_a).unwrap();
    std::fs::create_dir_all(&ws_b).unwrap();
    let pool = otto_state::db::open(&tmp.path().join("test.db"))
        .await
        .unwrap();
    let mgr = EnvironmentManager::new(tmp.path().join("data"));
    let target_a = IdeTargetSpec::local(ws_a.to_string_lossy().into_owned());
    let target_b = IdeTargetSpec::local(ws_b.to_string_lossy().into_owned());

    let saved = mgr
        .upsert_run_config_parts(
            &pool,
            &target_a,
            IdeRunConfig {
                id: "rc_shared_guess".into(),
                name: "Spring Dev".into(),
                runner: IdeRunnerKind::Maven,
                args: vec!["spring-boot:run".into()],
                maven_profiles: vec!["dev".into(), "cloud".into()],
                spring_profiles: vec!["local".into()],
                skip_tests: true,
                offline: true,
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let fetched = mgr
        .get_run_config(&pool, &target_a, &saved.id)
        .await
        .unwrap();
    assert_eq!(fetched.maven_profiles, vec!["dev", "cloud"]);
    assert_eq!(fetched.spring_profiles, vec!["local"]);
    assert!(fetched.skip_tests);
    assert!(fetched.offline);

    // Workspace B cannot read, overwrite, or delete Workspace A's run config by ID.
    assert!(matches!(
        mgr.get_run_config(&pool, &target_b, &saved.id).await,
        Err(otto_core::Error::NotFound(_))
    ));
    assert!(matches!(
        mgr.upsert_run_config_parts(
            &pool,
            &target_b,
            IdeRunConfig {
                id: saved.id.clone(),
                name: "Hijack".into(),
                runner: IdeRunnerKind::Shell,
                command: Some("id".into()),
                ..Default::default()
            },
        )
        .await,
        Err(otto_core::Error::Forbidden(_))
    ));
    assert!(matches!(
        mgr.delete_run_config(&pool, &target_b, &saved.id).await,
        Err(otto_core::Error::NotFound(_))
    ));
    // Workspace A's row remains intact.
    let still_intact = mgr
        .get_run_config(&pool, &target_a, &saved.id)
        .await
        .unwrap();
    assert_eq!(still_intact.name, "Spring Dev");
}

#[test]
fn subscribe_run_for_user_streams_live_events_and_enforces_owner_rbac() {
    let mgr = EnvironmentManager::new(PathBuf::from("/tmp/otto-ide-sse-test"));
    let (stop_tx, _) = watch::channel(false);
    let (events_tx, _) = broadcast::channel(32);
    let handle = Arc::new(ActiveRunHandle {
        seq: 1,
        owner_user_id: "alice".into(),
        state: Mutex::new(RunStateInner::new(
            "run_sse_1".into(),
            "SSE Stream".into(),
            IdeRunnerKind::GoRun,
            "2026-01-01T00:00:00Z".into(),
        )),
        stop_tx,
        events_tx,
    });
    mgr.runs.insert("run_sse_1".into(), Arc::clone(&handle));

    let alice = sample_user("alice", false);
    let bob = sample_user("bob", false);
    assert!(matches!(
        mgr.subscribe_run_for_user(&bob, "run_sse_1"),
        Err(otto_core::Error::Forbidden(_))
    ));

    let (initial, mut rx) = mgr.subscribe_run_for_user(&alice, "run_sse_1").unwrap();
    assert_eq!(initial.status, IdeRunStatus::Running);
    append_run_line(&handle, "hello from sse".into());
    let ev1 = rx.try_recv().unwrap();
    assert_eq!(
        ev1.output.last().map(String::as_str),
        Some("hello from sse")
    );

    finish_run(
        &handle,
        IdeRunStatus::Passed,
        Some(0),
        Duration::from_millis(12),
        None,
    );
    let ev2 = rx.try_recv().unwrap();
    assert_eq!(ev2.status, IdeRunStatus::Passed);
    assert_eq!(ev2.exit_code, Some(0));
}

#[test]
fn build_runner_command_merges_go_tags_and_coverage_flags() {
    let tmp = tempfile::tempdir().unwrap();
    let test_file = tmp.path().join("assign_component_test.go");
    std::fs::write(
        &test_file,
        "//go:build component\n\npackage tests\n\nfunc TestAssign() {}\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join("package.json"),
        r#"{"name":"ui","devDependencies":{"vitest":"^1.0.0"}}"#,
    )
    .unwrap();

    let cwd = tmp.path().to_string_lossy().into_owned();
    let go_cfg = IdeRunConfig {
        id: "rc_go".into(),
        name: "Go Test".into(),
        runner: IdeRunnerKind::GoTest,
        build_flags: vec!["-tags=integration".into()],
        args: vec!["-tags=component".into(), "./...".into()],
        target_path: Some("assign_component_test.go".into()),
        coverage: true,
        ..Default::default()
    };
    let (prog, args) = super::runner_cmd::build_runner_command(
        &go_cfg,
        IdeTargetKind::Local,
        &cwd,
        &BTreeMap::new(),
    );
    assert_eq!(prog, "go");
    assert!(args.contains(&"-cover".to_string()), "args={args:?}");
    let tag_flags: Vec<&String> = args.iter().filter(|a| a.starts_with("-tags=")).collect();
    assert_eq!(tag_flags, vec![&"-tags=integration,component".to_string()]);

    let npm_cfg = IdeRunConfig {
        id: "rc_npm".into(),
        name: "NPM Test".into(),
        runner: IdeRunnerKind::Npm,
        coverage: true,
        ..Default::default()
    };
    let (npm_prog, npm_args) = super::runner_cmd::build_runner_command(
        &npm_cfg,
        IdeTargetKind::Local,
        &cwd,
        &BTreeMap::new(),
    );
    assert_eq!(npm_prog, "npm");
    assert_eq!(npm_args, vec!["test", "--", "--coverage"]);
}
