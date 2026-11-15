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

#[allow(clippy::disallowed_methods)]
fn extract_local_go_target_tags(
    target_kind: IdeTargetKind,
    cwd: &str,
    target_path: Option<&str>,
    args: &[String],
) -> Vec<String> {
    if target_kind != IdeTargetKind::Local {
        return Vec::new();
    }
    let mut tags = Vec::new();
    let mut candidates: Vec<&str> = Vec::new();
    if let Some(tp) = target_path.map(str::trim).filter(|s| !s.is_empty()) {
        candidates.push(tp);
    }
    for arg in args {
        if arg == "--" {
            break;
        }
        let t = arg.trim();
        if t.ends_with(".go") && !t.starts_with('-') {
            candidates.push(t);
        }
    }
    let cwd_path = Path::new(cwd);
    for cand in candidates {
        if !cand.ends_with(".go") {
            continue;
        }
        let resolved = if Path::new(cand).is_absolute() {
            PathBuf::from(cand)
        } else {
            cwd_path.join(cand)
        };
        if let Ok(src) = std::fs::read_to_string(&resolved) {
            for tag in crate::lsp_service::extract_go_file_build_tags(&src) {
                if !tags.contains(&tag) {
                    tags.push(tag);
                }
            }
        }
    }
    tags
}

fn split_and_strip_go_tags_before_sep(
    items: Vec<String>,
    allow_sep: bool,
) -> (Vec<String>, Vec<String>) {
    let mut clean = Vec::with_capacity(items.len());
    let mut extracted = Vec::new();
    let mut idx = 0;
    let mut past_sep = false;
    while idx < items.len() {
        let tok = &items[idx];
        if allow_sep && tok == "--" {
            past_sep = true;
            clean.push(tok.clone());
            idx += 1;
            continue;
        }
        if !past_sep {
            if let Some(rest) = tok
                .strip_prefix("-tags=")
                .or_else(|| tok.strip_prefix("--tags="))
            {
                for part in rest.split(|c: char| c == ',' || c.is_whitespace()) {
                    let p = part.trim();
                    if !p.is_empty() && !extracted.iter().any(|e| e == p) {
                        extracted.push(p.to_string());
                    }
                }
                idx += 1;
                continue;
            }
            if tok == "-tags" || tok == "--tags" {
                if let Some(next) = items.get(idx + 1) {
                    for part in next.split(|c: char| c == ',' || c.is_whitespace()) {
                        let p = part.trim();
                        if !p.is_empty() && !extracted.iter().any(|e| e == p) {
                            extracted.push(p.to_string());
                        }
                    }
                    idx += 2;
                    continue;
                }
            }
        }
        clean.push(tok.clone());
        idx += 1;
    }
    (clean, extracted)
}

fn unify_go_build_flags_and_args(
    build_flags: Vec<String>,
    args: Vec<String>,
    file_tags: &[String],
) -> (Vec<String>, Vec<String>) {
    let (mut clean_flags, mut merged_tags) = split_and_strip_go_tags_before_sep(build_flags, false);
    let (clean_args, arg_tags) = split_and_strip_go_tags_before_sep(args, true);
    for t in arg_tags.into_iter().chain(file_tags.iter().cloned()) {
        if !t.is_empty() && !merged_tags.contains(&t) {
            merged_tags.push(t);
        }
    }
    crate::lsp_service::merge_go_tags_into_args(&mut clean_flags, &merged_tags);
    (clean_flags, clean_args)
}

#[allow(clippy::disallowed_methods)]
fn local_python_has_pytest_cov(cwd: &str, env_map: &BTreeMap<String, String>) -> bool {
    let mut roots: Vec<PathBuf> = Vec::new();
    if let Some(venv) = env_map.get("VIRTUAL_ENV").filter(|s| !s.trim().is_empty()) {
        roots.push(PathBuf::from(venv));
    }
    let cwd_p = Path::new(cwd);
    roots.push(cwd_p.join(".venv"));
    roots.push(cwd_p.join("venv"));
    for root in roots {
        let lib_dir = root.join("lib");
        if let Ok(rd) = std::fs::read_dir(&lib_dir) {
            for entry in rd.flatten() {
                let sp = entry.path().join("site-packages");
                if sp.join("pytest_cov").is_dir() {
                    return true;
                }
            }
        }
        if root.join("Lib/site-packages/pytest_cov").is_dir() {
            return true;
        }
    }
    false
}

#[allow(clippy::disallowed_methods)]
fn read_local_manifest_head(target_kind: IdeTargetKind, cwd: &str, rel: &str) -> Option<String> {
    if target_kind != IdeTargetKind::Local {
        return None;
    }
    std::fs::read_to_string(Path::new(cwd).join(rel)).ok()
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
    let raw_build_flags: Vec<String> = cfg
        .build_flags
        .iter()
        .map(|f| f.trim().to_string())
        .filter(|f| !f.is_empty())
        .collect();
    let file_tags = extract_local_go_target_tags(target.kind, cwd, Some(&pkg_spec), &args);
    let (build_flags, args) = unify_go_build_flags_and_args(raw_build_flags, args, &file_tags);
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
            if args.first().map(String::as_str) == Some("test") {
                args.remove(0);
            }
            let file_tags =
                extract_local_go_target_tags(target_kind, cwd, cfg.target_path.as_deref(), &args);
            let (build_flags, args) = unify_go_build_flags_and_args(build_flags, args, &file_tags);
            let mut out = vec!["test".into()];
            if !args.iter().any(|a| a == "-v") && !build_flags.iter().any(|a| a == "-v") {
                out.push("-v".into());
            }
            if cfg.coverage
                && !args
                    .iter()
                    .any(|a| a == "-cover" || a.starts_with("-coverprofile"))
                && !build_flags
                    .iter()
                    .any(|a| a == "-cover" || a.starts_with("-coverprofile"))
            {
                out.push("-cover".into());
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
            let file_tags =
                extract_local_go_target_tags(target_kind, cwd, cfg.target_path.as_deref(), &args);
            let (build_flags, args) = unify_go_build_flags_and_args(build_flags, args, &file_tags);
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
            let file_tags =
                extract_local_go_target_tags(target_kind, cwd, cfg.target_path.as_deref(), &args);
            let (build_flags, args) = unify_go_build_flags_and_args(build_flags, args, &file_tags);
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
            let use_llvm_cov = cfg.coverage
                && cmd_opt.is_none()
                && is_executable_file(&resolve_program_in_env("cargo-llvm-cov", env_map));
            let mut out = vec![if use_llvm_cov { "llvm-cov" } else { "test" }.into()];
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
            let has_cov_flag = args.iter().any(|a| a == "--cov" || a.starts_with("--cov="))
                || build_flags
                    .iter()
                    .any(|a| a == "--cov" || a.starts_with("--cov="));
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
                if cfg.coverage && !has_cov_flag && local_python_has_pytest_cov(cwd, env_map) {
                    args.extend([
                        "--cov=.".into(),
                        "--cov-report=term-missing:skip-covered".into(),
                    ]);
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
        IdeRunnerKind::Npm => {
            let mut npm_args = if args.is_empty() {
                vec!["test".into()]
            } else {
                args
            };
            if cfg.coverage && (npm_args == ["test"] || npm_args == ["run", "test"]) {
                if let Some(pkg_json) = read_local_manifest_head(target_kind, cwd, "package.json") {
                    let has_test_cov_script = serde_json::from_str::<serde_json::Value>(&pkg_json)
                        .ok()
                        .and_then(|v| v.get("scripts")?.get("test:coverage").cloned())
                        .is_some();
                    if has_test_cov_script {
                        npm_args = vec!["run".into(), "test:coverage".into()];
                    } else if pkg_json.contains("vitest")
                        || pkg_json.contains("jest")
                        || pkg_json.contains("\"c8\"")
                    {
                        npm_args = vec!["test".into(), "--".into(), "--coverage".into()];
                    }
                }
            }
            (cmd_opt.unwrap_or("npm").to_string(), npm_args)
        }
        IdeRunnerKind::Vitest => {
            let prog = cmd_opt.unwrap_or("npx").to_string();
            if args.first().map(String::as_str) != Some("vitest") {
                let mut v = vec!["vitest".into(), "run".into()];
                v.extend(build_flags);
                v.extend(args);
                args = v;
            }
            if cfg.coverage
                && !args
                    .iter()
                    .any(|a| a == "--coverage" || a.starts_with("--coverage="))
            {
                args.push("--coverage".into());
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
            if cfg.coverage
                && read_local_manifest_head(target_kind, cwd, "pom.xml")
                    .is_some_and(|s| s.to_ascii_lowercase().contains("jacoco"))
            {
                if !args.iter().any(|a| a.starts_with("-Djacoco.skip=")) {
                    args.push("-Djacoco.skip=false".into());
                }
                if !args.iter().any(|a| a.contains("jacoco:report")) {
                    args.push("jacoco:report".into());
                }
            }
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
            if cfg.coverage {
                let has_jacoco = read_local_manifest_head(target_kind, cwd, "build.gradle")
                    .is_some_and(|s| s.to_ascii_lowercase().contains("jacoco"))
                    || read_local_manifest_head(target_kind, cwd, "build.gradle.kts")
                        .is_some_and(|s| s.to_ascii_lowercase().contains("jacoco"));
                if has_jacoco && !args.iter().any(|a| a == "jacocoTestReport") {
                    args.push("jacocoTestReport".into());
                }
            }
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
