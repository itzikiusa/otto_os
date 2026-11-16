//! Per-ecosystem command builders (`Pytest`, `Python`/`Node`, `Npm`, `Vitest`,
//! `JavaMain`, `JavaJunit`/`Maven`, `Gradle`, `Shell`, `Compound`/`Custom`) for
//! [`super::build_runner_command`].

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use super::runner_java::GRADLE_BUILD_FILES;
use super::{csv_list, resolve_program_in_env};
use crate::toolchains::is_executable_file;
use crate::types::{IdeRunConfig, IdeRunnerKind, IdeTargetKind};

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

#[allow(clippy::too_many_arguments)]
pub(super) fn build_pytest_cmd(
    cfg: &IdeRunConfig,
    target_kind: IdeTargetKind,
    cwd: &str,
    env_map: &BTreeMap<String, String>,
    cmd_opt: Option<&str>,
    mut args: Vec<String>,
    build_flags: Vec<String>,
    filter: Option<&str>,
) -> (String, Vec<String>) {
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

pub(super) fn build_script_cmd(
    cfg: &IdeRunConfig,
    env_map: &BTreeMap<String, String>,
    cmd_opt: Option<&str>,
    args: Vec<String>,
    build_flags: Vec<String>,
) -> (String, Vec<String>) {
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

pub(super) fn build_npm_cmd(
    cfg: &IdeRunConfig,
    target_kind: IdeTargetKind,
    cwd: &str,
    cmd_opt: Option<&str>,
    args: Vec<String>,
) -> (String, Vec<String>) {
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

pub(super) fn build_vitest_cmd(
    cfg: &IdeRunConfig,
    cmd_opt: Option<&str>,
    mut args: Vec<String>,
    build_flags: Vec<String>,
) -> (String, Vec<String>) {
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

pub(super) fn build_java_main_cmd(
    cfg: &IdeRunConfig,
    cmd_opt: Option<&str>,
    args: Vec<String>,
    build_flags: Vec<String>,
    filter: Option<&str>,
) -> (String, Vec<String>) {
    let spring_csv = csv_list(&cfg.spring_profiles);
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

pub(super) fn build_maven_cmd(
    cfg: &IdeRunConfig,
    target_kind: IdeTargetKind,
    cwd: &str,
    cmd_opt: Option<&str>,
    mut args: Vec<String>,
    build_flags: Vec<String>,
    filter: Option<&str>,
) -> (String, Vec<String>) {
    let mvn_csv = csv_list(&cfg.maven_profiles);
    let spring_csv = csv_list(&cfg.spring_profiles);
    if args.is_empty() {
        args.push("test".into());
    }
    args.extend(build_flags);
    let pom = read_local_manifest_head(target_kind, cwd, "pom.xml");
    if cfg.coverage
        && pom
            .as_deref()
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
        // From a reactor root, modules without the test must not fail the build.
        if pom.as_deref().is_some_and(|s| s.contains("<modules>")) {
            args.push("-Dsurefire.failIfNoSpecifiedTests=false".into());
        }
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
    let prog = find_build_wrapper(cwd, "mvnw", &["pom.xml"]).unwrap_or_else(|| "mvn".into());
    (prog, args)
}

pub(super) fn build_gradle_cmd(
    cfg: &IdeRunConfig,
    target_kind: IdeTargetKind,
    cwd: &str,
    cmd_opt: Option<&str>,
    mut args: Vec<String>,
    build_flags: Vec<String>,
    filter: Option<&str>,
) -> (String, Vec<String>) {
    let mvn_csv = csv_list(&cfg.maven_profiles);
    let spring_csv = csv_list(&cfg.spring_profiles);
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
        // Gradle selects a method as `Class.method`, not JUnit/Surefire's `Class#method`.
        args.extend(["--tests".into(), f.replace('#', ".")]);
    }
    if let Some(explicit) = cmd_opt {
        return (explicit.to_string(), args);
    }
    if target_kind == IdeTargetKind::Ssh {
        let mut sh_args = vec![
            "-c".into(),
            "if [ -x ./gradlew ]; then exec ./gradlew \"$@\"; else exec gradle \"$@\"; fi".into(),
            "otto-gradle".into(),
        ];
        sh_args.extend(args);
        return ("sh".into(), sh_args);
    }
    let prog =
        find_build_wrapper(cwd, "gradlew", &GRADLE_BUILD_FILES).unwrap_or_else(|| "gradle".into());
    (prog, args)
}

/// The build wrapper lives at the top of a multi-module build, above the module the build
/// runs in: look in `cwd`, then up while the parent still belongs to the build.
pub(crate) fn find_build_wrapper(cwd: &str, wrapper: &str, build_files: &[&str]) -> Option<String> {
    let mut dir = Path::new(cwd);
    loop {
        let candidate = dir.join(wrapper);
        if is_executable_file(&candidate) {
            return Some(if dir == Path::new(cwd) {
                format!("./{wrapper}")
            } else {
                candidate.to_string_lossy().into_owned()
            });
        }
        dir = dir.parent()?;
        if !build_files.iter().any(|f| dir.join(f).is_file()) {
            return None;
        }
    }
}

pub(super) fn build_shell_cmd(
    cfg: &IdeRunConfig,
    cmd_opt: Option<&str>,
    args: Vec<String>,
    build_flags: Vec<String>,
) -> (String, Vec<String>) {
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

pub(super) fn build_custom_cmd(
    cmd_opt: Option<&str>,
    args: Vec<String>,
    build_flags: Vec<String>,
) -> (String, Vec<String>) {
    let c = cmd_opt.unwrap_or("echo");
    if args.is_empty() && c.contains(' ') {
        ("sh".into(), vec!["-c".into(), c.to_string()])
    } else {
        let mut out = build_flags;
        out.extend(args);
        (c.to_string(), out)
    }
}
