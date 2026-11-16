//! Spring Boot profile env injection, per-module `.venv` activation, and
//! persistent incremental Java main compilation for [`super::super::runner`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::super::runner::{append_run_line, ActiveRunHandle};
use super::{csv_list, resolve_program_in_env};
use crate::go_build_cache::ide_build_cache_root;
use crate::toolchains::is_executable_file;
use crate::types::{IdeRunConfig, IdeTargetKind, IdeTargetSpec};

pub(in crate::environments) fn apply_runtime_profile_and_module_venv(
    cfg: &IdeRunConfig,
    target: &IdeTargetSpec,
    cwd: &str,
    env_map: &mut BTreeMap<String, String>,
) {
    let spring_csv = csv_list(&cfg.spring_profiles);
    if !spring_csv.is_empty() && !env_map.contains_key("SPRING_PROFILES_ACTIVE") {
        env_map.insert("SPRING_PROFILES_ACTIVE".into(), spring_csv);
    }
    if target.kind == IdeTargetKind::Local && !env_map.contains_key("VIRTUAL_ENV") {
        for vname in [".venv", "venv", ".env"] {
            let cand = Path::new(cwd).join(vname);
            if cand.join("pyvenv.cfg").is_file() && cand.join("bin/python").is_file() {
                let bin = cand.join("bin").to_string_lossy().into_owned();
                let cur_path = env_map.get("PATH").cloned().unwrap_or_default();
                env_map.insert(
                    "PATH".into(),
                    if cur_path.is_empty() {
                        bin
                    } else {
                        format!("{bin}:{cur_path}")
                    },
                );
                env_map.insert("VIRTUAL_ENV".into(), cand.to_string_lossy().into_owned());
                break;
            }
        }
    }
}

pub(in crate::environments) fn has_manual_classpath(cfg: &IdeRunConfig) -> bool {
    cfg.args
        .iter()
        .chain(cfg.build_flags.iter())
        .any(|a| a == "-cp" || a == "-classpath" || a == "--class-path")
}

pub(in crate::environments) async fn compile_java_main_step(
    cfg: &IdeRunConfig,
    target: &IdeTargetSpec,
    cwd: &str,
    env_map: &BTreeMap<String, String>,
    handle: &ActiveRunHandle,
) -> std::result::Result<(String, Vec<String>), String> {
    let mut roots = vec![PathBuf::from(&target.root)];
    for extra in target.merged_extra_roots() {
        let p = PathBuf::from(extra);
        if !roots.contains(&p) {
            roots.push(p);
        }
    }
    let cwd_path = PathBuf::from(cwd);
    if !roots.iter().any(|r| cwd_path.starts_with(r)) {
        roots.insert(0, cwd_path.clone());
    }

    let target_file_or_dir = cfg
        .target_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|tp| {
            let p = Path::new(tp);
            if p.is_absolute() {
                p.to_path_buf()
            } else {
                cwd_path.join(p)
            }
        })
        .unwrap_or_else(|| cwd_path.clone());

    let mut prog_args = cfg.args.clone();
    let explicit_main = if let Some(tf) = cfg
        .test_filter
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if prog_args.first().map(String::as_str) == Some(tf) {
            prog_args.remove(0);
        }
        Some(tf.to_string())
    } else if cfg.target_path.is_none()
        && prog_args
            .first()
            .is_some_and(|a| !a.starts_with('-') && !a.contains('/'))
    {
        Some(prog_args.remove(0))
    } else {
        None
    };

    let javac_bin = env_map
        .get("JAVA_HOME")
        .map(|jh| Path::new(jh).join("bin/javac"))
        .filter(|p| is_executable_file(p))
        .unwrap_or_else(|| resolve_program_in_env("javac", env_map))
        .to_string_lossy()
        .into_owned();
    let out_dir = ide_build_cache_root()
        .join("java")
        .join(target.project_hash());
    let env_clone = env_map.clone();
    let out_clone = out_dir.clone();

    let plan = tokio::task::spawn_blocking(move || {
        crate::java_workspace::prepare_java_execution(
            &target_file_or_dir,
            explicit_main.as_deref(),
            true,
            &roots,
            &env_clone,
            &javac_bin,
            &out_clone,
        )
    })
    .await
    .map_err(|e| format!("java compile worker panicked: {e}"))?
    .map_err(|e| e.to_string())?;

    if !plan.compiler_output.trim().is_empty() {
        for line in plan.compiler_output.lines() {
            append_run_line(handle, line.to_string());
        }
    }

    let java_prog = cfg
        .command
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            env_map
                .get("JAVA_HOME")
                .map(|jh| format!("{}/bin/java", jh.trim_end_matches('/')))
                .filter(|p| is_executable_file(Path::new(p)))
        })
        .unwrap_or_else(|| "java".into());

    let mut jvm_args: Vec<String> = cfg
        .build_flags
        .iter()
        .map(|f| f.trim().to_string())
        .filter(|f| !f.is_empty())
        .collect();
    let spring_csv = csv_list(&cfg.spring_profiles);
    if !spring_csv.is_empty()
        && !jvm_args
            .iter()
            .any(|a| a.starts_with("-Dspring.profiles.active="))
    {
        jvm_args.push(format!("-Dspring.profiles.active={spring_csv}"));
    }
    jvm_args.extend(["-cp".into(), plan.classpath, plan.main_class]);
    jvm_args.extend(prog_args);
    Ok((java_prog, jvm_args))
}
