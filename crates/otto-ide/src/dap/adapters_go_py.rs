//! Go (`dlv dap`) and Python (`debugpy.adapter` + stdlib `bdb`) DAP adapter resolvers.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use otto_core::{Error, Result};
use serde_json::{json, Value};

use super::adapters::{
    locate_binary, normalize_target_path, DapAdapterTransport, ResolvedDapAdapter,
};
use super::py_bridge::PY_STDLIB_DAP_BRIDGE;
use super::{run_target_command, DapStartContext};
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
    let effective_cwd = find_go_mod_dir(&req.target, &raw_prog, cwd);

    let mut flag_parts: Vec<String> = Vec::new();
    if let Some(rc) = &ctx.run_config {
        flag_parts.extend(
            rc.build_flags
                .iter()
                .filter(|s| !s.trim().is_empty())
                .cloned(),
        );
    }
    if let Some(env_flags) = env
        .get("DLV_BUILD_FLAGS")
        .or_else(|| env.get("GO_BUILD_FLAGS"))
        .filter(|s| !s.trim().is_empty())
    {
        flag_parts.push(env_flags.clone());
    }
    let build_flags = flag_parts.join(" ");

    let (launch_command, launch_args) = if let Some(port) = req.launch.attach_port {
        if let Some(pid_str) = env.get("DLV_ATTACH_PID") {
            let pid: i64 = pid_str.parse().unwrap_or(0);
            (
                "attach".to_string(),
                json!({ "mode": "local", "processId": pid }),
            )
        } else {
            return Ok(ResolvedDapAdapter {
                adapter_name: "dlv".into(),
                language: "go".into(),
                transport: DapAdapterTransport::DirectTcpAttach {
                    host: "127.0.0.1".into(),
                    port,
                },
                initialize_args: json!({
                    "adapterID": "dlv-dap",
                    "linesStartAt1": true,
                    "columnsStartAt1": true,
                    "pathFormat": "path"
                }),
                launch_command: "attach".into(),
                launch_args: json!({ "mode": "remote" }),
                normalized_cwd: effective_cwd,
                initial_output: Vec::new(),
            });
        }
    } else if req.launch.test_name.is_some() || raw_prog.ends_with("_test.go") {
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
            let clean = tname.trim_start_matches('^').trim_end_matches('$');
            test_args.push("-test.v".to_string());
            test_args.push("-test.run".to_string());
            test_args.push(format!("^{}$", regex::escape(clean)));
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
        ("launch".to_string(), body)
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
        ("launch".to_string(), body)
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
        initialize_args: json!({
            "adapterID": "dlv-dap",
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "pathFormat": "path"
        }),
        launch_command,
        launch_args,
        normalized_cwd: effective_cwd,
        initial_output: Vec::new(),
    })
}

fn find_go_mod_dir(target: &IdeTargetSpec, program: &str, fallback_cwd: &str) -> String {
    if target.kind == IdeTargetKind::Local && !program.is_empty() {
        let mut cur = Path::new(program);
        if cur.is_file() {
            cur = cur.parent().unwrap_or(cur);
        }
        loop {
            if cur.join("go.work").is_file() || cur.join("go.mod").is_file() {
                return cur.to_string_lossy().into_owned();
            }
            let Some(parent) = cur.parent() else { break };
            cur = parent;
        }
    }
    fallback_cwd.to_string()
}

pub(super) async fn resolve_python_adapter(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    let program = normalize_target_path(&req.target, &req.launch.program);
    let mut candidates = Vec::new();
    if let Some(py) = ctx
        .project_env
        .as_ref()
        .and_then(|p| p.python_interpreter.as_deref())
        .filter(|s| !s.is_empty())
    {
        candidates.push(py.to_string());
    }
    if req.target.kind == IdeTargetKind::Local && !program.is_empty() {
        let mut cur = Path::new(&program);
        if cur.is_file() {
            cur = cur.parent().unwrap_or(cur);
        }
        for _ in 0..8 {
            for rel in [".venv/bin/python", "venv/bin/python"] {
                let p = cur.join(rel);
                if p.is_file() {
                    candidates.push(p.to_string_lossy().into_owned());
                }
            }
            let Some(parent) = cur.parent() else { break };
            cur = parent;
        }
    }
    let root_path = Path::new(&req.target.root);
    for rel in [".venv/bin/python", "venv/bin/python"] {
        candidates.push(root_path.join(rel).to_string_lossy().into_owned());
    }
    if req.target.kind == IdeTargetKind::Local {
        candidates.push(
            data_dir
                .join("toolchains/python/current/bin/python3")
                .to_string_lossy()
                .into_owned(),
        );
    }
    candidates.push("python3".into());
    candidates.push("python".into());

    let py_bin = locate_binary(&req.target, ctx, &env, &candidates)
        .await
        .ok_or_else(|| {
            Error::NotFound(
                "Python 3 interpreter not found; install Python via IDE → Toolchains".into(),
            )
        })?;

    let mut adapter_py_candidates = vec![py_bin.clone()];
    if req.target.kind == IdeTargetKind::Local {
        let managed_dap_py = data_dir.join("toolchains/python/dap/bin/python");
        if managed_dap_py.is_file() {
            adapter_py_candidates.push(managed_dap_py.to_string_lossy().into_owned());
        }
    } else if let Some(found) = locate_binary(
        &req.target,
        ctx,
        &env,
        &["$HOME/.otto/toolchains/python/dap/bin/python".into()],
    )
    .await
    {
        adapter_py_candidates.push(found);
    }

    let mut debugpy_host_py: Option<String> = None;
    for cand_py in &adapter_py_candidates {
        let ok = run_target_command(
            &req.target,
            ctx.ssh.as_ref(),
            cand_py,
            &["-c".into(), "import debugpy.adapter".into()],
            cwd,
            &env,
            Duration::from_secs(4),
        )
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
        if ok {
            debugpy_host_py = Some(cand_py.clone());
            break;
        }
    }

    let (launch_command, launch_args) = if let Some(port) = req.launch.attach_port {
        (
            "attach".to_string(),
            json!({
                "connect": { "host": "127.0.0.1", "port": port },
                "justMyCode": false,
            }),
        )
    } else if let Some(test_name) = req.launch.test_name.as_deref().filter(|s| !s.is_empty()) {
        let mut args = if test_name.contains("::") {
            vec![test_name.to_string(), "-v".to_string()]
        } else {
            vec![
                program.clone(),
                "-k".to_string(),
                test_name.to_string(),
                "-v".to_string(),
            ]
        };
        args.extend(req.launch.args.clone());
        (
            "launch".to_string(),
            json!({
                "module": "pytest",
                "program": program,
                "testName": test_name,
                "args": args,
                "cwd": cwd,
                "console": "internalConsole",
                "justMyCode": false,
                "python": [py_bin],
            }),
        )
    } else {
        (
            "launch".to_string(),
            json!({
                "program": program,
                "args": req.launch.args,
                "cwd": cwd,
                "console": "internalConsole",
                "justMyCode": false,
                "python": [py_bin],
            }),
        )
    };

    let (adapter_name, command, args) = if let Some(host_py) = debugpy_host_py {
        (
            "debugpy".to_string(),
            host_py,
            vec!["-m".into(), "debugpy.adapter".into()],
        )
    } else if req.launch.attach_port.is_some() {
        (
            "debugpy".to_string(),
            py_bin,
            vec!["-m".into(), "debugpy.adapter".into()],
        )
    } else {
        (
            "python-bdb-dap".to_string(),
            py_bin,
            vec!["-u".into(), "-c".into(), PY_STDLIB_DAP_BRIDGE.into()],
        )
    };

    Ok(ResolvedDapAdapter {
        adapter_name,
        language: "python".into(),
        transport: DapAdapterTransport::Stdio {
            command,
            args,
            cwd: cwd.to_string(),
            env,
        },
        initialize_args: json!({
            "adapterID": "python",
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "pathFormat": "path"
        }),
        launch_command,
        launch_args,
        normalized_cwd: cwd.to_string(),
        initial_output: Vec::new(),
    })
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
        let (_, has_pkg_main) = inspect_go_package_dir(prog_path);
        if has_pkg_main {
            return raw_prog.to_string();
        }
        let cmd_dir = prog_path.join("cmd");
        if let Ok(rd) = std::fs::read_dir(&cmd_dir) {
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
        if p.extension().and_then(|e| e.to_str()) != Some("go") {
            continue;
        }
        if p.file_name()
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
