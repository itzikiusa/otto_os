//! Unit & SQLite integration tests for `EnvironmentManager` and `runner`.

use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use dashmap::DashMap;
use otto_core::secrets::SecretStore;
use otto_core::Result;
use tokio::process::Command;
use tokio::sync::watch;

use crate::target::TargetManager;
use crate::types::{
    IdeEnvVarEntry, IdeRunConfig, IdeRunStatus, IdeRunnerKind, IdeTargetSpec,
    IdeUpdateEnvironmentReq, IdeUpsertRunConfigReq,
};

use super::runner::{
    append_run_line, build_runner_command, evict_finished_runs, execute_run_task, finish_run,
    ActiveRunHandle, RunStateInner, MAX_FINISHED_RUNS, MAX_RUN_OUTPUT_BYTES, MAX_RUN_OUTPUT_LINES,
};
use super::{EnvironmentManager, ShellEnvResolver};

#[derive(Default)]
struct MemSecrets(Mutex<HashMap<String, String>>);

impl SecretStore for MemSecrets {
    fn put(&self, key: &str, value: &str) -> Result<()> {
        self.0
            .lock()
            .unwrap()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn get(&self, key: &str) -> Result<Option<String>> {
        Ok(self.0.lock().unwrap().get(key).cloned())
    }

    fn delete(&self, key: &str) -> Result<()> {
        self.0.lock().unwrap().remove(key);
        Ok(())
    }
}

#[tokio::test]
async fn sqlite_crud_redacts_secrets_preserves_multiroot_and_layers_intellij_env_order() {
    let tmp = tempfile::tempdir().unwrap();
    let pool = otto_state::db::open(&tmp.path().join("test.db"))
        .await
        .unwrap();
    let secrets: Arc<dyn SecretStore> = Arc::new(MemSecrets::default());
    std::fs::write(
        tmp.path().join(".env"),
        "FROM_DOTENV=\"hello\\nworld\"\nAPP_ENV=dev\nCFG_WIN=from_dotenv\n",
    )
    .unwrap();
    std::fs::write(
        tmp.path().join(".env.qa"),
        "CFG_WIN=from_cfg_env_file\nQA_ONLY=1\n",
    )
    .unwrap();

    let sibling = tmp.path().join("go_casino_kit");
    std::fs::create_dir_all(&sibling).unwrap();

    let mut seeded_shell = BTreeMap::new();
    seeded_shell.insert("JAVA_HOME".into(), "/shell/java".into());
    seeded_shell.insert("GOROOT".into(), "/shell/go".into());
    seeded_shell.insert("RUSTUP_TOOLCHAIN".into(), "nightly".into());
    seeded_shell.insert("GO_DEBUGGING_PRODAMS".into(), "from_zshrc_sentinel".into());
    let mgr = EnvironmentManager::with_shell_env(
        tmp.path().join("data"),
        ShellEnvResolver::with_seeded_vars(seeded_shell),
    );
    let target = IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned());

    let after_attach = mgr
        .add_project_folder(&pool, &target, &sibling.to_string_lossy())
        .await
        .unwrap();
    assert_eq!(after_attach.additional_roots.len(), 1);

    let saved = mgr
        .update_environment(
            &pool,
            &secrets,
            IdeUpdateEnvironmentReq {
                target: target.clone(),
                java_home: Some("/opt/jdk-21".into()),
                go_root: Some("/opt/go-1.24".into()),
                rust_toolchain: Some("stable".into()),
                node_home: None,
                python_interpreter: None,
                dotenv_path: Some(".env".into()),
                extra_path: Some("/custom/bin".into()),
                inherit_shell_env: None,
                additional_roots: None,
                env_vars: vec![
                    IdeEnvVarEntry {
                        key: "APP_ENV".into(),
                        value: Some("staging".into()),
                        secret: false,
                        has_secret: false,
                    },
                    IdeEnvVarEntry {
                        key: "STRIPE_API_SECRET".into(),
                        value: Some("sk_test_super_secret".into()),
                        secret: true,
                        has_secret: false,
                    },
                ],
            },
        )
        .await
        .unwrap();

    assert_eq!(saved.additional_roots.len(), 1);
    assert!(saved.inherit_shell_env);
    let sec_entry = saved
        .env_vars
        .iter()
        .find(|e| e.key == "STRIPE_API_SECRET")
        .unwrap();
    assert!(sec_entry.has_secret);
    assert_eq!(sec_entry.value, None);

    let raw: String = sqlx::query_scalar("SELECT env_vars_json FROM ide_project_environments")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(!raw.contains("sk_test_super_secret"));

    let mut overrides = BTreeMap::new();
    overrides.insert("APP_ENV".into(), "override_wins".into());
    let rc = mgr
        .upsert_run_config(
            &pool,
            IdeUpsertRunConfigReq {
                target: target.clone(),
                config: IdeRunConfig {
                    id: "rc_1".into(),
                    name: "Go Suite".into(),
                    runner: IdeRunnerKind::GoTest,
                    args: vec!["./calc/...".into()],
                    build_flags: vec!["-race".into()],
                    env_overrides: overrides,
                    env_file: Some(".env.qa".into()),
                    sdk_path: Some("/opt/go-1.25".into()),
                    ..Default::default()
                },
            },
        )
        .await
        .unwrap();

    let eff = mgr
        .build_effective_env_for_run(&pool, secrets.as_ref(), &target, Some(&rc))
        .await
        .unwrap();
    assert_eq!(eff.get("FROM_DOTENV").unwrap(), "hello\nworld");
    assert_eq!(eff.get("QA_ONLY").unwrap(), "1");
    assert_eq!(eff.get("CFG_WIN").unwrap(), "from_cfg_env_file");
    assert_eq!(eff.get("APP_ENV").unwrap(), "override_wins");
    assert_eq!(
        eff.get("STRIPE_API_SECRET").unwrap(),
        "sk_test_super_secret"
    );
    assert_eq!(
        eff.get("GO_DEBUGGING_PRODAMS").unwrap(),
        "from_zshrc_sentinel"
    );
    assert_eq!(eff.get("RUSTUP_TOOLCHAIN").unwrap(), "stable");
    assert_eq!(eff.get("GOROOT").unwrap(), "/opt/go-1.25");
    assert_eq!(eff.get("JAVA_HOME").unwrap(), "/opt/jdk-21");
    assert!(eff
        .get("PATH")
        .unwrap()
        .starts_with("/custom/bin:/opt/go-1.25/bin:/opt/jdk-21/bin:/opt/go-1.24/bin:"));

    mgr.delete_run_config(&pool, &rc.id).await.unwrap();
    assert!(mgr
        .list_run_configs(&pool, &target)
        .await
        .unwrap()
        .is_empty());

    let after_detach = mgr
        .remove_project_folder(&pool, &target, &sibling.to_string_lossy())
        .await
        .unwrap();
    assert!(after_detach.additional_roots.is_empty());
}

#[test]
fn finished_runs_evict_cleanly_past_max_finished_runs_preserving_running() {
    let mgr = EnvironmentManager::new(PathBuf::from("/tmp/otto-ide-test"));
    let total_finished = (MAX_FINISHED_RUNS + 8) as u64;
    for i in 0..total_finished {
        let (stop_tx, _) = watch::channel(false);
        let id = format!("run_fin_{i:02}");
        let handle = Arc::new(ActiveRunHandle {
            seq: i,
            state: Mutex::new(RunStateInner::new(
                id.clone(),
                "Finished".into(),
                IdeRunnerKind::Custom,
                format!("2026-01-01T00:00:{i:02}Z"),
            )),
            stop_tx,
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
    mgr.runs.insert(
        "run_active".into(),
        Arc::new(ActiveRunHandle {
            seq: 100,
            state: Mutex::new(RunStateInner::new(
                "run_active".into(),
                "Active".into(),
                IdeRunnerKind::Custom,
                "2026-01-01T00:00:00Z".into(),
            )),
            stop_tx,
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
fn streaming_10k_lines_and_maven_spring_profile_flags_work() {
    let (stop_tx, _stop_rx) = watch::channel(false);
    let handle = ActiveRunHandle {
        seq: 1,
        state: Mutex::new(RunStateInner::new(
            "run_perf".into(),
            "CargoTest".into(),
            IdeRunnerKind::CargoTest,
            "2026-01-01T00:00:00Z".into(),
        )),
        stop_tx,
    };

    let started = Instant::now();
    for i in 0..10_000 {
        append_run_line(&handle, format!("stdout log line {i}"));
    }
    assert!(started.elapsed() < Duration::from_millis(50));
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
    let (prog, args) = build_runner_command(&mvn_cfg, "/tmp", &BTreeMap::new());
    assert_eq!(prog, "mvn");
    assert!(args.contains(&"-Pdev,aws".to_string()));
    assert!(args.contains(&"-Dspring.profiles.active=local".to_string()));
    assert!(args.contains(&"-Dspring-boot.run.profiles=local".to_string()));
    assert!(args.contains(&"-DskipTests".to_string()));
    assert!(args.contains(&"-o".to_string()));
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
    let handle = Arc::new(ActiveRunHandle {
        seq: 1,
        state: Mutex::new(RunStateInner::new(
            "run_pg".into(),
            "PG Test".into(),
            IdeRunnerKind::Shell,
            "2026-01-01T00:00:00Z".into(),
        )),
        stop_tx,
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
