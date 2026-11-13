//! Multi-language DAP adapter resolver & launch/attach configuration builders.
//!
//! Supports Local and Remote SSH targets transparently for:
//! 1. **Go** (`dlv dap` — `debug`, `test` with `-test.run` & `buildFlags`, `attach`)
//! 2. **Python** (`debugpy.adapter` + built-in zero-install stdlib `bdb` DAP adapter)
//! 3. **Rust** (`codelldb` / `lldb-dap` + `cargo build/test --message-format=json` locator)
//! 4. **JavaScript / TypeScript** (`js-debug` `dapDebugServer.js` + built-in V8 Inspector bridge)
//! 5. **Java** (`java-debug` / `jdtls` DAP + built-in `javac -g` + JDWP + `jdb` bridge)
//! 6. **Runtime IDE Plugins** (`PluginDapAdapterSpec` from `IdePluginRegistry`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use otto_core::{Error, Result};
use serde_json::{json, Value};

use super::adapters_jvm_js::{resolve_java_adapter, resolve_js_ts_adapter};
use super::java_bridge::JavaJdbBridgeSpec;
use super::node_bridge::NodeInspectorBridgeSpec;
use super::py_bridge::PY_STDLIB_DAP_BRIDGE;
use super::shell_bridge::ShellBridgeSpec;
use super::{run_target_command, DapStartContext};
use crate::types::{
    IdeDebugOutputEntry, IdeStartDebugReq, IdeTargetKind, IdeTargetSpec, PluginDapAdapterSpec,
};

/// Transport mechanism used by [`super::session::DapManager`] to connect to an adapter.
#[derive(Debug, Clone)]
pub enum DapAdapterTransport {
    /// Adapter speaks `Content-Length` DAP directly on child stdin/stdout.
    Stdio {
        command: String,
        args: Vec<String>,
        cwd: String,
        env: BTreeMap<String, String>,
    },
    /// Adapter spawns a headless loopback TCP server and prints its port on stdout/stderr
    /// (e.g. `dlv dap --listen=127.0.0.1:0`, `codelldb --port 0`, `dapDebugServer.js 0`).
    TcpListen {
        command: String,
        args: Vec<String>,
        cwd: String,
        env: BTreeMap<String, String>,
    },
    /// Connect directly to an already-listening DAP TCP server (`host:port`).
    DirectTcpAttach { host: String, port: u16 },
    /// Built-in Node.js / V8 Inspector-to-DAP bridge (`node --inspect-brk=127.0.0.1:0`).
    NodeInspectorBridge(NodeInspectorBridgeSpec),
    /// Built-in JDK `jdb` / JDWP-to-DAP bridge (`java -agentlib:jdwp=...` + `jdb`).
    JavaJdbBridge(JavaJdbBridgeSpec),
    /// Built-in zero-install Bash `trap ... DEBUG` bridge for `.sh` / `.bash` scripts.
    ShellBridge(ShellBridgeSpec),
}

/// Fully resolved DAP launch/attach plan ready to start a session.
#[derive(Debug, Clone)]
pub struct ResolvedDapAdapter {
    pub adapter_name: String,
    pub language: String,
    pub transport: DapAdapterTransport,
    pub initialize_args: Value,
    /// `"launch"` or `"attach"`.
    pub launch_command: String,
    pub launch_args: Value,
    pub normalized_cwd: String,
    pub initial_output: Vec<IdeDebugOutputEntry>,
}

/// Canonicalize a path on `Local` targets (even when prospective or prefixed with `file://`)
/// so macOS `/var/folders` symlinks always match `/private/var/folders` DWARF compile units.
pub fn normalize_target_path(target: &IdeTargetSpec, raw: &str) -> String {
    if raw.trim().is_empty() {
        return raw.to_string();
    }
    let clean = crate::target::sdk_paths::strip_file_uri(raw.trim());
    if target.kind == IdeTargetKind::Local {
        let candidate = if Path::new(&clean).is_absolute() {
            PathBuf::from(&clean)
        } else {
            Path::new(&target.root).join(&clean)
        };
        if let Ok(canon) = std::fs::canonicalize(&candidate) {
            return canon.to_string_lossy().into_owned();
        }
        if let Ok(prospective) = otto_core::secret_paths::canonicalize_prospective(&candidate) {
            return prospective.to_string_lossy().into_owned();
        }
    }
    clean
}

/// Merge user login+interactive shell env (`~/.zshenv`/`~/.zshrc`), project SDK paths,
/// `.env`/secret vars, run-config overrides, and launch-specific overrides.
pub fn build_effective_env(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
) -> BTreeMap<String, String> {
    let mut env = ctx.effective_env.clone();

    if req.target.kind == IdeTargetKind::Local {
        let inherit_shell = ctx
            .project_env
            .as_ref()
            .map(|p| p.inherit_shell_env)
            .unwrap_or(true)
            && ctx
                .run_config
                .as_ref()
                .map(|r| r.inherit_shell_env)
                .unwrap_or(true);
        if inherit_shell {
            for (k, v) in
                crate::environments::cached_effective_env_for_local_root(data_dir, &req.target.root)
            {
                env.entry(k).or_insert(v);
            }
            for (k, v) in crate::environments::cached_shell_env_snapshot() {
                env.entry(k).or_insert(v);
            }
        }
        for (k, v) in std::env::vars() {
            env.entry(k).or_insert(v);
        }
    }

    let mut path_entries: Vec<String> = Vec::new();
    if let Some(pe) = &ctx.project_env {
        if let Some(jh) = pe.java_home.as_deref().filter(|s| !s.is_empty()) {
            env.insert("JAVA_HOME".into(), jh.to_string());
            path_entries.push(format!("{jh}/bin"));
        }
        if let Some(gr) = pe.go_root.as_deref().filter(|s| !s.is_empty()) {
            env.insert("GOROOT".into(), gr.to_string());
            path_entries.push(format!("{gr}/bin"));
        }
        if let Some(nh) = pe.node_home.as_deref().filter(|s| !s.is_empty()) {
            path_entries.push(format!("{nh}/bin"));
        }
        if let Some(py) = pe.python_interpreter.as_deref().filter(|s| !s.is_empty()) {
            if let Some(parent) = Path::new(py).parent().and_then(|p| p.to_str()) {
                path_entries.push(parent.to_string());
            }
        }
        if let Some(extra) = pe.extra_path.as_deref().filter(|s| !s.is_empty()) {
            for seg in extra.split(':').filter(|s| !s.is_empty()) {
                path_entries.push(seg.to_string());
            }
        }
        for entry in pe.env_vars.iter().filter(|e| !e.secret) {
            if let Some(val) = &entry.value {
                env.insert(entry.key.clone(), val.clone());
            }
        }
    }

    if req.target.kind == IdeTargetKind::Local {
        let tc = data_dir.join("toolchains");
        for sub in [
            "go/bin",
            "go/current/bin",
            "dlv",
            "rust/cargo/bin",
            "rust/bin",
            "node/current/bin",
            "python/current/bin",
            "python/bin",
            "java/current/bin",
        ] {
            let p = tc.join(sub);
            if p.is_dir() {
                path_entries.push(p.to_string_lossy().into_owned());
            }
        }
        if let Some(home) = dirs::home_dir() {
            for rel in ["go/bin", ".cargo/bin", ".local/bin"] {
                let p = home.join(rel);
                if p.is_dir() {
                    path_entries.push(p.to_string_lossy().into_owned());
                }
            }
        }
        for sys_bin in ["/opt/homebrew/bin", "/usr/local/bin", "/usr/local/go/bin"] {
            if Path::new(sys_bin).is_dir() {
                path_entries.push(sys_bin.to_string());
            }
        }
    }

    if let Some(existing_path) = env.get("PATH").cloned() {
        for seg in existing_path.split(':').filter(|s| !s.is_empty()) {
            if !path_entries.iter().any(|e| e == seg) {
                path_entries.push(seg.to_string());
            }
        }
    }
    if !path_entries.is_empty() {
        env.insert("PATH".into(), path_entries.join(":"));
    }

    if let Some(rc) = &ctx.run_config {
        for (k, v) in &rc.env_overrides {
            env.insert(k.clone(), v.clone());
        }
    }
    for (k, v) in &req.launch.env {
        env.insert(k.clone(), v.clone());
    }
    env
}

pub(super) async fn locate_binary(
    target: &IdeTargetSpec,
    ctx: &DapStartContext,
    env: &BTreeMap<String, String>,
    candidates: &[String],
) -> Option<String> {
    match target.kind {
        IdeTargetKind::Local => {
            let path_var = env.get("PATH").cloned().unwrap_or_default();
            for cand in candidates {
                if cand.contains('/') {
                    if Path::new(cand).is_file() {
                        return Some(cand.clone());
                    }
                } else {
                    for dir in path_var.split(':').filter(|d| !d.is_empty()) {
                        let full = Path::new(dir).join(cand);
                        if full.is_file() {
                            return Some(full.to_string_lossy().into_owned());
                        }
                    }
                }
            }
            None
        }
        IdeTargetKind::Ssh => {
            for cand in candidates {
                let probe_cmd = if cand.starts_with("$HOME/") {
                    format!("[ -x \"{cand}\" ] && printf '%s\\n' \"{cand}\"")
                } else {
                    format!(
                        "command -v {} 2>/dev/null | head -n 1",
                        super::shell_quote(cand)
                    )
                };
                let probe_args = vec!["-lc".to_string(), probe_cmd];
                if let Ok(out) = run_target_command(
                    target,
                    ctx.ssh.as_ref(),
                    "sh",
                    &probe_args,
                    &target.root,
                    env,
                    Duration::from_secs(4),
                )
                .await
                {
                    let found = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if out.status.success() && !found.is_empty() {
                        return Some(found);
                    }
                }
            }
            None
        }
    }
}

/// Resolve the appropriate DAP adapter and build its launch/attach configuration.
pub async fn resolve_dap_adapter(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
) -> Result<ResolvedDapAdapter> {
    req.target.validate()?;
    let env = build_effective_env(data_dir, req, ctx);
    let raw_cwd = req
        .launch
        .cwd
        .as_deref()
        .filter(|s| !s.trim().is_empty())
        .or_else(|| {
            ctx.run_config
                .as_ref()
                .and_then(|c| c.cwd.as_deref())
                .filter(|s| !s.trim().is_empty())
        })
        .unwrap_or(&req.target.root);
    let cwd = normalize_target_path(&req.target, raw_cwd);
    let lang = req.language.trim().to_ascii_lowercase();

    if !matches!(
        lang.as_str(),
        "go" | "golang"
            | "python"
            | "py"
            | "rust"
            | "rs"
            | "javascript"
            | "js"
            | "typescript"
            | "ts"
            | "node"
            | "java"
            | "shell"
            | "bash"
            | "sh"
            | "zsh"
    ) {
        if let Some(plugin_spec) = ctx
            .plugin_adapters
            .iter()
            .find(|p| p.language.eq_ignore_ascii_case(&lang))
        {
            return resolve_plugin_adapter(&lang, plugin_spec, req, &cwd, env);
        }
    }

    match lang.as_str() {
        "go" | "golang" => resolve_go_adapter(data_dir, req, ctx, &cwd, env).await,
        "python" | "py" => resolve_python_adapter(data_dir, req, ctx, &cwd, env).await,
        "rust" | "rs" => resolve_rust_adapter(data_dir, req, ctx, &cwd, env).await,
        "javascript" | "js" | "typescript" | "ts" | "node" => {
            resolve_js_ts_adapter(data_dir, &lang, req, ctx, &cwd, env).await
        }
        "java" => resolve_java_adapter(data_dir, req, ctx, &cwd, env).await,
        "shell" | "bash" | "sh" | "zsh" => resolve_shell_adapter(req, ctx, &cwd, env).await,
        other => {
            if let Some(plugin_spec) = ctx
                .plugin_adapters
                .iter()
                .find(|p| p.language.eq_ignore_ascii_case(other))
            {
                resolve_plugin_adapter(other, plugin_spec, req, &cwd, env)
            } else {
                Err(Error::Invalid(format!(
                    "unsupported debug language '{other}'; install a DAP plugin or choose go, python, rust, javascript, typescript, java, or shell"
                )))
            }
        }
    }
}

fn resolve_plugin_adapter(
    language: &str,
    spec: &PluginDapAdapterSpec,
    req: &IdeStartDebugReq,
    cwd: &str,
    env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    let program = normalize_target_path(&req.target, &req.launch.program);
    let test_name = req.launch.test_name.clone().unwrap_or_default();
    let subst = |s: &str| {
        s.replace("${program}", &program)
            .replace("${cwd}", cwd)
            .replace("${workspaceFolder}", &req.target.root)
            .replace("${test_name}", &test_name)
    };
    let args: Vec<String> = spec.args.iter().map(|a| subst(a)).collect();
    let launch_args = if let Some(tpl) = &spec.launch_template {
        let raw = serde_json::to_string(tpl).unwrap_or_else(|_| "{}".into());
        serde_json::from_str(&subst(&raw))
            .unwrap_or_else(|_| json!({ "program": program, "cwd": cwd }))
    } else {
        json!({
            "program": program,
            "args": req.launch.args,
            "cwd": cwd,
        })
    };
    let is_tcp = args
        .iter()
        .any(|a| a.contains("--port") || a.contains("--listen"));
    let transport = if is_tcp {
        DapAdapterTransport::TcpListen {
            command: spec.command.clone(),
            args,
            cwd: cwd.to_string(),
            env,
        }
    } else {
        DapAdapterTransport::Stdio {
            command: spec.command.clone(),
            args,
            cwd: cwd.to_string(),
            env,
        }
    };
    Ok(ResolvedDapAdapter {
        adapter_name: spec.adapter_name.clone(),
        language: language.to_string(),
        transport,
        initialize_args: json!({
            "adapterID": spec.adapter_name,
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "pathFormat": "path"
        }),
        launch_command: if req.launch.attach_port.is_some() {
            "attach".into()
        } else {
            "launch".into()
        },
        launch_args,
        normalized_cwd: cwd.to_string(),
        initial_output: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// 1. Go (`dlv dap`)
// ---------------------------------------------------------------------------

async fn resolve_go_adapter(
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
                normalized_cwd: cwd.to_string(),
                initial_output: Vec::new(),
            });
        }
    } else if req.launch.test_name.is_some() || raw_prog.ends_with("_test.go") {
        let pkg_dir = if raw_prog.ends_with(".go") {
            Path::new(&raw_prog)
                .parent()
                .and_then(|p| p.to_str())
                .unwrap_or(cwd)
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
            "cwd": cwd,
            "args": test_args,
            "env": env,
        });
        if !build_flags.is_empty() {
            body["buildFlags"] = Value::String(build_flags);
        }
        ("launch".to_string(), body)
    } else {
        let debug_pkg = resolve_go_debug_program(&req.target, &raw_prog, cwd);
        let mut body = json!({
            "mode": "debug",
            "program": debug_pkg,
            "cwd": cwd,
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
            cwd: cwd.to_string(),
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
        normalized_cwd: cwd.to_string(),
        initial_output: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// 2. Python (`debugpy.adapter` + Zero-Install Stdlib `bdb` DAP Fallback)
// ---------------------------------------------------------------------------

async fn resolve_python_adapter(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    let mut candidates = Vec::new();
    if let Some(py) = ctx
        .project_env
        .as_ref()
        .and_then(|p| p.python_interpreter.as_deref())
        .filter(|s| !s.is_empty())
    {
        candidates.push(py.to_string());
    }
    let root_path = Path::new(&req.target.root);
    candidates.push(
        root_path
            .join(".venv/bin/python")
            .to_string_lossy()
            .into_owned(),
    );
    candidates.push(
        root_path
            .join("venv/bin/python")
            .to_string_lossy()
            .into_owned(),
    );
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

    // Check whether `py_bin` itself or Otto's managed `toolchains/python/dap` venv has `debugpy`.
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

    let program = normalize_target_path(&req.target, &req.launch.program);
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

// ---------------------------------------------------------------------------
// 3. Rust (`codelldb` / `lldb-dap` + Cargo JSON artifact locator)
// ---------------------------------------------------------------------------

/// Parse `cargo {build,test} --message-format=json` stdout to extract the compiled
/// debug executable path.
pub fn parse_cargo_artifact_executable(
    stdout_jsonl: &str,
    want_test: bool,
    target_hint: Option<&str>,
) -> Option<String> {
    let mut candidates: Vec<(bool, String, String)> = Vec::new();
    for line in stdout_jsonl.lines() {
        let Ok(val) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if val.get("reason").and_then(Value::as_str) != Some("compiler-artifact") {
            continue;
        }
        let Some(exe) = val
            .get("executable")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let is_test = val
            .get("profile")
            .and_then(|p| p.get("test"))
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let name = val
            .get("target")
            .and_then(|t| t.get("name"))
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        candidates.push((is_test, name, exe.to_string()));
    }
    if let Some(hint) = target_hint.filter(|h| !h.is_empty()) {
        if let Some((_, _, exe)) = candidates
            .iter()
            .rev()
            .find(|(t, n, _)| *t == want_test && n == hint)
        {
            return Some(exe.clone());
        }
        if let Some((_, _, exe)) = candidates.iter().rev().find(|(_, n, _)| n == hint) {
            return Some(exe.clone());
        }
    }
    if let Some((_, _, exe)) = candidates.iter().rev().find(|(t, _, _)| *t == want_test) {
        return Some(exe.clone());
    }
    candidates.pop().map(|(_, _, exe)| exe)
}

async fn resolve_rust_adapter(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    mut env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    let mut initial_output = Vec::new();
    let raw_prog = normalize_target_path(&req.target, &req.launch.program);
    let want_test = req.launch.test_name.is_some();

    let executable = if req.launch.attach_port.is_some()
        || (!raw_prog.is_empty()
            && !raw_prog.ends_with(".rs")
            && !raw_prog.ends_with("Cargo.toml")
            && (req.target.kind == IdeTargetKind::Ssh || Path::new(&raw_prog).is_file()))
    {
        raw_prog
    } else {
        env.insert("CARGO_PROFILE_DEV_DEBUG".into(), "true".into());
        compile_rust_debug_artifact(
            req,
            ctx,
            cwd,
            &raw_prog,
            want_test,
            &env,
            &mut initial_output,
        )
        .await?
    };

    let mut codelldb_candidates = Vec::new();
    if req.target.kind == IdeTargetKind::Local {
        for rel in [
            "toolchains/rust/dap/extension/adapter/codelldb",
            "toolchains/rust/codelldb/adapter/codelldb",
            "toolchains/codelldb/adapter/codelldb",
        ] {
            codelldb_candidates.push(data_dir.join(rel).to_string_lossy().into_owned());
        }
    } else {
        codelldb_candidates
            .push("$HOME/.otto/toolchains/rust/dap/extension/adapter/codelldb".into());
        codelldb_candidates.push("$HOME/.otto/toolchains/rust/codelldb/adapter/codelldb".into());
    }
    codelldb_candidates.push("codelldb".into());

    let mut prog_args = Vec::new();
    if let Some(tname) = req.launch.test_name.as_deref().filter(|s| !s.is_empty()) {
        prog_args.push(tname.to_string());
        prog_args.push("--exact".into());
        prog_args.push("--nocapture".into());
    }
    prog_args.extend(req.launch.args.clone());

    if let Some(codelldb_bin) = locate_binary(&req.target, ctx, &env, &codelldb_candidates).await {
        let (launch_command, launch_args) = if let Some(port) = req.launch.attach_port {
            (
                "attach".to_string(),
                json!({ "gdbRemotePort": port, "cwd": cwd }),
            )
        } else {
            (
                "launch".to_string(),
                json!({
                    "program": executable,
                    "args": prog_args,
                    "cwd": cwd,
                    "stopOnEntry": false
                }),
            )
        };
        return Ok(ResolvedDapAdapter {
            adapter_name: "codelldb".into(),
            language: "rust".into(),
            transport: DapAdapterTransport::TcpListen {
                command: codelldb_bin,
                args: vec!["--port".into(), "0".into()],
                cwd: cwd.to_string(),
                env,
            },
            initialize_args: json!({
                "adapterID": "lldb",
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "pathFormat": "path"
            }),
            launch_command,
            launch_args,
            normalized_cwd: cwd.to_string(),
            initial_output,
        });
    }

    let lldb_dap_candidates = vec![
        "lldb-dap".to_string(),
        "lldb-vscode".to_string(),
        "/Library/Developer/CommandLineTools/usr/bin/lldb-dap".to_string(),
        "/Applications/Xcode.app/Contents/Developer/usr/bin/lldb-dap".to_string(),
        "/opt/homebrew/opt/llvm/bin/lldb-dap".to_string(),
        "/usr/bin/lldb-dap".to_string(),
    ];
    let lldb_bin = locate_binary(&req.target, ctx, &env, &lldb_dap_candidates)
        .await
        .ok_or_else(|| {
            Error::NotFound(
                "Rust debugger ('codelldb' or 'lldb-dap') not found; install via IDE → Toolchains"
                    .into(),
            )
        })?;

    let (launch_command, launch_args) = if let Some(port) = req.launch.attach_port {
        (
            "attach".to_string(),
            json!({ "gdb-remote-port": port, "cwd": cwd }),
        )
    } else {
        (
            "launch".to_string(),
            json!({
                "program": executable,
                "args": prog_args,
                "cwd": cwd,
                "stopOnEntry": false
            }),
        )
    };

    Ok(ResolvedDapAdapter {
        adapter_name: "lldb-dap".into(),
        language: "rust".into(),
        transport: DapAdapterTransport::Stdio {
            command: lldb_bin,
            args: Vec::new(),
            cwd: cwd.to_string(),
            env,
        },
        initialize_args: json!({
            "adapterID": "lldb-dap",
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "pathFormat": "path"
        }),
        launch_command,
        launch_args,
        normalized_cwd: cwd.to_string(),
        initial_output,
    })
}

async fn compile_rust_debug_artifact(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    raw_prog: &str,
    want_test: bool,
    env: &BTreeMap<String, String>,
    initial_output: &mut Vec<IdeDebugOutputEntry>,
) -> Result<String> {
    let cargo_cwd = find_cargo_dir(&req.target, raw_prog, cwd);
    let mut cargo_args = if want_test {
        vec![
            "test".into(),
            "--no-run".into(),
            "--message-format=json".into(),
        ]
    } else {
        vec!["build".into(), "--message-format=json".into()]
    };
    if let Some(toolchain) = ctx
        .project_env
        .as_ref()
        .and_then(|p| p.rust_toolchain.as_deref())
        .filter(|s| !s.is_empty())
    {
        cargo_args.insert(0, format!("+{toolchain}"));
    }
    let out = run_target_command(
        &req.target,
        ctx.ssh.as_ref(),
        "cargo",
        &cargo_args,
        &cargo_cwd,
        env,
        Duration::from_secs(120),
    )
    .await?;
    let stderr_txt = String::from_utf8_lossy(&out.stderr).to_string();
    if !stderr_txt.trim().is_empty() {
        initial_output.push(IdeDebugOutputEntry {
            category: "console".into(),
            text: stderr_txt.clone(),
        });
    }
    if !out.status.success() {
        return Err(Error::Upstream(format!(
            "cargo debug build failed: {}",
            stderr_txt.trim()
        )));
    }
    let stdout_txt = String::from_utf8_lossy(&out.stdout);
    let hint = Path::new(raw_prog)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| *s != "lib" && *s != "main");
    parse_cargo_artifact_executable(&stdout_txt, want_test, hint).ok_or_else(|| {
        Error::Upstream("cargo succeeded but produced no debuggable binary artifact".into())
    })
}

fn find_cargo_dir(target: &IdeTargetSpec, program: &str, fallback_cwd: &str) -> String {
    if target.kind == IdeTargetKind::Local && !program.is_empty() {
        let mut cur = Path::new(program);
        if cur.is_file() {
            cur = cur.parent().unwrap_or(cur);
        }
        loop {
            if cur.join("Cargo.toml").is_file() {
                return cur.to_string_lossy().into_owned();
            }
            let Some(parent) = cur.parent() else { break };
            cur = parent;
        }
    }
    fallback_cwd.to_string()
}

/// Resolve a Go debug target so multi-file `package main` directories (`cmd/server/main.go` +
/// sibling `.go` files) compile as a package rather than a broken single file, and workspace-root
/// launches on `cmd/*` microservice layouts auto-locate `package main`.
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
        // Check `cmd/<service>` layout (standard Go microservice convention, e.g. `go_admission`).
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

async fn resolve_shell_adapter(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    let bash_candidates = vec![
        "/opt/homebrew/bin/bash".to_string(),
        "/usr/local/bin/bash".to_string(),
        "bash".to_string(),
        "/bin/bash".to_string(),
    ];
    let bash_bin = locate_binary(&req.target, ctx, &env, &bash_candidates)
        .await
        .ok_or_else(|| Error::NotFound("Bash shell ('bash') not found on target".into()))?;
    let program = normalize_target_path(&req.target, &req.launch.program);
    if program.trim().is_empty() {
        return Err(Error::Invalid(
            "program path (.sh/.bash script) is required to debug a shell script".into(),
        ));
    }
    Ok(ResolvedDapAdapter {
        adapter_name: "bash-trap-dap".into(),
        language: "shell".into(),
        transport: DapAdapterTransport::ShellBridge(ShellBridgeSpec {
            bash_bin,
            program: program.clone(),
            args: req.launch.args.clone(),
            cwd: cwd.to_string(),
            env,
        }),
        initialize_args: json!({
            "adapterID": "bash",
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "pathFormat": "path"
        }),
        launch_command: "launch".into(),
        launch_args: json!({ "program": program, "cwd": cwd, "args": req.launch.args }),
        normalized_cwd: cwd.to_string(),
        initial_output: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::IdeDebugLaunchSpec;

    #[test]
    fn parses_cargo_json_compiler_artifacts_for_bin_and_test() {
        let sample = r#"
{"reason":"compiler-artifact","target":{"name":"helper","kind":["lib"]},"profile":{"test":false},"executable":null}
{"reason":"compiler-artifact","target":{"name":"demo","kind":["bin"]},"profile":{"test":false},"executable":"/tmp/target/debug/demo"}
{"reason":"compiler-artifact","target":{"name":"demo","kind":["bin"]},"profile":{"test":true},"executable":"/tmp/target/debug/deps/demo-1234abcd"}
{"reason":"build-finished","success":true}
"#;
        assert_eq!(
            parse_cargo_artifact_executable(sample, false, Some("demo")).as_deref(),
            Some("/tmp/target/debug/demo")
        );
        assert_eq!(
            parse_cargo_artifact_executable(sample, true, Some("demo")).as_deref(),
            Some("/tmp/target/debug/deps/demo-1234abcd")
        );
    }

    #[test]
    fn resolves_custom_plugin_dap_adapter_with_template_substitution() {
        let req = IdeStartDebugReq {
            target: IdeTargetSpec::local("/workspace/kt"),
            language: "kotlin".into(),
            config_id: None,
            launch: IdeDebugLaunchSpec {
                program: "/workspace/kt/Main.kt".into(),
                args: vec!["--flag".into()],
                cwd: Some("/workspace/kt".into()),
                test_name: Some("testGreeting".into()),
                attach_port: None,
                env: BTreeMap::new(),
            },
            breakpoints: Vec::new(),
        };
        let plugin = PluginDapAdapterSpec {
            language: "kotlin".into(),
            adapter_name: "kotlin-debug-adapter".into(),
            command: "kotlin-debug-adapter".into(),
            args: vec!["--workspace=${workspaceFolder}".into()],
            launch_template: Some(json!({
                "mainClass": "${program}",
                "projectRoot": "${cwd}",
                "testFilter": "${test_name}"
            })),
        };
        let resolved =
            resolve_plugin_adapter("kotlin", &plugin, &req, "/workspace/kt", BTreeMap::new())
                .unwrap();
        assert_eq!(resolved.adapter_name, "kotlin-debug-adapter");
        assert_eq!(resolved.launch_command, "launch");
        assert_eq!(resolved.launch_args["mainClass"], "/workspace/kt/Main.kt");
        assert_eq!(resolved.launch_args["testFilter"], "testGreeting");
    }
}
