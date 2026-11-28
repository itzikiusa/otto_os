use super::*;

fn unity_project(dir: &Path) -> PathBuf {
    let root = dir.join("MyGame");
    std::fs::create_dir_all(root.join("Assets/Scripts")).unwrap();
    std::fs::create_dir_all(root.join("ProjectSettings")).unwrap();
    std::fs::create_dir_all(root.join("Library/ScriptAssemblies")).unwrap();
    root.canonicalize().unwrap()
}

/// `EditorInstance.json` for this test process: alive, and its executable is
/// the test binary, so `app_path` is the binary's folder.
fn write_instance(root: &Path, pid: u32, app_path: &Path) {
    std::fs::write(
        root.join("Library/EditorInstance.json"),
        serde_json::to_vec(&json!({
            "process_id": pid,
            "version": "6000.0.23f1",
            "app_path": app_path,
            "app_contents_path": app_path.join("Contents"),
        }))
        .unwrap(),
    )
    .unwrap();
}

fn this_exe_dir() -> PathBuf {
    std::env::current_exe()
        .unwrap()
        .canonicalize()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

/// monodbg advertises no `supportsConfigurationDoneRequest` and exits on the
/// request: Otto sends it only when advertised (as VS Code does), so the attach
/// reaches running instead of "adapter closed during configurationDone". It
/// reads exception filters only from `filterOptions`, which Otto sends too.
#[tokio::test]
async fn an_adapter_without_configuration_done_is_never_sent_it() {
    use crate::dap::wire::{DapMessage, DapReader, DapWriter};
    use crate::dap::DapManager;
    use crate::types::{IdeDebugLaunchSpec, IdeDebugState, IdeTargetSpec};
    use std::sync::{Arc, Mutex};

    let tmp = tempfile::tempdir().unwrap();
    let seen: Arc<Mutex<Vec<(String, serde_json::Value)>>> = Arc::default();
    let seen_srv = Arc::clone(&seen);
    let (client_io, server_io) = tokio::io::duplex(16384);
    tokio::spawn(async move {
        let (r, w) = tokio::io::split(server_io);
        let mut reader = DapReader::new(r);
        let mut writer = DapWriter::new(w);
        while let Ok(Some(DapMessage::Request(req))) = reader.next_message().await {
            seen_srv.lock().unwrap().push((
                req.command.clone(),
                req.arguments.clone().unwrap_or_default(),
            ));
            if req.command == "configurationDone" {
                break; // monodbg: "No handler implemented", the process exits
            }
            let body = if req.command == "initialize" {
                json!({ "exceptionBreakpointFilters": [{ "filter": "all", "label": "All Exceptions" }] })
            } else {
                json!({})
            };
            let _ = writer
                .send_response(req.seq, &req.command, Ok(Some(body)))
                .await;
            if req.command == "attach" {
                let _ = writer.send_event("initialized", Some(json!({}))).await;
            }
        }
    });
    let mgr = DapManager::new(tmp.path().to_path_buf());
    let req = IdeStartDebugReq {
        target: IdeTargetSpec::local(tmp.path().to_string_lossy().into_owned()),
        language: "csharp".into(),
        launch: IdeDebugLaunchSpec {
            program: "unity://editor".into(),
            attach_port: Some(56242),
            ..IdeDebugLaunchSpec::default()
        },
        break_on_exceptions: true,
        ..IdeStartDebugReq::default()
    };
    let snap = mgr
        .start_session_with_stream(req, MONODBG, client_io, true)
        .await
        .expect("attach reaches running");
    assert_eq!(snap.state, IdeDebugState::Running, "{:?}", snap.error);
    let seen = seen.lock().unwrap().clone();
    assert!(seen.iter().any(|(c, _)| c == "attach"), "{seen:?}");
    assert!(
        !seen.iter().any(|(c, _)| c == "configurationDone"),
        "{seen:?}"
    );
    let (_, exc) = seen
        .iter()
        .find(|(c, _)| c == "setExceptionBreakpoints")
        .expect("exception breakpoints sent");
    assert_eq!(
        exc["filterOptions"],
        json!([{ "filterId": "all" }]),
        "{exc}"
    );
}

#[test]
fn editor_port_is_unitys_formula() {
    assert_eq!(editor_port(4242), 56242);
    assert_eq!(editor_port(1000), 56000);
    assert_eq!(editor_port(99_999), 56999);
}

#[tokio::test]
async fn finds_a_live_editor_and_rejects_dead_foreign_and_escaping_instances() {
    let tmp = tempfile::tempdir().unwrap();
    let root = unity_project(tmp.path());
    assert!(
        find_unity_editor(&root).await.is_none(),
        "no EditorInstance.json"
    );

    let pid = std::process::id();
    write_instance(&root, pid, &this_exe_dir());
    let ed = find_unity_editor(&root).await.expect("live editor");
    assert_eq!(ed.pid, pid);
    assert_eq!(ed.version, "6000.0.23f1");
    assert_eq!(ed.port, editor_port(pid));
    assert_eq!(ed.project_root, root.to_string_lossy());

    // The PID runs something outside app_path (a reused PID): not this Editor.
    let other = tmp.path().join("Unity.app");
    std::fs::create_dir_all(&other).unwrap();
    write_instance(&root, pid, &other);
    assert!(
        find_unity_editor(&root).await.is_none(),
        "exe outside app_path"
    );

    // A dead PID.
    let mut child = std::process::Command::new("/bin/sh")
        .args(["-c", "exit 0"])
        .spawn()
        .unwrap();
    let dead = child.id();
    child.wait().unwrap();
    write_instance(&root, dead, &this_exe_dir());
    assert!(find_unity_editor(&root).await.is_none(), "dead pid");

    // A symlink out of the project is not followed.
    let outside = tmp.path().join("outside.json");
    std::fs::write(
        &outside,
        serde_json::to_vec(&json!({"process_id": pid, "version": "x", "app_path": this_exe_dir()}))
            .unwrap(),
    )
    .unwrap();
    std::fs::remove_file(root.join("Library/EditorInstance.json")).unwrap();
    std::os::unix::fs::symlink(&outside, root.join("Library/EditorInstance.json")).unwrap();
    assert!(find_unity_editor(&root).await.is_none(), "symlink escape");

    // Oversized.
    std::fs::remove_file(root.join("Library/EditorInstance.json")).unwrap();
    std::fs::write(root.join("Library/EditorInstance.json"), vec![b' '; 5000]).unwrap();
    assert!(find_unity_editor(&root).await.is_none(), "oversized");
}

#[test]
fn user_assemblies_keep_asmdef_assemblies_with_symbols_and_drop_unitys_own() {
    let tmp = tempfile::tempdir().unwrap();
    let root = unity_project(tmp.path());
    let sa = root.join("Library/ScriptAssemblies");
    for name in [
        "Assembly-CSharp.dll",
        "Assembly-CSharp.pdb",
        "Game.Core.dll",
        "Game.Core.pdb",
        "NoSymbols.dll",
        "Unity.TextMeshPro.dll",
        "Unity.TextMeshPro.pdb",
        "UnityEngine.UI.dll",
        "UnityEngine.UI.pdb",
    ] {
        std::fs::write(sa.join(name), b"x").unwrap();
    }
    let got: Vec<String> = user_assemblies(&root)
        .into_iter()
        .map(|p| {
            Path::new(&p)
                .file_name()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(got, ["Assembly-CSharp.dll", "Game.Core.dll"]);
}

fn req(root: &Path, program: &str) -> IdeStartDebugReq {
    serde_json::from_value(json!({
        "target": {"kind": "local", "root": root},
        "language": "csharp",
        "launch": {"program": program, "args": []}
    }))
    .unwrap()
}

#[tokio::test]
async fn resolves_monodbg_attach_with_the_discovered_port_and_says_what_is_missing() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let root = unity_project(tmp.path());
    let cwd = root.to_string_lossy().into_owned();
    let r = req(&root, "unity://editor");

    let err = resolve_unity_attach(&data, &r, &cwd, BTreeMap::new())
        .await
        .unwrap_err();
    assert!(
        matches!(&err, Error::NotFound(m) if m == "The Unity debugger isn't installed."),
        "{err}"
    );

    let dll = monodbg_dll(&data.join("toolchains"));
    std::fs::create_dir_all(dll.parent().unwrap()).unwrap();
    std::fs::write(&dll, b"x").unwrap();
    // A fake .NET 10 muxer in Otto's managed slot.
    let dn = data.join("toolchains/dotnet/current");
    std::fs::create_dir_all(&dn).unwrap();
    let fake = dn.join("dotnet");
    std::fs::write(
        &fake,
        "#!/bin/sh\necho 'Microsoft.NETCore.App 10.0.15 [/x]'\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();

    let err = resolve_unity_attach(&data, &r, &cwd, BTreeMap::new())
        .await
        .unwrap_err();
    assert!(
        err.to_string()
            .contains("No Unity Editor has this project open"),
        "{err}"
    );

    let pid = std::process::id();
    write_instance(&root, pid, &this_exe_dir());
    std::fs::write(
        root.join("Library/ScriptAssemblies/Assembly-CSharp.dll"),
        b"x",
    )
    .unwrap();
    std::fs::write(
        root.join("Library/ScriptAssemblies/Assembly-CSharp.pdb"),
        b"x",
    )
    .unwrap();
    let res = resolve_unity_attach(&data, &r, &cwd, BTreeMap::new())
        .await
        .unwrap();
    assert_eq!(res.adapter_name, MONODBG);
    assert_eq!(res.launch_command, "attach");
    assert_eq!(res.launch_args["processId"], json!(editor_port(pid)));
    assert_eq!(res.launch_args["cwd"], json!(cwd));
    assert_eq!(
        res.launch_args["userAssemblies"].as_array().unwrap().len(),
        1
    );
    match &res.transport {
        DapAdapterTransport::Stdio { command, args, .. } => {
            assert_eq!(command, &fake.to_string_lossy());
            assert_eq!(args, &[dll.to_string_lossy().into_owned()]);
        }
        other => panic!("expected stdio, got {other:?}"),
    }

    // The withdrawn raw soft-debugger "attach" and SSH stay refused.
    let mut ssh = r.clone();
    ssh.target.kind = IdeTargetKind::Ssh;
    assert!(resolve_unity_attach(&data, &ssh, &cwd, BTreeMap::new())
        .await
        .is_err());
    assert!(resolve_unity_attach(
        &data,
        &req(&root, "unity://player/1"),
        &cwd,
        BTreeMap::new()
    )
    .await
    .is_err());
}
