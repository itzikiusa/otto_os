//! Multi-language DAP adapter resolver & launch/attach configuration builders.
//!
//! Supports Local and Remote SSH targets transparently for:
//! 1. **Go** (`dlv dap` — `debug`, `test` with `-test.run` & `buildFlags`, `attach`)
//! 2. **Python** (`debugpy.adapter` + built-in zero-install stdlib `bdb` DAP adapter)
//! 3. **Rust** (`codelldb` / `lldb-dap` / `lldb -P` Python bridge + Cargo JSON locator)
//! 4. **JavaScript / TypeScript** (`js-debug` + built-in V8 Inspector WebSocket bridge)
//! 5. **Java** (`java-debug` + built-in direct binary JDWP TCP bridge)
//! 6. **Shell** (`bash` `trap ... DEBUG` + `zsh` `TRAPDEBUG` FIFO bridge)
//! 7. **Runtime IDE Plugins** (`PluginDapAdapterSpec` from `IdePluginRegistry`).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use otto_core::{Error, Result};
use serde_json::{json, Value};

#[cfg(test)]
pub(crate) use super::adapters_go_py::resolve_go_debug_program;
use super::adapters_go_py::{resolve_go_adapter, resolve_python_adapter};
use super::adapters_jvm_js::{resolve_java_adapter, resolve_js_ts_adapter};
pub use super::adapters_rust_sh::parse_cargo_artifact_executable;
use super::adapters_rust_sh::{resolve_rust_adapter, resolve_shell_adapter};
use super::java_bridge::JavaJdwpBridgeSpec;
use super::node_bridge::NodeInspectorBridgeSpec;
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
    /// Adapter spawns a headless loopback TCP server and prints its port on stdout/stderr.
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
    /// Built-in direct binary JDWP TCP-to-DAP bridge (`java -agentlib:jdwp=...`).
    JavaJdwpBridge(JavaJdwpBridgeSpec),
    /// Built-in zero-install Bash/Zsh `DEBUG` trap bridge for `.sh` / `.bash` / `.zsh` scripts.
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

/// Canonicalize a path on `Local` targets (expanding `~` and resolving macOS `/var/folders` ->
/// `/private/var/folders` symlinks) so breakpoints and DWARF/CDP/JDWP stack frames never mismatch.
pub fn normalize_target_path(target: &IdeTargetSpec, raw: &str) -> String {
    if raw.trim().is_empty() {
        return raw.to_string();
    }
    let mut clean = crate::target::sdk_paths::strip_file_uri(raw.trim());
    if target.kind == IdeTargetKind::Local {
        if clean == "~" {
            if let Some(home) = dirs::home_dir() {
                clean = home.to_string_lossy().into_owned();
            }
        } else if let Some(rest) = clean.strip_prefix("~/") {
            if let Some(home) = dirs::home_dir() {
                clean = home.join(rest).to_string_lossy().into_owned();
            }
        }
        let candidate = if Path::new(&clean).is_absolute() {
            PathBuf::from(&clean)
        } else {
            Path::new(&target.root).join(&clean)
        };
        if let Ok(canon) = std::fs::canonicalize(&candidate) {
            return canon.to_string_lossy().into_owned();
        }
        if let Some(prospective) = otto_core::secret_paths::canonicalize_prospective(&candidate) {
            return prospective.to_string_lossy().into_owned();
        }
        #[cfg(target_os = "macos")]
        if let Some(rest) = clean.strip_prefix("/var/") {
            if Path::new("/private/var").is_dir() {
                return format!("/private/var/{rest}");
            }
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
            launch: IdeDebugLaunchSpec {
                program: "/workspace/kt/Main.kt".into(),
                args: vec!["--flag".into()],
                cwd: Some("/workspace/kt".into()),
                test_name: Some("testGreeting".into()),
                ..IdeDebugLaunchSpec::default()
            },
            ..IdeStartDebugReq::default()
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
