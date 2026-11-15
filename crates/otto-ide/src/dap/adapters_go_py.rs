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
use crate::types::{IdeStartDebugReq, IdeTargetKind, IdeTargetSpec};

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
    let build_flags = flag_parts.join(" ");
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

    let is_test = req.launch.test_name.is_some() || raw_prog.ends_with("_test.go");
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
                let pkg_dir = if raw_prog.ends_with(".go") {
                    Path::new(&raw_prog)
                        .parent()
                        .and_then(|p| p.to_str())
                        .unwrap_or(&effective_cwd)
                        .to_string()
                } else {
                    raw_prog.clone()
                };
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
                    initial_output: Vec::new(),
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
        let pkg_dir = if raw_prog.ends_with(".go") {
            Path::new(&raw_prog)
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or(&effective_cwd)
                .to_string()
        } else {
            raw_prog
        };
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
        initial_output: Vec::new(),
    })
}

fn find_go_mod_dir(
    target: &IdeTargetSpec,
    program: &str,
    fallback_cwd: &str,
    allowed_roots: &[PathBuf],
) -> String {
    if target.kind == IdeTargetKind::Local && !program.is_empty() {
        let mut cur = Path::new(program);
        if cur.is_file() {
            cur = cur.parent().unwrap_or(cur);
        }
        loop {
            if !allowed_roots.is_empty() && !allowed_roots.iter().any(|r| cur.starts_with(r)) {
                break;
            }
            if cur.join("go.work").is_file() || cur.join("go.mod").is_file() {
                return cur.to_string_lossy().into_owned();
            }
            let Some(parent) = cur.parent() else { break };
            cur = parent;
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
