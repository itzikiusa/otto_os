//! JavaScript / TypeScript (`js-debug` + V8 Inspector) and Java (`java-debug` + `jdb`/JDWP)
//! DAP adapter resolvers.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::time::Duration;

use otto_core::{Error, Result};
use serde_json::json;

use super::adapters::{
    locate_binary, normalize_target_path, DapAdapterTransport, ResolvedDapAdapter,
};
use super::java_bridge::{infer_java_fqcn, JavaJdbBridgeSpec};
use super::node_bridge::NodeInspectorBridgeSpec;
use super::{run_target_command, DapStartContext};
use crate::types::{IdeDebugOutputEntry, IdeStartDebugReq, IdeTargetKind, IdeTargetSpec};

pub(super) async fn resolve_js_ts_adapter(
    data_dir: &Path,
    language: &str,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    let mut node_candidates = Vec::new();
    if let Some(nh) = ctx
        .project_env
        .as_ref()
        .and_then(|p| p.node_home.as_deref())
        .filter(|s| !s.is_empty())
    {
        node_candidates.push(format!("{nh}/bin/node"));
    }
    if req.target.kind == IdeTargetKind::Local {
        node_candidates.push(
            data_dir
                .join("toolchains/node/current/bin/node")
                .to_string_lossy()
                .into_owned(),
        );
    }
    node_candidates.push("node".into());

    let node_bin = locate_binary(&req.target, ctx, &env, &node_candidates)
        .await
        .ok_or_else(|| {
            Error::NotFound(
                "Node.js runtime not found; install Node.js via IDE → Toolchains".into(),
            )
        })?;

    let program = normalize_target_path(&req.target, &req.launch.program);

    let js_debug_candidates = [
        data_dir.join("toolchains/node/dap/js-debug/src/dapDebugServer.js"),
        data_dir.join("toolchains/node/js-debug/src/dapDebugServer.js"),
        data_dir.join("toolchains/js-debug/src/dapDebugServer.js"),
    ];
    if env.get("OTTO_JS_ADAPTER").map(String::as_str) == Some("js-debug") {
        if let Some(script) = js_debug_candidates.iter().find(|p| p.is_file()) {
            return Ok(ResolvedDapAdapter {
                adapter_name: "js-debug".into(),
                language: language.to_string(),
                transport: DapAdapterTransport::TcpListen {
                    command: node_bin.clone(),
                    args: vec![
                        script.to_string_lossy().into_owned(),
                        "0".into(),
                        "127.0.0.1".into(),
                    ],
                    cwd: cwd.to_string(),
                    env,
                },
                initialize_args: json!({
                    "adapterID": "pwa-node",
                    "linesStartAt1": true,
                    "columnsStartAt1": true,
                    "pathFormat": "path"
                }),
                launch_command: if req.launch.attach_port.is_some() {
                    "attach".into()
                } else {
                    "launch".into()
                },
                launch_args: json!({
                    "type": "pwa-node",
                    "program": program,
                    "args": req.launch.args,
                    "cwd": cwd,
                    "runtimeExecutable": node_bin,
                }),
                normalized_cwd: cwd.to_string(),
                initial_output: Vec::new(),
            });
        }
    }

    let mut node_flags = Vec::new();
    let is_ts = program.ends_with(".ts") || program.ends_with(".mts") || program.ends_with(".cts");
    if is_ts {
        let local_tsx = Path::new(cwd).join("node_modules/.bin/tsx");
        if req.target.kind == IdeTargetKind::Local && local_tsx.is_file() {
            node_flags.push("--import".into());
            node_flags.push("tsx".into());
        } else {
            node_flags.push("--experimental-strip-types".into());
        }
    }
    if let Some(tname) = req.launch.test_name.as_deref().filter(|s| !s.is_empty()) {
        node_flags.push("--test".into());
        node_flags.push(format!("--test-name-pattern={}", regex::escape(tname)));
    }

    Ok(ResolvedDapAdapter {
        adapter_name: "node-inspector-dap".into(),
        language: language.to_string(),
        transport: DapAdapterTransport::NodeInspectorBridge(NodeInspectorBridgeSpec {
            node_bin,
            node_flags,
            program: program.clone(),
            args: req.launch.args.clone(),
            cwd: cwd.to_string(),
            env,
            attach_port: req.launch.attach_port,
        }),
        initialize_args: json!({
            "adapterID": "node",
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "pathFormat": "path"
        }),
        launch_command: if req.launch.attach_port.is_some() {
            "attach".into()
        } else {
            "launch".into()
        },
        launch_args: json!({ "program": program, "cwd": cwd }),
        normalized_cwd: cwd.to_string(),
        initial_output: Vec::new(),
    })
}

pub(super) async fn resolve_java_adapter(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    if let Some(dap_port) = env
        .get("OTTO_JAVA_DEBUG_DAP_PORT")
        .and_then(|s| s.parse::<u16>().ok())
    {
        return Ok(ResolvedDapAdapter {
            adapter_name: "java-debug".into(),
            language: "java".into(),
            transport: DapAdapterTransport::DirectTcpAttach {
                host: "127.0.0.1".into(),
                port: dap_port,
            },
            initialize_args: json!({
                "adapterID": "java",
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "pathFormat": "path"
            }),
            launch_command: if req.launch.attach_port.is_some() {
                "attach".into()
            } else {
                "launch".into()
            },
            launch_args: json!({
                "mainClass": req.launch.program,
                "cwd": cwd,
                "args": req.launch.args,
                "hostName": "127.0.0.1",
                "port": req.launch.attach_port.unwrap_or(0),
            }),
            normalized_cwd: cwd.to_string(),
            initial_output: Vec::new(),
        });
    }

    let mut jdk_bins = Vec::new();
    if let Some(jh) = env.get("JAVA_HOME").filter(|s| !s.is_empty()) {
        jdk_bins.push(jh.clone());
    }
    if req.target.kind == IdeTargetKind::Local {
        let managed = data_dir.join("toolchains/java/current");
        if managed.is_dir() {
            jdk_bins.push(managed.to_string_lossy().into_owned());
        }
    }

    let make_candidates = |tool: &str| -> Vec<String> {
        let mut out: Vec<String> = jdk_bins.iter().map(|h| format!("{h}/bin/{tool}")).collect();
        out.push(tool.to_string());
        out
    };

    let java_bin = locate_binary(&req.target, ctx, &env, &make_candidates("java"))
        .await
        .ok_or_else(|| {
            Error::NotFound("JDK 'java' not found; install Temurin JDK via IDE → Toolchains".into())
        })?;
    let jdb_bin = locate_binary(&req.target, ctx, &env, &make_candidates("jdb"))
        .await
        .ok_or_else(|| {
            Error::NotFound(
                "JDK 'jdb' debugger not found; install a full JDK via IDE → Toolchains".into(),
            )
        })?;

    let program = resolve_java_source_or_fqcn(&req.target, &req.launch.program, cwd);
    let mut initial_output = Vec::new();
    let mut source_map = HashMap::new();
    for bp in &req.breakpoints {
        let p = normalize_target_path(&req.target, &bp.path);
        if let Some(fname) = Path::new(&p).file_name().and_then(|n| n.to_str()) {
            source_map.insert(fname.to_string(), p);
        }
    }

    let (classpath, main_class) = if req.launch.attach_port.is_some() {
        (cwd.to_string(), String::new())
    } else if program.ends_with(".java") {
        let javac_bin = locate_binary(&req.target, ctx, &env, &make_candidates("javac"))
            .await
            .ok_or_else(|| Error::NotFound("JDK 'javac' compiler not found".into()))?;
        let out_dir = format!("{cwd}/.otto-java-debug-classes");
        let src_root = infer_java_source_root(&program, cwd);
        let compile_cmd = format!(
            "mkdir -p {} && {} -g -d {} -sourcepath {} {}",
            super::shell_quote(&out_dir),
            super::shell_quote(&javac_bin),
            super::shell_quote(&out_dir),
            super::shell_quote(&src_root),
            super::shell_quote(&program),
        );
        let out = run_target_command(
            &req.target,
            ctx.ssh.as_ref(),
            "sh",
            &["-lc".into(), compile_cmd],
            cwd,
            &env,
            Duration::from_secs(30),
        )
        .await?;
        if !out.status.success() {
            let err_s = String::from_utf8_lossy(&out.stderr);
            return Err(Error::Upstream(format!(
                "javac debug compile failed: {}",
                err_s.trim()
            )));
        }
        initial_output.push(IdeDebugOutputEntry {
            category: "console".into(),
            text: format!("Compiled {program} with debug symbols (-g)\n"),
        });
        if let Some(fname) = Path::new(&program).file_name().and_then(|n| n.to_str()) {
            source_map.insert(fname.to_string(), program.clone());
        }
        let fqcn = infer_java_fqcn(&req.target, &program).await;
        (out_dir, fqcn)
    } else {
        let cp = env
            .get("CLASSPATH")
            .cloned()
            .unwrap_or_else(|| cwd.to_string());
        (cp, program)
    };

    Ok(ResolvedDapAdapter {
        adapter_name: "jdb-jdwp-dap".into(),
        language: "java".into(),
        transport: DapAdapterTransport::JavaJdbBridge(JavaJdbBridgeSpec {
            java_bin,
            jdb_bin,
            classpath,
            main_class: main_class.clone(),
            args: req.launch.args.clone(),
            cwd: cwd.to_string(),
            env,
            attach_port: req.launch.attach_port,
            source_map,
        }),
        initialize_args: json!({
            "adapterID": "java",
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "pathFormat": "path"
        }),
        launch_command: if req.launch.attach_port.is_some() {
            "attach".into()
        } else {
            "launch".into()
        },
        launch_args: json!({ "mainClass": main_class, "cwd": cwd }),
        normalized_cwd: cwd.to_string(),
        initial_output,
    })
}

fn resolve_java_source_or_fqcn(target: &IdeTargetSpec, raw_prog: &str, cwd: &str) -> String {
    let norm = normalize_target_path(target, raw_prog);
    if norm.ends_with(".java") || target.kind != IdeTargetKind::Local {
        return norm;
    }
    let simple = raw_prog.rsplit('.').next().unwrap_or(raw_prog);
    let in_cwd = Path::new(cwd).join(format!("{simple}.java"));
    if in_cwd.is_file() {
        return normalize_target_path(target, &in_cwd.to_string_lossy());
    }
    let rel_java = format!("{}.java", raw_prog.replace('.', "/"));
    for root_sub in ["src/main/java", "src/test/java", "src", "."] {
        let cand = Path::new(&target.root).join(root_sub).join(&rel_java);
        if cand.is_file() {
            return normalize_target_path(target, &cand.to_string_lossy());
        }
    }
    norm
}

pub(super) fn infer_java_source_root(java_file: &str, fallback: &str) -> String {
    for marker in ["/src/main/java/", "/src/test/java/", "/src/"] {
        if let Some((prefix, _)) = java_file.split_once(marker) {
            let trimmed = marker.trim_end_matches('/');
            return format!("{prefix}{trimmed}");
        }
    }
    Path::new(java_file)
        .parent()
        .and_then(|p| p.to_str())
        .unwrap_or(fallback)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn infers_maven_gradle_java_source_roots() {
        assert_eq!(
            infer_java_source_root("/repo/java/src/main/java/com/otto/App.java", "/repo"),
            "/repo/java/src/main/java"
        );
        assert_eq!(
            infer_java_source_root("/repo/java/src/test/java/com/otto/AppTest.java", "/repo"),
            "/repo/java/src/test/java"
        );
    }
}
