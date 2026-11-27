//! .NET (`netcoredbg` / `vsdbg` + Unity Editor attach) DAP adapter resolver.
//!
//! Supports Local and Remote SSH targets transparently for C# and F#:
//! - **Application launch:** locates nearest `.csproj`/`.fsproj`, builds Debug
//!   artifacts (`dotnet build -c Debug`), resolves `bin/Debug/<tfm>/<Assembly>.dll`,
//!   and launches `netcoredbg --interpreter=vscode` over stdio.
//! - **Unit test debugging (xUnit / NUnit / MSTest):** builds Debug test project,
//!   launches `dotnet test --no-build --filter FullyQualifiedName~<name>` with
//!   `VSTEST_HOST_DEBUG=1` to obtain the paused `testhost` PID, and attaches
//!   `netcoredbg` directly to that PID.
//! - **Process & Unity Editor attach:** supports explicit PID (`DOTNET_ATTACH_PID`),
//!   Unity Editor project detection (`Assets/` + `ProjectSettings/ProjectVersion.txt`),
//!   running Unity Editor PID + Mono SDB port (`56000 + (pid % 1000)`) discovery,
//!   and direct TCP DAP attach when `attach_port` is provided.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use otto_core::{Error, Result};
use serde_json::json;

use super::adapters::{
    locate_binary, normalize_target_path, DapAdapterTransport, ResolvedDapAdapter,
};
use super::{run_target_command, DapStartContext};
use crate::types::{IdeDebugOutputEntry, IdeStartDebugReq, IdeTargetKind};

/// Parse `<TargetFramework>`, `<TargetFrameworks>`, and `<AssemblyName>` from a
/// `.csproj` / `.fsproj` XML document.
pub(crate) fn parse_dotnet_project_meta(proj_path: &Path, xml: &str) -> (Option<String>, String) {
    fn tag_value(xml: &str, tag: &str) -> Option<String> {
        let open = format!("<{tag}>");
        let close = format!("</{tag}>");
        let start = xml.find(&open)? + open.len();
        let end = xml[start..].find(&close)?;
        let val = xml[start..start + end].trim();
        (!val.is_empty()).then(|| val.to_string())
    }

    let tfm = tag_value(xml, "TargetFramework").or_else(|| {
        tag_value(xml, "TargetFrameworks").and_then(|list| {
            list.split(';')
                .map(str::trim)
                .find(|s| !s.is_empty())
                .map(str::to_string)
        })
    });
    let asm = tag_value(xml, "AssemblyName").unwrap_or_else(|| {
        proj_path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("App")
            .to_string()
    });
    (tfm, asm)
}

/// Compute the Unity Editor Mono Soft Debugger port from the editor's PID.
pub(crate) const fn unity_editor_debug_port(pid: u32) -> u16 {
    56000 + (pid % 1000) as u16
}

/// Parse `Process Id: <pid>` emitted by `dotnet test` when `VSTEST_HOST_DEBUG=1`.
pub(crate) fn parse_vstest_host_pid(output: &str) -> Option<u32> {
    for line in output.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed
            .strip_prefix("Process Id:")
            .or_else(|| trimmed.strip_prefix("Process ID:"))
        {
            let tok = rest
                .trim()
                .split(|c: char| !c.is_ascii_digit())
                .find(|s| !s.is_empty())?;
            if let Ok(pid) = tok.parse::<u32>() {
                if pid > 0 {
                    return Some(pid);
                }
            }
        }
    }
    None
}

/// Check whether `dir` (or an ancestor up to `root`) is a Unity project (`Assets/` + `ProjectSettings/`).
#[allow(clippy::disallowed_methods)]
pub(crate) fn detect_unity_project_root(
    start: &Path,
    root: &Path,
) -> Option<(PathBuf, Option<String>)> {
    let mut cur = Some(if start.is_file() {
        start.parent().unwrap_or(start)
    } else {
        start
    });
    for _ in 0..8 {
        let d = cur?;
        if d.join("Assets").is_dir() && d.join("ProjectSettings").is_dir() {
            let ver = crate::environments::discovery::parse_unity_editor_version(d);
            return Some((d.to_path_buf(), ver));
        }
        if d == root {
            break;
        }
        cur = d.parent();
    }
    None
}

/// Locate the nearest `.csproj` or `.fsproj` walking upward from `program` / `cwd`.
#[allow(clippy::disallowed_methods)]
pub(crate) fn find_local_dotnet_project(program: &str, cwd: &str) -> Option<PathBuf> {
    let prog_path = Path::new(program);
    if prog_path.is_file()
        && prog_path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("csproj") || e.eq_ignore_ascii_case("fsproj"))
    {
        return Some(prog_path.to_path_buf());
    }

    let start_dir = if !program.trim().is_empty() && prog_path.exists() {
        if prog_path.is_dir() {
            prog_path
        } else {
            prog_path.parent().unwrap_or_else(|| Path::new(cwd))
        }
    } else {
        Path::new(cwd)
    };

    let mut cur = Some(start_dir);
    for _ in 0..8 {
        let d = cur?;
        if let Ok(rd) = std::fs::read_dir(d) {
            let mut found: Vec<PathBuf> = rd
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.is_file()
                        && p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                            e.eq_ignore_ascii_case("csproj") || e.eq_ignore_ascii_case("fsproj")
                        })
                })
                .collect();
            found.sort();
            if let Some(first) = found.into_iter().next() {
                return Some(first);
            }
        }
        cur = d.parent();
    }
    None
}

/// Resolve the compiled Debug `.dll` for a `.csproj`/`.fsproj` after `dotnet build -c Debug`.
#[allow(clippy::disallowed_methods)]
pub(crate) fn locate_dotnet_debug_dll(proj_path: &Path) -> Option<PathBuf> {
    let proj_dir = proj_path.parent()?;
    let xml = std::fs::read_to_string(proj_path).unwrap_or_default();
    let (tfm_opt, asm_name) = parse_dotnet_project_meta(proj_path, &xml);
    let dll_name = format!("{asm_name}.dll");

    let bin_debug = proj_dir.join("bin/Debug");
    if let Some(tfm) = tfm_opt {
        let exact = bin_debug.join(&tfm).join(&dll_name);
        if exact.is_file() {
            return Some(exact);
        }
    }
    if let Ok(rd) = std::fs::read_dir(&bin_debug) {
        let mut dirs: Vec<PathBuf> = rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.is_dir())
            .collect();
        dirs.sort();
        for tfm_dir in dirs.into_iter().rev() {
            let cand = tfm_dir.join(&dll_name);
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    let flat = bin_debug.join(&dll_name);
    flat.is_file().then_some(flat)
}

fn netcoredbg_candidates(data_dir: &Path, target_kind: IdeTargetKind) -> Vec<String> {
    let mut cands = Vec::new();
    match target_kind {
        IdeTargetKind::Local => {
            for rel in [
                "toolchains/dotnet/netcoredbg/netcoredbg",
                "toolchains/dotnet/dap/netcoredbg/netcoredbg",
                "toolchains/dotnet/dap/netcoredbg",
                "toolchains/dotnet/bin/netcoredbg",
            ] {
                cands.push(data_dir.join(rel).to_string_lossy().into_owned());
            }
            if !otto_core::e2e::isolated() {
                if let Some(home) = dirs::home_dir() {
                    for rel in [
                        ".otto/toolchains/dotnet/netcoredbg/netcoredbg",
                        ".otto/toolchains/dotnet/dap/netcoredbg/netcoredbg",
                        ".local/bin/netcoredbg",
                    ] {
                        cands.push(home.join(rel).to_string_lossy().into_owned());
                    }
                }
            }
            cands.push("netcoredbg".into());
            cands.push("/opt/homebrew/bin/netcoredbg".into());
            cands.push("/usr/local/bin/netcoredbg".into());
            cands.push("vsdbg".into());
        }
        IdeTargetKind::Ssh => {
            cands.push("$HOME/.otto/toolchains/dotnet/netcoredbg/netcoredbg".into());
            cands.push("$HOME/.otto/toolchains/dotnet/dap/netcoredbg/netcoredbg".into());
            cands.push("netcoredbg".into());
            cands.push("vsdbg".into());
        }
    }
    cands
}

fn dotnet_bin_candidates(
    data_dir: &Path,
    target_kind: IdeTargetKind,
    env: &BTreeMap<String, String>,
) -> Vec<String> {
    let mut cands = Vec::new();
    if let Some(dr) = env.get("DOTNET_ROOT").filter(|s| !s.trim().is_empty()) {
        let base = dr.trim().trim_end_matches('/');
        cands.push(format!("{base}/dotnet"));
        cands.push(format!("{base}/bin/dotnet"));
    }
    match target_kind {
        IdeTargetKind::Local => {
            cands.push(
                data_dir
                    .join("toolchains/dotnet/current/dotnet")
                    .to_string_lossy()
                    .into_owned(),
            );
            cands.push("dotnet".into());
            cands.push("/usr/local/share/dotnet/dotnet".into());
            cands.push("/opt/homebrew/opt/dotnet/libexec/dotnet".into());
            cands.push("/usr/share/dotnet/dotnet".into());
        }
        IdeTargetKind::Ssh => {
            cands.push("$HOME/.otto/toolchains/dotnet/current/dotnet".into());
            cands.push("$HOME/.dotnet/dotnet".into());
            cands.push("dotnet".into());
        }
    }
    cands
}

/// Probe for a running local Unity Editor process (`Unity.app/Contents/MacOS/Unity` on macOS).
#[allow(clippy::disallowed_methods)]
fn discover_running_unity_editor_pid() -> Option<u32> {
    let out = std::process::Command::new("pgrep")
        .args(["-f", "Unity.app/Contents/MacOS/Unity"])
        .stdin(std::process::Stdio::null())
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .find_map(|l| l.trim().parse::<u32>().ok())
}

pub(super) async fn resolve_dotnet_adapter(
    data_dir: &Path,
    language: &str,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    mut env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    env.entry("DOTNET_CLI_TELEMETRY_OPTOUT".into())
        .or_insert_with(|| "1".into());
    env.entry("DOTNET_NOLOGO".into())
        .or_insert_with(|| "1".into());

    let raw_program = req.launch.program.trim();
    let is_unity_attach = raw_program.eq_ignore_ascii_case("unity://attach")
        || req.launch.env.contains_key("UNITY_ATTACH_PID")
        || (req.target.kind == IdeTargetKind::Local
            && req.launch.attach_port.is_some()
            && detect_unity_project_root(Path::new(cwd), Path::new(&req.target.root)).is_some());

    if is_unity_attach {
        return resolve_unity_attach(req, cwd);
    }

    // Check for explicit direct TCP DAP attach port (e.g. remote netcoredbg --server=PORT).
    if let Some(port) = req.launch.attach_port {
        return Ok(ResolvedDapAdapter {
            adapter_name: "netcoredbg".into(),
            language: language.to_string(),
            transport: DapAdapterTransport::DirectTcpAttach {
                host: "127.0.0.1".into(),
                port,
            },
            initialize_args: json!({
                "adapterID": "coreclr",
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "pathFormat": "path",
                "supportsVariableType": true,
                "supportsRunInTerminalRequest": false
            }),
            launch_command: "attach".into(),
            launch_args: json!({
                "name": ".NET Attach",
                "type": "coreclr",
                "request": "attach",
                "justMyCode": false
            }),
            normalized_cwd: cwd.to_string(),
            initial_output: Vec::new(),
        });
    }

    let adapter_cands = netcoredbg_candidates(data_dir, req.target.kind);
    let adapter_bin = locate_binary(&req.target, ctx, &env, &adapter_cands)
        .await
        .ok_or_else(|| {
            Error::Invalid(
                "netcoredbg was not found. Install it from IDE → Toolchains → .NET (DAP) or via `brew install netcoredbg` / Samsung/netcoredbg releases.".into(),
            )
        })?;
    let adapter_name = if adapter_bin.ends_with("vsdbg") {
        "vsdbg"
    } else {
        "netcoredbg"
    };

    // Explicit PID attach via `DOTNET_ATTACH_PID`
    if let Some(pid_str) = req
        .launch
        .env
        .get("DOTNET_ATTACH_PID")
        .or_else(|| env.get("DOTNET_ATTACH_PID"))
    {
        let pid: u32 = pid_str
            .trim()
            .parse()
            .map_err(|_| Error::Invalid(format!("invalid DOTNET_ATTACH_PID '{pid_str}'")))?;
        return Ok(ResolvedDapAdapter {
            adapter_name: adapter_name.into(),
            language: language.to_string(),
            transport: DapAdapterTransport::Stdio {
                command: adapter_bin,
                args: vec!["--interpreter=vscode".into()],
                cwd: cwd.to_string(),
                env,
            },
            initialize_args: dotnet_initialize_args(),
            launch_command: "attach".into(),
            launch_args: json!({
                "name": ".NET Attach",
                "type": "coreclr",
                "request": "attach",
                "processId": pid,
                "justMyCode": false
            }),
            normalized_cwd: cwd.to_string(),
            initial_output: Vec::new(),
        });
    }

    let dotnet_cands = dotnet_bin_candidates(data_dir, req.target.kind, &env);
    let dotnet_bin = locate_binary(&req.target, ctx, &env, &dotnet_cands)
        .await
        .ok_or_else(|| {
            Error::Invalid(
                "dotnet SDK was not found. Install it from IDE → Toolchains → .NET or set DOTNET_ROOT.".into(),
            )
        })?;

    let norm_program = normalize_target_path(&req.target, raw_program);
    let mut initial_output: Vec<IdeDebugOutputEntry> = Vec::new();

    // Test debugging mode (`xUnit` / `NUnit` / `MSTest`)
    if let Some(test_name) = req
        .launch
        .test_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return resolve_dotnet_test_debug(
            language,
            adapter_name,
            adapter_bin,
            dotnet_bin,
            &norm_program,
            test_name,
            req,
            ctx,
            cwd,
            env,
            initial_output,
        )
        .await;
    }

    // Application launch mode: if `program` is already a `.dll`, launch directly;
    // otherwise locate nearest `.csproj`/`.fsproj`, build Debug, and resolve `bin/Debug/<tfm>/<asm>.dll`.
    let target_dll = if norm_program.ends_with(".dll") {
        norm_program
    } else {
        let proj_path = match req.target.kind {
            IdeTargetKind::Local => {
                find_local_dotnet_project(&norm_program, cwd).ok_or_else(|| {
                    Error::Invalid(format!(
                        "no .csproj or .fsproj found near '{}' (cwd '{}')",
                        raw_program, cwd
                    ))
                })?
            }
            IdeTargetKind::Ssh => PathBuf::from(cwd),
        };
        let build_target = proj_path.to_string_lossy().into_owned();
        let build_args = vec![
            "build".to_string(),
            "-c".to_string(),
            "Debug".to_string(),
            build_target.clone(),
        ];
        let build_out = run_target_command(
            &req.target,
            ctx.ssh.as_ref(),
            &dotnet_bin,
            &build_args,
            cwd,
            &env,
            Duration::from_secs(120),
        )
        .await?;
        let combined = format!(
            "{}{}",
            String::from_utf8_lossy(&build_out.stdout),
            String::from_utf8_lossy(&build_out.stderr)
        );
        if !build_out.status.success() {
            return Err(Error::Invalid(format!(
                "dotnet build failed ({}): {}",
                build_out.status,
                combined.trim()
            )));
        }
        if !combined.trim().is_empty() {
            initial_output.push(IdeDebugOutputEntry {
                seq: 0,
                category: "console".into(),
                text: format!("{}\n", combined.trim()),
            });
        }
        match req.target.kind {
            IdeTargetKind::Local => locate_dotnet_debug_dll(&proj_path)
                .ok_or_else(|| {
                    Error::Invalid(format!(
                        "built '{}' but could not locate bin/Debug/<tfm>/<Assembly>.dll",
                        proj_path.display()
                    ))
                })?
                .to_string_lossy()
                .into_owned(),
            IdeTargetKind::Ssh => {
                // On SSH targets, locate the built Debug DLL remotely.
                let find_args = vec![
                    "-lc".into(),
                    "find bin/Debug -maxdepth 2 -name '*.dll' ! -name 'ref*' | head -n 1".into(),
                ];
                let out = run_target_command(
                    &req.target,
                    ctx.ssh.as_ref(),
                    "sh",
                    &find_args,
                    cwd,
                    &env,
                    Duration::from_secs(10),
                )
                .await?;
                let rel = String::from_utf8_lossy(&out.stdout).trim().to_string();
                if rel.is_empty() {
                    return Err(Error::Invalid(
                        "remote dotnet build succeeded but no bin/Debug/<tfm>/*.dll was found"
                            .into(),
                    ));
                }
                if rel.starts_with('/') {
                    rel
                } else {
                    format!("{}/{}", cwd.trim_end_matches('/'), rel)
                }
            }
        }
    };

    let launch_cwd = Path::new(&target_dll)
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.to_string());

    Ok(ResolvedDapAdapter {
        adapter_name: adapter_name.into(),
        language: language.to_string(),
        transport: DapAdapterTransport::Stdio {
            command: adapter_bin,
            args: vec!["--interpreter=vscode".into()],
            cwd: launch_cwd.clone(),
            env: env.clone(),
        },
        initialize_args: dotnet_initialize_args(),
        launch_command: "launch".into(),
        launch_args: json!({
            "name": ".NET Launch",
            "type": "coreclr",
            "request": "launch",
            "program": dotnet_bin,
            "args": std::iter::once(target_dll).chain(req.launch.args.iter().cloned()).collect::<Vec<_>>(),
            "cwd": launch_cwd,
            "env": env,
            "stopAtEntry": false,
            "justMyCode": false,
            "enableStepFiltering": false
        }),
        normalized_cwd: cwd.to_string(),
        initial_output,
    })
}

#[allow(clippy::too_many_arguments)]
async fn resolve_dotnet_test_debug(
    language: &str,
    adapter_name: &str,
    adapter_bin: String,
    dotnet_bin: String,
    norm_program: &str,
    test_name: &str,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    env: BTreeMap<String, String>,
    mut initial_output: Vec<IdeDebugOutputEntry>,
) -> Result<ResolvedDapAdapter> {
    let proj_path = match req.target.kind {
        IdeTargetKind::Local => find_local_dotnet_project(norm_program, cwd)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| cwd.to_string()),
        IdeTargetKind::Ssh => cwd.to_string(),
    };

    let build_out = run_target_command(
        &req.target,
        ctx.ssh.as_ref(),
        &dotnet_bin,
        &[
            "build".into(),
            "-c".into(),
            "Debug".into(),
            proj_path.clone(),
        ],
        cwd,
        &env,
        Duration::from_secs(120),
    )
    .await?;
    if !build_out.status.success() {
        let err_txt = format!(
            "{}{}",
            String::from_utf8_lossy(&build_out.stdout),
            String::from_utf8_lossy(&build_out.stderr)
        );
        return Err(Error::Invalid(format!(
            "dotnet test build failed ({}): {}",
            build_out.status,
            err_txt.trim()
        )));
    }

    let filter_expr = if test_name.contains('=') || test_name.contains('~') {
        test_name.to_string()
    } else {
        format!("FullyQualifiedName~{test_name}")
    };

    if req.target.kind == IdeTargetKind::Local {
        // Use official VSTest debug handshake (`VSTEST_HOST_DEBUG=1`): `dotnet test`
        // pauses `testhost` and prints `Process Id: <PID>`, then exits once `netcoredbg`
        // attaches and the test completes.
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut cmd = tokio::process::Command::new(&dotnet_bin);
        cmd.args([
            "test",
            "--no-build",
            "-c",
            "Debug",
            "--filter",
            &filter_expr,
            &proj_path,
        ])
        .current_dir(cwd)
        .env_clear()
        .envs(&env)
        .env("VSTEST_HOST_DEBUG", "1")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());

        let mut child = cmd
            .spawn()
            .map_err(|e| Error::Internal(format!("spawn dotnet test for debugging: {e}")))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Internal("missing dotnet test stdout".into()))?;
        let mut lines = BufReader::new(stdout).lines();
        let mut host_pid: Option<u32> = None;
        let mut captured = String::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
        while tokio::time::Instant::now() < deadline {
            match tokio::time::timeout(Duration::from_secs(2), lines.next_line()).await {
                Ok(Ok(Some(line))) => {
                    captured.push_str(&line);
                    captured.push('\n');
                    if let Some(pid) = parse_vstest_host_pid(&line) {
                        host_pid = Some(pid);
                        break;
                    }
                }
                Ok(Ok(None)) => break,
                Ok(Err(_)) => break,
                Err(_) => continue,
            }
        }
        if let Some(pid) = host_pid {
            // Reap `dotnet test` in the background once the debugged testhost finishes.
            tokio::spawn(async move {
                let _ = child.wait().await;
            });
            initial_output.push(IdeDebugOutputEntry {
                seq: 0,
                category: "console".into(),
                text: format!(
                    "Attached {adapter_name} to dotnet testhost PID {pid} (filter: {filter_expr})\n"
                ),
            });
            return Ok(ResolvedDapAdapter {
                adapter_name: adapter_name.into(),
                language: language.to_string(),
                transport: DapAdapterTransport::Stdio {
                    command: adapter_bin,
                    args: vec!["--interpreter=vscode".into()],
                    cwd: cwd.to_string(),
                    env,
                },
                initialize_args: dotnet_initialize_args(),
                launch_command: "attach".into(),
                launch_args: json!({
                    "name": format!(".NET Test: {test_name}"),
                    "type": "coreclr",
                    "request": "attach",
                    "processId": pid,
                    "justMyCode": false
                }),
                normalized_cwd: cwd.to_string(),
                initial_output,
            });
        }
        let _ = child.start_kill();
        return Err(Error::Invalid(format!(
            "dotnet test did not emit VSTEST_HOST_DEBUG Process Id for filter '{filter_expr}': {}",
            captured.trim()
        )));
    }

    // Remote SSH fallback: launch `dotnet test --no-build --filter <expr>` directly under netcoredbg.
    Ok(ResolvedDapAdapter {
        adapter_name: adapter_name.into(),
        language: language.to_string(),
        transport: DapAdapterTransport::Stdio {
            command: adapter_bin,
            args: vec!["--interpreter=vscode".into()],
            cwd: cwd.to_string(),
            env: env.clone(),
        },
        initialize_args: dotnet_initialize_args(),
        launch_command: "launch".into(),
        launch_args: json!({
            "name": format!(".NET Test: {test_name}"),
            "type": "coreclr",
            "request": "launch",
            "program": dotnet_bin,
            "args": ["test", "--no-build", "-c", "Debug", "--filter", filter_expr, proj_path],
            "cwd": cwd,
            "env": env,
            "stopAtEntry": false,
            "justMyCode": false
        }),
        normalized_cwd: cwd.to_string(),
        initial_output,
    })
}

fn resolve_unity_attach(req: &IdeStartDebugReq, cwd: &str) -> Result<ResolvedDapAdapter> {
    let unity_info = detect_unity_project_root(Path::new(cwd), Path::new(&req.target.root));
    let ver_label = unity_info
        .as_ref()
        .and_then(|(_, v)| v.as_deref())
        .unwrap_or("unknown");

    let explicit_pid = req
        .launch
        .env
        .get("UNITY_ATTACH_PID")
        .and_then(|s| s.trim().parse::<u32>().ok());
    let pid = explicit_pid.or_else(discover_running_unity_editor_pid);
    let port = req
        .launch
        .attach_port
        .or_else(|| pid.map(unity_editor_debug_port));

    let Some(target_port) = port else {
        return Err(Error::Invalid(format!(
            "Unity Editor is not running (detected project Unity {ver_label}). Launch Unity Editor with Script Debugging enabled, or specify an attach port (56000 + PID % 1000) / UNITY_ATTACH_PID."
        )));
    };

    Ok(ResolvedDapAdapter {
        adapter_name: "unity-attach".into(),
        language: "csharp".into(),
        transport: DapAdapterTransport::DirectTcpAttach {
            host: "127.0.0.1".into(),
            port: target_port,
        },
        initialize_args: json!({
            "adapterID": "unity",
            "linesStartAt1": true,
            "columnsStartAt1": true,
            "pathFormat": "path"
        }),
        launch_command: "attach".into(),
        launch_args: json!({
            "name": format!("Unity Editor ({ver_label})"),
            "type": "vstuc",
            "request": "attach",
            "endPoint": format!("127.0.0.1:{target_port}"),
            "projectPath": unity_info
                .as_ref()
                .map(|(p, _)| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| cwd.to_string())
        }),
        normalized_cwd: cwd.to_string(),
        initial_output: vec![IdeDebugOutputEntry {
            seq: 0,
            category: "console".into(),
            text: format!(
                "Attaching to Unity Editor ({ver_label}) on 127.0.0.1:{target_port}{}\n",
                pid.map(|p| format!(" (PID {p})")).unwrap_or_default()
            ),
        }],
    })
}

fn dotnet_initialize_args() -> serde_json::Value {
    json!({
        "adapterID": "coreclr",
        "clientID": "otto-ide",
        "clientName": "Otto IDE",
        "linesStartAt1": true,
        "columnsStartAt1": true,
        "pathFormat": "path",
        "supportsVariableType": true,
        "supportsVariablePaging": false,
        "supportsRunInTerminalRequest": false
    })
}

#[cfg(test)]
mod dotnet_support_tests {
    use super::*;

    #[test]
    fn parses_csproj_meta_and_vstest_pid_and_unity_port() {
        let xml = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <PropertyGroup>
    <OutputType>Exe</OutputType>
    <TargetFramework>net9.0</TargetFramework>
    <AssemblyName>HelloWorld</AssemblyName>
  </PropertyGroup>
</Project>"#;
        let (tfm, asm) = parse_dotnet_project_meta(Path::new("/ws/App.csproj"), xml);
        assert_eq!(tfm.as_deref(), Some("net9.0"));
        assert_eq!(asm, "HelloWorld");

        let multi = r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFrameworks>net8.0;net9.0</TargetFrameworks></PropertyGroup></Project>"#;
        let (tfm2, asm2) = parse_dotnet_project_meta(Path::new("/ws/Lib.fsproj"), multi);
        assert_eq!(tfm2.as_deref(), Some("net8.0"));
        assert_eq!(asm2, "Lib");

        assert_eq!(
            parse_vstest_host_pid(
                "Host debugging is enabled. Please attach debugger to testhost process to continue.\nProcess Id: 48123, Name: dotnet\n"
            ),
            Some(48123)
        );
        assert_eq!(unity_editor_debug_port(12345), 56345);
    }

    #[test]
    fn locates_local_dotnet_project_and_debug_dll_and_unity_root() {
        let tmp = tempfile::tempdir().unwrap();
        let app = tmp.path().join("src/App");
        std::fs::create_dir_all(app.join("bin/Debug/net9.0")).unwrap();
        let csproj = app.join("App.csproj");
        std::fs::write(
            &csproj,
            "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net9.0</TargetFramework></PropertyGroup></Project>",
        )
        .unwrap();
        let prog = app.join("Program.cs");
        std::fs::write(&prog, "Console.WriteLine(42);\n").unwrap();
        let dll = app.join("bin/Debug/net9.0/App.dll");
        std::fs::write(&dll, "").unwrap();

        let found = find_local_dotnet_project(&prog.to_string_lossy(), &app.to_string_lossy())
            .expect("find csproj");
        assert_eq!(found, csproj);
        assert_eq!(locate_dotnet_debug_dll(&csproj), Some(dll));

        // Unity project detection
        let unity = tmp.path().join("MyGame");
        std::fs::create_dir_all(unity.join("Assets/Scripts")).unwrap();
        std::fs::create_dir_all(unity.join("ProjectSettings")).unwrap();
        std::fs::write(
            unity.join("ProjectSettings/ProjectVersion.txt"),
            "m_EditorVersion: 6000.0.23f1\n",
        )
        .unwrap();
        let (uroot, uver) =
            detect_unity_project_root(&unity.join("Assets/Scripts"), &unity).expect("detect unity");
        assert_eq!(uroot, unity);
        assert_eq!(uver.as_deref(), Some("6000.0.23f1"));
    }

    #[test]
    fn dotnet_gutter_and_test_parser_cover_xunit_nunit_mstest_and_fsharp() {
        let cs_tests = r#"
namespace Demo.Tests;
public class MathTests
{
    [Fact]
    public void AddsNumbers() {}

    [Theory]
    [InlineData(1, 2)]
    public void MultipliesNumbers(int a, int b) {}

    [Test]
    public void NUnitWorks() {}

    [TestMethod]
    public void MSTestWorks() {}
}
"#;
        let items = crate::gutter::detect_gutter_items("/ws/MathTests.cs", cs_tests, &[]);
        let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
        assert!(labels.contains(&"MathTests (all tests)"), "{labels:?}");
        assert!(labels.contains(&"AddsNumbers"), "{labels:?}");
        assert!(labels.contains(&"MultipliesNumbers"), "{labels:?}");
        assert!(labels.contains(&"NUnitWorks"), "{labels:?}");
        assert!(labels.contains(&"MSTestWorks"), "{labels:?}");

        let top_level = "using System;\nConsole.WriteLine(\"hi\");\n";
        let main_items = crate::gutter::detect_gutter_items("/ws/Program.cs", top_level, &[]);
        assert_eq!(main_items.len(), 1);
        assert_eq!(main_items[0].label, "Program (top-level)");

        let fs_src = "[<EntryPoint>]\nlet main argv = 0\n\n[<Fact>]\nlet ``adds values`` () = ()\n";
        let fs_items = crate::gutter::detect_gutter_items("/ws/Program.fs", fs_src, &[]);
        assert_eq!(fs_items.len(), 2);

        let parsed = crate::environments::parse_test_output(
            crate::types::IdeRunnerKind::DotnetTest,
            &[
                "  Passed Demo.Tests.MathTests.AddsNumbers [12 ms]".into(),
                "  Failed Demo.Tests.MathTests.Divides [< 1 ms]".into(),
                "  Error Message:".into(),
                "   Assert.Equal() Failure: Values differ".into(),
                "  Stack Trace:".into(),
                "     at Demo.Tests.MathTests.Divides() in /ws/MathTests.cs:line 24".into(),
            ],
        );
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].name, "AddsNumbers");
        assert_eq!(parsed[0].suite.as_deref(), Some("Demo.Tests.MathTests"));
        assert_eq!(parsed[0].duration_ms, Some(12));
        assert_eq!(parsed[1].name, "Divides");
        assert_eq!(parsed[1].file.as_deref(), Some("/ws/MathTests.cs"));
        assert_eq!(parsed[1].line, Some(24));
        assert!(parsed[1]
            .message
            .as_deref()
            .is_some_and(|m| m.contains("Assert.Equal")));
    }

    #[test]
    fn dotnet_nuget_parser_extracts_csproj_and_central_and_packages_config() {
        let csproj = r#"
<Project Sdk="Microsoft.NET.Sdk">
  <ItemGroup>
    <PackageReference Include="Newtonsoft.Json" Version="13.0.3" />
    <PackageReference Include="xunit">
      <Version>2.9.2</Version>
    </PackageReference>
    <PackageVersion Include="Serilog" Version="4.2.0" />
  </ItemGroup>
</Project>
"#;
        let deps = crate::deps_inspector::parse_dotnet_deps(csproj);
        assert_eq!(deps.len(), 3);
        assert_eq!(deps[0].name, "Newtonsoft.Json");
        assert_eq!(deps[0].current_version, "13.0.3");
        assert_eq!(deps[1].name, "xunit");
        assert_eq!(deps[1].current_version, "2.9.2");
        assert_eq!(deps[2].name, "Serilog");
        assert_eq!(deps[2].current_version, "4.2.0");
    }
}
