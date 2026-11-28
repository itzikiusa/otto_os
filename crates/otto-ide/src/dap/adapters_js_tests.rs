//! Debug Playwright tests and Angular Karma specs through Microsoft js-debug
//! (BUG-30). Playwright runs each test in a forked worker and Karma runs specs
//! in a browser, so the built-in Node inspector bridge never reaches them:
//! js-debug's `pwa-node` follows the worker and its `pwa-chrome` debugs
//! Karma's page, each as a child target (`session_children`). Plain Karma
//! (no `angular.json`) stays Run-only.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use otto_core::{Error, Result};
use regex::Regex;
use serde_json::{json, Value};

use super::adapters::{DapAdapterTransport, ResolvedDapAdapter};
use super::adapters_jvm_js::JsTestRunner;
use super::{spawn_target_process, BridgeGuard, DapStartContext};
use crate::types::{IdeDebugOutputEntry, IdeStartDebugReq, IdeTargetKind};

pub(super) const PLAIN_KARMA: &str = "Debug supports Karma in Angular CLI projects. For this Karma config, use Run, or start karma with --no-single-run and use Browser attach.";

/// How long the first `ng test` build may take before Karma serves.
const KARMA_START_TIMEOUT: Duration = Duration::from_secs(300);

/// js-debug's DAP server under the managed toolchains (`installer_tools`).
pub(super) fn js_debug_server(data_dir: &Path) -> Option<PathBuf> {
    [
        "toolchains/node/dap/js-debug/src/dapDebugServer.js",
        "toolchains/node/js-debug/src/dapDebugServer.js",
        "toolchains/js-debug/src/dapDebugServer.js",
    ]
    .iter()
    .map(|p| data_dir.join(p))
    .find(|p| p.is_file())
}

fn js_debug_missing() -> Error {
    Error::NotFound(
        "Debugging tests needs js-debug (1.2 MB), the JavaScript debugger. Install it from IDE → Toolchains.".into(),
    )
}

/// The js-debug transport and its `initialize` arguments.
fn js_debug_adapter(
    server: &Path,
    node_bin: &str,
    cwd: &str,
    mut env: BTreeMap<String, String>,
    companion: Option<std::sync::Arc<BridgeGuard>>,
) -> (DapAdapterTransport, Value) {
    // Playwright's own inspector would pause the run under the IDE debugger.
    env.remove("PWDEBUG");
    let transport = DapAdapterTransport::JsDebug {
        command: node_bin.to_string(),
        args: vec![
            server.to_string_lossy().into_owned(),
            "0".into(),
            "127.0.0.1".into(),
        ],
        cwd: cwd.to_string(),
        env,
        companion,
    };
    let init = json!({
        "adapterID": "pwa-node",
        "linesStartAt1": true,
        "columnsStartAt1": true,
        "pathFormat": "path",
        "supportsStartDebuggingRequest": true,
    });
    (transport, init)
}

fn console(text: impl Into<String>) -> IdeDebugOutputEntry {
    IdeDebugOutputEntry {
        category: "console".into(),
        text: text.into(),
        seq: 0,
    }
}

/// Resolve a Playwright or Angular Karma debug start (`program`/`args`/`cwd` as
/// the launch named them: a spec file from the gutter, a run configuration's
/// command line from the toolbar).
#[allow(clippy::too_many_arguments)]
pub(super) async fn resolve_js_test_debug(
    data_dir: &Path,
    language: &str,
    runner: JsTestRunner,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    node_bin: &str,
    (program, args, cwd): (&str, &[String], &str),
    env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    require_local(req)?;
    let spec_file = Path::new(program)
        .extension()
        .is_some_and(|e| {
            matches!(
                e.to_str(),
                Some("ts" | "js" | "mts" | "cts" | "mjs" | "cjs" | "tsx" | "jsx")
            )
        })
        .then_some(program);
    match runner {
        JsTestRunner::Playwright => {
            let server = js_debug_server(data_dir).ok_or_else(js_debug_missing)?;
            resolve_playwright(
                &server,
                language,
                req,
                ctx,
                node_bin,
                spec_file,
                (args, cwd),
                env,
            )
        }
        JsTestRunner::Karma => {
            let ng_root = angular_root(spec_file.unwrap_or(cwd), cwd)
                .ok_or_else(|| Error::Invalid(PLAIN_KARMA.into()))?;
            let server = js_debug_server(data_dir).ok_or_else(js_debug_missing)?;
            resolve_angular_karma(
                &server, language, req, ctx, node_bin, spec_file, &ng_root, env,
            )
            .await
        }
    }
}

/// Playwright and Karma debugging is local-only (it is not verified over SSH yet).
pub(super) fn require_local(req: &IdeStartDebugReq) -> Result<()> {
    if req.target.kind == IdeTargetKind::Local {
        return Ok(());
    }
    Err(Error::Invalid(
        "Debugging Playwright and Karma tests works on local projects. On an SSH target, use Run."
            .into(),
    ))
}

/// The folder of the `angular.json` that owns `path` (searched up from it, then from `cwd`).
fn angular_root(path: &str, cwd: &str) -> Option<PathBuf> {
    [Path::new(path), Path::new(cwd)]
        .into_iter()
        .flat_map(Path::ancestors)
        .find(|d| d.join("angular.json").is_file())
        .map(Path::to_path_buf)
}

/// `entry` (relative to a `node_modules`) in `dir` or a hoisting ancestor.
fn find_in_node_modules(dir: &Path, entries: &[&str]) -> Option<PathBuf> {
    entries.iter().find_map(|rel| {
        dir.ancestors()
            .map(|d| d.join("node_modules").join(rel))
            .find(|p| p.is_file())
    })
}

/// The Playwright CLI arguments of a toolbar launch: the configuration's own
/// selection (files, `--project`, `-g`) without the flags the debug run sets.
fn playwright_toolbar_args(args: &[String], ctx: &DapStartContext) -> Vec<String> {
    let after_test = args
        .iter()
        .position(|a| a == "test")
        .filter(|_| args.iter().any(|a| a == "playwright"))
        .map_or(args, |i| &args[i + 1..]);
    let mut out: Vec<String> = after_test
        .iter()
        .filter(|a| {
            ![
                "--workers",
                "--timeout",
                "--reporter",
                "-j",
                "--debug",
                "--ui",
                "--headed",
            ]
            .iter()
            .any(|f| a.as_str() == *f || a.starts_with(&format!("{f}=")))
        })
        .cloned()
        .collect();
    if let Some(tp) = ctx
        .run_config
        .as_ref()
        .and_then(|c| c.target_path.as_deref())
        .map(str::trim)
        .filter(|t| !t.is_empty() && *t != "package.json" && !out.iter().any(|a| a == t))
    {
        out.push(tp.to_string());
    }
    out
}

#[allow(clippy::too_many_arguments)]
fn resolve_playwright(
    server: &Path,
    language: &str,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    node_bin: &str,
    spec_file: Option<&str>,
    (args, cwd): (&[String], &str),
    env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    let pkg_dir = spec_file
        .and_then(crate::gutter::js_test_package)
        .map(|p| p.dir)
        .unwrap_or_else(|| PathBuf::from(cwd));
    let cli = find_in_node_modules(&pkg_dir, &["@playwright/test/cli.js", "playwright/cli.js"])
        .ok_or_else(|| {
            Error::Invalid(format!(
                "Playwright is not installed in {}. Run npm install, then Debug again.",
                pkg_dir.display()
            ))
        })?;
    let mut pw_args = vec!["test".to_string()];
    match spec_file {
        Some(file) => {
            let rel = Path::new(file)
                .strip_prefix(&pkg_dir)
                .map_or_else(|_| file.to_string(), |r| r.to_string_lossy().into_owned());
            match (req.launch.test_line, req.launch.test_name.as_deref()) {
                (Some(line), _) => pw_args.push(format!("{rel}:{line}")),
                (None, Some(name)) if !name.is_empty() => {
                    pw_args.extend([rel, "-g".into(), regex::escape(name)]);
                }
                _ => pw_args.push(rel),
            }
        }
        None => pw_args.extend(playwright_toolbar_args(args, ctx)),
    }
    pw_args.extend(["--workers=1", "--timeout=0", "--reporter=list", "--headed"].map(String::from));
    let pkg = pkg_dir.to_string_lossy().into_owned();
    let (transport, initialize_args) = js_debug_adapter(server, node_bin, &pkg, env, None);
    Ok(ResolvedDapAdapter {
        adapter_name: "js-debug".into(),
        language: language.to_string(),
        transport,
        initialize_args,
        launch_command: "launch".into(),
        launch_args: json!({
            "type": "pwa-node",
            "request": "launch",
            "name": "Playwright",
            "program": cli.to_string_lossy(),
            "args": pw_args,
            "cwd": pkg,
            "runtimeExecutable": node_bin,
            "console": "internalConsole",
            // The reporter writes to stdout, not through `console`.
            "outputCapture": "std",
            "skipFiles": [
                "<node_internals>/**",
                "**/node_modules/{playwright,playwright-core,@playwright}/**",
            ],
        }),
        normalized_cwd: pkg,
        initial_output: vec![console("Starting Playwright test (1 worker)…\n")],
    })
}

/// The Karma builder of the Angular project that owns `spec` (`angular.json`'s
/// `architect.test`) and whether it takes `--browsers=false`.
fn karma_builder(ng_root: &Path, spec: Option<&str>) -> Result<bool> {
    let raw = std::fs::read_to_string(ng_root.join("angular.json"))
        .map_err(|e| Error::Invalid(format!("cannot read angular.json: {e}")))?;
    let val: Value = serde_json::from_str(&raw)
        .map_err(|e| Error::Invalid(format!("angular.json is not valid JSON: {e}")))?;
    let projects = val.get("projects").and_then(Value::as_object);
    let owns = |p: &Value| {
        let root = p.get("root").and_then(Value::as_str).unwrap_or("");
        spec.is_some_and(|s| Path::new(s).starts_with(ng_root.join(root)))
    };
    let test = projects
        .and_then(|ps| {
            ps.values()
                .filter(|p| {
                    !p.get("root")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .is_empty()
                })
                .find(|p| owns(p))
                .or_else(|| ps.values().next())
        })
        .and_then(|p| p.pointer("/architect/test"))
        .ok_or_else(|| Error::Invalid("angular.json has no test target".into()))?;
    let builder = test.get("builder").and_then(Value::as_str).unwrap_or("");
    match builder {
        "@angular/build:karma" | "@angular-devkit/build-angular:karma" => Ok(true),
        "@angular/build:unit-test"
            if test.pointer("/options/runner").and_then(Value::as_str) == Some("karma") =>
        {
            Ok(false)
        }
        other => Err(Error::Invalid(format!(
            "This Angular project's test builder ({other}) does not run Karma. {PLAIN_KARMA}"
        ))),
    }
}

/// Karma's URL in an `ng test` output line.
fn karma_url(line: &str) -> Option<String> {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(r"https?://(?:localhost|127\.0\.0\.1):\d+/").expect("static karma url regex")
    });
    re.find(line).map(|m| m.as_str().to_string())
}

fn strip_ansi(line: &str) -> String {
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(r"\x1b\[[0-9;]*[A-Za-z]").expect("static ansi regex"));
    re.replace_all(line, "").into_owned()
}

/// The `debug.html` URL that runs the spec `full_name` (a suite runs all of its specs).
fn karma_debug_url(base: &str, full_name: Option<&str>) -> String {
    let mut url = format!("{}/debug.html", base.trim_end_matches('/'));
    if let Some(name) = full_name.filter(|n| !n.is_empty()) {
        url.push_str("?spec=");
        for b in name.bytes() {
            if b.is_ascii_alphanumeric() || b"-_.~".contains(&b) {
                url.push(char::from(b));
            } else {
                url.push_str(&format!("%{b:02X}"));
            }
        }
    }
    url
}

/// Where js-debug finds Chrome: `CHROME_PATH`, else the installed stable build.
fn chrome_executable(env: &BTreeMap<String, String>) -> Result<String> {
    if let Some(path) = env.get("CHROME_PATH").filter(|p| !p.trim().is_empty()) {
        return Ok(path.clone());
    }
    let home = env.get("HOME").cloned().unwrap_or_default();
    let found = [
        "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome".to_string(),
        format!("{home}/Applications/Google Chrome.app/Contents/MacOS/Google Chrome"),
        "/usr/bin/google-chrome".to_string(),
        "/usr/bin/google-chrome-stable".to_string(),
    ]
    .into_iter()
    .any(|p| Path::new(&p).is_file());
    if found {
        Ok("stable".into())
    } else {
        Err(Error::NotFound(
            "No Chrome or Chromium found on this computer. Install Chrome or set CHROME_PATH."
                .into(),
        ))
    }
}

#[allow(clippy::too_many_arguments)]
async fn resolve_angular_karma(
    server: &Path,
    language: &str,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    node_bin: &str,
    spec_file: Option<&str>,
    ng_root: &Path,
    env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    let takes_no_browser = karma_builder(ng_root, spec_file)?;
    let chrome = chrome_executable(&env)?;
    let ng = find_in_node_modules(ng_root, &["@angular/cli/bin/ng.js"]).ok_or_else(|| {
        Error::Invalid(format!(
            "The Angular CLI is not installed in {}. Run npm install, then Debug again.",
            ng_root.display()
        ))
    })?;
    let root = ng_root.to_string_lossy().into_owned();
    let mut ng_args = vec![
        ng.to_string_lossy().into_owned(),
        "test".into(),
        "--watch".into(),
    ];
    let mut initial_output = vec![console("Building tests (ng test)…\n")];
    if takes_no_browser {
        ng_args.push("--browsers=false".into());
    } else {
        initial_output.push(console(
            "This test builder cannot start Karma without a browser, so the configured Karma browser runs the tests too.\n",
        ));
    }
    let include = spec_file.or_else(|| {
        ctx.run_config
            .as_ref()
            .and_then(|c| c.target_path.as_deref())
            .filter(|t| !t.is_empty() && *t != "package.json" && *t != "angular.json")
    });
    if let Some(file) = include {
        let rel = Path::new(file)
            .strip_prefix(ng_root)
            .map_or_else(|_| file.to_string(), |r| r.to_string_lossy().into_owned());
        ng_args.push(format!("--include={rel}"));
    }
    let (companion, url, captured) =
        start_ng_test(req, ctx, node_bin, &ng_args, &root, &env).await?;
    initial_output.push(IdeDebugOutputEntry {
        category: "stdout".into(),
        text: captured,
        seq: 0,
    });
    initial_output.push(console("Opening Karma debug page\n"));
    let spec_name = spec_file.and(req.launch.test_name.as_deref());
    let (transport, initialize_args) =
        js_debug_adapter(server, node_bin, &root, env, Some(companion));
    Ok(ResolvedDapAdapter {
        adapter_name: "js-debug".into(),
        language: language.to_string(),
        transport,
        initialize_args,
        launch_command: "launch".into(),
        launch_args: json!({
            "type": "pwa-chrome",
            "request": "launch",
            "name": "Karma",
            "url": karma_debug_url(&url, spec_name),
            "webRoot": root,
            // The esbuild builders serve the bundles at `/base/` with sources
            // relative to the workspace (`src/app/app.ts`); the webpack builder's
            // `webpack:///src/…` sources map through js-debug's defaults.
            "pathMapping": { "/base/": format!("{root}/") },
            "pauseForSourceMap": true,
            "runtimeExecutable": chrome,
            "console": "internalConsole",
            "skipFiles": ["**/node_modules/**"],
        }),
        normalized_cwd: root,
        initial_output,
    })
}

/// Start `ng test --watch` and wait for Karma's URL (the first build can be slow):
/// the companion guard, the URL and the output so far. Later output keeps flowing
/// through the guard to the session console.
async fn start_ng_test(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    node_bin: &str,
    args: &[String],
    cwd: &str,
    env: &BTreeMap<String, String>,
) -> Result<(std::sync::Arc<BridgeGuard>, String, String)> {
    let mut child =
        spawn_target_process(&req.target, ctx.ssh.as_ref(), node_bin, args, cwd, env).await?;
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<String>();
    for pipe in [
        child
            .stdout
            .take()
            .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
        child
            .stderr
            .take()
            .map(|p| Box::new(p) as Box<dyn tokio::io::AsyncRead + Send + Unpin>),
    ]
    .into_iter()
    .flatten()
    {
        let tx = tx.clone();
        crate::line_reader::spawn_line_drain(
            pipe,
            crate::line_reader::MAX_OUTPUT_LINE_BYTES,
            move |l| {
                let _ = tx.send(strip_ansi(&l));
            },
        );
    }
    drop(tx);
    // Owned before the wait: a Stop that cancels this start kills `ng test`.
    let companion = BridgeGuard::new();
    companion.push_child(child);
    let mut captured = String::new();
    let tail = |c: &str| {
        c.lines()
            .rev()
            .take(40)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n")
    };
    let deadline = tokio::time::Instant::now() + KARMA_START_TIMEOUT;
    loop {
        let line = match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(line)) => line,
            Ok(None) => {
                return Err(Error::Invalid(format!(
                    "The test build failed: ng test exited.\n{}",
                    tail(&captured)
                )))
            }
            Err(_) => {
                return Err(Error::Upstream(format!(
                    "Karma did not start within 5 minutes.\n{}",
                    tail(&captured)
                )))
            }
        };
        captured.push_str(&line);
        captured.push('\n');
        if line.contains("bundle generation failed") || line.contains("Build failed") {
            // The errors themselves follow the summary line.
            while let Ok(Some(more)) = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await
            {
                captured.push_str(&more);
                captured.push('\n');
            }
            return Err(Error::Invalid(format!(
                "The test build failed.\n{}",
                tail(&captured)
            )));
        }
        if let Some(url) = karma_url(&line) {
            let (out_tx, out_rx) = tokio::sync::mpsc::unbounded_channel();
            companion.set_output(out_rx);
            tokio::spawn(async move {
                while let Some(line) = rx.recv().await {
                    if out_tx.send(line).is_err() {
                        break;
                    }
                }
            });
            return Ok((companion, url, captured));
        }
    }
}

#[cfg(test)]
#[path = "adapters_js_tests_tests.rs"]
mod tests;
