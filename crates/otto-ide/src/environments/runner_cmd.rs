//! Command-line and working-directory builders for multi-language IDE Run Configurations.

#[path = "runner_java.rs"]
mod runner_java;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(super) use self::runner_java::{
    apply_runtime_profile_and_module_venv, compile_java_main_step, has_manual_classpath,
};
use crate::go_build_cache::{ensure_go_binary, GoBuildMode, GoBuildRequest};
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

pub(super) fn is_default_go_command(cfg: &IdeRunConfig) -> bool {
    let default_cmd = cfg
        .command
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .is_none_or(|c| c == "go");
    if !default_cmd {
        return false;
    }
    let has_target_path = cfg
        .target_path
        .as_deref()
        .map(str::trim)
        .is_some_and(|s| !s.is_empty());
    if has_target_path {
        return true;
    }
    let rest = if cfg.args.first().map(String::as_str) == Some("run") {
        &cfg.args[1..]
    } else {
        &cfg.args[..]
    };
    !rest.first().is_some_and(|a| a.starts_with('-'))
}

pub(super) async fn compile_go_run_step(
    cfg: &IdeRunConfig,
    target: &IdeTargetSpec,
    cwd: &str,
    env_map: &BTreeMap<String, String>,
) -> std::result::Result<(String, Vec<String>), String> {
    let mut args = cfg.args.clone();
    if args.first().map(String::as_str) == Some("run") {
        args.remove(0);
    }
    let pkg_spec = if let Some(tp) = cfg
        .target_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if args.first().map(String::as_str) == Some(tp) {
            args.remove(0);
        }
        tp.to_string()
    } else if !args.is_empty() {
        args.remove(0)
    } else {
        ".".to_string()
    };

    let mut allowed_roots = vec![PathBuf::from(&target.root)];
    for extra in target.merged_extra_roots() {
        let p = PathBuf::from(extra);
        if !allowed_roots.contains(&p) {
            allowed_roots.push(p);
        }
    }
    let go_bin = env_map
        .get("GOROOT")
        .map(|gr| Path::new(gr).join("bin/go"))
        .filter(|p| is_executable_file(p))
        .unwrap_or_else(|| resolve_program_in_env("go", env_map))
        .to_string_lossy()
        .into_owned();
    let build_flags: Vec<String> = cfg
        .build_flags
        .iter()
        .map(|f| f.trim().to_string())
        .filter(|f| !f.is_empty())
        .collect();
    let artifact = ensure_go_binary(GoBuildRequest {
        go_bin: &go_bin,
        cwd: Path::new(cwd),
        pkg_spec: &pkg_spec,
        mode: GoBuildMode::Run,
        extra_build_flags: &build_flags,
        env_map,
        allowed_roots: &allowed_roots,
    })
    .await
    .map_err(|e| e.to_string())?;

    Ok((artifact.binary_path.to_string_lossy().into_owned(), args))
}

pub(super) fn build_runner_command(
    cfg: &IdeRunConfig,
    target_kind: IdeTargetKind,
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
            // Never inspect the local host filesystem for remote SSH runs; invoke
            // `python -m pytest` on the remote host so `-k <filter>` is preserved.
            if target_kind == IdeTargetKind::Ssh {
                let mut p_args = vec!["-m".into(), "pytest".into()];
                if !args.iter().any(|a| a == "-v" || a == "-q") {
                    p_args.push("-v".into());
                }
                p_args.extend(build_flags);
                if let Some(f) = filter {
                    if !args.iter().any(|a| a == "-k") {
                        p_args.extend(["-k".into(), f.to_string()]);
                    }
                }
                p_args.extend(args);
                return (py, p_args);
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
        IdeRunnerKind::Python | IdeRunnerKind::Node => {
            let default_prog = if cfg.runner == IdeRunnerKind::Python {
                env_map
                    .get("VIRTUAL_ENV")
                    .map(|v| format!("{v}/bin/python"))
                    .unwrap_or_else(|| "python3".into())
            } else {
                "node".into()
            };
            let prog = cmd_opt.map(str::to_string).unwrap_or(default_prog);
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
            if let Some(explicit) = cmd_opt {
                return (explicit.to_string(), args);
            }
            if target_kind == IdeTargetKind::Ssh {
                let mut sh_args = vec![
                    "-c".into(),
                    "if [ -x ./mvnw ]; then exec ./mvnw \"$@\"; else exec mvn \"$@\"; fi".into(),
                    "otto-mvn".into(),
                ];
                sh_args.extend(args);
                return ("sh".into(), sh_args);
            }
            let mvnw = Path::new(cwd).join("mvnw");
            let prog = if is_executable_file(&mvnw) {
                "./mvnw"
            } else {
                "mvn"
            };
            (prog.into(), args)
        }
        IdeRunnerKind::Gradle => {
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
            if let Some(explicit) = cmd_opt {
                return (explicit.to_string(), args);
            }
            if target_kind == IdeTargetKind::Ssh {
                let mut sh_args = vec![
                    "-c".into(),
                    "if [ -x ./gradlew ]; then exec ./gradlew \"$@\"; else exec gradle \"$@\"; fi"
                        .into(),
                    "otto-gradle".into(),
                ];
                sh_args.extend(args);
                return ("sh".into(), sh_args);
            }
            let gradlew = Path::new(cwd).join("gradlew");
            let prog = if is_executable_file(&gradlew) {
                "./gradlew"
            } else {
                "gradle"
            };
            (prog.into(), args)
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
