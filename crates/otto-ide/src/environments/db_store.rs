//! SQLite persistence, `.env` parsing, Python virtualenv discovery, and SDK
//! override helpers for `EnvironmentManager`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use chrono::Utc;
use otto_core::{Error, Result};
use otto_state::DbPool;
use serde::{Deserialize, Serialize};
use sqlx::Row;

use crate::netbridge::sanitize_network_bridge;
use crate::toolchains::read_subdirs;
use crate::types::{
    IdeNetworkBridgeConfig, IdeProjectEnvironment, IdePythonVenvInfo, IdeRunConfig, IdeRunnerKind,
    IdeTargetKind, IdeTargetSpec,
};

use super::module_sdks::sanitize_module_sdks;

/// Reserved internal `StoredEnvVar.key` persisted inside `env_vars_json` to store
/// per-module SDK overrides (`IdeProjectEnvironment.module_sdks`) without altering
/// SQLite migrations (compatible with rollback binaries). Stripped on load and never
/// exposed in `env_vars` or exported as an OS environment variable.
pub(super) const RESERVED_MODULE_SDKS_KEY: &str = "__otto_module_sdks__";
pub(super) const RESERVED_NETBRIDGE_KEY: &str = "__otto_network_bridge__";

/// Internal SQLite representation inside `ide_project_environments.env_vars_json`.
/// Never stores plaintext values when `secret == true`; stores `secret_ref` only.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct StoredEnvVar {
    pub(super) key: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) value: Option<String>,
    #[serde(default)]
    pub(super) secret: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) secret_ref: Option<String>,
}

/// Safe, non-evaluating `.env` key-value parser supporting `#` comments,
/// `export KEY=VAL`, single/double quotes, and `\n` / `\"` escapes.
pub fn parse_dotenv(content: &str) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for raw_line in content.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let line = line.strip_prefix("export ").map(str::trim).unwrap_or(line);
        let Some((raw_k, raw_v)) = line.split_once('=') else {
            continue;
        };
        let key = raw_k.trim();
        if !is_valid_env_key(key) {
            continue;
        }
        let v = raw_v.trim();
        let val = if v.len() >= 2 && v.starts_with('"') && v.ends_with('"') {
            v[1..v.len() - 1]
                .replace("\\n", "\n")
                .replace("\\r", "\r")
                .replace("\\t", "\t")
                .replace("\\\"", "\"")
        } else if v.len() >= 2 && v.starts_with('\'') && v.ends_with('\'') {
            v[1..v.len() - 1].to_string()
        } else {
            v.split(" #").next().unwrap_or(v).trim().to_string()
        };
        out.insert(key.to_string(), val);
    }
    out
}

pub(super) fn is_valid_env_key(key: &str) -> bool {
    let mut chars = key.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

pub(super) fn secret_key_ref(project_key: &str, var_name: &str) -> String {
    let mut hash: u64 = 0xcbf29ce484222325;
    for b in project_key.as_bytes() {
        hash ^= u64::from(*b);
        hash = hash.wrapping_mul(0x100000001b3);
    }
    format!("ide-env:{hash:016x}:{var_name}")
}

pub(super) fn clean_opt(opt: &Option<String>) -> Option<String> {
    opt.as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

pub(super) fn push_unique(vec: &mut Vec<String>, item: String) {
    if !item.is_empty() && !vec.contains(&item) {
        vec.push(item);
    }
}

pub(super) fn apply_run_config_sdk_override(
    runner: IdeRunnerKind,
    sdk_path: &str,
    target_kind: IdeTargetKind,
    map: &mut BTreeMap<String, String>,
    path_dirs: &mut Vec<String>,
) {
    let trimmed = sdk_path.trim_end_matches('/');
    match runner {
        IdeRunnerKind::GoRun | IdeRunnerKind::GoBuild | IdeRunnerKind::GoTest => {
            map.insert("GOROOT".into(), trimmed.to_string());
            push_unique(path_dirs, format!("{trimmed}/bin"));
        }
        IdeRunnerKind::JavaMain
        | IdeRunnerKind::JavaJunit
        | IdeRunnerKind::Maven
        | IdeRunnerKind::Gradle => {
            map.insert("JAVA_HOME".into(), trimmed.to_string());
            push_unique(path_dirs, format!("{trimmed}/bin"));
        }
        IdeRunnerKind::CargoRun | IdeRunnerKind::CargoTest => {
            if trimmed.starts_with('/') {
                push_unique(path_dirs, format!("{trimmed}/bin"));
            } else {
                map.insert("RUSTUP_TOOLCHAIN".into(), trimmed.to_string());
            }
        }
        IdeRunnerKind::Python | IdeRunnerKind::Pytest => {
            let p = Path::new(trimmed);
            if p.is_file() {
                if let Some(bin_dir) = p.parent() {
                    push_unique(path_dirs, bin_dir.to_string_lossy().into_owned());
                    if let Some(venv) = bin_dir.parent() {
                        if venv.join("pyvenv.cfg").is_file() || target_kind == IdeTargetKind::Ssh {
                            map.insert("VIRTUAL_ENV".into(), venv.to_string_lossy().into_owned());
                        }
                    }
                }
            } else {
                if Path::new(trimmed).join("pyvenv.cfg").is_file() {
                    map.insert("VIRTUAL_ENV".into(), trimmed.to_string());
                }
                push_unique(path_dirs, format!("{trimmed}/bin"));
            }
        }
        IdeRunnerKind::Node | IdeRunnerKind::Npm | IdeRunnerKind::Vitest => {
            push_unique(path_dirs, format!("{trimmed}/bin"));
        }
        IdeRunnerKind::Shell | IdeRunnerKind::Compound | IdeRunnerKind::Custom => {
            if trimmed.starts_with('/') {
                push_unique(path_dirs, format!("{trimmed}/bin"));
            }
        }
    }
}

pub(super) fn validate_attached_folder_path(
    target: &IdeTargetSpec,
    raw_folder: &str,
) -> Result<String> {
    let trimmed = raw_folder.trim();
    if trimmed.is_empty() || trimmed.chars().any(char::is_control) {
        return Err(Error::Invalid("folder path must not be empty".into()));
    }
    if trimmed.contains("..") {
        return Err(Error::Invalid("folder path must not contain '..'".into()));
    }
    if !trimmed.starts_with('/') {
        return Err(Error::Invalid(
            "attached project folder must be an absolute path".into(),
        ));
    }
    if target.kind == IdeTargetKind::Local {
        let canon = std::fs::canonicalize(trimmed).map_err(|e| {
            Error::Invalid(format!(
                "attached project folder '{trimmed}' not found: {e}"
            ))
        })?;
        if !canon.is_dir() {
            return Err(Error::Invalid(format!(
                "attached project folder '{}' is not a directory",
                canon.display()
            )));
        }
        let canon_str = canon.to_string_lossy().into_owned();
        if is_forbidden_sensitive_dir(&canon_str) {
            return Err(Error::Forbidden(
                "refusing to attach sensitive credential directory".into(),
            ));
        }
        crate::target::enforce_non_root_path(&canon)?;
        Ok(canon_str)
    } else {
        Ok(trimmed.trim_end_matches('/').to_string())
    }
}

fn is_forbidden_sensitive_dir(path: &str) -> bool {
    if path == "/" || path == "/etc" || path == "/private/etc" {
        return true;
    }
    if let Some(home) = dirs::home_dir() {
        let hs = home.to_string_lossy();
        for rel in [".ssh", ".aws", ".gnupg", ".kube", ".config/gcloud"] {
            let blocked = format!("{}/{rel}", hs.trim_end_matches('/'));
            if path == blocked || path.starts_with(&format!("{blocked}/")) {
                return true;
            }
        }
    }
    false
}

pub(super) async fn persist_project_roots(
    pool: &DbPool,
    target: &IdeTargetSpec,
    roots: &[String],
) -> Result<()> {
    let roots_json = serde_json::to_string(roots)
        .map_err(|e| Error::Internal(format!("serialize additional_roots_json: {e}")))?;
    let now = Utc::now().to_rfc3339();
    let target_kind = match target.kind {
        IdeTargetKind::Local => "local",
        IdeTargetKind::Ssh => "ssh",
    };
    sqlx::query(
        r#"INSERT INTO ide_project_environments (
               project_key, target_kind, root_path, connection_id,
               additional_roots_json, updated_at
           ) VALUES (?, ?, ?, ?, ?, ?)
           ON CONFLICT(project_key) DO UPDATE SET
               additional_roots_json = excluded.additional_roots_json,
               updated_at = excluded.updated_at"#,
    )
    .bind(target.project_key())
    .bind(target_kind)
    .bind(&target.root)
    .bind(&target.connection_id)
    .bind(&roots_json)
    .bind(&now)
    .execute(pool)
    .await
    .map_err(|e| Error::Internal(format!("persist additional_roots_json: {e}")))?;
    Ok(())
}

pub(super) fn safe_join_allowed_roots(
    allowed_roots: &[String],
    rel_or_abs: &str,
) -> Option<PathBuf> {
    if Path::new(rel_or_abs).is_absolute() {
        let cc = std::fs::canonicalize(rel_or_abs).ok()?;
        for root in allowed_roots {
            if let Ok(cr) = std::fs::canonicalize(root) {
                if cc.starts_with(&cr) {
                    return Some(cc);
                }
            }
        }
        return None;
    }
    if rel_or_abs.contains("..") {
        return None;
    }
    for root in allowed_roots {
        let cand = Path::new(root).join(rel_or_abs);
        if let (Ok(cr), Ok(cc)) = (std::fs::canonicalize(root), std::fs::canonicalize(&cand)) {
            if cc.starts_with(&cr) {
                return Some(cc);
            }
        }
    }
    None
}

fn decode_project_env_row(
    row: &sqlx::sqlite::SqliteRow,
) -> (IdeProjectEnvironment, Vec<StoredEnvVar>) {
    let raw_json: String = row.try_get("env_vars_json").unwrap_or_else(|_| "[]".into());
    let raw_stored: Vec<StoredEnvVar> = serde_json::from_str(&raw_json).unwrap_or_default();
    let mut module_sdks: BTreeMap<String, String> = BTreeMap::new();
    let mut network_bridge: Option<IdeNetworkBridgeConfig> = None;
    let mut stored: Vec<StoredEnvVar> = Vec::with_capacity(raw_stored.len());
    for item in raw_stored {
        if item.key == RESERVED_MODULE_SDKS_KEY {
            if let Some(raw_map) = &item.value {
                if let Ok(parsed) = serde_json::from_str::<BTreeMap<String, String>>(raw_map) {
                    module_sdks = sanitize_module_sdks(&parsed);
                }
            }
        } else if item.key == RESERVED_NETBRIDGE_KEY {
            if let Some(raw_nb) = &item.value {
                if let Ok(parsed) = serde_json::from_str::<IdeNetworkBridgeConfig>(raw_nb) {
                    network_bridge = sanitize_network_bridge(Some(parsed)).ok().flatten();
                }
            }
        } else {
            stored.push(item);
        }
    }
    let roots_json: String = row
        .try_get("additional_roots_json")
        .unwrap_or_else(|_| "[]".into());
    let additional_roots: Vec<String> = serde_json::from_str(&roots_json).unwrap_or_default();
    let inherit_shell_env = row.try_get::<i64, _>("inherit_shell_env").unwrap_or(1) != 0;

    let env = IdeProjectEnvironment {
        java_home: row.try_get("java_home").ok().flatten(),
        go_root: row.try_get("go_root").ok().flatten(),
        rust_toolchain: row.try_get("rust_toolchain").ok().flatten(),
        node_home: row.try_get("node_home").ok().flatten(),
        python_interpreter: row.try_get("python_interpreter").ok().flatten(),
        dotenv_path: row.try_get("dotenv_path").ok().flatten(),
        extra_path: row.try_get("extra_path").ok().flatten(),
        inherit_shell_env,
        additional_roots,
        module_sdks,
        network_bridge,
        env_vars: Vec::new(),
    };
    (env, stored)
}

pub(super) async fn load_stored_project_env(
    pool: &DbPool,
    project_key: &str,
) -> Result<(IdeProjectEnvironment, Vec<StoredEnvVar>)> {
    let row = sqlx::query(
        r#"SELECT java_home, go_root, rust_toolchain, node_home, python_interpreter,
                  dotenv_path, extra_path, env_vars_json, additional_roots_json, inherit_shell_env
           FROM ide_project_environments WHERE project_key = ?"#,
    )
    .bind(project_key)
    .fetch_optional(pool)
    .await
    .map_err(|e| Error::Internal(format!("query ide_project_environments: {e}")))?;

    let Some(row) = row else {
        return Ok((IdeProjectEnvironment::default(), Vec::new()));
    };
    Ok(decode_project_env_row(&row))
}

pub(super) async fn load_all_stored_project_envs(
    pool: &DbPool,
) -> Result<Vec<(String, IdeProjectEnvironment, Vec<StoredEnvVar>)>> {
    let rows = sqlx::query(
        r#"SELECT project_key, java_home, go_root, rust_toolchain, node_home, python_interpreter,
                  dotenv_path, extra_path, env_vars_json, additional_roots_json, inherit_shell_env
           FROM ide_project_environments"#,
    )
    .fetch_all(pool)
    .await
    .map_err(|e| Error::Internal(format!("query all ide_project_environments: {e}")))?;

    let mut out = Vec::with_capacity(rows.len());
    for row in &rows {
        let key: String = row.try_get("project_key").unwrap_or_default();
        if !key.is_empty() {
            let (env, stored) = decode_project_env_row(row);
            out.push((key, env, stored));
        }
    }
    Ok(out)
}

pub(super) fn row_to_run_config(row: &sqlx::sqlite::SqliteRow) -> IdeRunConfig {
    let runner_s: String = row.try_get("runner").unwrap_or_else(|_| "custom".into());
    let runner: IdeRunnerKind = serde_json::from_value(serde_json::Value::String(runner_s))
        .unwrap_or(IdeRunnerKind::Custom);
    let args_s: String = row.try_get("args_json").unwrap_or_else(|_| "[]".into());
    let flags_s: String = row
        .try_get("build_flags_json")
        .unwrap_or_else(|_| "[]".into());
    let env_s: String = row
        .try_get("env_overrides_json")
        .unwrap_or_else(|_| "{}".into());
    let compound_s: String = row
        .try_get("compound_ids_json")
        .unwrap_or_else(|_| "[]".into());
    let maven_s: String = row
        .try_get("maven_profiles_json")
        .unwrap_or_else(|_| "[]".into());
    let spring_s: String = row
        .try_get("spring_profiles_json")
        .unwrap_or_else(|_| "[]".into());
    let inherit_shell_env = row.try_get::<i64, _>("inherit_shell_env").unwrap_or(1) != 0;
    let skip_tests = row.try_get::<i64, _>("skip_tests").unwrap_or(0) != 0;
    let offline = row.try_get::<i64, _>("offline").unwrap_or(0) != 0;

    IdeRunConfig {
        id: row.try_get("id").unwrap_or_default(),
        name: row.try_get("name").unwrap_or_default(),
        runner,
        command: row.try_get("command").ok().flatten(),
        args: serde_json::from_str(&args_s).unwrap_or_default(),
        build_flags: serde_json::from_str(&flags_s).unwrap_or_default(),
        cwd: row.try_get("cwd").ok().flatten(),
        env_overrides: serde_json::from_str(&env_s).unwrap_or_default(),
        env_file: row.try_get("env_file").ok().flatten(),
        inherit_shell_env,
        sdk_path: row.try_get("sdk_path").ok().flatten(),
        toolchain_id: row.try_get("toolchain_id").ok().flatten(),
        before_launch: row.try_get("before_launch").ok().flatten(),
        target_path: row.try_get("target_path").ok().flatten(),
        test_filter: row.try_get("test_filter").ok().flatten(),
        compound_configs: serde_json::from_str(&compound_s).unwrap_or_default(),
        maven_profiles: serde_json::from_str(&maven_s).unwrap_or_default(),
        spring_profiles: serde_json::from_str(&spring_s).unwrap_or_default(),
        skip_tests,
        offline,
        auto_detected: false,
    }
}

pub(super) fn discover_local_dotenv_files(root: &Path) -> Vec<String> {
    const CANDIDATES: &[&str] = &[
        ".env",
        ".env.local",
        ".env.development",
        ".env.test",
        ".env.staging",
        ".env.production",
    ];
    CANDIDATES
        .iter()
        .filter(|c| root.join(c).is_file())
        .map(|c| (*c).to_string())
        .collect()
}

pub(super) fn discover_local_python_venvs(root: &Path, data_dir: &Path) -> Vec<IdePythonVenvInfo> {
    let mut out = Vec::new();
    for dir_name in [".venv", "venv", "env", ".env"] {
        let p = root.join(dir_name);
        if p.join("bin/python").is_file() {
            let kind = if p.join("pyvenv.cfg").is_file()
                && std::fs::read_to_string(p.join("pyvenv.cfg"))
                    .map(|c| c.contains("uv ="))
                    .unwrap_or(false)
            {
                "uv"
            } else {
                "venv"
            };
            out.push(IdePythonVenvInfo {
                version: read_pyvenv_version(&p).unwrap_or_else(|| "3".into()),
                path: p.to_string_lossy().into_owned(),
                kind: kind.into(),
            });
        }
    }
    for py_dir in read_subdirs(&data_dir.join("toolchains/python/interpreters"))
        .into_iter()
        .take(16)
    {
        if py_dir.join("bin/python3").is_file() {
            out.push(IdePythonVenvInfo {
                version: py_dir
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("3")
                    .to_string(),
                path: py_dir.to_string_lossy().into_owned(),
                kind: "uv".into(),
            });
        }
    }
    if !otto_core::e2e::isolated() {
        if let Some(home) = dirs::home_dir() {
            for (kind, parent) in [
                ("conda", home.join(".conda/envs")),
                ("conda", home.join("miniconda3/envs")),
                ("poetry", home.join("Library/Caches/pypoetry/virtualenvs")),
                ("poetry", home.join(".cache/pypoetry/virtualenvs")),
            ] {
                for d in read_subdirs(&parent).into_iter().take(12) {
                    if d.join("bin/python").is_file() {
                        out.push(IdePythonVenvInfo {
                            version: read_pyvenv_version(&d).unwrap_or_else(|| "3".into()),
                            path: d.to_string_lossy().into_owned(),
                            kind: kind.into(),
                        });
                    }
                }
            }
        }
    }
    out
}

pub(super) fn read_pyvenv_version(venv: &Path) -> Option<String> {
    let cfg = std::fs::read_to_string(venv.join("pyvenv.cfg")).ok()?;
    cfg.lines()
        .find_map(|l| {
            let (k, v) = l.split_once('=')?;
            matches!(k.trim(), "version" | "version_info").then(|| v.trim().to_string())
        })
        .filter(|s| !s.is_empty())
}
