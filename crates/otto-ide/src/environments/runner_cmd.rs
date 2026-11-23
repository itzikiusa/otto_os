//! Command-line and working-directory builders for multi-language IDE Run Configurations.

#[path = "runner_cmd_langs.rs"]
mod runner_cmd_langs;
#[path = "runner_java.rs"]
mod runner_java;

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub(crate) use self::runner_cmd_langs::find_build_wrapper;
use self::runner_cmd_langs::{
    build_custom_cmd, build_flutter_cmd, build_gradle_cmd, build_java_main_cmd, build_maven_cmd,
    build_npm_cmd, build_pytest_cmd, build_react_native_cmd, build_script_cmd, build_shell_cmd,
    build_vitest_cmd, build_xcode_cmd,
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
        IdeRunnerKind::Vitest => build_vitest_cmd(cfg, cmd_opt, args, build_flags),
        IdeRunnerKind::JavaMain => build_java_main_cmd(cfg, cmd_opt, args, build_flags, filter),
        IdeRunnerKind::JavaJunit | IdeRunnerKind::Maven => {
            build_maven_cmd(cfg, target_kind, cwd, cmd_opt, args, build_flags, filter)
        }
        IdeRunnerKind::Gradle => {
            build_gradle_cmd(cfg, target_kind, cwd, cmd_opt, args, build_flags, filter)
        }
        IdeRunnerKind::Shell => build_shell_cmd(cfg, cmd_opt, args, build_flags),
        IdeRunnerKind::Xcode => build_xcode_cmd(env_map, cmd_opt, args, build_flags),
        IdeRunnerKind::Flutter => build_flutter_cmd(env_map, cmd_opt, args, build_flags),
        IdeRunnerKind::ReactNative => build_react_native_cmd(env_map, cmd_opt, args, build_flags),
        IdeRunnerKind::Compound | IdeRunnerKind::Custom => {
            build_custom_cmd(cmd_opt, args, build_flags)
        }
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
