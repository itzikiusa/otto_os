//! JavaScript / TypeScript (`js-debug` + V8 Inspector) and Java (`java-debug` + direct JDWP)
//! DAP adapter resolvers.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};

use otto_core::{Error, Result};
use serde_json::json;

use super::adapters::{
    locate_binary, normalize_target_path, resolve_debug_sdk_override, DapAdapterTransport,
    ResolvedDapAdapter,
};
use super::java_bridge::{infer_java_fqcn, JavaJdwpBridgeSpec};
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
    if let Some(sdk) = resolve_debug_sdk_override(req, ctx, cwd) {
        let trimmed = sdk.trim_end_matches('/');
        if trimmed.ends_with("/node") || Path::new(trimmed).is_file() {
            node_candidates.push(trimmed.to_string());
        } else {
            node_candidates.push(format!("{trimmed}/bin/node"));
        }
    }
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
    let mut program = program;
    let mut args = req.launch.args.clone();
    let mut cwd = cwd.to_string();
    if let Some(tname) = req.launch.test_name.as_deref().filter(|s| !s.is_empty()) {
        let pattern = regex::escape(tname);
        let runner = (req.target.kind == IdeTargetKind::Local)
            .then(|| js_runner_debug_launch(&program, &pattern))
            .flatten();
        match runner {
            Some((entry, runner_args, pkg_dir)) => {
                (program, args, cwd) = (entry, runner_args, pkg_dir)
            }
            None => {
                node_flags.push("--test".into());
                node_flags.push(format!("--test-name-pattern={pattern}"));
            }
        }
    }
    let is_ts = program.ends_with(".ts") || program.ends_with(".mts") || program.ends_with(".cts");
    if is_ts {
        let local_tsx = Path::new(&cwd).join("node_modules/.bin/tsx");
        if req.target.kind == IdeTargetKind::Local && local_tsx.is_file() {
            node_flags.push("--import".into());
            node_flags.push("tsx".into());
        } else {
            node_flags.push("--experimental-strip-types".into());
        }
    }

    Ok(ResolvedDapAdapter {
        adapter_name: "node-inspector-dap".into(),
        language: language.to_string(),
        transport: DapAdapterTransport::NodeInspectorBridge(NodeInspectorBridgeSpec {
            node_bin,
            node_flags,
            program: program.clone(),
            args,
            cwd: cwd.clone(),
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
        normalized_cwd: cwd,
        initial_output: Vec::new(),
    })
}

/// Debug a Jest/Vitest test through its runner's own entry from the package root, as
/// `(program, args, cwd)`; a `node:test` file (or no runner installed) keeps `node --test`.
fn js_runner_debug_launch(file: &str, pattern: &str) -> Option<(String, Vec<String>, String)> {
    if std::fs::read_to_string(file).ok()?.contains("node:test") {
        return None;
    }
    let pkg = crate::gutter::js_test_package(file)?;
    let rel = Path::new(file)
        .strip_prefix(&pkg.dir)
        .ok()?
        .to_string_lossy()
        .into_owned();
    let entry = if pkg.jest {
        "node_modules/jest/bin/jest.js"
    } else {
        "node_modules/vitest/vitest.mjs"
    };
    // A hoisted workspace keeps the runner in an ancestor's node_modules.
    let entry = pkg
        .dir
        .ancestors()
        .map(|d| d.join(entry))
        .find(|p| p.is_file())?;
    let args = if pkg.jest {
        // Jest runs the tests in this process, under the bridge's inspector.
        vec!["--runInBand".into(), rel, "-t".into(), pattern.into()]
    } else {
        // Vitest runs them in a worker: its own `--inspect-brk` opens the inspector there (it
        // treats port 0 as "default 9229", so pick a free one).
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .ok()?
            .local_addr()
            .ok()?
            .port();
        vec![
            "run".into(),
            rel,
            "-t".into(),
            pattern.into(),
            "--no-file-parallelism".into(),
            format!("--inspect-brk=127.0.0.1:{port}"),
        ]
    };
    Some((
        entry.to_string_lossy().into_owned(),
        args,
        pkg.dir.to_string_lossy().into_owned(),
    ))
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
    if let Some(sdk) = resolve_debug_sdk_override(req, ctx, cwd) {
        let trimmed = sdk.trim_end_matches('/');
        let home = trimmed
            .strip_suffix("/bin/java")
            .or_else(|| trimmed.strip_suffix("/bin/javac"))
            .or_else(|| trimmed.strip_suffix("/bin"))
            .unwrap_or(trimmed);
        jdk_bins.push(home.to_string());
    }
    if let Some(jh) = env.get("JAVA_HOME").filter(|s| !s.is_empty()) {
        if !jdk_bins.contains(jh) {
            jdk_bins.push(jh.clone());
        }
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

    let program = resolve_java_source_or_fqcn(&req.target, &req.launch.program, cwd);
    let mut initial_output = Vec::new();
    let mut source_map = HashMap::new();
    let mut source_roots = Vec::new();
    for bp in &req.breakpoints {
        let p = normalize_target_path(&req.target, &bp.path);
        if let Some(fname) = Path::new(&p).file_name().and_then(|n| n.to_str()) {
            source_map.insert(fname.to_string(), p.clone());
        }
        let sroot = infer_java_source_root(&p, cwd);
        if !source_roots.contains(&sroot) {
            source_roots.push(sroot);
        }
    }

    let mut java_args = req.launch.args.clone();
    let (classpath, main_class) = if req.launch.attach_port.is_some() {
        (cwd.to_string(), String::new())
    } else if req.target.kind == IdeTargetKind::Local {
        let javac_bin = locate_binary(&req.target, ctx, &env, &make_candidates("javac"))
            .await
            .ok_or_else(|| Error::NotFound("JDK 'javac' compiler not found".into()))?;
        let mut proj_roots = Vec::new();
        for r in std::iter::once(req.target.root.as_str())
            .chain(req.target.additional_roots.iter().map(String::as_str))
            .chain(req.project_folders.iter().map(String::as_str))
            .chain(std::iter::once(cwd))
        {
            if !r.trim().is_empty() {
                let pb = PathBuf::from(normalize_target_path(&req.target, r));
                if !proj_roots.contains(&pb) {
                    proj_roots.push(pb);
                }
            }
        }
        let prog_path = PathBuf::from(&program);
        let (target_path, explicit_main) = if program.ends_with(".java") || prog_path.exists() {
            (prog_path, None)
        } else {
            (PathBuf::from(cwd), Some(program.clone()))
        };
        let include_tests = req
            .launch
            .test_name
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty());
        // A test selector (`com.acme.CalcTest#adds`) runs through a JUnit launcher,
        // unless the class is an app with its own `main` (the gutter's main item also
        // names its class as the filter).
        let has_main =
            std::fs::read_to_string(&program).is_ok_and(|src| src.contains("static void main("));
        let test_selector = req
            .launch
            .test_name
            .clone()
            .filter(|_| include_tests && !has_main);
        let mut cwd_hash: u64 = 0xcbf29ce484222325;
        for b in cwd.as_bytes() {
            cwd_hash ^= u64::from(*b);
            cwd_hash = cwd_hash.wrapping_mul(0x100000001b3);
        }
        let out_base = crate::go_build_cache::ide_build_cache_root()
            .join("java")
            .join(format!("{cwd_hash:016x}"));
        let env_clone = env.clone();
        // A Stop that cancels this debug start drops the guard, which kills javac.
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let _cancel_on_drop = crate::proc_group::CancelOnDrop(std::sync::Arc::clone(&cancel));
        let (plan, launcher_args) = tokio::task::spawn_blocking(move || {
            let mut plan = crate::java_workspace::prepare_java_execution_cancellable(
                &target_path,
                explicit_main.as_deref(),
                include_tests,
                &proj_roots,
                &env_clone,
                &javac_bin,
                &out_base,
                cancel,
            )?;
            let args = match &test_selector {
                Some(sel) => crate::java_workspace::test_launcher::use_junit_launcher(
                    &mut plan, sel, &javac_bin, &out_base, &env_clone,
                )?,
                None => Vec::new(),
            };
            Ok::<_, Error>((plan, args))
        })
        .await
        .map_err(|e| Error::Internal(format!("java debug build task failed: {e}")))??;
        if !launcher_args.is_empty() {
            java_args = launcher_args;
        }

        for sr in &plan.source_roots {
            let s = sr.to_string_lossy().into_owned();
            if !source_roots.contains(&s) {
                source_roots.push(s);
            }
        }
        if program.ends_with(".java") {
            if let Some(fname) = Path::new(&program).file_name().and_then(|n| n.to_str()) {
                source_map.insert(fname.to_string(), program.clone());
            }
            let sroot = infer_java_source_root(&program, cwd);
            if !source_roots.contains(&sroot) {
                source_roots.push(sroot);
            }
        }
        initial_output.push(IdeDebugOutputEntry {
            category: "console".into(),
            text: format!("Compiled {} with debug symbols (-g)\n", plan.main_class),
        });
        (plan.classpath, plan.main_class)
    } else if program.ends_with(".java") {
        let javac_bin = locate_binary(&req.target, ctx, &env, &make_candidates("javac"))
            .await
            .ok_or_else(|| Error::NotFound("JDK 'javac' compiler not found".into()))?;
        let out_dir = format!("{cwd}/.otto-java-debug-classes");
        let src_root = infer_java_source_root(&program, cwd);
        if !source_roots.contains(&src_root) {
            source_roots.push(src_root.clone());
        }
        let compile_cmd = format!(
            "mkdir -p {} && {} -g -d {} -sourcepath {} {}",
            crate::target::ssh_exec::posix_quote(&out_dir),
            crate::target::ssh_exec::posix_quote(&javac_bin),
            crate::target::ssh_exec::posix_quote(&out_dir),
            crate::target::ssh_exec::posix_quote(&src_root),
            crate::target::ssh_exec::posix_quote(&program),
        );
        let out = run_target_command(
            &req.target,
            ctx.ssh.as_ref(),
            "sh",
            &["-lc".into(), compile_cmd],
            cwd,
            &env,
            crate::proc_group::BUILD_STEP_TIMEOUT,
        )
        .await?;
        if !out.status.success() {
            let err_s = String::from_utf8_lossy(&out.stderr);
            return Err(Error::Upstream(format!(
                "javac debug compile failed: {}",
                err_s.trim()
            )));
        }
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

    let mut jvm_flags = Vec::new();
    let mut spring: Vec<String> = req
        .spring_profiles
        .iter()
        .chain(req.launch.spring_profiles.iter())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    spring.dedup();
    if spring.is_empty() {
        if let Some(sp) = env
            .get("SPRING_PROFILES_ACTIVE")
            .filter(|s| !s.trim().is_empty())
        {
            spring.push(sp.trim().to_string());
        }
    }
    if !spring.is_empty() {
        jvm_flags.push(format!("-Dspring.profiles.active={}", spring.join(",")));
    }

    Ok(ResolvedDapAdapter {
        adapter_name: "java-jdwp-dap".into(),
        language: "java".into(),
        transport: DapAdapterTransport::JavaJdwpBridge(JavaJdwpBridgeSpec {
            java_bin,
            jvm_flags,
            classpath,
            main_class: main_class.clone(),
            args: java_args,
            cwd: cwd.to_string(),
            env,
            attach_port: req.launch.attach_port,
            source_roots,
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
    // `cwd` is the Maven module a test runs from: its own sources come first.
    for root in [cwd, target.root.as_str()]
        .into_iter()
        .chain(target.additional_roots.iter().map(String::as_str))
    {
        for root_sub in ["src/main/java", "src/test/java", "src", "."] {
            let cand = Path::new(root).join(root_sub).join(&rel_java);
            if cand.is_file() {
                return normalize_target_path(target, &cand.to_string_lossy());
            }
        }
    }
    // A class name no source matched stays a class name, not a path under the root.
    if raw_prog.contains('/') {
        norm
    } else {
        raw_prog.to_string()
    }
}

pub(crate) fn infer_java_source_root(java_file: &str, fallback: &str) -> String {
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
    fn java_class_name_resolves_inside_the_module_it_runs_from() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let module = root.join("legacy");
        let src = module.join("src/test/java/com/acme/legacy/OldTest.java");
        std::fs::create_dir_all(src.parent().unwrap()).unwrap();
        std::fs::write(&src, "package com.acme.legacy;\n").unwrap();
        let target = IdeTargetSpec {
            root: root.to_string_lossy().into_owned(),
            ..Default::default()
        };
        let cwd = module.to_string_lossy();
        assert_eq!(
            resolve_java_source_or_fqcn(&target, "com.acme.legacy.OldTest", &cwd),
            src.to_string_lossy()
        );
        // An unknown class stays a class name, not a path under the root.
        assert_eq!(
            resolve_java_source_or_fqcn(&target, "com.acme.Missing", &cwd),
            "com.acme.Missing"
        );
    }

    #[test]
    fn js_test_debug_launches_the_packages_runner_not_node_test() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let pkg = root.join("apps/web");
        std::fs::create_dir_all(pkg.join("src")).unwrap();
        // A hoisted workspace: the runner lives in the root node_modules.
        std::fs::create_dir_all(root.join("node_modules/vitest")).unwrap();
        std::fs::write(root.join("node_modules/vitest/vitest.mjs"), "").unwrap();
        std::fs::write(
            pkg.join("package.json"),
            r#"{"devDependencies":{"vitest":"^3"}}"#,
        )
        .unwrap();
        let file = pkg.join("src/a.test.ts");
        std::fs::write(&file, "import { it } from 'vitest';\n").unwrap();

        let (program, args, cwd) =
            js_runner_debug_launch(&file.to_string_lossy(), r"a \(b\)").unwrap();
        assert_eq!(
            program,
            root.join("node_modules/vitest/vitest.mjs")
                .to_string_lossy()
        );
        assert_eq!(cwd, pkg.to_string_lossy());
        assert_eq!(
            args[..5],
            [
                "run",
                "src/a.test.ts",
                "-t",
                r"a \(b\)",
                "--no-file-parallelism"
            ]
        );
        assert!(args[5].starts_with("--inspect-brk=127.0.0.1:"));
        assert!(!args.iter().any(|a| a == "--test"));

        // A node:test file keeps `node --test`.
        std::fs::write(&file, "import test from 'node:test';\n").unwrap();
        assert!(js_runner_debug_launch(&file.to_string_lossy(), "x").is_none());
    }

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
