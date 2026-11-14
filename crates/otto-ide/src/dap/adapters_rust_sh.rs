//! Rust (`codelldb`, `lldb-dap`, and built-in `lldb -P` `SBDebugger` Python DAP bridge)
//! and Shell (`bash` + `zsh`) DAP adapter resolvers.

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use otto_core::{Error, Result};
use serde_json::{json, Value};

use super::adapters::{
    locate_binary, normalize_target_path, DapAdapterTransport, ResolvedDapAdapter,
};
use super::lldb_bridge::LLDB_PYTHON_DAP_BRIDGE;
use super::shell_bridge::{ShellBridgeSpec, ShellFlavor};
use super::{run_target_command, DapStartContext};
use crate::types::{IdeDebugOutputEntry, IdeStartDebugReq, IdeTargetKind, IdeTargetSpec};

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

pub(super) async fn resolve_rust_adapter(
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

    let mut prog_args = Vec::new();
    if let Some(tname) = req.launch.test_name.as_deref().filter(|s| !s.is_empty()) {
        prog_args.push(tname.to_string());
        prog_args.push("--exact".into());
        prog_args.push("--nocapture".into());
    }
    prog_args.extend(req.launch.args.clone());

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
    if let Some(lldb_bin) = locate_binary(&req.target, ctx, &env, &lldb_dap_candidates).await {
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
        return Ok(ResolvedDapAdapter {
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
        });
    }

    // Fallback: Built-in `lldb -P` (`SBDebugger` Python API) stdio DAP bridge — works out
    // of the box on macOS with Xcode Command Line Tools (`/usr/bin/lldb` + `/usr/bin/python3`)!
    if let Some((py_bin, lldb_py_dir)) = locate_lldb_python_bridge(&req.target, ctx, &env).await {
        let mut bridge_env = env;
        let merged_pypath = match bridge_env.get("PYTHONPATH") {
            Some(prev) if !prev.is_empty() => format!("{lldb_py_dir}:{prev}"),
            _ => lldb_py_dir,
        };
        bridge_env.insert("PYTHONPATH".into(), merged_pypath);
        return Ok(ResolvedDapAdapter {
            adapter_name: "lldb-python-dap".into(),
            language: "rust".into(),
            transport: DapAdapterTransport::Stdio {
                command: py_bin,
                args: vec!["-u".into(), "-c".into(), LLDB_PYTHON_DAP_BRIDGE.into()],
                cwd: cwd.to_string(),
                env: bridge_env,
            },
            initialize_args: json!({
                "adapterID": "lldb",
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "pathFormat": "path"
            }),
            launch_command: "launch".into(),
            launch_args: json!({
                "program": executable,
                "args": prog_args,
                "cwd": cwd,
                "stopOnEntry": false
            }),
            normalized_cwd: cwd.to_string(),
            initial_output,
        });
    }

    Err(Error::NotFound(
        "Rust debugger ('codelldb', 'lldb-dap', or Xcode 'lldb') not found; install Xcode Command Line Tools or CodeLLDB via IDE → Toolchains".into(),
    ))
}

async fn locate_lldb_python_bridge(
    target: &IdeTargetSpec,
    ctx: &DapStartContext,
    env: &BTreeMap<String, String>,
) -> Option<(String, String)> {
    let lldb_candidates = vec![
        "/usr/bin/lldb".to_string(),
        "/Library/Developer/CommandLineTools/usr/bin/lldb".to_string(),
        "/Applications/Xcode.app/Contents/Developer/usr/bin/lldb".to_string(),
        "lldb".to_string(),
    ];
    let lldb_bin = locate_binary(target, ctx, env, &lldb_candidates).await?;
    let p_out = run_target_command(
        target,
        ctx.ssh.as_ref(),
        &lldb_bin,
        &["-P".into()],
        &target.root,
        env,
        Duration::from_secs(5),
    )
    .await
    .ok()?;
    let lldb_py_dir = String::from_utf8_lossy(&p_out.stdout).trim().to_string();
    if !p_out.status.success() || lldb_py_dir.is_empty() {
        return None;
    }

    let py_candidates = [
        "/usr/bin/python3".to_string(),
        "/Library/Developer/CommandLineTools/usr/bin/python3".to_string(),
        "/Applications/Xcode.app/Contents/Developer/usr/bin/python3".to_string(),
        "python3".to_string(),
    ];
    let mut probe_env = env.clone();
    probe_env.insert("PYTHONPATH".into(), lldb_py_dir.clone());
    for py in &py_candidates {
        if target.kind == IdeTargetKind::Local && py.contains('/') && !Path::new(py).is_file() {
            continue;
        }
        let ok = run_target_command(
            target,
            ctx.ssh.as_ref(),
            py,
            &["-c".into(), "import lldb".into()],
            &target.root,
            &probe_env,
            Duration::from_secs(5),
        )
        .await
        .map(|o| o.status.success())
        .unwrap_or(false);
        if ok {
            return Some((py.clone(), lldb_py_dir));
        }
    }
    None
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

pub(super) async fn resolve_shell_adapter(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    let program = normalize_target_path(&req.target, &req.launch.program);
    if program.trim().is_empty() {
        return Err(Error::Invalid(
            "program path (.sh/.bash/.zsh script) is required to debug a shell script".into(),
        ));
    }

    let mut want_zsh = req.language.eq_ignore_ascii_case("zsh") || program.ends_with(".zsh");
    if !want_zsh && req.target.kind == IdeTargetKind::Local {
        if let Ok(head) = tokio::fs::read_to_string(&program).await {
            if let Some(first_line) = head.lines().next() {
                if first_line.starts_with("#!") && first_line.contains("zsh") {
                    want_zsh = true;
                }
            }
        }
    }

    let (flavor, adapter_name, shell_bin) = if want_zsh {
        let zsh_candidates = vec![
            "/opt/homebrew/bin/zsh".to_string(),
            "/usr/local/bin/zsh".to_string(),
            "/bin/zsh".to_string(),
            "zsh".to_string(),
        ];
        let bin = locate_binary(&req.target, ctx, &env, &zsh_candidates)
            .await
            .ok_or_else(|| Error::NotFound("Zsh shell ('zsh') not found on target".into()))?;
        (ShellFlavor::Zsh, "zsh-trap-dap", bin)
    } else {
        let bash_candidates = vec![
            "/opt/homebrew/bin/bash".to_string(),
            "/usr/local/bin/bash".to_string(),
            "bash".to_string(),
            "/bin/bash".to_string(),
        ];
        let bin = locate_binary(&req.target, ctx, &env, &bash_candidates)
            .await
            .ok_or_else(|| Error::NotFound("Bash shell ('bash') not found on target".into()))?;
        (ShellFlavor::Bash, "bash-trap-dap", bin)
    };

    Ok(ResolvedDapAdapter {
        adapter_name: adapter_name.into(),
        language: "shell".into(),
        transport: DapAdapterTransport::ShellBridge(ShellBridgeSpec {
            shell_bin,
            flavor,
            program: program.clone(),
            args: req.launch.args.clone(),
            cwd: cwd.to_string(),
            env,
        }),
        initialize_args: json!({
            "adapterID": adapter_name,
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
