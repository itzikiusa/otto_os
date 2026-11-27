use std::collections::BTreeMap;
use std::path::Path;

use super::*;
use crate::dap::start_error::{classify_start_error, KIND_ADAPTER_MISSING};
use crate::types::{IdeDebugLaunchSpec, IdeRunConfig, IdeTargetSpec};

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn launch_req(root: &Path, program: &Path, test_line: Option<u32>) -> IdeStartDebugReq {
    IdeStartDebugReq {
        target: IdeTargetSpec::local(root.to_string_lossy().into_owned()),
        language: "typescript".into(),
        launch: IdeDebugLaunchSpec {
            program: program.to_string_lossy().into_owned(),
            test_name: Some("second test".into()),
            test_line,
            ..Default::default()
        },
        ..Default::default()
    }
}

#[tokio::test]
async fn playwright_test_debugs_its_line_with_one_worker_under_js_debug() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let pkg = tmp.path().join("pw");
    write(
        &data.join("toolchains/node/dap/js-debug/src/dapDebugServer.js"),
        "",
    );
    write(&pkg.join("node_modules/@playwright/test/cli.js"), "");
    write(
        &pkg.join("package.json"),
        r#"{"devDependencies":{"@playwright/test":"1.65.1"}}"#,
    );
    write(&pkg.join("playwright.config.ts"), "");
    let spec = pkg.join("tests/example.spec.ts");
    write(&spec, "import { test } from '@playwright/test';\n");
    let req = launch_req(&pkg, &spec, Some(9));
    let program = spec.to_string_lossy().into_owned();
    let resolved = resolve_js_test_debug(
        &data,
        "typescript",
        JsTestRunner::Playwright,
        &req,
        &DapStartContext::default(),
        "/usr/bin/node",
        (&program, &[], &pkg.to_string_lossy()),
        BTreeMap::from([("PWDEBUG".to_string(), "1".to_string())]),
    )
    .await
    .unwrap();
    assert_eq!(resolved.adapter_name, "js-debug");
    let DapAdapterTransport::JsDebug { args, env, .. } = &resolved.transport else {
        panic!("js-debug transport: {:?}", resolved.transport);
    };
    assert_eq!(args[1..], ["0", "127.0.0.1"]);
    assert!(
        !env.contains_key("PWDEBUG"),
        "Playwright's own inspector stays off"
    );
    let launch = &resolved.launch_args;
    assert_eq!(launch["type"], "pwa-node");
    assert!(launch["program"]
        .as_str()
        .unwrap()
        .ends_with("node_modules/@playwright/test/cli.js"));
    assert_eq!(
        launch["args"],
        json!([
            "test",
            "tests/example.spec.ts:9",
            "--workers=1",
            "--timeout=0",
            "--reporter=list",
            "--headed"
        ])
    );
    assert_eq!(launch["cwd"], pkg.to_string_lossy().as_ref());
}

#[tokio::test]
async fn missing_js_debug_offers_the_install_and_missing_playwright_says_npm_install() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("pw");
    let spec = pkg.join("e2e/a.spec.ts");
    write(&spec, "import { test } from '@playwright/test';\n");
    write(&pkg.join("package.json"), "{}");
    let req = launch_req(&pkg, &spec, Some(2));
    let program = spec.to_string_lossy().into_owned();
    let run = |data: std::path::PathBuf| {
        let (req, program, pkg) = (req.clone(), program.clone(), pkg.clone());
        async move {
            resolve_js_test_debug(
                &data,
                "typescript",
                JsTestRunner::Playwright,
                &req,
                &DapStartContext::default(),
                "node",
                (&program, &[], &pkg.to_string_lossy()),
                BTreeMap::new(),
            )
            .await
            .unwrap_err()
        }
    };
    let err = run(tmp.path().join("empty")).await;
    let failure = classify_start_error(&err, "typescript");
    assert_eq!(failure.kind, Some(KIND_ADAPTER_MISSING), "{err}");
    assert_eq!(failure.ecosystem, Some("node"));
    assert!(failure
        .message
        .starts_with("Debugging tests needs js-debug (1.2 MB)"));

    let data = tmp.path().join("data");
    write(
        &data.join("toolchains/node/dap/js-debug/src/dapDebugServer.js"),
        "",
    );
    let err = run(data).await.to_string();
    assert!(
        err.contains("Playwright is not installed in") && err.contains("npm install"),
        "{err}"
    );
}

#[tokio::test]
async fn plain_karma_stays_run_only_and_ssh_is_refused() {
    let tmp = tempfile::tempdir().unwrap();
    let pkg = tmp.path().join("karma-app");
    let spec = pkg.join("src/a.spec.js");
    write(&spec, "describe('a', () => {});\n");
    write(
        &pkg.join("package.json"),
        r#"{"devDependencies":{"karma":"6"}}"#,
    );
    let req = launch_req(&pkg, &spec, None);
    let program = spec.to_string_lossy().into_owned();
    let err = resolve_js_test_debug(
        tmp.path(),
        "javascript",
        JsTestRunner::Karma,
        &req,
        &DapStartContext::default(),
        "node",
        (&program, &[], &pkg.to_string_lossy()),
        BTreeMap::new(),
    )
    .await
    .unwrap_err();
    assert_eq!(err.to_string(), format!("invalid: {PLAIN_KARMA}"));

    let mut ssh = req.clone();
    ssh.target = IdeTargetSpec::ssh("conn-1".into(), "/srv/app");
    let err = resolve_js_test_debug(
        tmp.path(),
        "javascript",
        JsTestRunner::Playwright,
        &ssh,
        &DapStartContext::default(),
        "node",
        ("/srv/app/e2e/a.spec.ts", &[], "/srv/app"),
        BTreeMap::new(),
    )
    .await
    .unwrap_err();
    assert!(
        err.to_string().contains("On an SSH target, use Run"),
        "{err}"
    );
}

#[test]
fn karma_builders_decide_whether_ng_test_runs_without_a_browser() {
    let tmp = tempfile::tempdir().unwrap();
    let ng = |builder: &str, runner: &str| {
        write(
            &tmp.path().join("angular.json"),
            &json!({ "projects": { "app": { "root": "", "architect": { "test": {
                "builder": builder, "options": { "runner": runner } } } } } })
            .to_string(),
        );
        karma_builder(tmp.path(), None).map_err(|e| e.to_string())
    };
    assert_eq!(ng("@angular/build:karma", ""), Ok(true));
    assert_eq!(ng("@angular-devkit/build-angular:karma", ""), Ok(true));
    // Its --browsers is an array: "false" names a browser Karma cannot load.
    assert_eq!(ng("@angular/build:unit-test", "karma"), Ok(false));
    assert!(ng("@angular/build:unit-test", "vitest")
        .unwrap_err()
        .contains("does not run Karma"));
}

#[test]
fn karma_url_and_debug_page_selection() {
    assert_eq!(
        karma_url("WARN [karma]: No captured browser, open http://localhost:9876/").as_deref(),
        Some("http://localhost:9876/")
    );
    let colored = strip_ansi("\x1b[32m28 11 2026 INFO [karma-server]: \x1b[39mKarma v6.4.4 server started at http://localhost:9877/");
    assert_eq!(
        karma_url(&colored).as_deref(),
        Some("http://localhost:9877/")
    );
    assert_eq!(karma_url("Building..."), None);
    assert_eq!(
        karma_debug_url(
            "http://localhost:9876/",
            Some("App should greet the visitor")
        ),
        "http://localhost:9876/debug.html?spec=App%20should%20greet%20the%20visitor"
    );
    assert_eq!(
        karma_debug_url("http://localhost:9876/", None),
        "http://localhost:9876/debug.html"
    );
}

#[test]
fn toolbar_playwright_keeps_the_configured_selection_but_not_debug_conflicting_flags() {
    let args: Vec<String> = [
        "--no-install",
        "playwright",
        "test",
        "e2e/",
        "--project=chromium",
        "--workers=4",
        "--ui",
    ]
    .map(String::from)
    .to_vec();
    let ctx = DapStartContext {
        run_config: Some(IdeRunConfig {
            target_path: Some("e2e/login.spec.ts".into()),
            ..Default::default()
        }),
        ..Default::default()
    };
    assert_eq!(
        playwright_toolbar_args(&args, &ctx),
        ["e2e/", "--project=chromium", "e2e/login.spec.ts"]
    );
}
