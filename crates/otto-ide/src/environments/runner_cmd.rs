//! Command-line and working-directory builders for multi-language IDE Run Configurations.

#[path = "runner_cmd_langs.rs"]
mod runner_cmd_langs;
#[path = "runner_java.rs"]
pub(crate) mod runner_java;
#[path = "runner_kotest.rs"]
pub(crate) mod runner_kotest;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) use self::runner_cmd_langs::find_build_wrapper;
use self::runner_cmd_langs::{
    build_custom_cmd, build_flutter_cmd, build_gradle_cmd, build_java_main_cmd, build_jest_cmd,
    build_karma_cmd, build_maven_cmd, build_npm_cmd, build_playwright_cmd, build_pytest_cmd,
    build_react_native_cmd, build_script_cmd, build_shell_cmd, build_vitest_cmd, build_xcode_cmd,
    ssh_wrapper_exec,
};
pub(crate) use self::runner_java::pytest_rootdir;
pub(super) use self::runner_java::{
    apply_runtime_profile_and_module_venv, compile_java_main_step, has_manual_classpath,
    resolve_jvm_build_dir,
};
use crate::go_build_cache::{ensure_go_binary, GoBuildMode, GoBuildRequest};
use crate::go_build_tags::unify_go_build_flags_and_args;
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
    build_tool_fallback(program, dirs::home_dir().as_deref()).unwrap_or_else(|| program.into())
}

/// `mvn` / `gradle` that the run's PATH lacks (a slow or skipped login-shell capture drops
/// sdkman's init from it): sdkman's current candidate, then Homebrew, as the JDK scan does.
pub(super) fn build_tool_fallback(program: &str, home: Option<&Path>) -> Option<PathBuf> {
    let candidate = match program {
        "mvn" => "maven",
        "gradle" => "gradle",
        _ => return None,
    };
    let mut dirs: Vec<PathBuf> = home
        .map(|h| {
            h.join(".sdkman/candidates")
                .join(candidate)
                .join("current/bin")
        })
        .into_iter()
        .collect();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(PathBuf::from));
    find_executable_in(&dirs, &[program])
}

pub(super) fn resolve_run_cwd(target: &IdeTargetSpec, raw_cwd: Option<&str>) -> String {
    let Some(c) = raw_cwd
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != ".")
    else {
        return target.root.clone();
    };
    if c.chars().any(char::is_control) {
        return target.root.clone();
    }
    let extra = target.merged_extra_roots();
    match target.kind {
        IdeTargetKind::Local => {
            let candidate = if Path::new(c).is_absolute() {
                PathBuf::from(c)
            } else {
                if Path::new(c)
                    .components()
                    .any(|comp| matches!(comp, std::path::Component::ParentDir))
                {
                    return target.root.clone();
                }
                Path::new(&target.root).join(c)
            };
            let Some(canon_c) = std::fs::canonicalize(&candidate)
                .ok()
                .or_else(|| otto_core::secret_paths::canonicalize_prospective(&candidate))
            else {
                return target.root.clone();
            };
            let allowed_roots =
                std::iter::once(target.root.as_str()).chain(extra.iter().map(String::as_str));
            for allowed in allowed_roots {
                if let Ok(canon_root) = std::fs::canonicalize(allowed) {
                    if canon_c.starts_with(&canon_root) {
                        return canon_c.to_string_lossy().into_owned();
                    }
                }
            }
            target.root.clone()
        }
        IdeTargetKind::Ssh => {
            crate::target::ssh_exec::resolve_remote_in_roots(&target.root, &extra, c)
                .map(|(resolved, _)| resolved)
                .unwrap_or_else(|_| target.root.clone())
        }
    }
}

pub(super) fn csv_list(items: &[String]) -> String {
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

#[allow(clippy::too_many_arguments)]
fn build_go_runner_cmd(
    subcmd: &'static str,
    default_target: &'static str,
    cfg: &IdeRunConfig,
    target_kind: IdeTargetKind,
    cwd: &str,
    cmd_opt: Option<&str>,
    mut args: Vec<String>,
    build_flags: Vec<String>,
    filter: Option<&str>,
) -> (String, Vec<String>) {
    let prog = cmd_opt.unwrap_or("go").to_string();
    if args.first().map(String::as_str) == Some(subcmd) {
        args.remove(0);
    }
    let file_tags =
        extract_local_go_target_tags(target_kind, cwd, cfg.target_path.as_deref(), &args);
    let (build_flags, args) = unify_go_build_flags_and_args(build_flags, args, &file_tags);
    let mut out = vec![subcmd.into()];
    if subcmd == "test" {
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
    }
    out.extend(build_flags);
    if subcmd == "test" {
        if let Some(f) = filter {
            if !args.iter().any(|a| a == "-run") {
                out.extend(["-run".into(), f.to_string()]);
            }
        }
    }
    if args.is_empty() {
        out.push(
            cfg.target_path
                .clone()
                .unwrap_or_else(|| default_target.into()),
        );
    } else {
        out.extend(args);
    }
    (prog, out)
}

#[allow(clippy::too_many_arguments)]
fn build_cargo_runner_cmd(
    is_test: bool,
    cfg: &IdeRunConfig,
    target_kind: IdeTargetKind,
    env_map: &BTreeMap<String, String>,
    cmd_opt: Option<&str>,
    mut args: Vec<String>,
    build_flags: Vec<String>,
    filter: Option<&str>,
) -> (String, Vec<String>) {
    let prog = cmd_opt.unwrap_or("cargo").to_string();
    let sub = if is_test { "test" } else { "run" };
    if args.first().map(String::as_str) == Some(sub) {
        args.remove(0);
    }
    let use_llvm_cov = is_test
        && cfg.coverage
        && cmd_opt.is_none()
        && target_kind == IdeTargetKind::Local
        && is_executable_file(&resolve_program_in_env("cargo-llvm-cov", env_map));
    let mut out = vec![if use_llvm_cov { "llvm-cov" } else { sub }.into()];
    out.extend(build_flags);
    if is_test {
        if let Some(f) = filter {
            if !args.iter().any(|a| a == f) {
                args.insert(0, f.to_string());
            }
        }
    }
    out.extend(args);
    (prog, out)
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn build_zig_runner_cmd(
    runner: IdeRunnerKind,
    cfg: &IdeRunConfig,
    _cwd: &str,
    cmd_opt: Option<&str>,
    mut args: Vec<String>,
    build_flags: Vec<String>,
    filter: Option<&str>,
) -> (String, Vec<String>) {
    let prog = cmd_opt.unwrap_or("zig").to_string();
    let tp = cfg
        .target_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    match runner {
        IdeRunnerKind::ZigBuild => {
            if args.first().map(String::as_str) == Some("build") {
                args.remove(0);
            }
            let mut out = vec!["build".to_string()];
            if let Some(step) = tp.filter(|s| !s.ends_with(".zig") && !s.contains('/')) {
                if !args.iter().any(|a| a == step) {
                    out.push(step.to_string());
                }
            }
            out.extend(build_flags);
            out.extend(args);
            (prog, out)
        }
        IdeRunnerKind::ZigRun => {
            if args.first().map(String::as_str) == Some("run") {
                args.remove(0);
            }
            let mut out = vec!["run".to_string()];
            out.extend(build_flags);
            if let Some(file) = tp {
                let already = args.first().map(String::as_str) == Some(file);
                if already {
                    args.remove(0);
                }
                out.push(file.to_string());
                if !args.is_empty() {
                    if args.first().map(String::as_str) != Some("--") {
                        out.push("--".to_string());
                    }
                    out.extend(args);
                }
            } else {
                out.extend(args);
            }
            (prog, out)
        }
        IdeRunnerKind::ZigTest => {
            // Project-wide Zig test runs (no `.zig` file and no single-case filter) use
            // `zig build test` without checking local disk (so SSH remote projects work too, BUG-36).
            // When a specific test case `filter` is selected (BUG2-2), NEVER silently drop the
            // filter into unfiltered `zig build test` (which would run failing sibling tests):
            // - if `--build-runner` was injected by `runner.rs` (carrying `OTTO_ZIG_TEST_FILTER`),
            //   preserve `zig build --build-runner ... test`;
            // - otherwise compile and run the selected `.zig` file with `--test-filter` and
            //   the project's transitive `build.zig` module graph (BUG-35 + BUG2-2).
            let has_build_runner = args.iter().any(|a| a == "--build-runner");
            let is_build_test =
                if filter.is_some() && !has_build_runner && tp.is_some_and(|p| p.ends_with(".zig"))
                {
                    false
                } else {
                    args.first().map(String::as_str) == Some("build")
                        || (tp.is_none_or(|p| !p.ends_with(".zig"))
                            && !args.iter().any(|a| a.ends_with(".zig")))
                };
            if is_build_test {
                if args.first().map(String::as_str) == Some("build") {
                    args.remove(0);
                }
                let mut out = vec!["build".to_string()];
                if !args.iter().any(|a| a == "test") {
                    out.push("test".to_string());
                }
                out.extend(build_flags);
                out.extend(args);
                return (prog, out);
            }
            if args.first().map(String::as_str) == Some("build") {
                args.remove(0);
            }
            if args.first().map(String::as_str) == Some("test") {
                args.remove(0);
            }
            let mut out = vec!["test".to_string()];
            out.extend(build_flags);
            if let Some(f) = filter {
                if !args.iter().any(|a| a == "--test-filter") {
                    out.push("--test-filter".to_string());
                    out.push(f.to_string());
                }
            }
            if let Some(file) = tp {
                if !args.iter().any(|a| a == file) {
                    let build_zig_path = Path::new(_cwd).join("build.zig");
                    #[allow(clippy::disallowed_methods)]
                    let graph = std::fs::read_to_string(&build_zig_path)
                        .ok()
                        .map(|src| {
                            crate::dap::adapters_zig::parse_build_zig_module_graph(
                                &src,
                                Path::new(_cwd),
                            )
                        })
                        .unwrap_or_default();
                    crate::dap::adapters_zig::append_zig_source_and_modules(&mut out, file, &graph);
                }
            }
            out.extend(args);
            (prog, out)
        }
        _ => (prog, args),
    }
}

#[allow(clippy::too_many_arguments, clippy::disallowed_methods)]
fn build_cpp_runner_cmd(
    runner: IdeRunnerKind,
    cfg: &IdeRunConfig,
    target_kind: IdeTargetKind,
    cwd: &str,
    env_map: &BTreeMap<String, String>,
    cmd_opt: Option<&str>,
    args: Vec<String>,
    build_flags: Vec<String>,
    filter: Option<&str>,
) -> (String, Vec<String>) {
    let tp = cfg
        .target_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    match runner {
        IdeRunnerKind::Cmake => {
            let prog = cmd_opt.unwrap_or("cmake").to_string();
            let mut out = if args.is_empty() {
                if let Some(preset) = tp
                    .and_then(|t| t.strip_prefix("preset:"))
                    .filter(|p| !p.is_empty())
                {
                    vec!["--build".into(), "--preset".into(), preset.to_string()]
                } else {
                    let build_dir = find_cpp_build_dir(cwd).unwrap_or_else(|| "build".into());
                    let mut v = vec!["--build".into(), build_dir];
                    if let Some(target) = tp.filter(|t| {
                        *t != "CMakeLists.txt" && !is_cpp_source_path(t) && !t.contains('/')
                    }) {
                        v.extend(["--target".into(), target.to_string()]);
                    }
                    v
                }
            } else {
                args
            };
            out.extend(build_flags);
            (prog, out)
        }
        IdeRunnerKind::Ctest => {
            // Direct test binary invocation (e.g., GoogleTest/Catch2 binary or gutter test).
            if let Some(explicit) = cmd_opt.filter(|c| *c != "ctest" && *c != "cmake") {
                let mut out = build_flags;
                out.extend(args);
                if let Some(f) = filter {
                    let has_filter = out.iter().any(|a| {
                        a.starts_with("--gtest_filter=")
                            || a.starts_with("--test-case=")
                            || a == "-R"
                            || a == f
                    });
                    if !has_filter {
                        out.push(cpp_test_filter_arg(cwd, tp, f));
                    }
                }
                return (explicit.to_string(), out);
            }
            // BUG-11: Gutter GoogleTest/Catch2 run inside a CMake project: build the test
            // target and invoke the test executable directly with `--gtest_filter` (GoogleTest)
            // or the test name directly (Catch2, even when the name contains a period).
            if let Some(f) = filter.filter(|_| {
                cmd_opt.is_none()
                    && args.is_empty()
                    && target_kind == IdeTargetKind::Local
                    && Path::new(cwd).join("CMakeLists.txt").is_file()
            }) {
                if let Some(test_target) =
                    infer_cmake_target_for_source(cwd, tp.unwrap_or(""), true)
                {
                    let build_dir = find_cpp_build_dir(cwd).unwrap_or_else(|| "build".into());
                    let mut test_args = vec![cpp_test_filter_arg(cwd, tp, f)];
                    test_args.extend(build_flags);
                    return build_cmake_target_exec_cmd(&build_dir, &test_target, test_args);
                }
            }
            let prog = cmd_opt.unwrap_or("ctest").to_string();
            let mut out = if args.is_empty() {
                let test_dir = tp
                    .filter(|t| *t != "CMakeLists.txt" && !is_cpp_source_path(t))
                    .map(str::to_string)
                    .or_else(|| find_cpp_build_dir(cwd))
                    .unwrap_or_else(|| "build".into());
                vec!["--test-dir".into(), test_dir, "--output-on-failure".into()]
            } else {
                args
            };
            out.extend(build_flags);
            if let Some(f) = filter {
                if !out.iter().any(|a| a == "-R" || a == "--tests-regex") {
                    out.extend(["-R".into(), f.to_string()]);
                }
            }
            (prog, out)
        }
        IdeRunnerKind::Make => {
            let prog = cmd_opt.unwrap_or("make").to_string();
            let mut out = build_flags;
            if let Some(t) = tp {
                if !args.iter().any(|a| a == t) {
                    out.push(t.to_string());
                }
            }
            out.extend(args);
            (prog, out)
        }
        IdeRunnerKind::Meson => {
            let prog = cmd_opt.unwrap_or("meson").to_string();
            let mut out = if args.is_empty() {
                let build_dir = tp
                    .filter(|t| *t != "meson.build" && !is_cpp_source_path(t))
                    .unwrap_or("builddir");
                vec!["compile".into(), "-C".into(), build_dir.to_string()]
            } else {
                args
            };
            out.extend(build_flags);
            (prog, out)
        }
        IdeRunnerKind::CppRun => {
            if let Some(explicit) = cmd_opt {
                let mut out = build_flags;
                out.extend(args);
                return (explicit.to_string(), out);
            }
            let is_source = tp.is_some_and(is_cpp_source_path);
            if let Some(target_name) = tp.filter(|_| !is_source) {
                if target_kind == IdeTargetKind::Local {
                    // BUG2-3: Resolve explicit relative or absolute executable paths against
                    // `cwd` and existing build directories FIRST before treating bare target
                    // names as CMake `--target` rebuilds.
                    if target_name.contains('/') {
                        let direct = if Path::new(target_name).is_absolute() {
                            PathBuf::from(target_name)
                        } else {
                            Path::new(cwd).join(target_name)
                        };
                        if is_executable_file(&direct) {
                            return (direct.to_string_lossy().into_owned(), args);
                        }
                        if let Some(exe) = find_built_cpp_executable(cwd, target_name) {
                            return (exe, args);
                        }
                    }
                    // BUG-12: For bare CMake target names, always run incremental `cmake --build`
                    // before executing so source edits never silently run a stale binary.
                    if !target_name.contains('/') && Path::new(cwd).join("CMakeLists.txt").is_file()
                    {
                        let build_dir = find_cpp_build_dir(cwd).unwrap_or_else(|| "build".into());
                        return build_cmake_target_exec_cmd(&build_dir, target_name, args);
                    }
                    if let Some(exe) = find_built_cpp_executable(cwd, target_name) {
                        return (exe, args);
                    }
                }
                return (format!("./build/{target_name}"), args);
            }
            // BUG-10: Gutter Run beside `main()` inside a CMake project builds & runs the
            // owning CMake target instead of compiling a single `.cpp` file standalone; a
            // project source that maps to no executable fails with the reason, as Debug does.
            let src = tp.unwrap_or("main.cpp");
            if target_kind == IdeTargetKind::Local {
                if Path::new(cwd).join("CMakeLists.txt").is_file() {
                    if let Some(cmake_target) = infer_cmake_target_for_source(cwd, src, false) {
                        let build_dir = find_cpp_build_dir(cwd).unwrap_or_else(|| "build".into());
                        return build_cmake_target_exec_cmd(&build_dir, &cmake_target, args);
                    }
                }
                if let Some(reason) = cpp_unmapped_source_error(cwd, src) {
                    let script = r#"printf '%s\n' "$1" >&2; exit 1"#;
                    return (
                        "sh".into(),
                        vec!["-c".into(), script.into(), "otto-cpp-run".into(), reason],
                    );
                }
            }
            // Compile and run a standalone C or C++ source file cleanly via a temp binary.
            let is_c = src.to_ascii_lowercase().ends_with(".c");
            let compiler = if is_c {
                env_map
                    .get("CC")
                    .filter(|s| !s.trim().is_empty())
                    .map(String::as_str)
                    .unwrap_or("clang")
            } else {
                env_map
                    .get("CXX")
                    .filter(|s| !s.trim().is_empty())
                    .map(String::as_str)
                    .unwrap_or("clang++")
            };
            let std_flag = if is_c { "-std=c17" } else { "-std=c++20" };
            let extra_flags = if build_flags.is_empty() {
                String::new()
            } else {
                format!(" {}", shell_words::join(&build_flags))
            };
            let q_cc = shell_words::quote(compiler);
            let q_src = shell_words::quote(src);
            let script = format!(
                "tmp=$(mktemp \"${{TMPDIR:-/tmp}}/otto-cpp-XXXXXX\") || exit 1; \
                 trap 'rm -f \"$tmp\"' EXIT; \
                 {q_cc} -g -O0 {std_flag}{extra_flags} {q_src} -o \"$tmp\" && \"$tmp\" \"$@\""
            );
            let mut sh_args = vec!["-c".into(), script, "otto-cpp-run".into()];
            sh_args.extend(args);
            ("sh".into(), sh_args)
        }
        IdeRunnerKind::Unreal => build_unreal_runner_cmd(env_map, cmd_opt, args, build_flags),
        _ => (cmd_opt.unwrap_or("cmake").to_string(), args),
    }
}

pub(crate) fn is_cpp_source_path(p: &str) -> bool {
    let l = p.to_ascii_lowercase();
    l.ends_with(".c")
        || l.ends_with(".cpp")
        || l.ends_with(".cc")
        || l.ends_with(".cxx")
        || l.ends_with(".c++")
        || l.ends_with(".h")
        || l.ends_with(".hpp")
}

fn build_cmake_target_exec_cmd(
    build_dir: &str,
    target_name: &str,
    args: Vec<String>,
) -> (String, Vec<String>) {
    let q_bd = shell_words::quote(build_dir);
    let q_t = shell_words::quote(target_name);
    let script = format!(
        "if [ ! -f {q_bd}/CMakeCache.txt ]; then cmake -S . -B {q_bd} -DCMAKE_BUILD_TYPE=Debug -DCMAKE_EXPORT_COMPILE_COMMANDS=ON || exit $?; fi; \
         cmake --build {q_bd} --target {q_t} || exit $?; \
         if [ -x {q_bd}/{q_t} ]; then exec {q_bd}/{q_t} \"$@\"; \
         elif [ -x {q_bd}/Debug/{q_t} ]; then exec {q_bd}/Debug/{q_t} \"$@\"; \
         elif [ -x {q_bd}/bin/{q_t} ]; then exec {q_bd}/bin/{q_t} \"$@\"; \
         else found=$(find {q_bd} -maxdepth 3 -type f -name {q_t} -perm -111 2>/dev/null | head -n 1); \
         [ -n \"$found\" ] && exec \"$found\" \"$@\"; echo \"Built target '{target_name}' not found under {build_dir}\" >&2; exit 1; fi"
    );
    let mut sh_args = vec!["-c".into(), script, "otto-cpp-run".into()];
    sh_args.extend(args);
    ("sh".into(), sh_args)
}

/// The argument that selects one gutter test in its framework's own binary: GoogleTest
/// `--gtest_filter=`, doctest `--test-case=`, Catch2 the bare test name (BUG-11).
#[allow(clippy::disallowed_methods)]
pub(crate) fn cpp_test_filter_arg(cwd: &str, source_path: Option<&str>, filter: &str) -> String {
    static RE_DOCTEST: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static RE_CATCH_SRC: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    static RE_GTEST_SRC: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let src = source_path
        .filter(|s| is_cpp_source_path(s))
        .and_then(|sp| std::fs::read_to_string(Path::new(cwd).join(sp)).ok())
        .unwrap_or_default();
    let re_doctest = RE_DOCTEST.get_or_init(|| {
        regex::Regex::new(r#"(?m)#\s*include\s*[<"](?:[^>"]*/)?doctest\.h"#).unwrap()
    });
    if re_doctest.is_match(&src) {
        return format!("--test-case={filter}");
    }
    // The same whitespace-tolerant macros as `gutter/compiled.rs`, plus the Catch2 v3
    // (`catch2/`) and v2 single-header (`catch.hpp` / `catch_amalgamated.hpp`) includes.
    let re_catch = RE_CATCH_SRC.get_or_init(|| {
        regex::Regex::new(
            r#"(?m)(?:\b(?:TEST_CASE|SCENARIO|TEMPLATE_TEST_CASE|TEST_CASE_METHOD)\s*\(|#\s*include\s*[<"](?:[^>"]*/)?(?:catch2/|catch\.hpp|catch_amalgamated\.hpp))"#,
        )
        .unwrap()
    });
    let re_gtest = RE_GTEST_SRC.get_or_init(|| {
        regex::Regex::new(
            r#"(?m)(?:\b(?:TEST|TEST_F|TEST_P|TYPED_TEST|TYPED_TEST_P)\s*\(|#\s*include\s*[<"]gtest/gtest\.h[>"])"#,
        )
        .unwrap()
    });
    let (is_catch2, is_gtest) = (re_catch.is_match(&src), re_gtest.is_match(&src));
    let gtest = if is_catch2 != is_gtest {
        is_gtest
    } else {
        // Unreadable or ambiguous source: GoogleTest `Suite.Test` has a `.` and no spaces.
        !filter.chars().any(char::is_whitespace) && filter.contains('.')
    };
    if gtest {
        format!("--gtest_filter={filter}")
    } else {
        filter.to_string()
    }
}

struct CmakeTarget {
    name: String,
    is_executable: bool,
    /// Root-relative source paths, each resolved against the `CMakeLists.txt` directory
    /// that listed it (BUG-10); libraries and `target_sources` never make a target
    /// executable (BUG3-2).
    sources: Vec<String>,
}

/// Resolve one source argument of `add_executable` / `add_library` / `target_sources`,
/// declared in the root-relative directory `dir_prefix`, to a root-relative path.
fn cmake_source_rel(dir_prefix: &str, token: &str) -> Option<String> {
    let tok = token.trim_matches('"');
    let (base, rest) = if let Some(r) = tok
        .strip_prefix("${CMAKE_CURRENT_SOURCE_DIR}/")
        .or_else(|| tok.strip_prefix("${CMAKE_CURRENT_LIST_DIR}/"))
    {
        (dir_prefix, r)
    } else if let Some(r) = tok
        .strip_prefix("${PROJECT_SOURCE_DIR}/")
        .or_else(|| tok.strip_prefix("${CMAKE_SOURCE_DIR}/"))
    {
        ("", r)
    } else {
        (dir_prefix, tok)
    };
    if rest.contains('$') || rest.starts_with('/') || !is_cpp_source_path(rest) {
        return None;
    }
    let mut parts: Vec<&str> = base.split('/').filter(|s| !s.is_empty()).collect();
    for seg in rest.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            s => parts.push(s),
        }
    }
    Some(parts.join("/"))
}

#[allow(clippy::disallowed_methods)]
fn collect_cmake_targets_recursive(
    root_canon: &Path,
    dir: &Path,
    depth: usize,
    targets: &mut Vec<CmakeTarget>,
    uses_glob: &mut bool,
) {
    if depth > 4 {
        return;
    }
    let Ok(raw) = std::fs::read_to_string(dir.join("CMakeLists.txt")) else {
        return;
    };
    // Drop `#` comments so a commented-out target never matches.
    let cmake_text = raw
        .lines()
        .map(|l| l.split('#').next().unwrap_or(""))
        .collect::<Vec<_>>()
        .join("\n");
    let dir_prefix = dir
        .strip_prefix(root_canon)
        .ok()
        .map(|rel| rel.to_string_lossy().replace('\\', "/"))
        .unwrap_or_default();
    let lower_cm = cmake_text.to_ascii_lowercase();
    if lower_cm.contains("file(glob") || lower_cm.contains("aux_source_directory") {
        *uses_glob = true;
    }
    static RE_TARGET: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re_target = RE_TARGET.get_or_init(|| {
        regex::Regex::new(
            r"(?is)\b(add_executable|add_library|target_sources)\s*\(\s*([A-Za-z0-9_.-]+)([^)]*)\)",
        )
        .unwrap()
    });
    for cap in re_target.captures_iter(&cmake_text) {
        let idx = match targets.iter().position(|t| t.name == cap[2]) {
            Some(i) => i,
            None => {
                targets.push(CmakeTarget {
                    name: cap[2].to_string(),
                    is_executable: false,
                    sources: Vec::new(),
                });
                targets.len() - 1
            }
        };
        let target = &mut targets[idx];
        if cap[1].eq_ignore_ascii_case("add_executable") {
            target.is_executable = true;
        }
        target.sources.extend(
            cap[3]
                .split_whitespace()
                .filter_map(|tok| cmake_source_rel(&dir_prefix, tok)),
        );
    }
    if let Ok(re_sub) = regex::Regex::new(r"(?is)add_subdirectory\s*\(\s*([^\s)#]+)") {
        for cap in re_sub.captures_iter(&cmake_text) {
            let sub = cap[1].trim_matches(|c| c == '"' || c == '\'');
            if sub.is_empty() || sub.contains("${") || sub.contains("..") {
                continue;
            }
            let cand = dir.join(sub);
            let Ok(meta) = std::fs::symlink_metadata(&cand) else {
                continue;
            };
            if meta.file_type().is_symlink() || !meta.is_dir() {
                continue;
            }
            if let Ok(canon) = std::fs::canonicalize(&cand) {
                if canon.starts_with(root_canon) {
                    collect_cmake_targets_recursive(
                        root_canon,
                        &canon,
                        depth + 1,
                        targets,
                        uses_glob,
                    );
                }
            }
        }
    }
}

/// The CMake target that compiles `rel_source` according to the configured build's
/// `compile_commands.json`: its object file lives under `CMakeFiles/<target>.dir/`.
#[allow(clippy::disallowed_methods)]
fn cmake_target_from_compile_db(root: &Path, rel_source: &str) -> Option<String> {
    let build_dir = find_cpp_build_dir(&root.to_string_lossy())?;
    let raw = std::fs::read_to_string(root.join(build_dir).join("compile_commands.json")).ok()?;
    let entries: Vec<serde_json::Value> = serde_json::from_str(&raw).ok()?;
    let want = root.join(rel_source);
    let want_name = want.file_name()?;
    entries.iter().find_map(|e| {
        let file = Path::new(e.get("directory")?.as_str()?).join(e.get("file")?.as_str()?);
        if file.file_name() != Some(want_name) || std::fs::canonicalize(&file).ok()? != want {
            return None;
        }
        let output = match e.get("output").and_then(|v| v.as_str()) {
            Some(o) => o.to_string(),
            None => {
                let mut argv = e.get("command")?.as_str()?.split_whitespace();
                argv.find(|a| *a == "-o")?;
                argv.next()?.to_string()
            }
        };
        let (_, obj) = output.rsplit_once("CMakeFiles/")?;
        obj.split_once(".dir/").map(|(t, _)| t.to_string())
    })
}

#[allow(clippy::disallowed_methods)]
pub(crate) fn infer_cmake_target_for_source(
    cwd: &str,
    source_path: &str,
    want_test: bool,
) -> Option<String> {
    let root_canon = std::fs::canonicalize(cwd).unwrap_or_else(|_| PathBuf::from(cwd));
    let mut targets: Vec<CmakeTarget> = Vec::new();
    let mut uses_glob = false;
    collect_cmake_targets_recursive(&root_canon, &root_canon, 0, &mut targets, &mut uses_glob);
    let src_p = Path::new(source_path);
    let rel_source = if src_p.is_absolute() {
        std::fs::canonicalize(src_p)
            .ok()
            .and_then(|c| c.strip_prefix(&root_canon).ok().map(Path::to_path_buf))
            .unwrap_or_else(|| src_p.to_path_buf())
            .to_string_lossy()
            .replace('\\', "/")
    } else {
        source_path.trim_start_matches("./").replace('\\', "/")
    };
    // 1. The configured build's own answer, when there is one; a library that compiles the
    //    file falls through to the test-executable fallback below (BUG3-2).
    if let Some(name) = cmake_target_from_compile_db(&root_canon, &rel_source) {
        if !targets.iter().any(|t| t.name == name && !t.is_executable) {
            return Some(name);
        }
    }
    let exe_targets: Vec<&CmakeTarget> = targets.iter().filter(|t| t.is_executable).collect();
    // 2. The executable that lists exactly this root-relative path (BUG-10). No basename
    //    guess: a same-named file in another directory belongs to a different program.
    if let Some(t) = exe_targets.iter().find(|t| t.sources.contains(&rel_source)) {
        return Some(t.name.clone());
    }
    // 3. For tests, prefer executable targets whose name contains "test" / "spec"
    if want_test {
        if let Some(t) = exe_targets.iter().find(|t| {
            let l = t.name.to_ascii_lowercase();
            l.contains("test") || l.contains("spec")
        }) {
            return Some(t.name.clone());
        }
        return None;
    }
    // 4. Only fall back to the first non-test executable if CMake globs sources
    if uses_glob {
        exe_targets
            .iter()
            .find(|t| !t.name.to_ascii_lowercase().contains("test"))
            .or_else(|| exe_targets.first())
            .map(|t| t.name.clone())
    } else {
        None
    }
}

/// Why a local C/C++ `source` inside the CMake / Meson project at `project_dir` that maps
/// to no executable target can neither Run nor Debug: compiled on its own it would miss the
/// project's include paths and libraries (BUG-10). `None` for a file outside any build
/// project, which compiles standalone.
#[allow(clippy::disallowed_methods)]
pub(crate) fn cpp_unmapped_source_error(project_dir: &str, source: &str) -> Option<String> {
    let root = std::fs::canonicalize(project_dir).ok()?;
    let src = std::fs::canonicalize(root.join(source)).ok()?;
    let shown = src.strip_prefix(&root).ok()?.display();
    if root.join("CMakeLists.txt").is_file() {
        Some(if find_cpp_build_dir(project_dir).is_none() {
            format!(
                "'{shown}' maps to no executable target yet: the CMake project in '{project_dir}' was never configured, so sources listed through variables or globs are unknown. Configure the project first ('CMake Configure Debug' in the run configurations, or 'CMake Configure' on CMakeLists.txt), then try again."
            )
        } else {
            format!(
                "'{shown}' is not compiled into any executable target of the CMake project in '{project_dir}' (it may belong to a library). Run or debug the executable or test target that uses it instead ('Run <target>' or a CTest configuration)."
            )
        })
    } else if root.join("meson.build").is_file() {
        Some(format!(
            "'{shown}' belongs to the Meson project in '{project_dir}', and Otto does not map Meson sources to executables. Build it with 'Meson Compile', then run or debug the built executable under builddir/ instead."
        ))
    } else {
        None
    }
}

#[allow(clippy::disallowed_methods)]
pub(crate) fn find_cpp_build_dir(cwd: &str) -> Option<String> {
    let base = Path::new(cwd);
    for rel in [
        "build",
        "cmake-build-debug",
        "out/build",
        "out/build/debug",
        "builddir",
    ] {
        let cand = base.join(rel);
        if cand.join("CMakeCache.txt").is_file() || cand.join("CTestTestfile.cmake").is_file() {
            return Some(rel.to_string());
        }
    }
    None
}

#[allow(clippy::disallowed_methods)]
pub(crate) fn find_built_cpp_executable(cwd: &str, target_name: &str) -> Option<String> {
    let base = Path::new(cwd);
    for dir in [
        "build",
        "build/Debug",
        "build/bin",
        "cmake-build-debug",
        "out/build",
        "builddir",
    ] {
        let cand = base.join(dir).join(target_name);
        if is_executable_file(&cand) {
            return Some(cand.to_string_lossy().into_owned());
        }
    }
    None
}

pub(crate) const DEFAULT_UNREAL_SEARCH_BASES: &[&str] =
    &["/Users/Shared/Epic Games", "/opt/UnrealEngine"];

#[allow(clippy::disallowed_methods)]
pub(crate) fn resolve_unreal_engine_root(env_map: &BTreeMap<String, String>) -> Option<PathBuf> {
    resolve_unreal_engine_root_in(env_map, DEFAULT_UNREAL_SEARCH_BASES)
}

#[allow(clippy::disallowed_methods)]
pub(crate) fn resolve_unreal_engine_root_in(
    env_map: &BTreeMap<String, String>,
    search_bases: &[&str],
) -> Option<PathBuf> {
    for key in ["UE_ROOT", "UNREAL_ENGINE_ROOT"] {
        if let Some(val) = env_map.get(key).map(|s| s.trim()).filter(|s| !s.is_empty()) {
            let p = PathBuf::from(val);
            if p.join("Engine").is_dir() {
                return Some(p);
            }
        }
    }
    for base in search_bases {
        let bp = Path::new(base);
        if bp.join("Engine").is_dir() {
            return Some(bp.to_path_buf());
        }
        if let Ok(rd) = std::fs::read_dir(bp) {
            let mut versions: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir() && p.join("Engine").is_dir())
                .collect();
            versions.sort();
            if let Some(latest) = versions.pop() {
                return Some(latest);
            }
        }
    }
    None
}

fn build_unreal_runner_cmd(
    env_map: &BTreeMap<String, String>,
    cmd_opt: Option<&str>,
    args: Vec<String>,
    build_flags: Vec<String>,
) -> (String, Vec<String>) {
    if let Some(explicit) = cmd_opt {
        let mut out = build_flags;
        out.extend(args);
        return (explicit.to_string(), out);
    }
    let Some(ue_root) = resolve_unreal_engine_root(env_map) else {
        return (
            "sh".into(),
            vec![
                "-c".into(),
                "echo 'Unreal Engine not found: set UE_ROOT (or UNREAL_ENGINE_ROOT) in Project Environment, or install Unreal Engine under /Users/Shared/Epic Games/UE_5.*' >&2; exit 1".into(),
            ],
        );
    };
    let sub = args.first().map(String::as_str).unwrap_or("build");
    let platform = if cfg!(target_os = "macos") {
        "Mac"
    } else {
        "Linux"
    };
    match sub {
        "build" => {
            let script = ue_root.join(format!("Engine/Build/BatchFiles/{platform}/Build.sh"));
            let target = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| "UnrealEditor".into());
            let plat = args.get(2).cloned().unwrap_or_else(|| platform.into());
            let cfg_mode = args.get(3).cloned().unwrap_or_else(|| "Development".into());
            let mut out = vec![target, plat, cfg_mode];
            if let Some(uproj) = args.get(4) {
                out.push(format!("-Project={uproj}"));
            }
            out.push("-WaitMutex".into());
            out.extend(build_flags);
            (script.to_string_lossy().into_owned(), out)
        }
        "gen-cdb" => {
            let script = ue_root.join(format!("Engine/Build/BatchFiles/{platform}/Build.sh"));
            let target = args
                .get(1)
                .cloned()
                .unwrap_or_else(|| "UnrealEditor".into());
            let plat = args.get(2).cloned().unwrap_or_else(|| platform.into());
            let cfg_mode = args.get(3).cloned().unwrap_or_else(|| "Development".into());
            let mut out = vec!["-mode=GenerateClangDatabase".into(), target, plat, cfg_mode];
            if let Some(uproj) = args.get(4) {
                out.push(format!("-Project={uproj}"));
            }
            out.push("-OutputDir=.".into());
            out.extend(build_flags);
            (script.to_string_lossy().into_owned(), out)
        }
        "editor" => {
            let editor_bin = if cfg!(target_os = "macos") {
                ue_root.join("Engine/Binaries/Mac/UnrealEditor.app/Contents/MacOS/UnrealEditor")
            } else {
                ue_root.join("Engine/Binaries/Linux/UnrealEditor")
            };
            let mut out: Vec<String> = args.into_iter().skip(1).collect();
            out.extend(build_flags);
            (editor_bin.to_string_lossy().into_owned(), out)
        }
        "build-plugin" => {
            let uat = ue_root.join("Engine/Build/BatchFiles/RunUAT.sh");
            let uplugin = args.get(1).cloned().unwrap_or_default();
            let mut out = vec![
                "BuildPlugin".into(),
                format!("-Plugin={uplugin}"),
                "-Package=Saved/PackagedPlugin".into(),
            ];
            out.extend(build_flags);
            (uat.to_string_lossy().into_owned(), out)
        }
        _ => {
            let script = ue_root.join(format!("Engine/Build/BatchFiles/{platform}/Build.sh"));
            let mut out = args;
            out.extend(build_flags);
            (script.to_string_lossy().into_owned(), out)
        }
    }
}

fn build_dotnet_runner_cmd(
    runner: IdeRunnerKind,
    cfg: &IdeRunConfig,
    cmd_opt: Option<&str>,
    mut args: Vec<String>,
    build_flags: Vec<String>,
    filter: Option<&str>,
) -> (String, Vec<String>) {
    let prog = cmd_opt.unwrap_or("dotnet").to_string();
    let tp = cfg
        .target_path
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty());

    match runner {
        IdeRunnerKind::DotnetBuild => {
            if args.first().map(String::as_str) == Some("build") {
                args.remove(0);
            }
            let mut out = vec!["build".to_string()];
            if let Some(proj) = tp.filter(|p| {
                p.ends_with(".sln")
                    || p.ends_with(".slnx")
                    || p.ends_with(".csproj")
                    || p.ends_with(".fsproj")
            }) {
                if !args.iter().any(|a| a == proj) {
                    out.push(proj.to_string());
                }
            }
            if cfg.offline && !build_flags.iter().any(|f| f == "--no-restore") {
                out.push("--no-restore".to_string());
            }
            out.extend(build_flags);
            out.extend(args);
            (prog, out)
        }
        IdeRunnerKind::DotnetRun => {
            let is_watch = args.first().map(String::as_str) == Some("watch");
            let mut out = if is_watch {
                args.remove(0);
                if args.first().map(String::as_str) == Some("run") {
                    args.remove(0);
                }
                vec!["watch".to_string(), "run".to_string()]
            } else {
                if args.first().map(String::as_str) == Some("run") {
                    args.remove(0);
                }
                vec!["run".to_string()]
            };
            if let Some(proj) = tp.filter(|p| p.ends_with(".csproj") || p.ends_with(".fsproj")) {
                if !args.iter().any(|a| a == "--project" || a == proj) {
                    out.push("--project".to_string());
                    out.push(proj.to_string());
                }
            }
            if cfg.offline && !build_flags.iter().any(|f| f == "--no-restore") {
                out.push("--no-restore".to_string());
            }
            out.extend(build_flags);
            if !args.is_empty() {
                let starts_with_dotnet_flag = args[0].starts_with("--project")
                    || args[0] == "-p"
                    || args[0] == "-c"
                    || args[0] == "--configuration"
                    || args[0] == "--";
                if !starts_with_dotnet_flag {
                    out.push("--".to_string());
                }
                out.extend(args);
            }
            (prog, out)
        }
        IdeRunnerKind::DotnetTest => {
            if args.first().map(String::as_str) == Some("test") {
                args.remove(0);
            }
            let mut out = vec!["test".to_string()];
            if let Some(proj) = tp.filter(|p| {
                p.ends_with(".sln")
                    || p.ends_with(".slnx")
                    || p.ends_with(".csproj")
                    || p.ends_with(".fsproj")
            }) {
                if !args.iter().any(|a| a == proj) {
                    out.push(proj.to_string());
                }
            }
            if !args.iter().any(|a| a == "--verbosity" || a == "-v")
                && !build_flags.iter().any(|a| a == "--verbosity" || a == "-v")
            {
                out.push("--verbosity".to_string());
                out.push("normal".to_string());
            }
            if let Some(f) = filter {
                if !args.iter().any(|a| a == "--filter") {
                    out.push("--filter".to_string());
                    let expr = if f.contains('=') || f.contains('~') {
                        f.to_string()
                    } else {
                        format!("FullyQualifiedName~{f}")
                    };
                    out.push(expr);
                }
            }
            if cfg.offline && !build_flags.iter().any(|f| f == "--no-restore") {
                out.push("--no-restore".to_string());
            }
            if cfg.coverage
                && !build_flags
                    .iter()
                    .any(|f| f.contains("XPlat Code Coverage"))
            {
                out.push("--collect:XPlat Code Coverage".to_string());
            }
            out.extend(build_flags);
            out.extend(args);
            (prog, out)
        }
        _ => (prog, args),
    }
}

pub(crate) fn build_runner_command(
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
    let args = cfg.args.clone();
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

    match cfg.runner {
        IdeRunnerKind::GoTest => build_go_runner_cmd(
            "test",
            "./...",
            cfg,
            target_kind,
            cwd,
            cmd_opt,
            args,
            build_flags,
            filter,
        ),
        IdeRunnerKind::GoRun => build_go_runner_cmd(
            "run",
            ".",
            cfg,
            target_kind,
            cwd,
            cmd_opt,
            args,
            build_flags,
            filter,
        ),
        IdeRunnerKind::GoBuild => build_go_runner_cmd(
            "build",
            ".",
            cfg,
            target_kind,
            cwd,
            cmd_opt,
            args,
            build_flags,
            filter,
        ),
        IdeRunnerKind::CargoTest => build_cargo_runner_cmd(
            true,
            cfg,
            target_kind,
            env_map,
            cmd_opt,
            args,
            build_flags,
            filter,
        ),
        IdeRunnerKind::CargoRun => build_cargo_runner_cmd(
            false,
            cfg,
            target_kind,
            env_map,
            cmd_opt,
            args,
            build_flags,
            filter,
        ),
        IdeRunnerKind::ZigBuild | IdeRunnerKind::ZigRun | IdeRunnerKind::ZigTest => {
            build_zig_runner_cmd(cfg.runner, cfg, cwd, cmd_opt, args, build_flags, filter)
        }
        IdeRunnerKind::Cmake
        | IdeRunnerKind::Ctest
        | IdeRunnerKind::CppRun
        | IdeRunnerKind::Make
        | IdeRunnerKind::Meson
        | IdeRunnerKind::Unreal => build_cpp_runner_cmd(
            cfg.runner,
            cfg,
            target_kind,
            cwd,
            env_map,
            cmd_opt,
            args,
            build_flags,
            filter,
        ),
        IdeRunnerKind::DotnetBuild | IdeRunnerKind::DotnetRun | IdeRunnerKind::DotnetTest => {
            build_dotnet_runner_cmd(cfg.runner, cfg, cmd_opt, args, build_flags, filter)
        }
        IdeRunnerKind::Pytest => build_pytest_cmd(
            cfg,
            target_kind,
            cwd,
            env_map,
            cmd_opt,
            args,
            build_flags,
            filter,
        ),
        IdeRunnerKind::Python | IdeRunnerKind::Node => {
            build_script_cmd(cfg, env_map, cmd_opt, args, build_flags)
        }
        IdeRunnerKind::Npm => build_npm_cmd(cfg, target_kind, cwd, cmd_opt, args),
        IdeRunnerKind::Vitest => build_vitest_cmd(cfg, cmd_opt, args, build_flags, filter),
        IdeRunnerKind::Jest => build_jest_cmd(cfg, cmd_opt, args, build_flags, filter),
        IdeRunnerKind::Playwright => build_playwright_cmd(cfg, cmd_opt, args, build_flags, filter),
        IdeRunnerKind::Karma => build_karma_cmd(cfg, target_kind, cwd, cmd_opt, args, build_flags),
        IdeRunnerKind::JavaMain => build_java_main_cmd(cfg, cmd_opt, args, build_flags, filter),
        IdeRunnerKind::JavaJunit => jvm_test_cmd(
            cfg,
            target_kind,
            cwd,
            cmd_opt,
            (args.clone(), filter),
            (args, filter),
            build_flags,
        ),
        IdeRunnerKind::Kotest => jvm_test_cmd(
            cfg,
            target_kind,
            cwd,
            cmd_opt,
            (runner_kotest::kotest_gradle_args(cfg, args.clone()), None),
            (args, Some(&runner_kotest::kotest_maven_filter(cfg))),
            build_flags,
        ),
        IdeRunnerKind::Maven => {
            build_maven_cmd(cfg, target_kind, cwd, cmd_opt, args, build_flags, filter)
        }
        IdeRunnerKind::Gradle => {
            build_gradle_cmd(cfg, target_kind, cwd, cmd_opt, args, build_flags, filter)
        }
        IdeRunnerKind::Shell => build_shell_cmd(cfg, cmd_opt, args, build_flags),
        IdeRunnerKind::Xcode => build_xcode_cmd(env_map, cmd_opt, args, build_flags),
        IdeRunnerKind::Flutter => build_flutter_cmd(env_map, cmd_opt, args, build_flags),
        IdeRunnerKind::ReactNative => build_react_native_cmd(env_map, cmd_opt, args, build_flags),
        // UnityAttach never runs: start_run refuses it (debug-only).
        IdeRunnerKind::Compound | IdeRunnerKind::Custom | IdeRunnerKind::UnityAttach => {
            build_custom_cmd(cmd_opt, args, build_flags)
        }
    }
}

/// A local module with a Gradle build and no POM.
fn local_gradle_only(target_kind: IdeTargetKind, cwd: &str) -> bool {
    target_kind == IdeTargetKind::Local
        && !Path::new(cwd).join("pom.xml").is_file()
        && (Path::new(cwd).join("build.gradle.kts").is_file()
            || Path::new(cwd).join("build.gradle").is_file())
}

/// A JVM test run through the project's build tool, each with its own args and filter:
/// Gradle on a Gradle-only project, else Maven. On SSH the remote shell picks (BUG-16).
fn jvm_test_cmd(
    cfg: &IdeRunConfig,
    target_kind: IdeTargetKind,
    cwd: &str,
    cmd_opt: Option<&str>,
    (g_args, g_filter): (Vec<String>, Option<&str>),
    (m_args, m_filter): (Vec<String>, Option<&str>),
    build_flags: Vec<String>,
) -> (String, Vec<String>) {
    if target_kind == IdeTargetKind::Ssh && cmd_opt.is_none() {
        let (_, g_args) = build_gradle_cmd(
            cfg,
            target_kind,
            cwd,
            Some("gradle"),
            g_args,
            build_flags.clone(),
            g_filter,
        );
        let (_, m_args) = build_maven_cmd(
            cfg,
            target_kind,
            cwd,
            Some("mvn"),
            m_args,
            build_flags,
            m_filter,
        );
        let gq = g_args
            .iter()
            .map(|a| shell_words::quote(a).into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        let mq = m_args
            .iter()
            .map(|a| shell_words::quote(a).into_owned())
            .collect::<Vec<_>>()
            .join(" ");
        let script = format!(
            "if [ ! -f pom.xml ] && {{ [ -f build.gradle.kts ] || [ -f build.gradle ] || [ -f gradlew ] || [ -f ../gradlew ] || [ -f settings.gradle.kts ] || [ -f settings.gradle ]; }}; then \
               set -- {gq}; {}; \
             else \
               set -- {mq}; {}; \
             fi",
            ssh_wrapper_exec("gradlew", "gradle"),
            ssh_wrapper_exec("mvnw", "mvn"),
        );
        return ("sh".into(), vec!["-c".into(), script]);
    }
    if local_gradle_only(target_kind, cwd) {
        build_gradle_cmd(
            cfg,
            target_kind,
            cwd,
            cmd_opt,
            g_args,
            build_flags,
            g_filter,
        )
    } else {
        build_maven_cmd(
            cfg,
            target_kind,
            cwd,
            cmd_opt,
            m_args,
            build_flags,
            m_filter,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::resolve_run_cwd;
    use crate::types::{IdeTargetKind, IdeTargetSpec};

    fn local_target(root: &std::path::Path, extra: Vec<String>) -> IdeTargetSpec {
        IdeTargetSpec {
            kind: IdeTargetKind::Local,
            workspace_id: None,
            root: root.to_string_lossy().into_owned(),
            connection_id: None,
            additional_roots: extra,
            project_folders: vec![],
        }
    }

    fn ssh_target(root: &str, extra: Vec<String>) -> IdeTargetSpec {
        IdeTargetSpec {
            kind: IdeTargetKind::Ssh,
            workspace_id: None,
            root: root.into(),
            connection_id: Some("conn-1".into()),
            additional_roots: extra,
            project_folders: vec![],
        }
    }

    #[test]
    fn resolve_run_cwd_local_blocks_dotdot_and_symlink_escapes() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("workspace");
        let sub = root.join("pkg");
        let extra = tmp.path().join("extra");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::create_dir_all(&extra).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&outside, root.join("escape_link")).unwrap();

        let target = local_target(&root, vec![extra.to_string_lossy().into_owned()]);
        let canon_sub = sub.canonicalize().unwrap().to_string_lossy().into_owned();
        let canon_extra = extra.canonicalize().unwrap().to_string_lossy().into_owned();

        // Legitimate subdirs (relative, absolute, and extra root) resolve cleanly.
        assert_eq!(resolve_run_cwd(&target, Some("pkg")), canon_sub);
        assert_eq!(resolve_run_cwd(&target, Some("./pkg")), canon_sub);
        assert_eq!(
            resolve_run_cwd(&target, Some(&sub.to_string_lossy())),
            canon_sub
        );
        assert_eq!(
            resolve_run_cwd(&target, Some(&extra.to_string_lossy())),
            canon_extra
        );

        // Absolute `..` escape must NOT fall through to lexical prefix matching.
        let abs_dotdot = format!("{}/../outside", root.display());
        assert_eq!(resolve_run_cwd(&target, Some(&abs_dotdot)), target.root);

        // Nonexistent intermediate + `..` escape must also fail closed to root.
        let missing_dotdot = format!("{}/nonexistent/../../outside", root.display());
        assert_eq!(resolve_run_cwd(&target, Some(&missing_dotdot)), target.root);

        // Relative `..` escape.
        assert_eq!(resolve_run_cwd(&target, Some("../outside")), target.root);

        // Absolute and relative symlink escapes must fail closed to root.
        let abs_symlink = root.join("escape_link").to_string_lossy().into_owned();
        assert_eq!(resolve_run_cwd(&target, Some(&abs_symlink)), target.root);
        assert_eq!(resolve_run_cwd(&target, Some("escape_link")), target.root);

        // Symlink escape with nonexistent child tail.
        let symlink_tail = format!("{}/escape_link/future", root.display());
        assert_eq!(resolve_run_cwd(&target, Some(&symlink_tail)), target.root);
    }

    #[test]
    fn resolve_run_cwd_ssh_blocks_dotdot_on_absolute_and_relative_paths() {
        let target = ssh_target("/srv/app", vec!["/srv/lib".into()]);

        // Valid relative, absolute, extra-root, and internally normalized paths.
        assert_eq!(
            resolve_run_cwd(&target, Some("cmd/api")),
            "/srv/app/cmd/api"
        );
        assert_eq!(
            resolve_run_cwd(&target, Some("./cmd/api")),
            "/srv/app/cmd/api"
        );
        assert_eq!(
            resolve_run_cwd(&target, Some("/srv/app/pkg")),
            "/srv/app/pkg"
        );
        assert_eq!(
            resolve_run_cwd(&target, Some("/srv/app/a/../b")),
            "/srv/app/b"
        );
        assert_eq!(
            resolve_run_cwd(&target, Some("/srv/lib/mod")),
            "/srv/lib/mod"
        );

        // Absolute and relative `..` escapes and sibling-prefix tricks fall back to root.
        assert_eq!(
            resolve_run_cwd(&target, Some("/srv/app/../etc")),
            "/srv/app"
        );
        assert_eq!(
            resolve_run_cwd(&target, Some("/srv/app/../../etc")),
            "/srv/app"
        );
        assert_eq!(resolve_run_cwd(&target, Some("../etc")), "/srv/app");
        assert_eq!(resolve_run_cwd(&target, Some("/srv/app2")), "/srv/app");
        assert_eq!(resolve_run_cwd(&target, Some("/etc")), "/srv/app");

        // Tilde roots are confined too.
        let tilde_target = ssh_target("~/app", vec![]);
        assert_eq!(
            resolve_run_cwd(&tilde_target, Some("~/app/sub")),
            "~/app/sub"
        );
        assert_eq!(
            resolve_run_cwd(&tilde_target, Some("~/app/../../etc")),
            "~/app"
        );
        assert_eq!(resolve_run_cwd(&tilde_target, Some("~/other")), "~/app");
    }
}
