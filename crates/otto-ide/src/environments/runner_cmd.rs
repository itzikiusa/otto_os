//! Command-line and working-directory builders for multi-language IDE Run Configurations.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use otto_core::new_id;

use super::runner::{append_run_line, ActiveRunHandle};
use crate::toolchains::{find_executable_in, is_executable_file};
use crate::types::{IdeRunConfig, IdeRunnerKind, IdeTargetKind, IdeTargetSpec};

pub(super) fn resolve_program_in_env(program: &str, env_map: &BTreeMap<String, String>) -> PathBuf {
    if program.contains('/') {
        return PathBuf::from(program);
    }
    if let Some(path_str) = env_map.get("PATH") {
        let dirs: Vec<PathBuf> = std::env::split_paths(path_str).collect();
        if let Some(hit) = find_executable_in(&dirs, &[program]) {
            return hit;
        }
    }
    PathBuf::from(program)
}

pub(super) fn resolve_run_cwd(target: &IdeTargetSpec, raw_cwd: Option<&str>) -> String {
    let Some(c) = raw_cwd
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != ".")
    else {
        return target.root.clone();
    };
    if c.starts_with('/') {
        let extra = target.merged_extra_roots();
        let allowed_roots =
            std::iter::once(target.root.as_str()).chain(extra.iter().map(String::as_str));
        if target.kind == IdeTargetKind::Local {
            if let Ok(canon_c) = std::fs::canonicalize(c) {
                for allowed in allowed_roots.clone() {
                    if let Ok(canon_root) = std::fs::canonicalize(allowed) {
                        if canon_c.starts_with(&canon_root) {
                            return canon_c.to_string_lossy().into_owned();
                        }
                    }
                }
            }
        }
        for allowed in allowed_roots {
            if c == allowed || c.starts_with(&format!("{}/", allowed.trim_end_matches('/'))) {
                return c.to_string();
            }
        }
        return target.root.clone();
    }
    if c.contains("..") {
        return target.root.clone();
    }
    let rel = c.trim_start_matches("./");
    format!("{}/{rel}", target.root.trim_end_matches('/'))
}

fn csv_list(items: &[String]) -> String {
    items
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(",")
}

pub(super) fn build_runner_command(
    cfg: &IdeRunConfig,
    cwd: &str,
    env_map: &BTreeMap<String, String>,
) -> (String, Vec<String>) {
    let cmd_opt = cfg
        .command
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mut args = cfg.args.clone();
    let build_flags: Vec<String> = cfg
        .build_flags
        .iter()
        .map(|f| f.trim().to_string())
        .filter(|f| !f.is_empty())
        .collect();
    let filter = cfg
        .test_filter
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let mvn_csv = csv_list(&cfg.maven_profiles);
    let spring_csv = csv_list(&cfg.spring_profiles);

    match cfg.runner {
        IdeRunnerKind::GoTest => {
            let prog = cmd_opt.unwrap_or("go").to_string();
            let mut out = Vec::new();
            if args.first().map(String::as_str) == Some("test") {
                args.remove(0);
            }
            out.push("test".into());
            if !args.iter().any(|a| a == "-v") && !build_flags.iter().any(|a| a == "-v") {
                out.push("-v".into());
            }
            out.extend(build_flags);
            if let Some(f) = filter {
                if !args.iter().any(|a| a == "-run") {
                    out.extend(["-run".into(), f.to_string()]);
                }
            }
            if args.is_empty() {
                out.push(cfg.target_path.clone().unwrap_or_else(|| "./...".into()));
            } else {
                out.extend(args);
            }
            (prog, out)
        }
        IdeRunnerKind::GoRun => {
            let prog = cmd_opt.unwrap_or("go").to_string();
            if args.first().map(String::as_str) == Some("run") {
                args.remove(0);
            }
            let mut out = vec!["run".into()];
            out.extend(build_flags);
            if args.is_empty() {
                out.push(cfg.target_path.clone().unwrap_or_else(|| ".".into()));
            } else {
                out.extend(args);
            }
            (prog, out)
        }
        IdeRunnerKind::GoBuild => {
            let prog = cmd_opt.unwrap_or("go").to_string();
            if args.first().map(String::as_str) == Some("build") {
                args.remove(0);
            }
            let mut out = vec!["build".into()];
            out.extend(build_flags);
            if args.is_empty() {
                out.push(cfg.target_path.clone().unwrap_or_else(|| ".".into()));
            } else {
                out.extend(args);
            }
            (prog, out)
        }
        IdeRunnerKind::CargoTest => {
            let prog = cmd_opt.unwrap_or("cargo").to_string();
            if args.first().map(String::as_str) == Some("test") {
                args.remove(0);
            }
            let mut out = vec!["test".into()];
            out.extend(build_flags);
            if let Some(f) = filter {
                if !args.iter().any(|a| a == f) {
                    args.insert(0, f.to_string());
                }
            }
            out.extend(args);
            (prog, out)
        }
        IdeRunnerKind::CargoRun => {
            let prog = cmd_opt.unwrap_or("cargo").to_string();
            if args.first().map(String::as_str) == Some("run") {
                args.remove(0);
            }
            let mut out = vec!["run".into()];
            out.extend(build_flags);
            out.extend(args);
            (prog, out)
        }
        IdeRunnerKind::Pytest => {
            let py = env_map
                .get("VIRTUAL_ENV")
                .map(|v| format!("{v}/bin/python"))
                .unwrap_or_else(|| "python3".into());
            if let Some(explicit) = cmd_opt {
                return (explicit.to_string(), args);
            }
            let pytest_bin = resolve_program_in_env("pytest", env_map);
            if is_executable_file(&pytest_bin) {
                if !args.iter().any(|a| a == "-v") {
                    args.insert(0, "-v".into());
                }
                args.extend(build_flags);
                if let Some(f) = filter {
                    if !args.iter().any(|a| a == f) {
                        args.extend(["-k".into(), f.to_string()]);
                    }
                }
                ("pytest".into(), args)
            } else {
                let mut u = vec![
                    "-m".into(),
                    "unittest".into(),
                    "discover".into(),
                    "-v".into(),
                ];
                u.extend(build_flags);
                u.extend(args);
                (py, u)
            }
        }
        IdeRunnerKind::Python => {
            let default_py = env_map
                .get("VIRTUAL_ENV")
                .map(|v| format!("{v}/bin/python"))
                .unwrap_or_else(|| "python3".into());
            let prog = cmd_opt.map(str::to_string).unwrap_or(default_py);
            let mut out = build_flags;
            if args.is_empty() {
                if let Some(tp) = &cfg.target_path {
                    out.push(tp.clone());
                }
            } else {
                out.extend(args);
            }
            (prog, out)
        }
        IdeRunnerKind::Node => {
            let prog = cmd_opt.unwrap_or("node").to_string();
            let mut out = build_flags;
            if args.is_empty() {
                if let Some(tp) = &cfg.target_path {
                    out.push(tp.clone());
                }
            } else {
                out.extend(args);
            }
            (prog, out)
        }
        IdeRunnerKind::Npm => (
            cmd_opt.unwrap_or("npm").to_string(),
            if args.is_empty() {
                vec!["test".into()]
            } else {
                args
            },
        ),
        IdeRunnerKind::Vitest => {
            let prog = cmd_opt.unwrap_or("npx").to_string();
            if args.first().map(String::as_str) != Some("vitest") {
                let mut v = vec!["vitest".into(), "run".into()];
                v.extend(build_flags);
                v.extend(args);
                args = v;
            }
            (prog, args)
        }
        IdeRunnerKind::JavaMain => {
            let prog = cmd_opt.unwrap_or("java").to_string();
            let mut out = build_flags;
            if !spring_csv.is_empty()
                && !out
                    .iter()
                    .any(|a| a.starts_with("-Dspring.profiles.active="))
            {
                out.push(format!("-Dspring.profiles.active={spring_csv}"));
            }
            if args.is_empty() {
                if let Some(fqcn) = filter {
                    out.push(fqcn.to_string());
                } else if let Some(tp) = &cfg.target_path {
                    out.push(tp.clone());
                }
            } else {
                out.extend(args);
            }
            (prog, out)
        }
        IdeRunnerKind::JavaJunit | IdeRunnerKind::Maven => {
            let mvnw = Path::new(cwd).join("mvnw");
            let prog = cmd_opt.map(str::to_string).unwrap_or_else(|| {
                if is_executable_file(&mvnw) {
                    "./mvnw".into()
                } else {
                    "mvn".into()
                }
            });
            if args.is_empty() {
                args.push("test".into());
            }
            args.extend(build_flags);
            if !mvn_csv.is_empty() && !args.iter().any(|a| a.starts_with("-P")) {
                args.push(format!("-P{mvn_csv}"));
            }
            if !spring_csv.is_empty() {
                args.push(format!("-Dspring.profiles.active={spring_csv}"));
                if args.iter().any(|a| a.contains("spring-boot:run")) {
                    args.push(format!("-Dspring-boot.run.profiles={spring_csv}"));
                }
            }
            if cfg.skip_tests && !args.iter().any(|a| a == "-DskipTests") {
                args.push("-DskipTests".into());
            }
            if cfg.offline && !args.iter().any(|a| a == "-o" || a == "--offline") {
                args.push("-o".into());
            }
            if let Some(f) = filter {
                args.push(format!("-Dtest={f}"));
            }
            (prog, args)
        }
        IdeRunnerKind::Gradle => {
            let gradlew = Path::new(cwd).join("gradlew");
            let prog = cmd_opt.map(str::to_string).unwrap_or_else(|| {
                if is_executable_file(&gradlew) {
                    "./gradlew".into()
                } else {
                    "gradle".into()
                }
            });
            if args.is_empty() {
                args.push("test".into());
            }
            args.extend(build_flags);
            if !mvn_csv.is_empty() {
                args.push(format!("-Pprofile={mvn_csv}"));
            }
            if !spring_csv.is_empty() {
                args.push(format!("-Dspring.profiles.active={spring_csv}"));
            }
            if cfg.skip_tests && !args.iter().any(|a| a == "-x") {
                args.extend(["-x".into(), "test".into()]);
            }
            if cfg.offline && !args.iter().any(|a| a == "--offline") {
                args.push("--offline".into());
            }
            if let Some(f) = filter {
                args.extend(["--tests".into(), f.to_string()]);
            }
            (prog, args)
        }
        IdeRunnerKind::Shell => {
            let is_bats = cfg
                .target_path
                .as_deref()
                .is_some_and(|p| p.ends_with(".bats"));
            let prog = cmd_opt
                .unwrap_or(if is_bats { "bats" } else { "bash" })
                .to_string();
            let mut out = build_flags;
            if args.is_empty() {
                if let Some(tp) = &cfg.target_path {
                    out.push(tp.clone());
                }
            } else {
                out.extend(args);
            }
            (prog, out)
        }
        IdeRunnerKind::Compound | IdeRunnerKind::Custom => {
            let c = cmd_opt.unwrap_or("echo");
            if args.is_empty() && c.contains(' ') {
                ("sh".into(), vec!["-c".into(), c.to_string()])
            } else {
                let mut out = build_flags;
                out.extend(args);
                (c.to_string(), out)
            }
        }
    }
}

pub(super) fn apply_runtime_profile_and_module_venv(
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

pub(super) fn has_manual_classpath(cfg: &IdeRunConfig) -> bool {
    cfg.args
        .iter()
        .chain(cfg.build_flags.iter())
        .any(|a| a == "-cp" || a == "-classpath" || a == "--class-path")
}

pub(super) async fn compile_java_main_step(
    cfg: &IdeRunConfig,
    target: &IdeTargetSpec,
    cwd: &str,
    env_map: &BTreeMap<String, String>,
    handle: &ActiveRunHandle,
) -> std::result::Result<(String, Vec<String>, PathBuf), String> {
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
    let out_dir = std::env::temp_dir().join(format!("otto-jrun-{}", new_id()));
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
    Ok((java_prog, jvm_args, out_dir))
}
