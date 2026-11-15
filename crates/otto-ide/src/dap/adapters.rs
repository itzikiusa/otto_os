//! Multi-language DAP adapter resolver & launch/attach configuration builders.
//!
//! Supports Local and Remote SSH targets transparently for:
//! 1. **Go** (`dlv dap` — `exec` via external build cache, `test`, `attach`)
//! 2. **Python** (`debugpy.adapter` + built-in zero-install stdlib `bdb` DAP adapter)
//! 3. **Rust** (`codelldb` / `lldb-dap` / `lldb -P` Python bridge + Cargo JSON locator)
//! 4. **JavaScript / TypeScript** (`js-debug` + built-in V8 Inspector WebSocket bridge)
//! 5. **Java** (`java-debug` + built-in direct binary JDWP TCP bridge)
//! 6. **Shell** (`bash` `trap ... DEBUG` + `zsh` `TRAPDEBUG` FIFO bridge)
//! 7. **Runtime IDE Plugins** (`PluginDapAdapterSpec` from `IdePluginRegistry`).

use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration;

use otto_core::{Error, Result};
use serde_json::{json, Value};

pub(crate) use super::adapters_env::build_effective_env_with_manager;
pub use super::adapters_env::{build_effective_env, display_target_path, normalize_target_path};
pub(super) use super::adapters_env::{resolve_debug_sdk_override, validate_non_root_debug_request};
use super::adapters_go_py::resolve_go_adapter;
#[cfg(test)]
pub(crate) use super::adapters_go_py::resolve_go_debug_program;
use super::adapters_jvm_js::{resolve_java_adapter, resolve_js_ts_adapter};
use super::adapters_py::resolve_python_adapter;
pub use super::adapters_rust_sh::parse_cargo_artifact_executable;
use super::adapters_rust_sh::{resolve_rust_adapter, resolve_shell_adapter};
use super::java_bridge::JavaJdwpBridgeSpec;
use super::node_bridge::NodeInspectorBridgeSpec;
use super::shell_bridge::ShellBridgeSpec;
use super::{run_target_command, DapStartContext};
use crate::environments::EnvironmentManager;
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
    resolve_dap_adapter_with_manager(data_dir, req, ctx, None).await
}

struct DebugLaunchSpec<'a> {
    raw_lang: String,
    canonical_lang: String,
    provisional_adapter: String,
    plugin_spec: Option<&'a PluginDapAdapterSpec>,
    cwd: String,
    env: BTreeMap<String, String>,
}

fn resolve_debug_launch_spec<'a>(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &'a DapStartContext,
    env_mgr: Option<&EnvironmentManager>,
) -> Result<DebugLaunchSpec<'a>> {
    req.target.validate()?;
    let env = build_effective_env_with_manager(data_dir, req, ctx, env_mgr);
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
    validate_non_root_debug_request(req, ctx, &cwd, &env)?;

    let raw_lang = req.language.trim().to_ascii_lowercase();
    let (canonical_lang, provisional_adapter, plugin_spec) = match raw_lang.as_str() {
        "go" | "golang" => ("go".to_string(), "dlv".to_string(), None),
        "python" | "py" => ("python".to_string(), "python-dap".to_string(), None),
        "rust" | "rs" => ("rust".to_string(), "lldb".to_string(), None),
        "javascript" | "js" => ("javascript".to_string(), "node-inspector".to_string(), None),
        "typescript" | "ts" | "node" => {
            ("typescript".to_string(), "node-inspector".to_string(), None)
        }
        "java" => ("java".to_string(), "java-jdwp".to_string(), None),
        "shell" | "bash" | "sh" | "zsh" => ("shell".to_string(), "shell-debug".to_string(), None),
        other => {
            if let Some(ps) = ctx
                .plugin_adapters
                .iter()
                .find(|p| p.language.eq_ignore_ascii_case(other))
            {
                (other.to_string(), ps.adapter_name.clone(), Some(ps))
            } else {
                return Err(Error::Invalid(format!(
                    "unsupported debug language '{other}'; install a DAP plugin or choose go, python, rust, javascript, typescript, java, or shell"
                )));
            }
        }
    };
    Ok(DebugLaunchSpec {
        raw_lang,
        canonical_lang,
        provisional_adapter,
        plugin_spec,
        cwd,
        env,
    })
}

/// Synchronous `< 0.1 ms` preflight validation for `start_session_async`:
/// enforces target validity, non-root RBAC/path confinement (`403`), and
/// language/plugin support (`400`) before spawning heavy target builds.
pub(crate) fn preflight_debug_request(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    env_mgr: Option<&EnvironmentManager>,
) -> Result<(String, String)> {
    let spec = resolve_debug_launch_spec(data_dir, req, ctx, env_mgr)?;
    Ok((spec.canonical_lang, spec.provisional_adapter))
}

pub(crate) async fn resolve_dap_adapter_with_manager(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    env_mgr: Option<&EnvironmentManager>,
) -> Result<ResolvedDapAdapter> {
    let spec = resolve_debug_launch_spec(data_dir, req, ctx, env_mgr)?;
    if let Some(plugin_spec) = spec.plugin_spec {
        return resolve_plugin_adapter(&spec.raw_lang, plugin_spec, req, &spec.cwd, spec.env);
    }

    match spec.canonical_lang.as_str() {
        "go" => resolve_go_adapter(data_dir, req, ctx, &spec.cwd, spec.env).await,
        "python" => resolve_python_adapter(data_dir, req, ctx, &spec.cwd, spec.env).await,
        "rust" => resolve_rust_adapter(data_dir, req, ctx, &spec.cwd, spec.env).await,
        "javascript" | "typescript" => {
            resolve_js_ts_adapter(data_dir, &spec.raw_lang, req, ctx, &spec.cwd, spec.env).await
        }
        "java" => resolve_java_adapter(data_dir, req, ctx, &spec.cwd, spec.env).await,
        "shell" => resolve_shell_adapter(req, ctx, &spec.cwd, spec.env).await,
        other => Err(Error::Invalid(format!(
            "unsupported debug language '{other}'; install a DAP plugin or choose go, python, rust, javascript, typescript, java, or shell"
        ))),
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
