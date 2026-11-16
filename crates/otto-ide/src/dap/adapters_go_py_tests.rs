use super::*;
use crate::types::{IdeDebugLaunchSpec, IdeProjectEnvironment, IdeRunConfig, IdeRunnerKind};

#[test]
fn resolves_go_test_pkg_dir_and_extracts_build_tags_for_all_tests_debug() {
    let tmp = tempfile::tempdir().unwrap();
    let repo = tmp.path();
    std::fs::write(repo.join("go.mod"), "module example.com/svc\n\ngo 1.24\n").unwrap();
    let sub_a = repo.join("tests/feature_a");
    let sub_b = repo.join("tests/feature_b");
    std::fs::create_dir_all(&sub_a).unwrap();
    std::fs::create_dir_all(&sub_b).unwrap();
    std::fs::write(
        sub_a.join("a_test.go"),
        "//go:build integration\n\npackage feature_a\n",
    )
    .unwrap();
    let bp_file = sub_b.join("b_component_test.go");
    std::fs::write(&bp_file, "//go:build component\n\npackage feature_b\n").unwrap();

    let target = IdeTargetSpec::local(repo.to_string_lossy().into_owned());
    let cwd = repo.to_string_lossy().into_owned();
    let bps = vec![IdeBreakpoint {
        path: bp_file.to_string_lossy().into_owned(),
        line: 5,
        enabled: true,
        ..Default::default()
    }];
    let raw_prog = format!("{cwd}/...");
    let (pkg_dir, notes) = resolve_go_test_pkg_dir(&target, &raw_prog, &cwd, &bps);
    assert_eq!(pkg_dir, sub_b.to_string_lossy());
    assert_eq!(notes.len(), 1);
    assert!(notes[0].text.contains("feature_b"));

    let ctx = DapStartContext::new().with_go_build_tags(vec!["e2e".into()]);
    let tags = collect_dap_go_build_tags(&target, &ctx, &raw_prog, &cwd, Some(&pkg_dir), &bps);
    assert_eq!(tags, vec!["e2e".to_string(), "component".to_string()]);
}

#[test]
fn main_go_alongside_sibling_test_file_never_switches_to_dlv_test_mode() {
    let tmp = tempfile::tempdir().unwrap();
    let app = tmp.path().join("cmd/app");
    std::fs::create_dir_all(&app).unwrap();
    std::fs::write(
        tmp.path().join("go.mod"),
        "module example.com/demo\n\ngo 1.24\n",
    )
    .unwrap();
    let main_go = app.join("main.go");
    let helper_go = app.join("helper.go");
    let main_test_go = app.join("main_test.go");
    std::fs::write(&main_go, "package main\n\nfunc main() { helper() }\n").unwrap();
    std::fs::write(&helper_go, "package main\n\nfunc helper() {}\n").unwrap();
    std::fs::write(
        &main_test_go,
        "package main\n\nimport \"testing\"\n\nfunc TestMainHelper(t *testing.T) {}\n",
    )
    .unwrap();

    let target = IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned());
    let ctx = DapStartContext::new();

    // 1. Gutter / file launch targeting `main.go` alongside `main_test.go`
    let main_req = IdeStartDebugReq {
        target: target.clone(),
        language: "go".into(),
        launch: IdeDebugLaunchSpec {
            program: main_go.to_string_lossy().into_owned(),
            cwd: Some(app.to_string_lossy().into_owned()),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(
        !should_debug_go_as_test(&main_req, &ctx, &main_req.launch.program),
        "main.go must never use dlv test mode even when main_test.go is in the same directory"
    );
    // Multi-file `package main` resolves to the package directory for compilation.
    assert_eq!(
        resolve_go_debug_program(&target, &main_req.launch.program, &app.to_string_lossy()),
        app.to_string_lossy()
    );

    // 2. Directory launch on `package main` dir without `test_name` also stays binary mode.
    let dir_req = IdeStartDebugReq {
        target: target.clone(),
        language: "go".into(),
        launch: IdeDebugLaunchSpec {
            program: app.to_string_lossy().into_owned(),
            cwd: Some(app.to_string_lossy().into_owned()),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(
        !should_debug_go_as_test(&dir_req, &ctx, &dir_req.launch.program),
        "package main directory without test_name must launch main binary, not tests"
    );

    // 3. Explicit GoRun run_config with directory program stays binary mode.
    let go_run_ctx = DapStartContext::new().with_run_config(Some(IdeRunConfig {
        runner: IdeRunnerKind::GoRun,
        ..Default::default()
    }));
    assert!(!should_debug_go_as_test(
        &dir_req,
        &go_run_ctx,
        &dir_req.launch.program
    ));

    // 4. Test launch (`main_test.go` or `test_name: Some(...)`) uses test mode.
    let test_file_req = IdeStartDebugReq {
        target: target.clone(),
        language: "go".into(),
        launch: IdeDebugLaunchSpec {
            program: main_test_go.to_string_lossy().into_owned(),
            cwd: Some(app.to_string_lossy().into_owned()),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(should_debug_go_as_test(
        &test_file_req,
        &ctx,
        &test_file_req.launch.program
    ));

    let test_fn_req = IdeStartDebugReq {
        target: target.clone(),
        language: "go".into(),
        launch: IdeDebugLaunchSpec {
            program: app.to_string_lossy().into_owned(),
            cwd: Some(app.to_string_lossy().into_owned()),
            test_name: Some("^TestMainHelper$".into()),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(should_debug_go_as_test(
        &test_fn_req,
        &ctx,
        &test_fn_req.launch.program
    ));

    // 5. Pure library package directory (no `package main`) with `*_test.go` uses test mode.
    let lib_dir = tmp.path().join("pkg/calc");
    std::fs::create_dir_all(&lib_dir).unwrap();
    std::fs::write(
        lib_dir.join("calc.go"),
        "package calc\n\nfunc Add(a, b int) int { return a + b }\n",
    )
    .unwrap();
    std::fs::write(
        lib_dir.join("calc_test.go"),
        "package calc\n\nimport \"testing\"\n\nfunc TestAdd(t *testing.T) {}\n",
    )
    .unwrap();
    let lib_req = IdeStartDebugReq {
        target,
        language: "go".into(),
        launch: IdeDebugLaunchSpec {
            program: lib_dir.to_string_lossy().into_owned(),
            cwd: Some(lib_dir.to_string_lossy().into_owned()),
            ..Default::default()
        },
        ..Default::default()
    };
    assert!(should_debug_go_as_test(
        &lib_req,
        &ctx,
        &lib_req.launch.program
    ));
}

#[test]
fn collect_dap_go_build_tags_honors_empty_module_override() {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    let sub = root.join("submod");
    std::fs::create_dir_all(&sub).unwrap();
    let target = IdeTargetSpec::local(root.to_string_lossy().into_owned());
    let mut module_go_build_tags = BTreeMap::new();
    module_go_build_tags.insert(sub.to_string_lossy().into_owned(), Vec::new());

    let proj = IdeProjectEnvironment {
        go_build_tags: vec!["component".into()],
        module_go_build_tags,
        ..Default::default()
    };
    let ctx = DapStartContext::new().with_project_env(Some(proj));
    let sub_main = sub.join("main.go");
    std::fs::write(&sub_main, "package main\n\nfunc main() {}\n").unwrap();

    let tags = collect_dap_go_build_tags(
        &target,
        &ctx,
        &sub_main.to_string_lossy(),
        &sub.to_string_lossy(),
        None,
        &[],
    );
    assert!(
        tags.is_empty(),
        "empty [] module_go_build_tags override must suppress inherited workspace tags: {tags:?}"
    );
}

/// debugpy refuses a launch with both `module` and `program`; the bdb bridge needs `program`.
#[tokio::test]
#[cfg(unix)]
async fn pytest_debug_launch_omits_program_only_under_debugpy() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let test_file = tmp.path().join("test_cart.py");
    std::fs::write(&test_file, "def test_total():\n    pass\n").unwrap();
    // A stand-in interpreter that "imports" debugpy.adapter.
    let fake_py = tmp.path().join("fakepy");
    std::fs::write(&fake_py, "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(&fake_py, std::fs::Permissions::from_mode(0o755)).unwrap();
    let ws = tmp.path().to_string_lossy().into_owned();
    let req = IdeStartDebugReq {
        target: IdeTargetSpec::local(ws.clone()),
        language: "python".into(),
        launch: IdeDebugLaunchSpec {
            program: test_file.to_string_lossy().into_owned(),
            test_name: Some(format!("{}::test_total", test_file.display())),
            ..Default::default()
        },
        ..Default::default()
    };
    let ctx = DapStartContext {
        caller_is_root: true,
        project_env: Some(IdeProjectEnvironment {
            python_interpreter: Some(fake_py.to_string_lossy().into_owned()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let resolved = crate::dap::adapters_py::resolve_python_adapter(
        tmp.path(),
        &req,
        &ctx,
        &ws,
        BTreeMap::new(),
    )
    .await
    .unwrap();
    assert_eq!(resolved.adapter_name, "debugpy");
    assert_eq!(resolved.launch_args["module"], "pytest");
    assert!(
        resolved.launch_args.get("program").is_none(),
        "{}",
        resolved.launch_args
    );
}
