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
#[derive(Clone)]
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

impl std::fmt::Debug for NodeInspectorBridgeSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let env_keys: Vec<&str> = self.env.keys().map(String::as_str).collect();
        f.debug_struct("NodeInspectorBridgeSpec")
            .field("node_bin", &self.node_bin)
            .field("node_flags", &self.node_flags)
            .field("program", &self.program)
            .field("args", &self.args)
            .field("cwd", &self.cwd)
            .field("env_keys", &env_keys)
            .field("attach_port", &self.attach_port)
            .finish()
    }
}

impl std::fmt::Debug for JavaJdwpBridgeSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let env_keys: Vec<&str> = self.env.keys().map(String::as_str).collect();
        f.debug_struct("JavaJdwpBridgeSpec")
            .field("java_bin", &self.java_bin)
            .field("jvm_flags", &self.jvm_flags)
            .field("main_class", &self.main_class)
            .field("args", &self.args)
            .field("cwd", &self.cwd)
            .field("env_keys", &env_keys)
            .field("attach_port", &self.attach_port)
            .finish()
    }
}

impl std::fmt::Debug for ShellBridgeSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let env_keys: Vec<&str> = self.env.keys().map(String::as_str).collect();
        f.debug_struct("ShellBridgeSpec")
            .field("shell_bin", &self.shell_bin)
            .field("flavor", &self.flavor)
            .field("program", &self.program)
            .field("args", &self.args)
            .field("cwd", &self.cwd)
            .field("env_keys", &env_keys)
            .finish()
    }
}

impl std::fmt::Debug for DapAdapterTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Stdio {
                command,
                args,
                cwd,
                env,
            } => {
                let env_keys: Vec<&str> = env.keys().map(String::as_str).collect();
                f.debug_struct("Stdio")
                    .field("command", command)
                    .field("args", args)
                    .field("cwd", cwd)
                    .field("env_keys", &env_keys)
                    .finish()
            }
            Self::TcpListen {
                command,
                args,
                cwd,
                env,
            } => {
                let env_keys: Vec<&str> = env.keys().map(String::as_str).collect();
                f.debug_struct("TcpListen")
                    .field("command", command)
                    .field("args", args)
                    .field("cwd", cwd)
                    .field("env_keys", &env_keys)
                    .finish()
            }
            Self::DirectTcpAttach { host, port } => f
                .debug_struct("DirectTcpAttach")
                .field("host", host)
                .field("port", port)
                .finish(),
            Self::NodeInspectorBridge(spec) => {
                f.debug_tuple("NodeInspectorBridge").field(spec).finish()
            }
            Self::JavaJdwpBridge(spec) => f.debug_tuple("JavaJdwpBridge").field(spec).finish(),
            Self::ShellBridge(spec) => f.debug_tuple("ShellBridge").field(spec).finish(),
        }
    }
}

/// Fully resolved DAP launch/attach plan ready to start a session.
#[derive(Clone)]
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

impl std::fmt::Debug for ResolvedDapAdapter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ResolvedDapAdapter")
            .field("adapter_name", &self.adapter_name)
            .field("language", &self.language)
            .field("transport", &self.transport)
            .field("launch_command", &self.launch_command)
            .field("normalized_cwd", &self.normalized_cwd)
            .finish()
    }
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
                        crate::target::ssh_exec::posix_quote(cand)
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
    let canonical_alias = match raw_lang.as_str() {
        "go" | "golang" => "go",
        "python" | "py" => "python",
        "rust" | "rs" => "rust",
        "javascript" | "js" => "javascript",
        "typescript" | "ts" | "node" => "typescript",
        "java" => "java",
        "shell" | "bash" | "sh" | "zsh" => "shell",
        other => other,
    };
    let matched_plugin = ctx.plugin_adapters.iter().find(|p| {
        p.language.eq_ignore_ascii_case(&raw_lang)
            || p.language.eq_ignore_ascii_case(canonical_alias)
            || p.adapter_name.eq_ignore_ascii_case(&raw_lang)
    });
    let (canonical_lang, provisional_adapter, plugin_spec) = if let Some(ps) = matched_plugin {
        (
            canonical_alias.to_string(),
            ps.adapter_name.clone(),
            Some(ps),
        )
    } else {
        match canonical_alias {
            "go" => ("go".to_string(), "dlv".to_string(), None),
            "python" => ("python".to_string(), "python-dap".to_string(), None),
            "rust" => ("rust".to_string(), "lldb".to_string(), None),
            "javascript" => ("javascript".to_string(), "node-inspector".to_string(), None),
            "typescript" => ("typescript".to_string(), "node-inspector".to_string(), None),
            "java" => ("java".to_string(), "java-jdwp".to_string(), None),
            "shell" => ("shell".to_string(), "shell-debug".to_string(), None),
            other => {
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
        return resolve_plugin_adapter_with_target(
            data_dir,
            &spec.canonical_lang,
            plugin_spec,
            req,
            ctx,
            &spec.cwd,
            spec.env,
        )
        .await;
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

fn interpolate_json_value(
    val: &Value,
    subst: &dyn Fn(&str) -> String,
    attach_port: Option<u16>,
) -> Value {
    match val {
        Value::String(s) => {
            if s == "${port}" || s == "{port}" {
                if let Some(p) = attach_port {
                    return json!(p);
                }
            }
            Value::String(subst(s))
        }
        Value::Array(items) => Value::Array(
            items
                .iter()
                .map(|item| interpolate_json_value(item, subst, attach_port))
                .collect(),
        ),
        Value::Object(map) => Value::Object(
            map.iter()
                .map(|(k, v)| (subst(k), interpolate_json_value(v, subst, attach_port)))
                .collect(),
        ),
        other => other.clone(),
    }
}

async fn resolve_plugin_adapter_with_target(
    data_dir: &Path,
    language: &str,
    spec: &PluginDapAdapterSpec,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    mut env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    if req.target.kind == IdeTargetKind::Local {
        let tc_bins = crate::lsp_host::servers::collect_toolchain_bin_dirs(data_dir);
        if !tc_bins.is_empty() {
            let prefix: String = tc_bins
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(":");
            let cur_path = env.get("PATH").cloned().unwrap_or_default();
            env.insert(
                "PATH".into(),
                if cur_path.is_empty() {
                    prefix
                } else {
                    format!("{prefix}:{cur_path}")
                },
            );
        }
    }
    let mut resolved = resolve_plugin_adapter(language, spec, req, cwd, env)?;
    let (cmd_slot, env_ref) = match &mut resolved.transport {
        DapAdapterTransport::Stdio { command, env, .. }
        | DapAdapterTransport::TcpListen { command, env, .. } => (command, env),
        _ => return Ok(resolved),
    };
    if let Some(found) =
        locate_binary(&req.target, ctx, env_ref, std::slice::from_ref(cmd_slot)).await
    {
        *cmd_slot = found;
    }
    Ok(resolved)
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
    let port_str = req
        .launch
        .attach_port
        .map(|p| p.to_string())
        .unwrap_or_else(|| "0".to_string());
    let subst = |s: &str| {
        s.replace("${program}", &program)
            .replace("{program}", &program)
            .replace("${cwd}", cwd)
            .replace("{cwd}", cwd)
            .replace("${workspaceFolder}", &req.target.root)
            .replace("{workspaceFolder}", &req.target.root)
            .replace("${test_name}", &test_name)
            .replace("{test_name}", &test_name)
            .replace("${port}", &port_str)
            .replace("{port}", &port_str)
    };
    let command = subst(&spec.command);
    let args: Vec<String> = spec.args.iter().map(|a| subst(a)).collect();
    let launch_args = if let Some(tpl) = &spec.launch_template {
        interpolate_json_value(tpl, &subst, req.launch.attach_port)
    } else {
        let mut base = json!({
            "program": program,
            "args": req.launch.args,
            "cwd": cwd,
        });
        if let Some(p) = req.launch.attach_port {
            base["port"] = json!(p);
        }
        base
    };
    let is_tcp = args
        .iter()
        .any(|a| a.contains("--port") || a.contains("--listen"));
    let transport = if is_tcp {
        DapAdapterTransport::TcpListen {
            command,
            args,
            cwd: cwd.to_string(),
            env,
        }
    } else {
        DapAdapterTransport::Stdio {
            command,
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

    #[test]
    fn plugin_dap_adapter_escapes_quotes_and_backslashes_and_overrides_builtin() {
        let tmp = tempfile::tempdir().unwrap();
        let req = IdeStartDebugReq {
            target: IdeTargetSpec::local(tmp.path().to_string_lossy()),
            language: "py".into(),
            launch: IdeDebugLaunchSpec {
                program: "/ws/my \"quoted\"\\app.py".into(),
                cwd: Some("/ws/dir\\sub".into()),
                test_name: Some("test_\"quote\"".into()),
                attach_port: Some(5678),
                ..IdeDebugLaunchSpec::default()
            },
            ..IdeStartDebugReq::default()
        };
        let custom_py = PluginDapAdapterSpec {
            language: "python".into(),
            adapter_name: "custom-py-dap".into(),
            command: "${workspaceFolder}/bin/custom-py-dap".into(),
            args: vec!["--port=${port}".into()],
            launch_template: Some(json!({
                "program": "${program}",
                "cwd": "${cwd}",
                "port": "${port}",
                "nested": ["${test_name}", "127.0.0.1:${port}"]
            })),
        };
        let ctx = DapStartContext {
            caller_is_root: true,
            plugin_adapters: vec![custom_py.clone()],
            ..Default::default()
        };

        let (canon, adapter) = preflight_debug_request(tmp.path(), &req, &ctx, None).unwrap();
        assert_eq!(canon, "python");
        assert_eq!(adapter, "custom-py-dap");

        let resolved =
            resolve_plugin_adapter("python", &custom_py, &req, "/ws/dir\\sub", BTreeMap::new())
                .unwrap();
        assert_eq!(resolved.launch_command, "attach");
        assert_eq!(resolved.launch_args["program"], "/ws/my \"quoted\"\\app.py");
        assert_eq!(resolved.launch_args["cwd"], "/ws/dir\\sub");
        assert_eq!(resolved.launch_args["port"], json!(5678));
        assert_eq!(
            resolved.launch_args["nested"],
            json!(["test_\"quote\"", "127.0.0.1:5678"])
        );
    }
}
