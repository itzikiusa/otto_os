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
    let raw_program = req.launch.program.trim();
    let is_browser_attach = req.language.eq_ignore_ascii_case("browser")
        || req.language.eq_ignore_ascii_case("chrome")
        || raw_program.starts_with("chrome://")
        || raw_program.starts_with("browser://")
        || raw_program.starts_with("http://")
        || raw_program.starts_with("https://");
    let effective_attach_port = req
        .launch
        .attach_port
        .or_else(|| is_browser_attach.then_some(9222));

    if let Some(attach_port) = effective_attach_port {
        return Ok(ResolvedDapAdapter {
            adapter_name: if is_browser_attach {
                "browser-cdp-dap".into()
            } else {
                "node-inspector-dap".into()
            },
            language: language.to_string(),
            transport: DapAdapterTransport::NodeInspectorBridge(NodeInspectorBridgeSpec {
                node_bin: String::new(),
                node_flags: Vec::new(),
                program: raw_program.to_string(),
                args: Vec::new(),
                cwd: cwd.to_string(),
                env,
                attach_port: Some(attach_port),
            }),
            initialize_args: json!({
                "adapterID": if is_browser_attach { "chrome" } else { "node" },
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "pathFormat": "path"
            }),
            launch_command: "attach".into(),
            launch_args: json!({ "program": raw_program, "cwd": cwd, "port": attach_port }),
            normalized_cwd: cwd.to_string(),
            initial_output: vec![IdeDebugOutputEntry {
                category: "console".into(),
                text: format!("Attaching CDP debugger to 127.0.0.1:{attach_port} (cwd: {cwd})\n"),
                seq: 0,
            }],
        });
    }

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

    let mut node_flags = vec!["--enable-source-maps".into()];
    let mut program = program;
    let mut args = req.launch.args.clone();
    let mut cwd = cwd.to_string();
    if let Some(tname) = req.launch.test_name.as_deref().filter(|s| !s.is_empty()) {
        let pattern = regex::escape(tname);
        if req.target.kind == IdeTargetKind::Local {
            check_js_test_debug_supported(&program)?;
        }
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
    } else if req.target.kind == IdeTargetKind::Local {
        let (effective_prog, effective_args) =
            unwrap_framework_dev_program(&program, &args, ctx.run_config.as_ref());
        if let Some((entry, fw_args)) =
            resolve_web_framework_cli_entry(&cwd, &effective_prog, &effective_args)
        {
            program = entry;
            args = fw_args;
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

/// Unwrap a discovered framework dev run configuration (whose target_path may be
/// `package.json` or a framework CLI name) into `(cli_name, cli_args)` so Node DAP
/// attaches to the real framework CLI instead of launching `package.json` (BUG-27).
pub(crate) fn unwrap_framework_dev_program(
    program: &str,
    args: &[String],
    run_config: Option<&crate::types::IdeRunConfig>,
) -> (String, Vec<String>) {
    let base = Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program);
    if base.eq_ignore_ascii_case("package.json") {
        if let Some(cfg) = run_config {
            let mut rest = cfg.args.as_slice();
            if rest.first().map(String::as_str) == Some("--no-install") {
                rest = &rest[1..];
            }
            if let Some(cli) = rest.first().map(String::as_str) {
                if matches!(cli, "vite" | "next" | "ng" | "nuxt" | "svelte-kit") {
                    return (cli.to_string(), rest[1..].to_vec());
                }
            }
        }
    }
    (program.to_string(), args.to_vec())
}

/// Reject Playwright/Karma single-test Node-inspector debugging explicitly (BUG-30):
/// Playwright executes tests in a child worker process and Karma executes in a browser,
/// so attaching to the Node CLI launcher will never hit test breakpoints.
pub(crate) fn check_js_test_debug_supported(file: &str) -> Result<()> {
    let src = std::fs::read_to_string(file).unwrap_or_default();
    if src.contains("node:test") {
        return Ok(());
    }
    let pkg = crate::gutter::js_test_package(file);
    let runner = if src.contains("@playwright/test") {
        Some(crate::gutter::JsTestRunnerKind::Playwright)
    } else {
        pkg.as_ref().map(|p| p.runner)
    };
    match runner {
        Some(crate::gutter::JsTestRunnerKind::Playwright) => Err(Error::Invalid(
            "Single-test Debug is not supported for Playwright under the built-in Node inspector bridge (Playwright runs tests inside a separate worker process). Use Run or Playwright's inspector (`PWDEBUG=1`)."
                .into(),
        )),
        Some(crate::gutter::JsTestRunnerKind::Karma) => Err(Error::Invalid(
            "Single-test Debug is not supported for Karma/Angular browser tests under the Node inspector bridge. Use Run or attach a Chrome CDP session."
                .into(),
        )),
        _ => Ok(()),
    }
}

/// Resolve a web framework dev/CLI entrypoint (`vite`, `next`, `ng`, `nuxt`, `svelte-kit`)
/// inside local `node_modules` so Node `--inspect-brk` can debug SSR / dev servers directly.
pub(crate) fn resolve_web_framework_cli_entry(
    cwd: &str,
    program: &str,
    existing_args: &[String],
) -> Option<(String, Vec<String>)> {
    let base = Path::new(program)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program);
    let candidates: &[(&str, &[&str])] = match base {
        "vite" => &[("node_modules/vite/bin/vite.js", &[])],
        "next" => &[("node_modules/next/dist/bin/next", &["dev"])],
        "ng" => &[("node_modules/@angular/cli/bin/ng.js", &["serve"])],
        "nuxt" => &[("node_modules/nuxt/bin/nuxt.mjs", &["dev"])],
        "svelte-kit" => &[("node_modules/@sveltejs/kit/svelte-kit.js", &["dev"])],
        _ => return None,
    };
    let cwd_path = Path::new(cwd);
    for (rel_entry, default_args) in candidates {
        if let Some(found) = cwd_path
            .ancestors()
            .map(|d| d.join(rel_entry))
            .find(|p| p.is_file())
        {
            let args = if existing_args.is_empty() {
                default_args.iter().map(|s| (*s).to_string()).collect()
            } else {
                existing_args.to_vec()
            };
            return Some((found.to_string_lossy().into_owned(), args));
        }
    }
    None
}

/// Debug a Jest / Vitest / Playwright test through its runner's own entry from the package root,
/// as `(program, args, cwd)`; a `node:test` file (or no runner installed) keeps `node --test`.
fn js_runner_debug_launch(file: &str, pattern: &str) -> Option<(String, Vec<String>, String)> {
    let src = std::fs::read_to_string(file).ok()?;
    if src.contains("node:test") {
        return None;
    }
    let pkg = crate::gutter::js_test_package(file)?;
    let rel = Path::new(file)
        .strip_prefix(&pkg.dir)
        .ok()?
        .to_string_lossy()
        .into_owned();
    let runner = if src.contains("@playwright/test") {
        crate::gutter::JsTestRunnerKind::Playwright
    } else {
        pkg.runner
    };
    let entry_candidates: &[&str] = match runner {
        crate::gutter::JsTestRunnerKind::Jest => &["node_modules/jest/bin/jest.js"],
        crate::gutter::JsTestRunnerKind::Playwright => &[
            "node_modules/@playwright/test/cli.js",
            "node_modules/playwright/cli.js",
        ],
        crate::gutter::JsTestRunnerKind::Karma => &[
            "node_modules/@angular/cli/bin/ng.js",
            "node_modules/karma/bin/karma",
        ],
        crate::gutter::JsTestRunnerKind::Vitest => &["node_modules/vitest/vitest.mjs"],
    };
    // A hoisted workspace keeps the runner in an ancestor's node_modules.
    let entry = entry_candidates.iter().find_map(|rel_entry| {
        pkg.dir
            .ancestors()
            .map(|d| d.join(rel_entry))
            .find(|p| p.is_file())
    })?;
    let args = match runner {
        crate::gutter::JsTestRunnerKind::Jest => {
            // Jest runs the tests in this process, under the bridge's inspector.
            vec!["--runInBand".into(), rel, "-t".into(), pattern.into()]
        }
        crate::gutter::JsTestRunnerKind::Playwright => {
            vec![
                "test".into(),
                rel,
                "-g".into(),
                pattern.into(),
                "--workers=1".into(),
                "--timeout=0".into(),
                "--reporter=list".into(),
            ]
        }
        crate::gutter::JsTestRunnerKind::Karma => {
            if entry.to_string_lossy().contains("@angular/cli") {
                vec![
                    "test".into(),
                    "--watch=false".into(),
                    format!("--include={rel}"),
                ]
            } else {
                vec!["start".into(), "--single-run".into()]
            }
        }
        crate::gutter::JsTestRunnerKind::Vitest => {
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
        }
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
        let is_kotlin_file = program.ends_with(".kt") || program.ends_with(".kts");
        let (target_path, explicit_main) =
            if program.ends_with(".java") || is_kotlin_file || prog_path.exists() {
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
        let has_main = std::fs::read_to_string(&program).is_ok_and(|src| {
            src.contains("static void main(")
                || (is_kotlin_file && (src.contains("fun main(") || src.contains("fun main ()")))
        });
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
        if program.ends_with(".java") || program.ends_with(".kt") || program.ends_with(".kts") {
            if let Some(fname) = Path::new(&program).file_name().and_then(|n| n.to_str()) {
                source_map.insert(fname.to_string(), program.clone());
            }
            let sroot = infer_java_source_root(&program, cwd);
            if !source_roots.contains(&sroot) {
                source_roots.push(sroot);
            }
        }
        // Mixed Java/Kotlin modules: register sibling kotlin/java source roots so JDWP
        // breakpoints and stepping work seamlessly across Java <-> Kotlin calls.
        for extra_sub in [
            "src/main/kotlin",
            "src/test/kotlin",
            "src/main/java",
            "src/test/java",
        ] {
            let cand = Path::new(cwd).join(extra_sub);
            if cand.is_dir() {
                let s = cand.to_string_lossy().into_owned();
                if !source_roots.contains(&s) {
                    source_roots.push(s);
                }
            }
        }
        initial_output.push(IdeDebugOutputEntry {
            category: "console".into(),
            text: format!("Compiled {} with debug symbols (-g)\n", plan.main_class),
            seq: 0,
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
    if norm.ends_with(".java")
        || norm.ends_with(".kt")
        || norm.ends_with(".kts")
        || target.kind != IdeTargetKind::Local
    {
        return norm;
    }
    let simple = raw_prog.rsplit('.').next().unwrap_or(raw_prog);
    let simple_kt = simple.strip_suffix("Kt").unwrap_or(simple);
    for cand_name in [
        format!("{simple}.java"),
        format!("{simple}.kt"),
        format!("{simple_kt}.kt"),
    ] {
        let in_cwd = Path::new(cwd).join(&cand_name);
        if in_cwd.is_file() {
            return normalize_target_path(target, &in_cwd.to_string_lossy());
        }
    }
    let rel_java = format!("{}.java", raw_prog.replace('.', "/"));
    let rel_kt = format!("{}.kt", raw_prog.replace('.', "/"));
    let rel_kt_trimmed = raw_prog
        .strip_suffix("Kt")
        .map(|base| format!("{}.kt", base.replace('.', "/")));
    // `cwd` is the Maven/Gradle module a test runs from: its own sources come first.
    for root in [cwd, target.root.as_str()]
        .into_iter()
        .chain(target.additional_roots.iter().map(String::as_str))
    {
        for root_sub in [
            "src/main/java",
            "src/test/java",
            "src/main/kotlin",
            "src/test/kotlin",
            "src",
            ".",
        ] {
            for rel in std::iter::once(rel_java.as_str())
                .chain(std::iter::once(rel_kt.as_str()))
                .chain(rel_kt_trimmed.as_deref())
            {
                let cand = Path::new(root).join(root_sub).join(rel);
                if cand.is_file() {
                    return normalize_target_path(target, &cand.to_string_lossy());
                }
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
    for marker in [
        "/src/main/java/",
        "/src/test/java/",
        "/src/main/kotlin/",
        "/src/test/kotlin/",
        "/src/",
    ] {
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

    #[test]
    fn web_framework_detection_run_configs_gutter_and_parsers_cover_all_frameworks() {
        use crate::environments::discovery::{detect_web_framework, WebFrameworkKind};
        use crate::environments::runner_parsers::parse_js_test_line;
        use crate::gutter::detect_gutter_items;
        use crate::lsp_host::servers;
        use crate::types::{IdeRunnerKind, IdeTestCaseStatus};

        // 1. Framework LSPs in registry
        for (lang, cmd) in [
            ("svelte", "svelteserver"),
            ("vue", "vue-language-server"),
            ("angular", "ngserver"),
        ] {
            let def = servers::REGISTRY.iter().find(|d| d.lang == lang);
            assert!(def.is_some(), "missing LSP def for {lang}");
            assert_eq!(def.unwrap().cmd, cmd);
        }

        // 2. Framework detection across Angular, Next.js, SvelteKit, Nuxt, Vue, React, Vite
        let tmp = tempfile::tempdir().unwrap();
        let mk = |name: &str, files: &[(&str, &str)]| {
            let dir = tmp.path().join(name);
            std::fs::create_dir_all(&dir).unwrap();
            for (rel, body) in files {
                let p = dir.join(rel);
                if let Some(par) = p.parent() {
                    std::fs::create_dir_all(par).unwrap();
                }
                std::fs::write(&p, body).unwrap();
            }
            dir
        };

        let ng_dir = mk(
            "ng-app",
            &[
                ("angular.json", "{}"),
                (
                    "package.json",
                    r#"{"dependencies":{"@angular/core":"^19.0.0"},"scripts":{"start":"ng serve","test":"ng test"}}"#,
                ),
                (
                    "src/app/app.component.spec.ts",
                    "describe('AppComponent', () => {\n  it('should create', () => {});\n});\n",
                ),
            ],
        );
        let ng_pkg: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(ng_dir.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(
            detect_web_framework(&ng_dir, Some(&ng_pkg)),
            Some(WebFrameworkKind::Angular)
        );
        let ng_gutter = detect_gutter_items(
            &ng_dir
                .join("src/app/app.component.spec.ts")
                .to_string_lossy(),
            &std::fs::read_to_string(ng_dir.join("src/app/app.component.spec.ts")).unwrap(),
            &[],
        );
        assert_eq!(ng_gutter.len(), 2);
        assert_eq!(ng_gutter[1].run_spec.runner, IdeRunnerKind::Karma);
        assert!(ng_gutter[1].run_spec.args.iter().any(|a| a == "ng"));

        let next_dir = mk(
            "next-app",
            &[
                ("next.config.mjs", "export default {};"),
                (
                    "package.json",
                    r#"{"dependencies":{"next":"15.0.0","react":"19.0.0"}}"#,
                ),
            ],
        );
        assert_eq!(
            detect_web_framework(&next_dir, None),
            Some(WebFrameworkKind::NextJs)
        );

        let sk_dir = mk(
            "sveltekit-app",
            &[
                ("svelte.config.js", "export default {};"),
                (
                    "package.json",
                    r#"{"devDependencies":{"@sveltejs/kit":"^2","svelte":"^5"}}"#,
                ),
            ],
        );
        let sk_pkg: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(sk_dir.join("package.json")).unwrap())
                .unwrap();
        assert_eq!(
            detect_web_framework(&sk_dir, Some(&sk_pkg)),
            Some(WebFrameworkKind::SvelteKit)
        );

        let nuxt_dir = mk("nuxt-app", &[("nuxt.config.ts", "export default {};")]);
        assert_eq!(
            detect_web_framework(&nuxt_dir, None),
            Some(WebFrameworkKind::Nuxt)
        );

        let pw_dir = mk(
            "pw-app",
            &[
                ("playwright.config.ts", "export default {};"),
                ("package.json", r#"{"devDependencies":{"@playwright/test":"^1.49.0"}}"#),
                (
                    "e2e/home.spec.ts",
                    "import { test, expect } from '@playwright/test';\ntest.describe('Home', () => {\n  test('renders hero', async () => {});\n});\n",
                ),
            ],
        );
        let pw_file = pw_dir.join("e2e/home.spec.ts");
        let pw_gutter = detect_gutter_items(
            &pw_file.to_string_lossy(),
            &std::fs::read_to_string(&pw_file).unwrap(),
            &[],
        );
        assert_eq!(pw_gutter.len(), 2);
        assert_eq!(pw_gutter[1].run_spec.runner, IdeRunnerKind::Playwright);
        assert!(pw_gutter[1].run_spec.args.iter().any(|a| a == "playwright"));

        // 3. Structured test line parsing (Playwright, Jest, Karma)
        let pw_ok = parse_js_test_line(
            "  ✓  1 [chromium] › e2e/home.spec.ts:12:5 › Home › renders hero (120ms)",
        )
        .expect("playwright pass");
        assert_eq!(pw_ok.status, IdeTestCaseStatus::Passed);
        assert_eq!(pw_ok.file.as_deref(), Some("e2e/home.spec.ts"));
        assert_eq!(pw_ok.line, Some(12));

        let jest_fail = parse_js_test_line("  ✕ calculates subtotal (5 ms)").expect("jest fail");
        assert_eq!(jest_fail.status, IdeTestCaseStatus::Failed);
        let jest_skip = parse_js_test_line("  ○ skipped pending feature").expect("jest skip");
        assert_eq!(jest_skip.status, IdeTestCaseStatus::Skipped);
        let karma_fail = parse_js_test_line(
            "Chrome Headless 120.0.0.0 (Mac OS 10.15.7) AppComponent should create FAILED",
        )
        .expect("karma fail");
        assert_eq!(karma_fail.status, IdeTestCaseStatus::Failed);
        assert_eq!(karma_fail.name, "AppComponent should create");
    }

    #[test]
    fn browser_cdp_target_selection_and_dev_server_url_mapping() {
        use crate::dap::node_bridge::cdp::select_cdp_target_ws_url;
        use crate::dap::node_bridge::{file_url_to_path_in_cwd, path_to_url_regex_in_cwd};

        let targets = json!([
            { "type": "service_worker", "url": "chrome-extension://abc/sw.js", "webSocketDebuggerUrl": "ws://127.0.0.1:9222/devtools/page/sw" },
            { "type": "page", "url": "devtools://devtools/bundled/inspector.html", "webSocketDebuggerUrl": "ws://127.0.0.1:9222/devtools/page/dt" },
            { "type": "page", "url": "http://localhost:5173/", "webSocketDebuggerUrl": "ws://127.0.0.1:9222/devtools/page/APP" }
        ]);
        assert_eq!(
            select_cdp_target_ws_url(&targets).as_deref(),
            Some("ws://127.0.0.1:9222/devtools/page/APP")
        );

        // BUG-28: Requested page URL selects the matching browser tab and fails when absent
        use crate::dap::node_bridge::cdp::select_cdp_target_ws_url_for;
        let two_tabs = json!([
            { "type": "page", "url": "http://localhost:3000/admin", "webSocketDebuggerUrl": "ws://127.0.0.1:9222/devtools/page/ADMIN" },
            { "type": "page", "url": "http://localhost:5173/dashboard", "webSocketDebuggerUrl": "ws://127.0.0.1:9222/devtools/page/VITE" }
        ]);
        assert_eq!(
            select_cdp_target_ws_url_for(&two_tabs, Some("http://localhost:5173")).as_deref(),
            Some("ws://127.0.0.1:9222/devtools/page/VITE")
        );
        assert_eq!(
            select_cdp_target_ws_url_for(&two_tabs, Some("http://localhost:4200")),
            None
        );

        // BUG-31: Browser attach keeps compiled TS/Svelte/Vue breakpoints unverified when unmapped
        use crate::dap::node_bridge::is_initial_bp_verified;
        let loaded_bundle = HashMap::from([("s1".into(), "http://localhost:4200/main.js".into())]);
        assert!(!is_initial_bp_verified(
            "/ws/src/app/hero.component.ts",
            true,
            &loaded_bundle,
            "/ws"
        ));
        assert!(is_initial_bp_verified(
            "/ws/src/app/hero.component.ts",
            false,
            &loaded_bundle,
            "/ws"
        ));

        // BUG-27, BUG-29, BUG-30, BUG-33: Framework dev unwrap, Playwright script preservation,
        // Playwright/Karma debug rejection, and plain Node index.js fallback
        let fw_cfg = crate::types::IdeRunConfig {
            id: "auto:fw-dev".into(),
            name: "Vite Dev".into(),
            runner: crate::types::IdeRunnerKind::Npm,
            command: Some("npx".into()),
            args: vec![
                "--no-install".into(),
                "vite".into(),
                "--port".into(),
                "5173".into(),
            ],
            ..Default::default()
        };
        let (unwrapped_prog, unwrapped_args) =
            unwrap_framework_dev_program("/ws/package.json", &[], Some(&fw_cfg));
        assert_eq!(unwrapped_prog, "vite");
        assert_eq!(unwrapped_args, vec!["--port", "5173"]);

        let tmp_disc = tempfile::tempdir().unwrap();
        // BUG-29: package.json Playwright report script stays `npm run test:e2e:report`
        let pw_proj = tmp_disc.path().join("pw-scripts");
        std::fs::create_dir_all(&pw_proj).unwrap();
        std::fs::write(
            pw_proj.join("package.json"),
            r#"{"scripts":{"test:e2e:report":"playwright show-report"},"devDependencies":{"@playwright/test":"^1.49.0"}}"#,
        )
        .unwrap();
        let pw_cfgs = crate::environments::discover_run_configs(
            &crate::types::IdeTargetSpec::local(pw_proj.to_string_lossy().into_owned()),
            &[],
        );
        let report_cfg = pw_cfgs
            .iter()
            .find(|c| c.args == vec!["run", "test:e2e:report"])
            .expect("declared playwright report script must keep npm run test:e2e:report");
        assert_eq!(report_cfg.runner, crate::types::IdeRunnerKind::Npm);

        // BUG-33: Metadata-only package.json + index.js still gets `node index.js`
        let plain_node = tmp_disc.path().join("plain-node");
        std::fs::create_dir_all(&plain_node).unwrap();
        std::fs::write(
            plain_node.join("package.json"),
            r#"{"name":"plain","version":"1.0.0"}"#,
        )
        .unwrap();
        std::fs::write(plain_node.join("index.js"), "console.log('hi');\n").unwrap();
        let plain_cfgs = crate::environments::discover_run_configs(
            &crate::types::IdeTargetSpec::local(plain_node.to_string_lossy().into_owned()),
            &[],
        );
        assert!(
            plain_cfgs
                .iter()
                .any(|c| c.runner == crate::types::IdeRunnerKind::Node && c.name == "node index.js"),
            "plain Node app with metadata-only package.json must keep node index.js: {plain_cfgs:?}"
        );

        // BUG-30: Playwright single-test debug is rejected cleanly and hidden in gutter
        let pw_spec = pw_proj.join("e2e.spec.ts");
        std::fs::write(
            &pw_spec,
            "import { test } from '@playwright/test';\ntest('hi', async () => {});\n",
        )
        .unwrap();
        assert!(check_js_test_debug_supported(&pw_spec.to_string_lossy()).is_err());
        let pw_gutter = crate::gutter::detect_gutter_items(
            &pw_spec.to_string_lossy(),
            &std::fs::read_to_string(&pw_spec).unwrap(),
            &[],
        );
        assert!(pw_gutter.iter().all(|g| g.run_spec.debug_program.is_none()));

        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().canonicalize().unwrap();
        let app_file = root.join("src/App.svelte");
        std::fs::create_dir_all(app_file.parent().unwrap()).unwrap();
        std::fs::write(&app_file, "<script>let x = 1;</script>\n").unwrap();
        let root_s = root.to_string_lossy();

        assert_eq!(
            file_url_to_path_in_cwd("http://localhost:5173/src/App.svelte?t=1700000", &root_s)
                .as_deref(),
            Some(app_file.to_string_lossy().as_ref())
        );
        assert_eq!(
            file_url_to_path_in_cwd("webpack:///./src/App.svelte", &root_s).as_deref(),
            Some(app_file.to_string_lossy().as_ref())
        );
        let re_str = path_to_url_regex_in_cwd(&app_file.to_string_lossy(), &root_s);
        let re = regex::Regex::new(&re_str).unwrap();
        assert!(re.is_match("http://localhost:5173/src/App.svelte?t=123"));
        assert!(re.is_match("webpack:///./src/App.svelte"));
    }

    #[test]
    fn playwright_and_framework_cli_debug_entries_resolve_in_node_modules() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        std::fs::create_dir_all(root.join("node_modules/@playwright/test")).unwrap();
        std::fs::write(root.join("node_modules/@playwright/test/cli.js"), "").unwrap();
        std::fs::create_dir_all(root.join("node_modules/vite/bin")).unwrap();
        std::fs::write(root.join("node_modules/vite/bin/vite.js"), "").unwrap();
        std::fs::create_dir_all(root.join("e2e")).unwrap();
        std::fs::write(
            root.join("package.json"),
            r#"{"devDependencies":{"@playwright/test":"^1.49","vite":"^6"}}"#,
        )
        .unwrap();
        let spec = root.join("e2e/login.spec.ts");
        std::fs::write(
            &spec,
            "import { test } from '@playwright/test';\ntest('logs in', async () => {});\n",
        )
        .unwrap();

        let (entry, args, cwd) =
            js_runner_debug_launch(&spec.to_string_lossy(), "logs in").unwrap();
        assert!(entry.ends_with("node_modules/@playwright/test/cli.js"));
        assert_eq!(cwd, root.to_string_lossy());
        assert_eq!(args[0], "test");
        assert!(args.iter().any(|a| a == "--workers=1"));

        let (vite_entry, _) =
            resolve_web_framework_cli_entry(&root.to_string_lossy(), "vite", &[]).unwrap();
        assert!(vite_entry.ends_with("node_modules/vite/bin/vite.js"));
    }
}
