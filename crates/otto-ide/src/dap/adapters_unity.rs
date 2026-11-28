//! Unity Editor attach (rc.6 A1): debug C# scripts running in the Unity
//! Editor's Mono runtime through DotRush's open-source Mono debug adapter
//! (`monodbg`, MIT; Otto downloads it on request, see
//! `toolchains/installer_unity.rs`). Unity's Mono speaks the Soft Debugger wire
//! protocol, not DAP; `monodbg` speaks DAP on stdio and the Soft Debugger
//! protocol to the Editor.
//!
//! Discovery: while a project is open, Unity writes `Library/EditorInstance.json`
//! (`process_id`, `version`, `app_path`). The Editor's soft debugger listens on
//! loopback port `56000 + pid % 1000` (Unity's MIT
//! `MonoDevelop.Debugger.Soft.Unity`). Nothing here probes that port:
//! connecting to the soft debugger is itself an attach. This Mac only; players,
//! IL2CPP and SSH are out of scope.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use otto_core::{Error, Result};
use serde::Deserialize;
use serde_json::json;

use crate::dap::adapters::{DapAdapterTransport, ResolvedDapAdapter};
use crate::toolchains::installer_unity::{find_dotnet_10, monodbg_dll};
use crate::types::{IdeDebugOutputEntry, IdeStartDebugReq, IdeTargetKind, IdeUnityEditor};

/// The adapter name the session uses for a Unity attach.
pub(crate) const MONODBG: &str = "monodbg";

/// `EditorInstance.json` is a few hundred bytes; anything larger is not Unity's.
const MAX_EDITOR_INSTANCE_BYTES: u64 = 4096;

#[derive(Deserialize)]
struct EditorInstance {
    process_id: u32,
    #[serde(default)]
    version: String,
    #[serde(default)]
    app_path: String,
}

/// The Editor's Mono soft-debugger port (Unity's own formula).
pub(crate) fn editor_port(pid: u32) -> u16 {
    (56000 + pid % 1000) as u16
}

/// The running Unity Editor that has `project_root` open: `EditorInstance.json`
/// exists inside the project, its process is alive and runs an executable inside
/// the Editor's `app_path`. `None` when no Editor has the project open (or the
/// file is stale and its PID now runs something else).
pub(crate) async fn find_unity_editor(project_root: &Path) -> Option<IdeUnityEditor> {
    let root = project_root.canonicalize().ok()?;
    let file = root
        .join("Library/EditorInstance.json")
        .canonicalize()
        .ok()?;
    // Canonicalized, so a symlink pointing out of the project is rejected here.
    if !file.starts_with(&root) {
        return None;
    }
    let meta = tokio::fs::metadata(&file).await.ok()?;
    if !meta.is_file() || meta.len() > MAX_EDITOR_INSTANCE_BYTES {
        return None;
    }
    let inst: EditorInstance = serde_json::from_slice(&tokio::fs::read(&file).await.ok()?).ok()?;
    let app = Path::new(&inst.app_path).canonicalize().ok()?;
    let exe = process_executable(inst.process_id).await?;
    if !exe.starts_with(&app) {
        return None;
    }
    Some(IdeUnityEditor {
        pid: inst.process_id,
        version: inst.version,
        port: editor_port(inst.process_id),
        project_root: root.to_string_lossy().into_owned(),
    })
}

/// The executable a live process runs; `None` when the PID is not alive.
#[cfg(target_os = "macos")]
async fn process_executable(pid: u32) -> Option<PathBuf> {
    // macOS `ps -o comm=` prints the executable's full path (and fails for a dead PID).
    let out = tokio::process::Command::new("/bin/ps")
        .args(["-o", "comm=", "-p", &pid.to_string()])
        .kill_on_drop(true)
        .output()
        .await
        .ok()?;
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !path.is_empty())
        .then(|| Path::new(&path).canonicalize().ok())
        .flatten()
}

#[cfg(not(target_os = "macos"))]
async fn process_executable(pid: u32) -> Option<PathBuf> {
    tokio::fs::canonicalize(format!("/proc/{pid}/exe"))
        .await
        .ok()
}

/// The project's script assemblies the debugger loads symbols for: every
/// `Library/ScriptAssemblies/*.dll` with a sibling `.pdb`, except Unity's own
/// (`Unity.*`, `UnityEngine.*`, `UnityEditor.*`). Keeps `.asmdef` assemblies,
/// which the adapter's `Assembly-CSharp*` default would drop.
#[allow(clippy::disallowed_methods)] // one small directory, read once per attach
fn user_assemblies(root: &Path) -> Vec<String> {
    let Ok(rd) = std::fs::read_dir(root.join("Library/ScriptAssemblies")) else {
        return Vec::new();
    };
    let mut out: Vec<String> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
            name.ends_with(".dll")
                && !["Unity.", "UnityEngine.", "UnityEditor."]
                    .iter()
                    .any(|pre| name.starts_with(pre))
                && p.with_extension("pdb").is_file()
        })
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    out.sort();
    out
}

/// `unity://editor`: attach `monodbg` to the Editor that has the project open.
pub(crate) async fn resolve_unity_attach(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    cwd: &str,
    mut env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    if !req
        .launch
        .program
        .trim()
        .eq_ignore_ascii_case("unity://editor")
    {
        return Err(Error::Invalid(format!(
            "unsupported Unity debug target '{}'; use unity://editor",
            req.launch.program.trim()
        )));
    }
    if req.target.kind != IdeTargetKind::Local {
        return Err(Error::Invalid(
            "Unity Editor attach works on this Mac only, not over SSH.".into(),
        ));
    }
    let root = super::detect_unity_project_root(Path::new(cwd), Path::new(&req.target.root))
        .map(|(p, _)| p)
        .ok_or_else(|| {
            Error::Invalid(
                "This folder isn't a Unity project (no Assets and ProjectSettings folders).".into(),
            )
        })?;
    let toolchains = data_dir.join("toolchains");
    let dll = monodbg_dll(&toolchains);
    if !dll.is_file() {
        return Err(Error::NotFound(
            "The Unity debugger isn't installed.".into(),
        ));
    }
    let path_var = env.get("PATH").cloned().unwrap_or_default();
    let dotnet = find_dotnet_10(
        &toolchains,
        env.get("DOTNET_ROOT").map(String::as_str),
        &path_var,
    )
    .await
    .ok_or_else(|| {
        Error::NotFound("The Unity debugger needs .NET 10. Install .NET 10 from Toolchains.".into())
    })?;
    let editor = find_unity_editor(&root).await.ok_or_else(|| {
        Error::NotFound(
            "No Unity Editor has this project open. Open it in Unity, then attach.".into(),
        )
    })?;
    env.insert("DOTNET_CLI_TELEMETRY_OPTOUT".into(), "1".into());
    env.insert("DOTNET_NOLOGO".into(), "1".into());
    let root_s = root.to_string_lossy().into_owned();
    Ok(ResolvedDapAdapter {
        adapter_name: MONODBG.into(),
        language: "csharp".into(),
        transport: DapAdapterTransport::Stdio {
            command: dotnet,
            args: vec![dll.to_string_lossy().into_owned()],
            cwd: root_s.clone(),
            env,
        },
        initialize_args: json!({
            "adapterID": "unity",
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "pathFormat": "path",
            "supportsVariableType": true,
            "supportsRunInTerminalRequest": false
        }),
        launch_command: "attach".into(),
        // `processId` is the debugger PORT for monodbg (it falls back to its own
        // copy of the port formula only when it is 0): Otto's discovery decides.
        launch_args: json!({
            "name": "Attach to Unity Editor",
            "type": "unity",
            "request": "attach",
            "cwd": root_s,
            "processId": editor.port,
            "userAssemblies": user_assemblies(&root),
            // No `debuggerOptions`: monodbg's own defaults (1 s evaluation, 5 s
            // member evaluation, method calls and ToString allowed, project
            // assemblies only). A partial object would replace them wholesale.
        }),
        normalized_cwd: root_s,
        initial_output: vec![IdeDebugOutputEntry {
            seq: 0,
            category: "console".into(),
            text: format!(
                "Attaching to Unity Editor {} (pid {}, port {})…\n",
                editor.version, editor.pid, editor.port
            ),
        }],
    })
}

/// A failed Unity handshake, in the person's terms: the usual causes are the
/// Editor's Release code optimization or a port that is not listening.
pub(crate) fn attach_failed(port: u16, detail: &str) -> Error {
    Error::Upstream(format!(
        "Couldn't attach to the Unity Editor on port {port}. Set Code Optimization to Debug (bug icon, bottom right of the Editor) and try again. ({detail})"
    ))
}

/// The console line after a successful attach.
pub(crate) const ATTACHED_HINT: &str = "Unity Editor attached. Press Play in Unity to run your scripts. The Editor stays paused while a breakpoint is hit.\n";

#[cfg(test)]
#[path = "adapters_unity_tests.rs"]
mod tests;
