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
    resolve_jvm_build_dir, ActiveRunHandle, RunStateInner, MAX_FINISHED_RUNS, MAX_RUN_OUTPUT_BYTES,
    MAX_RUN_OUTPUT_LINES,
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
    let handle = Arc::new(ActiveRunHandle {
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
    });

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

#[tokio::test]
async fn subscribe_run_for_user_streams_live_events_and_enforces_owner_rbac() {
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
    // A burst of lines reaches watchers as one snapshot.
    append_run_line(&handle, "first line".into());
    append_run_line(&handle, "hello from sse".into());
    assert!(rx.try_recv().is_err(), "lines gather before a snapshot");
    let ev1 = rx.recv().await.unwrap();
    assert_eq!(ev1.output, ["first line", "hello from sse"]);
    assert!(rx.try_recv().is_err(), "one snapshot per burst");

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
fn vitest_runs_list_each_test_unless_a_reporter_is_chosen() {
    let gutter = IdeRunConfig {
        runner: IdeRunnerKind::Vitest,
        command: Some("npx".into()),
        args: ["--no-install", "vitest", "run", "a.test.ts", "-t", "adds"]
            .map(String::from)
            .to_vec(),
        ..Default::default()
    };
    let (_, args) = super::runner_cmd::build_runner_command(
        &gutter,
        IdeTargetKind::Local,
        "/w",
        &BTreeMap::new(),
    );
    assert_eq!(args.last().map(String::as_str), Some("--reporter=verbose"));
    let chosen = IdeRunConfig {
        args: vec!["--reporter=dot".into()],
        ..gutter
    };
    let (_, args) = super::runner_cmd::build_runner_command(
        &chosen,
        IdeTargetKind::Local,
        "/w",
        &BTreeMap::new(),
    );
    assert!(
        !args.iter().any(|a| a == "--reporter=verbose"),
        "args={args:?}"
    );
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

#[cfg(unix)]
type RunMap = Arc<DashMap<String, Arc<ActiveRunHandle>>>;

#[cfg(unix)]
fn local_run_handle(id: &str) -> (RunMap, Arc<ActiveRunHandle>, watch::Receiver<bool>) {
    let runs: RunMap = Arc::new(DashMap::new());
    let (stop_tx, stop_rx) = watch::channel(false);
    let (events_tx, _) = broadcast::channel(32);
    let handle = Arc::new(ActiveRunHandle {
        seq: 1,
        owner_user_id: "u_test".into(),
        state: Mutex::new(RunStateInner::new(
            id.into(),
            id.into(),
            IdeRunnerKind::Custom,
            "2026-01-01T00:00:00Z".into(),
        )),
        stop_tx,
        events_tx,
    });
    runs.insert(id.into(), Arc::clone(&handle));
    (runs, handle, stop_rx)
}

#[cfg(unix)]
fn sh_run_config(script: &str, before_launch: Option<String>) -> IdeRunConfig {
    IdeRunConfig {
        id: "rc_sh".into(),
        name: "sh".into(),
        runner: IdeRunnerKind::Custom,
        command: Some("/bin/sh".into()),
        args: vec!["-c".into(), script.into()],
        before_launch,
        ..Default::default()
    }
}

#[cfg(unix)]
async fn pid_alive(pid: u32) -> bool {
    Command::new("/bin/kill")
        .args(["-0", &pid.to_string()])
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .is_ok_and(|s| s.success())
}

/// A program that prints a non-UTF-8 byte and an endless line keeps running:
/// tokio's `lines()` stopped at the bad byte, closed the pipe, and the next
/// write killed the program with SIGPIPE.
#[tokio::test]
#[cfg(unix)]
async fn run_output_with_invalid_utf8_and_an_endless_line_keeps_streaming() {
    use crate::line_reader::TRUNCATED_MARKER;
    let tmp = tempfile::tempdir().unwrap();
    let (runs, handle, stop_rx) = local_run_handle("run_utf8");
    let script = "printf 'bad \\377 byte\\n'; head -c 200000 /dev/zero | tr '\\0' x; echo; \
                  i=0; while [ $i -lt 300 ]; do echo after-$i; i=$((i+1)); done";
    execute_run_task(
        runs,
        Arc::clone(&handle),
        TargetManager::new(),
        IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned()),
        None,
        vec![(sh_run_config(script, None), BTreeMap::new())],
        None,
        stop_rx,
    )
    .await;
    let snap = handle.snapshot();
    assert_eq!(
        snap.status,
        IdeRunStatus::Passed,
        "output: {:?}",
        snap.output
    );
    assert_eq!(snap.exit_code, Some(0));
    assert!(snap.output.iter().any(|l| l == "bad \u{fffd} byte"));
    assert!(snap
        .output
        .iter()
        .any(|l| l.starts_with("xxxx") && l.ends_with(TRUNCATED_MARKER) && l.len() < 20_000));
    assert_eq!(snap.output.last().map(String::as_str), Some("after-299"));
}

/// Stop while `before_launch` is still running kills the hook's whole process
/// group at once and never starts the program.
#[tokio::test]
#[cfg(unix)]
async fn stop_during_before_launch_kills_the_hook_group_and_skips_the_program() {
    let tmp = tempfile::tempdir().unwrap();
    let pid_file = tmp.path().join("hook.pid");
    let started = tmp.path().join("started");
    let (runs, handle, stop_rx) = local_run_handle("run_hook_stop");
    let hook = format!("sleep 60 & echo $! > '{}'; wait", pid_file.display());
    let cfg = sh_run_config(&format!("touch '{}'", started.display()), Some(hook));
    let target = IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned());
    let handle_c = Arc::clone(&handle);
    let task = tokio::spawn(async move {
        execute_run_task(
            runs,
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
    let mut hook_pid = None;
    for _ in 0..80 {
        if let Some(pid) = std::fs::read_to_string(&pid_file)
            .ok()
            .and_then(|s| s.trim().parse::<u32>().ok())
        {
            hook_pid = Some(pid);
            break;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    let hook_pid = hook_pid.expect("before_launch never started");

    handle.stop();
    tokio::time::timeout(Duration::from_secs(2), task)
        .await
        .expect("run task kept waiting on the hook after Stop")
        .unwrap();
    let mut gone = false;
    for _ in 0..50 {
        if !pid_alive(hook_pid).await {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(gone, "hook child {hook_pid} survived Stop");
    assert!(!started.exists(), "the program ran after Stop");
    assert_eq!(handle.snapshot().status, IdeRunStatus::Stopped);
}

/// A failing hook's stdout and stderr reach the console as they are written.
#[tokio::test]
#[cfg(unix)]
async fn failing_before_launch_streams_its_output_and_fails_the_run() {
    let tmp = tempfile::tempdir().unwrap();
    let (runs, handle, stop_rx) = local_run_handle("run_hook_fail");
    let cfg = sh_run_config(
        "echo program-ran",
        Some("echo hook-out; echo hook-err >&2; exit 3".into()),
    );
    execute_run_task(
        runs,
        Arc::clone(&handle),
        TargetManager::new(),
        IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned()),
        None,
        vec![(cfg, BTreeMap::new())],
        None,
        stop_rx,
    )
    .await;
    let snap = handle.snapshot();
    assert_eq!(snap.status, IdeRunStatus::Failed);
    assert!(snap.output.iter().any(|l| l == "hook-out"));
    assert!(snap.output.iter().any(|l| l == "hook-err"));
    assert!(snap
        .output
        .iter()
        .any(|l| l == "ERROR: before_launch failed (exit 3)"));
    assert!(!snap.output.iter().any(|l| l == "program-ran"));
}

/// A hook that never exits is bounded by its timeout and its group is killed.
#[tokio::test]
#[cfg(unix)]
async fn before_launch_timeout_fails_the_step_and_kills_the_group() {
    use super::runner_steps::{run_local_before_launch, StepOutcome};
    let tmp = tempfile::tempdir().unwrap();
    let pid_file = tmp.path().join("hook.pid");
    let (_runs, handle, mut stop_rx) = local_run_handle("run_hook_timeout");
    let hook = format!("sleep 60 & echo $! > '{}'; wait", pid_file.display());
    let outcome = run_local_before_launch(
        &handle,
        &hook,
        &tmp.path().to_string_lossy(),
        &BTreeMap::new(),
        &mut stop_rx,
        Duration::from_millis(400),
    )
    .await;
    match outcome {
        StepOutcome::Failed(msg) => assert!(msg.contains("timed out"), "{msg}"),
        _ => panic!("expected a timeout failure"),
    }
    let pid: u32 = std::fs::read_to_string(&pid_file)
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let mut gone = false;
    for _ in 0..50 {
        if !pid_alive(pid).await {
            gone = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(gone, "hook child {pid} survived the timeout");
}

/// Writes `files` under `root`; a `mvnw`/`gradlew` entry is made executable.
#[cfg(unix)]
fn write_jvm_fixture(root: &std::path::Path, files: &[(&str, &str)]) {
    use std::os::unix::fs::PermissionsExt;
    for (rel, body) in files {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, body).unwrap();
        if rel.ends_with("mvnw") || rel.ends_with("gradlew") {
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
}

/// Resolves the run dir the way `execute_run_task` does and builds the command there.
fn jvm_run(
    root: &std::path::Path,
    cwd: &std::path::Path,
    runner: IdeRunnerKind,
    filter: &str,
) -> (IdeRunnerKind, String, String, Vec<String>) {
    let target = IdeTargetSpec::local(root.to_string_lossy().into_owned());
    let mut cfg = IdeRunConfig {
        runner,
        test_filter: Some(filter.into()),
        cwd: Some(cwd.to_string_lossy().into_owned()),
        ..Default::default()
    };
    let dir = resolve_jvm_build_dir(&mut cfg, &target, cwd.to_string_lossy().into_owned());
    let (prog, args) = build_runner_command(&cfg, IdeTargetKind::Local, &dir, &BTreeMap::new());
    (cfg.runner, dir, prog, args)
}

fn canon(p: std::path::PathBuf) -> String {
    std::fs::canonicalize(p)
        .unwrap()
        .to_string_lossy()
        .into_owned()
}

#[test]
#[cfg(unix)]
fn jvm_test_runs_from_its_maven_module_not_the_source_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let reactor = "<project><modules><module>svc</module></modules></project>";
    write_jvm_fixture(
        root,
        &[
            ("pom.xml", reactor),
            ("svc/pom.xml", "<project/>"),
            ("svc/src/test/java/com/x/FooTest.java", ""),
            ("platform/core/pom.xml", "<project/>"),
            ("platform/core/src/test/java/com/y/BarTest.java", ""),
            ("src/test/java/com/z/RootTest.java", ""),
        ],
    );
    let src = root.join("svc/src/test/java/com/x");

    // Single class: the module that owns the file, not the package folder.
    let (runner, dir, prog, args) = jvm_run(root, &src, IdeRunnerKind::JavaJunit, "com.x.FooTest");
    assert_eq!(runner, IdeRunnerKind::JavaJunit);
    assert_eq!(dir, canon(root.join("svc")));
    assert_eq!(prog, "mvn");
    assert_eq!(args, vec!["test", "-Dtest=com.x.FooTest"]);

    // Single method keeps Surefire's `Class#method` selector.
    let (_, _, _, args) = jvm_run(root, &src, IdeRunnerKind::JavaJunit, "com.x.FooTest#bar");
    assert!(args.contains(&"-Dtest=com.x.FooTest#bar".to_string()));

    // A module nested two levels down.
    let nested = root.join("platform/core/src/test/java/com/y");
    let (_, dir, _, _) = jvm_run(root, &nested, IdeRunnerKind::Maven, "com.y.BarTest");
    assert_eq!(dir, canon(root.join("platform/core")));

    // A test in the reactor root itself runs there; other modules must not fail on `-Dtest`.
    let at_root = root.join("src/test/java/com/z");
    let (_, dir, _, args) = jvm_run(root, &at_root, IdeRunnerKind::JavaJunit, "com.z.RootTest");
    assert_eq!(dir, canon(root.to_path_buf()));
    assert!(args.contains(&"-Dsurefire.failIfNoSpecifiedTests=false".to_string()));
}

#[test]
#[cfg(unix)]
fn jvm_test_uses_the_reactor_root_maven_wrapper_from_a_module() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_jvm_fixture(
        root,
        &[
            (
                "pom.xml",
                "<project><modules><module>svc</module></modules></project>",
            ),
            ("mvnw", "#!/bin/sh\n"),
            ("svc/pom.xml", "<project/>"),
            ("svc/src/test/java/com/x/FooTest.java", ""),
        ],
    );
    let src = root.join("svc/src/test/java/com/x");
    let (_, dir, prog, _) = jvm_run(root, &src, IdeRunnerKind::JavaJunit, "com.x.FooTest");
    assert_eq!(dir, canon(root.join("svc")));
    assert_eq!(prog, format!("{}/mvnw", canon(root.to_path_buf())));

    let (_, _, prog, _) = jvm_run(root, root, IdeRunnerKind::Maven, "com.x.FooTest");
    assert_eq!(prog, "./mvnw");
}

#[test]
#[cfg(unix)]
fn jvm_test_in_a_gradle_module_runs_gradle_with_a_dotted_method_filter() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_jvm_fixture(
        root,
        &[
            ("settings.gradle", "include 'app'"),
            ("gradlew", "#!/bin/sh\n"),
            ("app/build.gradle.kts", ""),
            ("app/src/test/java/com/x/FooTest.java", ""),
        ],
    );
    let src = root.join("app/src/test/java/com/x");
    let (runner, dir, prog, args) =
        jvm_run(root, &src, IdeRunnerKind::JavaJunit, "com.x.FooTest#bar");
    assert_eq!(runner, IdeRunnerKind::Gradle);
    assert_eq!(dir, canon(root.join("app")));
    assert_eq!(prog, format!("{}/gradlew", canon(root.to_path_buf())));
    assert_eq!(args, vec!["test", "--tests", "com.x.FooTest.bar"]);
}

#[test]
#[cfg(unix)]
fn jvm_build_dir_stays_put_without_a_build_file_or_off_local() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_jvm_fixture(root, &[("src/test/java/com/x/FooTest.java", "")]);
    let src = root
        .join("src/test/java/com/x")
        .to_string_lossy()
        .into_owned();
    let (_, dir, _, _) = jvm_run(
        root,
        std::path::Path::new(&src),
        IdeRunnerKind::JavaJunit,
        "X",
    );
    assert_eq!(dir, src);

    // Never climbs above the workspace root, even when a parent has a POM.
    std::fs::write(root.join("pom.xml"), "<project/>").unwrap();
    let inner = root.join("src");
    let (_, dir, _, _) = jvm_run(
        &inner,
        &inner.join("test/java/com/x"),
        IdeRunnerKind::Maven,
        "X",
    );
    assert!(dir.ends_with("src/test/java/com/x"), "{dir}");

    let mut cfg = IdeRunConfig {
        runner: IdeRunnerKind::JavaJunit,
        ..Default::default()
    };
    let mut ssh = IdeTargetSpec::local("/remote/app");
    ssh.kind = IdeTargetKind::Ssh;
    let dir = resolve_jvm_build_dir(&mut cfg, &ssh, "/remote/app/svc/src/test".into());
    assert_eq!(dir, "/remote/app/svc/src/test");
}

#[test]
#[cfg(unix)]
fn kotlin_pack_gutter_items_run_gradle_from_the_module_with_the_root_wrapper() {
    let manifest: crate::types::IdePluginManifest = serde_json::from_str(include_str!(
        "../../../../examples/ide-plugins/kotlin-ide/otto-ide-plugin.json"
    ))
    .unwrap();
    let detectors = manifest.contributions.gutter_detectors;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    write_jvm_fixture(
        root,
        &[
            ("settings.gradle.kts", "include(\"core\")"),
            ("gradlew", "#!/bin/sh\n"),
            ("core/build.gradle.kts", ""),
            ("core/src/test/kotlin/CartTest.kt", ""),
        ],
    );
    let file = root.join("core/src/test/kotlin/CartTest.kt");
    let src = "fun main() {}\nfun `adds two items`() {}\nfun testTotal() {}\n";
    let items = crate::gutter::detect_gutter_items(&file.to_string_lossy(), src, &detectors);
    let target = IdeTargetSpec::local(root.to_string_lossy().into_owned());
    let run = |idx: usize| {
        let spec = &items[idx].run_spec;
        let mut cfg = IdeRunConfig {
            runner: spec.runner,
            command: spec.command.clone(),
            args: spec.args.clone(),
            test_filter: spec.test_filter.clone(),
            ..Default::default()
        };
        let dir = resolve_jvm_build_dir(&mut cfg, &target, spec.cwd.clone().unwrap());
        let (prog, args) = build_runner_command(&cfg, IdeTargetKind::Local, &dir, &BTreeMap::new());
        (dir, prog, args)
    };
    let wrapper = format!("{}/gradlew", canon(root.to_path_buf()));

    let (dir, prog, args) = run(0);
    assert_eq!(items[0].run_spec.test_filter, None);
    assert_eq!(
        (dir.as_str(), prog.as_str()),
        (canon(root.join("core")).as_str(), wrapper.as_str())
    );
    assert_eq!(args, ["run"]);

    assert_eq!(items[1].label, "Test adds two items");
    assert_eq!(run(1).2, ["test", "--tests", "*.adds two items"]);
    assert_eq!(run(2).2, ["test", "--tests", "*.testTotal"]);
}

const SUREFIRE_XML: &str = r#"<?xml version="1.0" encoding="UTF-8"?>
<testsuite name="com.x.CartTest" tests="5" failures="1">
  <properties><property name="java.version" value="21"/></properties>
  <testcase name="addsItem" classname="com.x.CartTest" time="0.012"/>
  <testcase name="removesItem" classname="com.x.CartTest" time="0.003"/>
  <testcase name="totals" classname="com.x.CartTest" time="0.5">
    <failure message="expected: &lt;3&gt; but was: &lt;2&gt;" type="AssertionFailedError">trace</failure>
  </testcase>
  <testcase name="empty" classname="com.x.CartTest" time="0"/>
  <testcase name="clears" classname="com.x.CartTest" time="0.001"></testcase>
</testsuite>
"#;

#[test]
fn junit_xml_reports_parse_per_test_rows_for_surefire_and_gradle() {
    use super::runner_parsers::parse_junit_xml;
    use crate::types::IdeTestCaseStatus;
    let rows = parse_junit_xml(SUREFIRE_XML);
    assert_eq!(rows.len(), 5);
    assert_eq!(rows[0].name, "addsItem");
    assert_eq!(rows[0].suite.as_deref(), Some("com.x.CartTest"));
    assert_eq!(rows[0].duration_ms, Some(12));
    assert_eq!(rows[2].status, IdeTestCaseStatus::Failed);
    assert_eq!(
        rows[2].message.as_deref(),
        Some("expected: <3> but was: <2>")
    );
    let failed = rows
        .iter()
        .filter(|r| r.status == IdeTestCaseStatus::Failed);
    assert_eq!(failed.count(), 1);

    let gradle = r#"<testsuite name="CartTest" tests="2" skipped="1">
  <testcase name="adds two items()" classname="CartTest" time="0.004"/>
  <testcase name="later()" classname="CartTest" time="0.0">
    <skipped/>
  </testcase>
</testsuite>"#;
    let rows = parse_junit_xml(gradle);
    assert_eq!(rows[0].name, "adds two items()");
    assert_eq!(rows[0].status, IdeTestCaseStatus::Passed);
    assert_eq!(rows[1].status, IdeTestCaseStatus::Skipped);
}

#[cfg(unix)]
fn maven_sh_run_config(script: &str) -> IdeRunConfig {
    IdeRunConfig {
        runner: IdeRunnerKind::Maven,
        test_filter: Some("CartTest".into()),
        ..sh_run_config(script, None)
    }
}

#[tokio::test]
#[cfg(unix)]
async fn maven_run_reports_tests_from_the_surefire_xml_it_wrote() {
    let tmp = tempfile::tempdir().unwrap();
    // A stale report from an earlier run must not count.
    let stale = tmp.path().join("old/target/surefire-reports");
    std::fs::create_dir_all(&stale).unwrap();
    std::fs::write(stale.join("TEST-com.x.OldTest.xml"), SUREFIRE_XML).unwrap();
    let old = std::time::SystemTime::now() - Duration::from_secs(3600);
    std::fs::File::options()
        .write(true)
        .open(stale.join("TEST-com.x.OldTest.xml"))
        .unwrap()
        .set_modified(old)
        .unwrap();
    let xml = tmp.path().join("CartTest.xml");
    std::fs::write(&xml, SUREFIRE_XML).unwrap();
    let script = format!(
        "mkdir -p core/target/surefire-reports && cp '{}' core/target/surefire-reports/TEST-com.x.CartTest.xml",
        xml.display()
    );
    let (runs, handle, stop_rx) = local_run_handle("run_surefire");
    let mut cfg = maven_sh_run_config(&script);
    cfg.cwd = Some(tmp.path().to_string_lossy().into_owned());
    execute_run_task(
        runs,
        Arc::clone(&handle),
        TargetManager::new(),
        IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned()),
        None,
        vec![(cfg, BTreeMap::new())],
        None,
        stop_rx,
    )
    .await;
    let snap = handle.snapshot();
    assert_eq!(snap.tests.len(), 5, "output: {:?}", snap.output);
    assert_eq!(snap.status, IdeRunStatus::Passed);
}

#[tokio::test]
#[cfg(unix)]
async fn filtered_test_run_that_ran_nothing_is_not_passed() {
    let tmp = tempfile::tempdir().unwrap();
    let (runs, handle, stop_rx) = local_run_handle("run_no_tests");
    execute_run_task(
        runs,
        Arc::clone(&handle),
        TargetManager::new(),
        IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned()),
        None,
        vec![(maven_sh_run_config("echo BUILD SUCCESS"), BTreeMap::new())],
        None,
        stop_rx,
    )
    .await;
    let snap = handle.snapshot();
    assert_eq!(snap.status, IdeRunStatus::Failed);
    assert!(
        snap.output
            .iter()
            .any(|l| l == "ERROR: No tests matched 'CartTest'"),
        "{:?}",
        snap.output
    );
}

#[test]
fn module_venv_is_found_up_from_the_run_dir_inside_the_roots() {
    use super::runner::apply_runtime_profile_and_module_venv;
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path().join("proj");
    std::fs::create_dir_all(root.join(".venv/bin")).unwrap();
    std::fs::create_dir_all(root.join("tests")).unwrap();
    std::fs::write(root.join(".venv/pyvenv.cfg"), "").unwrap();
    std::fs::write(root.join(".venv/bin/python"), "").unwrap();
    let target = IdeTargetSpec::local(root.to_string_lossy().into_owned());
    let mut env = BTreeMap::new();
    let tests_dir = root.join("tests").to_string_lossy().into_owned();
    apply_runtime_profile_and_module_venv(&IdeRunConfig::default(), &target, &tests_dir, &mut env);
    assert_eq!(env.get("VIRTUAL_ENV"), Some(&canon(root.join(".venv"))));
    assert!(env["PATH"].starts_with(&canon(root.join(".venv/bin"))));

    // Never above the workspace root: a venv above the roots is not the project's.
    let inner = root.join("tests");
    let target = IdeTargetSpec::local(inner.to_string_lossy().into_owned());
    let mut env = BTreeMap::new();
    apply_runtime_profile_and_module_venv(&IdeRunConfig::default(), &target, &tests_dir, &mut env);
    assert_eq!(env.get("VIRTUAL_ENV"), None);
}
