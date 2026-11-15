//! Python (`debugpy.adapter` + zero-install stdlib `bdb` bridge) DAP adapter resolver.

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::OnceLock;
use std::time::{Duration, UNIX_EPOCH};

use dashmap::DashMap;
use otto_core::{Error, Result};
use serde_json::json;

use super::adapters::{
    locate_binary, normalize_target_path, resolve_debug_sdk_override, DapAdapterTransport,
    ResolvedDapAdapter,
};
use super::adapters_env::canonical_target_roots;
use super::py_bridge::PY_STDLIB_DAP_BRIDGE;
use super::{run_target_command, DapStartContext};
use crate::types::{IdeStartDebugReq, IdeTargetKind, IdeTargetSpec};

fn debugpy_probe_cache() -> &'static DashMap<(String, u128), bool> {
    static CACHE: OnceLock<DashMap<(String, u128), bool>> = OnceLock::new();
    CACHE.get_or_init(DashMap::new)
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
    if let Some(sdk) = resolve_debug_sdk_override(req, ctx, cwd) {
        let trimmed = sdk.trim_end_matches('/');
        if trimmed.ends_with("/python")
            || trimmed.ends_with("/python3")
            || trimmed.contains("/python3.")
            || Path::new(trimmed).is_file()
        {
            candidates.push(trimmed.to_string());
        } else {
            candidates.push(format!("{trimmed}/bin/python"));
            candidates.push(format!("{trimmed}/bin/python3"));
        }
    }
    if let Some(py) = ctx
        .project_env
        .as_ref()
        .and_then(|p| p.python_interpreter.as_deref())
        .filter(|s| !s.is_empty())
    {
        candidates.push(py.to_string());
    }
    if req.target.kind == IdeTargetKind::Local && !program.is_empty() {
        let allowed_roots = if ctx.caller_is_root {
            Vec::new()
        } else {
            canonical_target_roots(&req.target)
        };
        let mut cur = Path::new(&program);
        if cur.is_file() {
            cur = cur.parent().unwrap_or(cur);
        }
        for _ in 0..8 {
            if !allowed_roots.is_empty() && !allowed_roots.iter().any(|r| cur.starts_with(r)) {
                break;
            }
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
        if probe_debugpy_cached(&req.target, ctx, cand_py, cwd, &env).await {
            debugpy_host_py = Some(cand_py.clone());
            break;
        }
    }

    let (launch_command, launch_args) = if let Some(port) = req.launch.attach_port {
        (
            "attach".to_string(),
            json!({ "connect": { "host": "127.0.0.1", "port": port }, "justMyCode": false }),
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
                "module": "pytest", "program": program, "testName": test_name, "args": args,
                "cwd": cwd, "console": "internalConsole", "justMyCode": false, "python": [py_bin],
            }),
        )
    } else {
        (
            "launch".to_string(),
            json!({
                "program": program, "args": req.launch.args, "cwd": cwd,
                "console": "internalConsole", "justMyCode": false, "python": [py_bin],
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
            "adapterID": "python", "linesStartAt1": true, "columnsStartAt1": true, "pathFormat": "path"
        }),
        launch_command,
        launch_args,
        normalized_cwd: cwd.to_string(),
        initial_output: Vec::new(),
    })
}

async fn probe_debugpy_cached(
    target: &IdeTargetSpec,
    ctx: &DapStartContext,
    cand_py: &str,
    cwd: &str,
    env: &BTreeMap<String, String>,
) -> bool {
    let cache_key = if target.kind == IdeTargetKind::Local {
        std::fs::metadata(cand_py)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|d| (cand_py.to_string(), d.as_nanos()))
    } else {
        None
    };
    if let Some(ref key) = cache_key {
        if let Some(hit) = debugpy_probe_cache().get(key) {
            return *hit;
        }
    }
    let ok = run_target_command(
        target,
        ctx.ssh.as_ref(),
        cand_py,
        &["-c".into(), "import debugpy.adapter".into()],
        cwd,
        env,
        Duration::from_secs(4),
    )
    .await
    .map(|o| o.status.success())
    .unwrap_or(false);
    if let Some(key) = cache_key {
        debugpy_probe_cache().insert(key, ok);
    }
    ok
}
