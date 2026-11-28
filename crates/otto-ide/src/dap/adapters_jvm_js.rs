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
                is_browser_attach,
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

    let mut program = normalize_target_path(&req.target, &req.launch.program);
    let mut args = req.launch.args.clone();
    let mut cwd = cwd.to_string();

    // BUG-30 + BUG3-9: Reject Playwright/Karma debugging up front, on local and SSH targets, for
    // single-test gutter launches AND toolbar/config launches (including auto-discovered
    // `npm run test` scripts) using structured runner/command/script classification — never
    // arbitrary path substrings. An auto-discovered script names `package.json` relative to its
    // own package directory (the launch cwd), not to the first project folder.
    let guard_program = if !Path::new(raw_program).is_absolute()
        && Path::new(raw_program).file_name() == Some("package.json".as_ref())
    {
        Path::new(&cwd)
            .join(raw_program)
            .to_string_lossy()
            .into_owned()
    } else {
        program.clone()
    };
    // On SSH the script is read from the target's own `package.json` (BUG-30).
    let remote_manifest = if req.target.kind == IdeTargetKind::Ssh
        && Path::new(&guard_program)
            .file_name()
            .is_some_and(|n| n.eq_ignore_ascii_case("package.json"))
    {
        super::read_target_file_for_inline(
            &req.target,
            ctx.ssh.as_ref(),
            &guard_program,
            ctx.caller_is_root,
        )
        .await
    } else {
        None
    };
    let test_runner = js_debug_test_runner(
        &guard_program,
        &args,
        &env,
        ctx.run_config.as_ref(),
        req.launch.test_name.as_deref(),
        req.launch.is_test,
        req.target.kind == IdeTargetKind::Local,
        remote_manifest.as_deref(),
    );

    if test_runner.is_some() {
        super::adapters_js_tests::require_local(req)?;
    }

    let mut node_candidates = Vec::new();
    if let Some(sdk) = resolve_debug_sdk_override(req, ctx, &cwd) {
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

    if let Some(runner) = test_runner {
        return super::adapters_js_tests::resolve_js_test_debug(
            data_dir,
            language,
            runner,
            req,
            ctx,
            &node_bin,
            (&program, &args, &cwd),
            env,
        )
        .await;
    }

    // BUG-27: Resolve framework dev CLIs (vite/next/ng/nuxt/svelte-kit) BEFORE BOTH
    // external `js-debug` and built-in `node-inspector-dap` so neither adapter tries
    // to launch a bare CLI name or `package.json`.
    let has_test_name = req
        .launch
        .test_name
        .as_deref()
        .is_some_and(|s| !s.is_empty());
    if !has_test_name && req.target.kind == IdeTargetKind::Local {
        let (effective_prog, effective_args) =
            unwrap_framework_dev_program(&program, &args, ctx.run_config.as_ref());
        if let Some((entry, fw_args)) =
            resolve_web_framework_cli_entry(&cwd, &effective_prog, &effective_args)
        {
            program = entry;
            args = fw_args;
        }
    }

    if env.get("OTTO_JS_ADAPTER").map(String::as_str) == Some("js-debug") {
        if let Some(script) = super::adapters_js_tests::js_debug_server(data_dir) {
            return Ok(ResolvedDapAdapter {
                adapter_name: "js-debug".into(),
                language: language.to_string(),
                transport: DapAdapterTransport::JsDebug {
                    command: node_bin.clone(),
                    args: vec![
                        script.to_string_lossy().into_owned(),
                        "0".into(),
                        "127.0.0.1".into(),
                    ],
                    cwd: cwd.clone(),
                    env,
                    companion: None,
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
                    "args": args,
                    "cwd": cwd,
                    "runtimeExecutable": node_bin,
                }),
                normalized_cwd: cwd,
                initial_output: Vec::new(),
            });
        }
    }

    let mut node_flags = vec!["--enable-source-maps".into()];
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
            is_browser_attach: false,
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

/// A test runner the Node inspector bridge cannot debug: Playwright runs tests in
/// a forked worker, Karma in a browser. Both go to js-debug (`adapters_js_tests`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum JsTestRunner {
    Playwright,
    Karma,
}

/// Classify a structured command + argument vector (never arbitrary source paths or user flags)
/// to detect Playwright or Karma/Angular test launchers (BUG-30, BUG3-9).
fn classify_js_cmd_tokens(cmd: &str, args: &[String]) -> Option<JsTestRunner> {
    let norm = cmd.trim().replace('\\', "/").to_ascii_lowercase();
    if norm.is_empty() {
        return None;
    }
    if norm.ends_with("/@playwright/test/cli.js") || norm.ends_with("/playwright/cli.js") {
        return Some(JsTestRunner::Playwright);
    }
    if norm.ends_with("/karma/bin/karma") {
        return Some(JsTestRunner::Karma);
    }
    let base = Path::new(&norm)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(&norm);
    let base_stem = base
        .strip_suffix(".cmd")
        .or_else(|| base.strip_suffix(".exe"))
        .unwrap_or(base);
    if base_stem == "playwright" {
        return Some(JsTestRunner::Playwright);
    }
    if base_stem == "karma" {
        return Some(JsTestRunner::Karma);
    }
    let first_non_flag = args
        .iter()
        .find(|a| !a.starts_with('-'))
        .map(|s| s.to_ascii_lowercase());
    if (base_stem == "ng" || norm.ends_with("/@angular/cli/bin/ng.js"))
        && first_non_flag.as_deref() == Some("test")
    {
        return Some(JsTestRunner::Karma);
    }
    if matches!(base_stem, "npx" | "npm" | "pnpm" | "yarn" | "bun" | "bunx") {
        let mut idx = 0usize;
        while idx < args.len() {
            let tok = args[idx].trim();
            if tok.is_empty() || tok.starts_with('-') || matches!(tok, "exec" | "dlx" | "--") {
                idx += 1;
                continue;
            }
            if tok == "run" {
                return None;
            }
            let sub = tok.to_ascii_lowercase();
            let sub_base = Path::new(&sub)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(&sub);
            if sub_base == "playwright" || sub == "@playwright/test" {
                return Some(JsTestRunner::Playwright);
            }
            if sub_base == "karma" {
                return Some(JsTestRunner::Karma);
            }
            if sub_base == "ng"
                && args
                    .get(idx + 1)
                    .is_some_and(|next| next.eq_ignore_ascii_case("test"))
            {
                return Some(JsTestRunner::Karma);
            }
            break;
        }
    }
    None
}

/// Inspect a `package.json` `"scripts"` command string (splitting `&&` / `||` / `;`)
/// for Playwright, Karma, or `ng test` invocations without matching arbitrary flags.
fn classify_npm_script_command(script_cmd: &str) -> Option<JsTestRunner> {
    for segment in script_cmd.split([';', '&', '|']) {
        let mut tokens = segment
            .split_whitespace()
            .skip_while(|t| t.contains('=') && !t.starts_with('-'));
        let Some(first) = tokens.next() else {
            continue;
        };
        let rest: Vec<String> = tokens.map(str::to_string).collect();
        if let Some(kind) = classify_js_cmd_tokens(first, &rest) {
            return Some(kind);
        }
    }
    None
}

fn inspect_package_json_script(
    pkg_json_path: &Path,
    explicit_script: Option<&str>,
    is_test: bool,
) -> Option<JsTestRunner> {
    let raw = std::fs::read_to_string(pkg_json_path).ok()?;
    package_json_script_runner(&raw, explicit_script, is_test)
}

/// Classify the script a `package.json` text runs (`explicit_script`, else `test` for a test launch).
fn package_json_script_runner(
    raw: &str,
    explicit_script: Option<&str>,
    is_test: bool,
) -> Option<JsTestRunner> {
    let val: serde_json::Value = serde_json::from_str(raw).ok()?;
    let scripts = val.get("scripts")?.as_object()?;
    let script_name = if let Some(s) = explicit_script.filter(|s| !s.is_empty()) {
        s
    } else if is_test {
        "test"
    } else if val
        .get("main")
        .and_then(serde_json::Value::as_str)
        .is_none()
        && scripts.contains_key("test")
    {
        // Fallback when toolbar Debug targets a test-only `package.json` with no `main` entry.
        "test"
    } else {
        return None;
    };
    let cmd = scripts.get(script_name)?.as_str()?;
    classify_npm_script_command(cmd)
}

/// The Playwright or Karma runner a debug start launches, for BOTH single-test launches
/// and toolbar/run-config launches (BUG-30), while ordinary JS/TS programs whose
/// directory/file names or CLI arguments mention "playwright" or "karma" stay `None` (BUG3-9).
/// On SSH (`!local`) `remote_manifest` is the target's text of `program` when it is a `package.json`.
#[allow(clippy::too_many_arguments)]
pub(super) fn js_debug_test_runner(
    program: &str,
    args: &[String],
    env: &BTreeMap<String, String>,
    run_config: Option<&crate::types::IdeRunConfig>,
    test_name: Option<&str>,
    is_test: bool,
    local: bool,
    remote_manifest: Option<&str>,
) -> Option<JsTestRunner> {
    if let Some(kind) = classify_js_cmd_tokens(program, args) {
        return Some(kind);
    }
    if let Some(cfg) = run_config {
        if let Some(cmd) = cfg.command.as_deref() {
            if let Some(kind) = classify_js_cmd_tokens(cmd, &cfg.args) {
                return Some(kind);
            }
        }
        // Only an npm runner's args are a package-manager command line; a Node or custom
        // runner's args belong to the program (or to `cfg.command`, classified above).
        if cfg.runner == crate::types::IdeRunnerKind::Npm {
            // Check `npx`/`npm`/`ng` inside structured runner args (e.g. `["--no-install", "ng", "test"]`).
            if let Some(kind) = classify_js_cmd_tokens("npx", &cfg.args) {
                return Some(kind);
            }
            let cfg_script = if cfg.args.first().map(String::as_str) == Some("run") {
                cfg.args.get(1).map(String::as_str)
            } else if cfg.args.first().map(String::as_str) == Some("test") {
                Some("test")
            } else {
                None
            };
            if let Some(script) = cfg_script.filter(|_| local) {
                if let Some(tp) = cfg.target_path.as_deref() {
                    let tp = match cfg.cwd.as_deref() {
                        Some(c) if !Path::new(tp).is_absolute() => Path::new(c).join(tp),
                        _ => PathBuf::from(tp),
                    };
                    let pkg = if tp.ends_with("package.json") {
                        tp
                    } else {
                        tp.join("package.json")
                    };
                    if let Some(kind) = inspect_package_json_script(&pkg, Some(script), true) {
                        return Some(kind);
                    }
                }
            }
        }
    }
    let prog_path = Path::new(program);
    let prog_base = prog_path
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or(program);
    if prog_base.eq_ignore_ascii_case("package.json") {
        // Toolbar Debug on a tool-runner config (`Karma / Angular Test`, `Playwright E2E`)
        // forwards that runner's own command line as args: `--no-install ng test`.
        if let Some(kind) = classify_js_cmd_tokens("npx", args) {
            return Some(kind);
        }
    }
    if prog_base.eq_ignore_ascii_case("package.json") {
        let explicit_script = env.get("OTTO_NPM_SCRIPT").map(String::as_str).or_else(|| {
            if args.first().map(String::as_str) == Some("run") {
                args.get(1).map(String::as_str)
            } else if args.first().map(String::as_str) == Some("test") {
                Some("test")
            } else {
                None
            }
        });
        let has_test_intent = is_test || test_name.is_some_and(|s| !s.is_empty());
        let kind = if local {
            inspect_package_json_script(prog_path, explicit_script, has_test_intent)
        } else {
            remote_manifest
                .and_then(|raw| package_json_script_runner(raw, explicit_script, has_test_intent))
        };
        if let Some(kind) = kind {
            return Some(kind);
        }
    }
    // The checks below read source files, which only exist on this host.
    if !local {
        return None;
    }
    if test_name.is_some_and(|s| !s.is_empty()) {
        return js_test_file_runner(program);
    }
    // Even without `test_name`, a Playwright test source file runs under Playwright.
    let is_js_source = matches!(
        prog_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase())
            .as_deref(),
        Some("js" | "jsx" | "ts" | "tsx" | "mjs" | "cjs" | "mts" | "cts")
    );
    if is_js_source {
        let src = std::fs::read_to_string(program).unwrap_or_default();
        if src.contains("@playwright/test") {
            return Some(JsTestRunner::Playwright);
        }
    }
    None
}

/// The runner of a single test in `file` that the Node inspector bridge cannot debug
/// (BUG-30): Playwright executes tests in a child worker process and Karma in a browser.
pub(super) fn js_test_file_runner(file: &str) -> Option<JsTestRunner> {
    let src = std::fs::read_to_string(file).unwrap_or_default();
    if src.contains("node:test") {
        return None;
    }
    if src.contains("@playwright/test") {
        return Some(JsTestRunner::Playwright);
    }
    match crate::gutter::js_test_package(file)?.runner {
        crate::gutter::JsTestRunnerKind::Playwright => Some(JsTestRunner::Playwright),
        crate::gutter::JsTestRunnerKind::Karma => Some(JsTestRunner::Karma),
        _ => None,
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
    if let Some(path) = &req.launch.test_path {
        return super::adapters_kotest::resolve_kotest_adapter(req, ctx, cwd, env, path).await;
    }
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
        // A test selector (`com.acme.CalcTest#adds`) runs through a JUnit launcher on the
        // test classpath, unless the class is an app with its own `main` (the gutter's main
        // item also names its class as the filter): that launches on main sources (BUG2-6).
        let has_main = std::fs::read_to_string(&program).is_ok_and(|src| {
            src.contains("static void main(")
                || (is_kotlin_file && (src.contains("fun main(") || src.contains("fun main ()")))
        });
        let test_selector = req
            .launch
            .test_name
            .clone()
            .filter(|s| !s.trim().is_empty() && !has_main);
        let include_tests = test_selector.is_some();
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
            companion: None,
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

        // BUG-30: a Playwright single test debugs through js-debug, from the gutter too
        let pw_spec = pw_proj.join("e2e.spec.ts");
        std::fs::write(
            &pw_spec,
            "import { test } from '@playwright/test';\ntest('hi', async () => {});\n",
        )
        .unwrap();
        assert_eq!(
            js_test_file_runner(&pw_spec.to_string_lossy()),
            Some(JsTestRunner::Playwright)
        );
        let pw_gutter = crate::gutter::detect_gutter_items(
            &pw_spec.to_string_lossy(),
            &std::fs::read_to_string(&pw_spec).unwrap(),
            &[],
        );
        assert!(pw_gutter.iter().all(|g| g.run_spec.debug_program.is_some()));

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

    #[tokio::test]
    async fn resolve_js_ts_adapter_resolves_vite_for_js_debug_rejects_toolbar_playwright_karma_and_preserves_browser_alias(
    ) {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let root = tmp.path().join("vite-app");
        let fake_node = data_dir.join("toolchains/node/current/bin/node");
        let fake_js_debug = data_dir.join("toolchains/node/dap/js-debug/src/dapDebugServer.js");
        let vite_bin = root.join("node_modules/vite/bin/vite.js");
        for p in [&fake_node, &fake_js_debug, &vite_bin] {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, "#!/bin/sh\nexit 0\n").unwrap();
        }
        std::fs::set_permissions(&fake_node, std::fs::Permissions::from_mode(0o755)).unwrap();

        let root_str = root.to_string_lossy().into_owned();
        let ctx = DapStartContext {
            caller_is_root: true,
            ..Default::default()
        };

        // BUG-27: Even when OTTO_JS_ADAPTER=js-debug is selected, bare `vite` resolves
        // to `node_modules/vite/bin/vite.js` before returning the js-debug adapter.
        let vite_req = IdeStartDebugReq {
            target: IdeTargetSpec::local(root_str.clone()),
            language: "typescript".into(),
            config_id: None,
            launch: crate::types::IdeDebugLaunchSpec {
                program: "vite".into(),
                args: vec!["--port".into(), "5173".into()],
                cwd: Some(root_str.clone()),
                ..Default::default()
            },
            ..Default::default()
        };
        let mut env = BTreeMap::new();
        env.insert("OTTO_JS_ADAPTER".into(), "js-debug".into());
        let resolved = resolve_js_ts_adapter(
            &data_dir,
            "typescript",
            &vite_req,
            &ctx,
            &root_str,
            env.clone(),
        )
        .await
        .expect("js-debug adapter should resolve vite CLI");
        assert_eq!(resolved.adapter_name, "js-debug");
        assert!(
            resolved.launch_args["program"]
                .as_str()
                .unwrap_or("")
                .ends_with("node_modules/vite/bin/vite.js"),
            "BUG-27: js-debug must receive resolved node_modules/vite/bin/vite.js, got: {}",
            resolved.launch_args
        );

        // Toolbar Debug on Playwright needs @playwright/test installed; plain Karma (no
        // angular.json) stays Run-only.
        let mut pw_ctx = ctx.clone();
        pw_ctx.run_config = Some(crate::types::IdeRunConfig {
            id: "auto:playwright".into(),
            name: "Playwright E2E".into(),
            runner: crate::types::IdeRunnerKind::Npm,
            command: Some("npx".into()),
            args: vec!["playwright".into(), "test".into()],
            ..Default::default()
        });
        let pw_req = IdeStartDebugReq {
            target: IdeTargetSpec::local(root_str.clone()),
            language: "typescript".into(),
            config_id: None,
            launch: crate::types::IdeDebugLaunchSpec {
                program: root.join("package.json").to_string_lossy().into_owned(),
                cwd: Some(root_str.clone()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(
            resolve_js_ts_adapter(
                &data_dir,
                "typescript",
                &pw_req,
                &pw_ctx,
                &root_str,
                BTreeMap::new()
            )
            .await
            .is_err(),
            "BUG-30: toolbar Playwright debug without test_name must be rejected"
        );

        let mut karma_ctx = ctx.clone();
        karma_ctx.run_config = Some(crate::types::IdeRunConfig {
            id: "auto:ng-test".into(),
            name: "Angular Karma Tests".into(),
            runner: crate::types::IdeRunnerKind::Npm,
            command: Some("npx".into()),
            args: vec!["--no-install".into(), "ng".into(), "test".into()],
            ..Default::default()
        });
        assert!(
            resolve_js_ts_adapter(
                &data_dir,
                "typescript",
                &pw_req,
                &karma_ctx,
                &root_str,
                BTreeMap::new()
            )
            .await
            .is_err(),
            "BUG-30: toolbar Karma debug without test_name must be rejected"
        );

        // BUG-30 (auto-discovered `npm run test` with no persisted run_config): Angular/Karma
        // package.json `"test": "ng test"` launched via `OTTO_NPM_SCRIPT=test` (or test-only
        // package.json) must still be rejected cleanly.
        let ng_karma_dir = tmp.path().join("ng-karma-app");
        std::fs::create_dir_all(&ng_karma_dir).unwrap();
        let ng_pkg = ng_karma_dir.join("package.json");
        std::fs::write(
            &ng_pkg,
            r#"{"name":"ng-karma-app","scripts":{"start":"ng serve","test":"ng test"}}"#,
        )
        .unwrap();
        let ng_test_req = IdeStartDebugReq {
            target: IdeTargetSpec::local(ng_karma_dir.to_string_lossy().into_owned()),
            language: "typescript".into(),
            config_id: None,
            launch: crate::types::IdeDebugLaunchSpec {
                program: ng_pkg.to_string_lossy().into_owned(),
                cwd: Some(ng_karma_dir.to_string_lossy().into_owned()),
                is_test: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut npm_env = BTreeMap::new();
        npm_env.insert("OTTO_NPM_SCRIPT".into(), "test".into());
        assert!(
            resolve_js_ts_adapter(
                &data_dir,
                "typescript",
                &ng_test_req,
                &ctx,
                &ng_karma_dir.to_string_lossy(),
                npm_env
            )
            .await
            .is_err(),
            "BUG-30: auto-discovered `npm run test` for `ng test` without run_config must be rejected"
        );

        // BUG3-9: Unrelated plain Node programs whose paths, args, or config names contain
        // "playwright" or "karma" MUST NOT be blocked by the test-runner guard.
        let pw_examples = tmp.path().join("playwright-examples");
        let karma_dash = tmp.path().join("karma-dashboard");
        std::fs::create_dir_all(&pw_examples).unwrap();
        std::fs::create_dir_all(&karma_dash).unwrap();
        let pw_srv = pw_examples.join("server.js");
        let karma_app = karma_dash.join("app.js");
        std::fs::write(&pw_srv, "console.log('hello');\n").unwrap();
        std::fs::write(&karma_app, "console.log('dashboard');\n").unwrap();
        let pw_srv_req = IdeStartDebugReq {
            target: IdeTargetSpec::local(pw_examples.to_string_lossy().into_owned()),
            language: "javascript".into(),
            launch: crate::types::IdeDebugLaunchSpec {
                program: pw_srv.to_string_lossy().into_owned(),
                args: vec!["--label=karma".into()],
                cwd: Some(pw_examples.to_string_lossy().into_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(
            resolve_js_ts_adapter(
                &data_dir,
                "javascript",
                &pw_srv_req,
                &ctx,
                &pw_examples.to_string_lossy(),
                BTreeMap::new()
            )
            .await
            .is_ok(),
            "BUG3-9: plain Node program under playwright-examples/ with --label=karma must not be rejected"
        );
        let mut karma_dash_ctx = ctx.clone();
        karma_dash_ctx.run_config = Some(crate::types::IdeRunConfig {
            id: "cfg:karma-dashboard".into(),
            name: "Karma Dashboard Server".into(),
            runner: crate::types::IdeRunnerKind::Node,
            command: Some("node".into()),
            target_path: Some(karma_app.to_string_lossy().into_owned()),
            args: vec!["--theme=playwright".into()],
            ..Default::default()
        });
        let karma_app_req = IdeStartDebugReq {
            target: IdeTargetSpec::local(karma_dash.to_string_lossy().into_owned()),
            language: "javascript".into(),
            launch: crate::types::IdeDebugLaunchSpec {
                program: karma_app.to_string_lossy().into_owned(),
                args: vec!["--theme=playwright".into()],
                cwd: Some(karma_dash.to_string_lossy().into_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        assert!(
            resolve_js_ts_adapter(
                &data_dir,
                "javascript",
                &karma_app_req,
                &karma_dash_ctx,
                &karma_dash.to_string_lossy(),
                BTreeMap::new()
            )
            .await
            .is_ok(),
            "BUG3-9: plain Node program named Karma Dashboard must not be rejected"
        );

        // BUG-31: `language: "browser"` + `attach_port` + empty `program` sets `is_browser_attach = true`.
        let browser_req = IdeStartDebugReq {
            target: IdeTargetSpec::local(root_str.clone()),
            language: "browser".into(),
            config_id: None,
            launch: crate::types::IdeDebugLaunchSpec {
                program: String::new(),
                attach_port: Some(9222),
                cwd: Some(root_str.clone()),
                ..Default::default()
            },
            ..Default::default()
        };
        let browser_res = resolve_js_ts_adapter(
            &data_dir,
            "browser",
            &browser_req,
            &ctx,
            &root_str,
            BTreeMap::new(),
        )
        .await
        .expect("browser alias attach");
        match browser_res.transport {
            DapAdapterTransport::NodeInspectorBridge(spec) => {
                assert!(
                    spec.is_browser_attach,
                    "BUG-31: browser language alias must propagate is_browser_attach=true"
                );
            }
            _ => panic!("expected NodeInspectorBridge transport"),
        }

        // BUG-28: Exact URL match must beat an earlier same-origin descendant tab (`/settings`).
        use crate::dap::node_bridge::cdp::select_cdp_target_ws_url_for;
        let tabs = json!([
            { "type": "page", "url": "http://localhost:5173/settings", "webSocketDebuggerUrl": "ws://127.0.0.1:9222/devtools/page/SETTINGS" },
            { "type": "page", "url": "http://localhost:5173/", "webSocketDebuggerUrl": "ws://127.0.0.1:9222/devtools/page/ROOT" }
        ]);
        assert_eq!(
            select_cdp_target_ws_url_for(&tabs, Some("http://localhost:5173/")).as_deref(),
            Some("ws://127.0.0.1:9222/devtools/page/ROOT"),
            "BUG-28: exact root URL tab must beat earlier /settings tab"
        );
    }

    async fn js_guard_error(
        data_dir: &Path,
        req: &IdeStartDebugReq,
        ctx: &DapStartContext,
        env: BTreeMap<String, String>,
    ) -> String {
        let cwd = req.launch.cwd.clone().unwrap_or_default();
        match resolve_js_ts_adapter(data_dir, "typescript", req, ctx, &cwd, env).await {
            Ok(_) => String::new(),
            Err(e) => e.to_string(),
        }
    }

    /// BUG-30: the toolbar's auto-discovered Karma / Playwright configs (argv only, no
    /// `config_id`), an npm script in a second project folder, and SSH targets all reach the
    /// guard; BUG3-9: a Node run configuration's own argv never classifies the program.
    #[tokio::test]
    async fn js_debug_guard_covers_toolbar_tool_runners_nested_packages_and_ssh() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let fake_node = data_dir.join("toolchains/node/current/bin/node");
        std::fs::create_dir_all(fake_node.parent().unwrap()).unwrap();
        std::fs::write(&fake_node, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::set_permissions(&fake_node, std::fs::Permissions::from_mode(0o755)).unwrap();
        // Two project folders: the first runs vitest, the Angular one runs Karma via `ng test`.
        let ws = tmp.path().join("ws");
        let web = tmp.path().join("web");
        for (dir, test) in [(&ws, "vitest"), (&web, "ng test")] {
            std::fs::create_dir_all(dir).unwrap();
            std::fs::write(
                dir.join("package.json"),
                format!(r#"{{"main":"main.js","scripts":{{"test":"{test}"}}}}"#),
            )
            .unwrap();
        }
        let app = ws.join("app.js");
        std::fs::write(&app, "console.log('app');\n").unwrap();
        let ctx = DapStartContext {
            caller_is_root: true,
            ..Default::default()
        };
        let req = |program: &str, args: &[&str], cwd: &Path, is_test: bool| IdeStartDebugReq {
            target: IdeTargetSpec::local(ws.to_string_lossy().into_owned()),
            language: "typescript".into(),
            launch: crate::types::IdeDebugLaunchSpec {
                program: program.into(),
                args: args.iter().map(|a| a.to_string()).collect(),
                cwd: Some(cwd.to_string_lossy().into_owned()),
                is_test,
                ..Default::default()
            },
            ..Default::default()
        };

        // Toolbar Debug on "Karma / Angular Test" and "Playwright E2E" sends their argv only.
        let karma = req(
            "package.json",
            &["--no-install", "ng", "test", "--watch=false"],
            &web,
            false,
        );
        let err = js_guard_error(&data_dir, &karma, &ctx, BTreeMap::new()).await;
        assert!(
            err.contains("Karma"),
            "auto Karma config must be rejected: {err:?}"
        );
        let pw = req(
            "package.json",
            &["--no-install", "playwright", "test"],
            &ws,
            false,
        );
        let err = js_guard_error(&data_dir, &pw, &ctx, BTreeMap::new()).await;
        assert!(
            err.contains("needs js-debug"),
            "auto Playwright config debugs through js-debug: {err:?}"
        );

        // `npm run test` of the second folder reads that folder's manifest, not the first one's.
        let npm_test = req("package.json", &[], &web, true);
        let env = BTreeMap::from([("OTTO_NPM_SCRIPT".to_string(), "test".to_string())]);
        let err = js_guard_error(&data_dir, &npm_test, &ctx, env).await;
        assert!(
            err.contains("Karma"),
            "nested ng test script must be rejected: {err:?}"
        );

        // SSH: the structured checks run before any remote probe.
        let mut ssh_pw = req(
            "/srv/app/node_modules/@playwright/test/cli.js",
            &["test"],
            &ws,
            false,
        );
        ssh_pw.target = IdeTargetSpec::ssh("conn-1".into(), "/srv/app");
        let err = js_guard_error(&data_dir, &ssh_pw, &ctx, BTreeMap::new()).await;
        assert!(
            err.contains("Playwright"),
            "SSH Playwright CLI must be rejected: {err:?}"
        );

        // A Node run configuration whose program takes `karma` as an argument still debugs.
        let mut node_ctx = ctx.clone();
        node_ctx.run_config = Some(crate::types::IdeRunConfig {
            id: "cfg:app".into(),
            name: "app".into(),
            runner: crate::types::IdeRunnerKind::Node,
            command: Some("node".into()),
            target_path: Some(app.to_string_lossy().into_owned()),
            args: vec!["karma".into()],
            ..Default::default()
        });
        let node_req = req(&app.to_string_lossy(), &["karma"], &ws, false);
        let err = js_guard_error(&data_dir, &node_req, &node_ctx, BTreeMap::new()).await;
        assert_eq!(err, "", "a Node program's own argv must not be classified");
    }

    /// BUG-30 (SSH): `npm run test` on an SSH target reads the script from the target's
    /// `package.json` text, so `ng test` / `karma start` / `playwright test` are refused there too.
    #[test]
    fn js_debug_guard_reads_remote_package_json_script_on_ssh() {
        let env = BTreeMap::from([("OTTO_NPM_SCRIPT".to_string(), "test".to_string())]);
        let check = |manifest: &str| {
            js_debug_test_runner(
                "/srv/repo/web/package.json",
                &[],
                &env,
                None,
                None,
                true,
                false,
                Some(manifest),
            )
        };
        assert_eq!(
            check(r#"{"scripts":{"test":"ng test"}}"#),
            Some(JsTestRunner::Karma)
        );
        assert_eq!(
            check(r#"{"scripts":{"test":"karma start --single-run"}}"#),
            Some(JsTestRunner::Karma)
        );
        assert_eq!(
            check(r#"{"scripts":{"test":"playwright test"}}"#),
            Some(JsTestRunner::Playwright)
        );
        assert_eq!(check(r#"{"scripts":{"test":"vitest run"}}"#), None);
    }
}
