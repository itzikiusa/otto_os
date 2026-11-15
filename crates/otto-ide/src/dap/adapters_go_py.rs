//! Go (`dlv dap`) DAP adapter resolver and warm external binary compilation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use otto_core::{Error, Result};
use serde_json::{json, Value};

use super::adapters::{
    locate_binary, normalize_target_path, resolve_debug_sdk_override, DapAdapterTransport,
    ResolvedDapAdapter,
};
use super::adapters_env::canonical_target_roots;
use super::DapStartContext;
use crate::go_build_cache::{ensure_go_binary, GoBuildMode, GoBuildRequest};
use crate::types::{
    IdeBreakpoint, IdeDebugOutputEntry, IdeStartDebugReq, IdeTargetKind, IdeTargetSpec,
};

pub(super) async fn resolve_go_adapter(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    let mut candidates = Vec::new();
    if req.target.kind == IdeTargetKind::Local {
        candidates.push(
            data_dir
                .join("toolchains/go/bin/dlv")
                .to_string_lossy()
                .into_owned(),
        );
        candidates.push(
            data_dir
                .join("toolchains/dlv/dlv")
                .to_string_lossy()
                .into_owned(),
        );
        if let Some(gopath) = env.get("GOPATH") {
            candidates.push(format!("{gopath}/bin/dlv"));
        }
        if let Some(home) = dirs::home_dir() {
            candidates.push(home.join("go/bin/dlv").to_string_lossy().into_owned());
        }
    } else {
        candidates.push("$HOME/.otto/toolchains/go/bin/dlv".into());
        candidates.push("$HOME/.otto/toolchains/dlv/dlv".into());
        candidates.push("$HOME/go/bin/dlv".into());
    }
    candidates.push("dlv".into());

    let dlv_bin = locate_binary(&req.target, ctx, &env, &candidates)
        .await
        .ok_or_else(|| {
            Error::NotFound(
                "Go debugger 'dlv' (Delve) not found; install it from IDE → Toolchains & SDKs"
                    .into(),
            )
        })?;

    let raw_prog = if req.launch.program.trim().is_empty() {
        cwd.to_string()
    } else {
        normalize_target_path(&req.target, &req.launch.program)
    };
    let allowed_roots = if ctx.caller_is_root {
        Vec::new()
    } else {
        canonical_target_roots(&req.target)
    };
    let effective_cwd = find_go_mod_dir(&req.target, &raw_prog, cwd, &allowed_roots);

    let mut flag_parts: Vec<String> = Vec::new();
    if let Some(rc) = &ctx.run_config {
        flag_parts.extend(
            rc.build_flags
                .iter()
                .filter(|s| !s.trim().is_empty())
                .cloned(),
        );
    }
    flag_parts.extend(
        req.launch
            .build_flags
            .iter()
            .filter(|s| !s.trim().is_empty())
            .cloned(),
    );
    if let Some(env_flags) = env
        .get("DLV_BUILD_FLAGS")
        .or_else(|| env.get("GO_BUILD_FLAGS"))
        .filter(|s| !s.trim().is_empty())
    {
        for tok in env_flags.split_whitespace() {
            flag_parts.push(tok.to_string());
        }
    }
    let init_args = json!({
        "adapterID": "dlv-dap",
        "linesStartAt1": true,
        "columnsStartAt1": true,
        "pathFormat": "path"
    });

    if let Some(port) = req.launch.attach_port {
        if let Some(pid_str) = env.get("DLV_ATTACH_PID") {
            let pid: i64 = pid_str.parse().unwrap_or(0);
            return Ok(ResolvedDapAdapter {
                adapter_name: "dlv".into(),
                language: "go".into(),
                transport: DapAdapterTransport::TcpListen {
                    command: dlv_bin,
                    args: vec!["dap".into(), "--listen=127.0.0.1:0".into()],
                    cwd: effective_cwd.clone(),
                    env,
                },
                initialize_args: init_args,
                launch_command: "attach".into(),
                launch_args: json!({ "mode": "local", "processId": pid }),
                normalized_cwd: effective_cwd,
                initial_output: Vec::new(),
            });
        }
        return Ok(ResolvedDapAdapter {
            adapter_name: "dlv".into(),
            language: "go".into(),
            transport: DapAdapterTransport::DirectTcpAttach {
                host: "127.0.0.1".into(),
                port,
            },
            initialize_args: init_args,
            launch_command: "attach".into(),
            launch_args: json!({ "mode": "remote" }),
            normalized_cwd: effective_cwd,
            initial_output: Vec::new(),
        });
    }

    let is_test = should_debug_go_as_test(req, ctx, &raw_prog);
    let (resolved_test_pkg_dir, initial_output) = if is_test {
        resolve_go_test_pkg_dir(&req.target, &raw_prog, &effective_cwd, &req.breakpoints)
    } else {
        (String::new(), Vec::new())
    };
    let merged_go_tags = collect_dap_go_build_tags(
        &req.target,
        ctx,
        &raw_prog,
        &effective_cwd,
        is_test.then_some(resolved_test_pkg_dir.as_str()),
        &req.breakpoints,
    );
    crate::go_build_tags::merge_go_tags_into_args(&mut flag_parts, &merged_go_tags);
    let build_flags = flag_parts.join(" ");
    if req.target.kind == IdeTargetKind::Local {
        let mut go_cands = Vec::new();
        if let Some(sdk) = resolve_debug_sdk_override(req, ctx, &effective_cwd) {
            let t = sdk.trim_end_matches('/');
            go_cands.push(if t.ends_with("/go") && Path::new(t).is_file() {
                t.to_string()
            } else {
                format!("{t}/bin/go")
            });
        }
        if let Some(goroot) = env.get("GOROOT").filter(|s| !s.is_empty()) {
            go_cands.push(format!("{}/bin/go", goroot.trim_end_matches('/')));
        }
        go_cands.push(
            data_dir
                .join("toolchains/go/current/bin/go")
                .to_string_lossy()
                .into_owned(),
        );
        go_cands.push(
            data_dir
                .join("toolchains/go/bin/go")
                .to_string_lossy()
                .into_owned(),
        );
        go_cands.push("go".into());

        if !is_test && !raw_prog.ends_with(".go") && Path::new(&raw_prog).is_file() {
            return Ok(ResolvedDapAdapter {
                adapter_name: "dlv".into(),
                language: "go".into(),
                transport: DapAdapterTransport::TcpListen {
                    command: dlv_bin,
                    args: vec!["dap".into(), "--listen=127.0.0.1:0".into()],
                    cwd: effective_cwd.clone(),
                    env: env.clone(),
                },
                initialize_args: init_args,
                launch_command: "launch".into(),
                launch_args: json!({
                    "mode": "exec",
                    "program": raw_prog,
                    "cwd": effective_cwd,
                    "args": req.launch.args,
                    "env": env,
                }),
                normalized_cwd: effective_cwd,
                initial_output: Vec::new(),
            });
        }

        if let Some(go_bin) = locate_binary(&req.target, ctx, &env, &go_cands).await {
            if is_test {
                let pkg_dir = resolved_test_pkg_dir.clone();
                let mut test_args = Vec::new();
                if let Some(tname) = req.launch.test_name.as_deref().filter(|s| !s.is_empty()) {
                    let pattern = format_go_test_run_pattern(tname);
                    if !pattern.is_empty() {
                        test_args.push("-test.v".to_string());
                        test_args.push("-test.run".to_string());
                        test_args.push(pattern);
                    }
                }
                test_args.extend(req.launch.args.clone());
                let artifact = ensure_go_binary(GoBuildRequest {
                    go_bin: &go_bin,
                    cwd: Path::new(&effective_cwd),
                    pkg_spec: &pkg_dir,
                    mode: GoBuildMode::DebugTest,
                    extra_build_flags: &flag_parts,
                    env_map: &env,
                    allowed_roots: &allowed_roots,
                })
                .await?;
                return Ok(ResolvedDapAdapter {
                    adapter_name: "dlv".into(),
                    language: "go".into(),
                    transport: DapAdapterTransport::TcpListen {
                        command: dlv_bin,
                        args: vec!["dap".into(), "--listen=127.0.0.1:0".into()],
                        cwd: pkg_dir.clone(),
                        env: env.clone(),
                    },
                    initialize_args: init_args,
                    launch_command: "launch".into(),
                    launch_args: json!({
                        "mode": "exec",
                        "program": artifact.binary_path.to_string_lossy(),
                        "cwd": pkg_dir,
                        "args": test_args,
                        "env": env,
                    }),
                    normalized_cwd: effective_cwd,
                    initial_output,
                });
            }

            let debug_pkg = resolve_go_debug_program(&req.target, &raw_prog, &effective_cwd);
            let artifact = ensure_go_binary(GoBuildRequest {
                go_bin: &go_bin,
                cwd: Path::new(&effective_cwd),
                pkg_spec: &debug_pkg,
                mode: GoBuildMode::Debug,
                extra_build_flags: &flag_parts,
                env_map: &env,
                allowed_roots: &allowed_roots,
            })
            .await?;
            return Ok(ResolvedDapAdapter {
                adapter_name: "dlv".into(),
                language: "go".into(),
                transport: DapAdapterTransport::TcpListen {
                    command: dlv_bin,
                    args: vec!["dap".into(), "--listen=127.0.0.1:0".into()],
                    cwd: effective_cwd.clone(),
                    env: env.clone(),
                },
                initialize_args: init_args,
                launch_command: "launch".into(),
                launch_args: json!({
                    "mode": "exec",
                    "program": artifact.binary_path.to_string_lossy(),
                    "cwd": effective_cwd,
                    "args": req.launch.args,
                    "env": env,
                }),
                normalized_cwd: effective_cwd,
                initial_output: Vec::new(),
            });
        }
    }

    let launch_args = if is_test {
        let pkg_dir = resolved_test_pkg_dir;
        let mut test_args = Vec::new();
        if let Some(tname) = req.launch.test_name.as_deref().filter(|s| !s.is_empty()) {
            let pattern = format_go_test_run_pattern(tname);
            if !pattern.is_empty() {
                test_args.push("-test.v".to_string());
                test_args.push("-test.run".to_string());
                test_args.push(pattern);
            }
        }
        test_args.extend(req.launch.args.clone());
        let mut body = json!({
            "mode": "test",
            "program": pkg_dir,
            "cwd": effective_cwd,
            "args": test_args,
            "env": env,
        });
        if !build_flags.is_empty() {
            body["buildFlags"] = Value::String(build_flags);
        }
        body
    } else {
        let debug_pkg = resolve_go_debug_program(&req.target, &raw_prog, &effective_cwd);
        let mut body = json!({
            "mode": "debug",
            "program": debug_pkg,
            "cwd": effective_cwd,
            "args": req.launch.args,
            "env": env,
        });
        if !build_flags.is_empty() {
            body["buildFlags"] = Value::String(build_flags);
        }
        body
    };

    Ok(ResolvedDapAdapter {
        adapter_name: "dlv".into(),
        language: "go".into(),
        transport: DapAdapterTransport::TcpListen {
            command: dlv_bin,
            args: vec!["dap".into(), "--listen=127.0.0.1:0".into()],
            cwd: effective_cwd.clone(),
            env,
        },
        initialize_args: init_args,
        launch_command: "launch".into(),
        launch_args,
        normalized_cwd: effective_cwd,
        initial_output,
    })
}

#[allow(clippy::disallowed_methods)]
fn dir_contains_go_tests(dir: &str) -> bool {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return false;
    };
    rd.flatten().any(|e| {
        e.path()
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.ends_with("_test.go"))
    })
}

#[allow(clippy::disallowed_methods)]
fn resolve_go_test_pkg_dir(
    target: &IdeTargetSpec,
    raw_prog: &str,
    effective_cwd: &str,
    breakpoints: &[IdeBreakpoint],
) -> (String, Vec<IdeDebugOutputEntry>) {
    if raw_prog.ends_with(".go") {
        let parent = Path::new(raw_prog)
            .parent()
            .and_then(|p| p.to_str())
            .unwrap_or(effective_cwd)
            .to_string();
        return (parent, Vec::new());
    }
    let was_wildcard = raw_prog.ends_with("/...") || raw_prog == "./...";
    if target.kind != IdeTargetKind::Local {
        let stripped = raw_prog.trim_end_matches("/...").trim_end_matches('/');
        let chosen = if stripped.is_empty() || stripped == "." {
            effective_cwd.to_string()
        } else {
            stripped.to_string()
        };
        return (chosen, Vec::new());
    }
    if !was_wildcard && dir_contains_go_tests(raw_prog) {
        return (raw_prog.to_string(), Vec::new());
    }

    let cwd_path = Path::new(effective_cwd);
    let mut chosen: Option<String> = None;
    for bp in breakpoints
        .iter()
        .filter(|b| b.enabled && b.path.ends_with(".go"))
    {
        let bp_path = if Path::new(&bp.path).is_absolute() {
            PathBuf::from(&bp.path)
        } else {
            cwd_path.join(&bp.path)
        };
        if bp_path.starts_with(cwd_path) {
            if let Some(parent_str) = bp_path.parent().and_then(|p| p.to_str()) {
                if dir_contains_go_tests(parent_str) {
                    chosen = Some(parent_str.to_string());
                    break;
                }
            }
        }
    }
    if chosen.is_none() {
        let tests_sub = cwd_path.join("tests");
        if let Some(s) = tests_sub.to_str().filter(|s| dir_contains_go_tests(s)) {
            chosen = Some(s.to_string());
        } else if dir_contains_go_tests(effective_cwd) {
            chosen = Some(effective_cwd.to_string());
        } else if let Ok(rd) = std::fs::read_dir(cwd_path) {
            let mut subdirs: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_dir()
                        && p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                            !n.starts_with('.')
                                && !matches!(n, "vendor" | "node_modules" | "target" | "testdata")
                        })
                })
                .take(32)
                .collect();
            subdirs.sort();
            'outer: for sub in &subdirs {
                if let Some(s) = sub.to_str().filter(|s| dir_contains_go_tests(s)) {
                    chosen = Some(s.to_string());
                    break;
                }
                if let Ok(inner_rd) = std::fs::read_dir(sub) {
                    let mut nested: Vec<PathBuf> = inner_rd
                        .flatten()
                        .map(|e| e.path())
                        .filter(|p| p.is_dir())
                        .take(16)
                        .collect();
                    nested.sort();
                    for n_dir in nested {
                        if let Some(ns) = n_dir.to_str().filter(|s| dir_contains_go_tests(s)) {
                            chosen = Some(ns.to_string());
                            break 'outer;
                        }
                    }
                }
            }
        }
    }

    let pkg_dir = chosen.unwrap_or_else(|| {
        let stripped = raw_prog.trim_end_matches("/...").trim_end_matches('/');
        if stripped.is_empty() || stripped == "." {
            effective_cwd.to_string()
        } else {
            stripped.to_string()
        }
    });
    let mut notes = Vec::new();
    if was_wildcard {
        notes.push(IdeDebugOutputEntry {
            category: "console".into(),
            text: format!("Delve Go test debugger attached to package: {pkg_dir}\n"),
        });
    }
    (pkg_dir, notes)
}

#[allow(clippy::disallowed_methods)]
fn collect_dap_go_build_tags(
    target: &IdeTargetSpec,
    ctx: &DapStartContext,
    raw_prog: &str,
    effective_cwd: &str,
    test_pkg_dir: Option<&str>,
    breakpoints: &[IdeBreakpoint],
) -> Vec<String> {
    let mut tags: Vec<String> = Vec::new();
    if !ctx.go_build_tags.is_empty() {
        for t in &ctx.go_build_tags {
            if !t.is_empty() && !tags.contains(t) {
                tags.push(t.clone());
            }
        }
    } else if let Some(proj) = &ctx.project_env {
        let probe_paths = vec![raw_prog.to_string(), effective_cwd.to_string()];
        for t in crate::environments::effective_go_build_tags_for_paths_in_project(
            Some(&target.root),
            proj,
            &probe_paths,
            target.kind == IdeTargetKind::Local,
        ) {
            if !t.is_empty() && !tags.contains(&t) {
                tags.push(t);
            }
        }
    }
    if target.kind != IdeTargetKind::Local {
        return tags;
    }
    let mut files_to_scan: Vec<PathBuf> = Vec::new();
    if raw_prog.ends_with(".go") {
        files_to_scan.push(PathBuf::from(raw_prog));
    }
    let cwd_path = Path::new(effective_cwd);
    for bp in breakpoints
        .iter()
        .filter(|b| b.enabled && b.path.ends_with(".go"))
    {
        let bp_path = if Path::new(&bp.path).is_absolute() {
            PathBuf::from(&bp.path)
        } else {
            cwd_path.join(&bp.path)
        };
        if bp_path.starts_with(cwd_path) {
            if let Some(pkg) = test_pkg_dir {
                if bp_path.parent() == Some(Path::new(pkg)) {
                    files_to_scan.push(bp_path);
                }
            } else {
                files_to_scan.push(bp_path);
            }
        }
    }
    if let Some(pkg) = test_pkg_dir {
        if let Ok(rd) = std::fs::read_dir(pkg) {
            for entry in rd.flatten().take(16) {
                let p = entry.path();
                if p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.ends_with("_test.go"))
                {
                    files_to_scan.push(p);
                }
            }
        }
    }
    for file in files_to_scan {
        if let Ok(src) = std::fs::read_to_string(&file) {
            for t in crate::go_build_tags::extract_go_file_build_tags(&src) {
                if !tags.contains(&t) {
                    tags.push(t);
                }
            }
        }
    }
    tags
}

/// Decide whether a Go DAP launch should run in test mode (`dlv test` / `go test -c`)
/// or binary mode (`dlv debug` / `dlv exec`).
///
/// Non-test `.go` files (e.g. `main.go`) and explicit `GoRun` configs without a
/// `test_name` NEVER switch into test mode even when sibling `*_test.go` files
/// exist in the same package directory.
pub(crate) fn should_debug_go_as_test(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    raw_prog: &str,
) -> bool {
    let has_test_name = req
        .launch
        .test_name
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty());
    if has_test_name || raw_prog.ends_with("_test.go") || raw_prog.ends_with("/...") {
        return true;
    }
    if raw_prog.ends_with(".go") {
        // Explicit non-test `.go` entrypoint (e.g. `main.go`).
        return false;
    }
    if let Some(rc) = &ctx.run_config {
        if rc.runner == crate::types::IdeRunnerKind::GoTest {
            return true;
        }
        if rc.runner == crate::types::IdeRunnerKind::GoRun {
            return false;
        }
    }
    if req.target.kind == IdeTargetKind::Local {
        let prog_path = Path::new(raw_prog);
        if prog_path.is_dir() && inspect_go_package_dir(prog_path).1 {
            return false;
        }
        return dir_contains_go_tests(raw_prog);
    }
    false
}

fn find_go_mod_dir(
    target: &IdeTargetSpec,
    program: &str,
    fallback_cwd: &str,
    allowed_roots: &[PathBuf],
) -> String {
    let clean_prog = program.trim_end_matches("/...").trim_end_matches('/');
    if target.kind == IdeTargetKind::Local && !clean_prog.is_empty() {
        let found =
            crate::go_build_cache::find_go_module_root(Path::new(clean_prog), allowed_roots);
        if found.join("go.work").is_file() || found.join("go.mod").is_file() {
            return super::adapters::display_target_path(target, &found.to_string_lossy());
        }
    }
    fallback_cwd.to_string()
}

/// Format a Go `-test.run` pattern so each `/`-delimited subtest segment is
/// anchored independently (`^Parent$/^Subtest$`) without double-escaping.
pub(crate) fn format_go_test_run_pattern(tname: &str) -> String {
    tname
        .split('/')
        .filter(|seg| !seg.is_empty())
        .map(|seg| {
            if seg.len() >= 2 && seg.starts_with('^') && seg.ends_with('$') {
                seg.to_string()
            } else {
                let clean = seg.trim_start_matches('^').trim_end_matches('$');
                format!("^{}$", regex::escape(clean))
            }
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Resolve a Go debug target so multi-file `package main` directories compile as a package.
#[allow(clippy::disallowed_methods)] // shallow 1-level Go package check during debug launch setup
pub(crate) fn resolve_go_debug_program(
    target: &IdeTargetSpec,
    raw_prog: &str,
    fallback_cwd: &str,
) -> String {
    if target.kind != IdeTargetKind::Local {
        return raw_prog.to_string();
    }
    let prog_path = Path::new(raw_prog);
    if raw_prog.ends_with(".go") && !raw_prog.ends_with("_test.go") {
        if let Some(parent) = prog_path.parent() {
            let (non_test_go_count, has_pkg_main) = inspect_go_package_dir(parent);
            if has_pkg_main && non_test_go_count > 1 {
                return parent.to_string_lossy().into_owned();
            }
        }
        return raw_prog.to_string();
    }
    if prog_path.is_dir() {
        if inspect_go_package_dir(prog_path).1 {
            return raw_prog.to_string();
        }
        if let Ok(rd) = std::fs::read_dir(prog_path.join("cmd")) {
            let mut main_dirs: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.is_dir() && inspect_go_package_dir(p).1)
                .collect();
            main_dirs.sort();
            if let Some(found) = main_dirs.into_iter().next() {
                return found.to_string_lossy().into_owned();
            }
        }
        for sub in ["app", "server", "src"] {
            let cand = prog_path.join(sub);
            if cand.is_dir() && inspect_go_package_dir(&cand).1 {
                return cand.to_string_lossy().into_owned();
            }
        }
    }
    if raw_prog.trim().is_empty() {
        return fallback_cwd.to_string();
    }
    raw_prog.to_string()
}

#[allow(clippy::disallowed_methods)] // shallow 1-level Go package check during debug launch setup
fn inspect_go_package_dir(dir: &Path) -> (usize, bool) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return (0, false);
    };
    let mut count = 0usize;
    let mut has_main = false;
    for entry in rd.flatten() {
        let p = entry.path();
        if p.extension().and_then(|e| e.to_str()) != Some("go")
            || p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.ends_with("_test.go"))
        {
            continue;
        }
        count += 1;
        if !has_main {
            if let Ok(txt) = std::fs::read_to_string(&p) {
                if txt
                    .lines()
                    .take(30)
                    .any(|l| l.trim() == "package main" || l.trim().starts_with("package main "))
                {
                    has_main = true;
                }
            }
        }
    }
    (count, has_main)
}

#[cfg(test)]
#[path = "adapters_go_py_tests.rs"]
mod tests;
