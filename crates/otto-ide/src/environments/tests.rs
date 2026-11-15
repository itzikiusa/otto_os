//! Unit & SQLite integration tests for `EnvironmentManager` CRUD, secret redaction,
//! non-root shell-environment scrubbing, and Python virtualenv confinement.

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex};

use otto_core::secrets::SecretStore;
use otto_core::Result;

use crate::target::TargetManager;
use crate::types::{
    IdeCreatePythonVenvReq, IdeEnvVarEntry, IdeRunConfig, IdeRunnerKind, IdeTargetSpec,
    IdeUpdateEnvironmentReq, IdeUpsertRunConfigReq,
};

use super::{EnvironmentManager, ShellEnvResolver};

#[derive(Default)]
pub(super) struct MemSecrets(Mutex<HashMap<String, String>>);

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
                module_sdks: None,
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

    let eff_root = mgr
        .build_effective_env_for_run_as(&pool, secrets.as_ref(), &target, Some(&rc), true)
        .await
        .unwrap();
    assert_eq!(eff_root.get("FROM_DOTENV").unwrap(), "hello\nworld");
    assert_eq!(eff_root.get("QA_ONLY").unwrap(), "1");
    assert_eq!(eff_root.get("CFG_WIN").unwrap(), "from_cfg_env_file");
    assert_eq!(eff_root.get("APP_ENV").unwrap(), "override_wins");
    assert_eq!(
        eff_root.get("STRIPE_API_SECRET").unwrap(),
        "sk_test_super_secret"
    );
    assert_eq!(
        eff_root.get("GO_DEBUGGING_PRODAMS").unwrap(),
        "from_zshrc_sentinel"
    );
    assert_eq!(eff_root.get("RUSTUP_TOOLCHAIN").unwrap(), "stable");
    assert_eq!(eff_root.get("GOROOT").unwrap(), "/opt/go-1.25");
    assert_eq!(eff_root.get("JAVA_HOME").unwrap(), "/opt/jdk-21");
    assert!(eff_root
        .get("PATH")
        .unwrap()
        .starts_with("/custom/bin:/opt/go-1.25/bin:/opt/jdk-21/bin:/opt/go-1.24/bin:"));

    // Non-root callers must not inherit host login-shell secrets while still receiving
    // explicit project / run-config environment layers.
    let eff_non_root = mgr
        .build_effective_env_for_run_as(&pool, secrets.as_ref(), &target, Some(&rc), false)
        .await
        .unwrap();
    assert!(!eff_non_root.contains_key("GO_DEBUGGING_PRODAMS"));
    assert_eq!(eff_non_root.get("APP_ENV").unwrap(), "override_wins");
    assert_eq!(
        eff_non_root.get("STRIPE_API_SECRET").unwrap(),
        "sk_test_super_secret"
    );

    mgr.delete_run_config(&pool, &target, &rc.id).await.unwrap();
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

#[tokio::test]
async fn create_python_venv_rejects_out_of_root_sensitive_and_traversal_paths() {
    let tmp = tempfile::tempdir().unwrap();
    let ws = tmp.path().join("workspace");
    let outside = tmp.path().join("outside");
    std::fs::create_dir_all(&ws).unwrap();
    std::fs::create_dir_all(&outside).unwrap();

    let pool = otto_state::db::open(&tmp.path().join("venv_sec.db"))
        .await
        .unwrap();
    let mgr = EnvironmentManager::new(tmp.path().join("data"));
    let targets = TargetManager::new();
    let target = IdeTargetSpec::local(ws.to_string_lossy().into_owned());

    let err_out = mgr
        .create_python_venv(
            &pool,
            &targets,
            IdeCreatePythonVenvReq {
                target: target.clone(),
                root: None,
                venv_dir: Some(outside.join("evil-venv").to_string_lossy().into_owned()),
                python_version: None,
            },
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err_out, otto_core::Error::Invalid(_)),
        "expected Invalid for out-of-root absolute venv_dir, got: {err_out:?}"
    );
    assert!(!outside.join("evil-venv").exists());

    if let Some(home) = dirs::home_dir() {
        let ssh_evil = home.join(".ssh/evil-venv");
        let err_ssh_venv = mgr
            .create_python_venv(
                &pool,
                &targets,
                IdeCreatePythonVenvReq {
                    target: target.clone(),
                    root: None,
                    venv_dir: Some(ssh_evil.to_string_lossy().into_owned()),
                    python_version: None,
                },
                None,
            )
            .await
            .unwrap_err();
        assert!(matches!(
            err_ssh_venv,
            otto_core::Error::Invalid(_) | otto_core::Error::Forbidden(_)
        ));
    }
    let err_etc_root = mgr
        .create_python_venv(
            &pool,
            &targets,
            IdeCreatePythonVenvReq {
                target: target.clone(),
                root: Some("/etc".into()),
                venv_dir: Some(".venv".into()),
                python_version: None,
            },
            None,
        )
        .await
        .unwrap_err();
    assert!(
        matches!(err_etc_root, otto_core::Error::Forbidden(_)),
        "expected Forbidden for sensitive root /etc, got: {err_etc_root:?}"
    );

    #[cfg(unix)]
    {
        let link = ws.join("escape_link");
        std::os::unix::fs::symlink(&outside, &link).unwrap();
        let err_sym = mgr
            .create_python_venv(
                &pool,
                &targets,
                IdeCreatePythonVenvReq {
                    target: target.clone(),
                    root: None,
                    venv_dir: Some("escape_link/.venv".into()),
                    python_version: None,
                },
                None,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err_sym, otto_core::Error::Forbidden(_)),
            "expected Forbidden for symlink escape venv_dir, got: {err_sym:?}"
        );
    }

    let ssh_target = IdeTargetSpec::ssh("conn_1".to_string(), "/home/dev/app");
    for bad_root in ["/home/dev/app/../../etc", "relative/sub"] {
        let err_ssh = mgr
            .create_python_venv(
                &pool,
                &targets,
                IdeCreatePythonVenvReq {
                    target: ssh_target.clone(),
                    root: Some(bad_root.into()),
                    venv_dir: Some(".venv".into()),
                    python_version: None,
                },
                None,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(err_ssh, otto_core::Error::Invalid(_)),
            "expected Invalid for SSH root '{bad_root}', got: {err_ssh:?}"
        );
    }

    for bad_py in ["--help", "/bin/sh", "3.99;id"] {
        let err_py = mgr
            .create_python_venv(
                &pool,
                &targets,
                IdeCreatePythonVenvReq {
                    target: target.clone(),
                    root: None,
                    venv_dir: Some(".venv".into()),
                    python_version: Some(bad_py.into()),
                },
                None,
            )
            .await
            .unwrap_err();
        assert!(
            matches!(
                err_py,
                otto_core::Error::Invalid(_) | otto_core::Error::Forbidden(_)
            ),
            "expected Invalid/Forbidden for python_version '{bad_py}', got: {err_py:?}"
        );
    }
}
