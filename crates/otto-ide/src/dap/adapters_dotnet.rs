//! .NET (`netcoredbg` / `vsdbg`) DAP adapter resolver.
//!
//! Supports Local and Remote SSH targets transparently for C# and F#:
//! - **Application launch:** locates nearest `.csproj`/`.fsproj`, builds Debug
//!   artifacts (`dotnet build -c Debug`), resolves `TargetPath` / `bin/Debug/<tfm>/<Assembly>.dll`,
//!   and launches `netcoredbg --interpreter=vscode` over stdio.
//! - **Unit test debugging (xUnit / NUnit / MSTest):** builds Debug test project,
//!   launches `dotnet test --no-build --filter FullyQualifiedName~<name>` with
//!   `VSTEST_HOST_DEBUG=1` to obtain the paused `testhost` PID, and attaches
//!   `netcoredbg` directly to that PID.
//! - **Process attach:** supports explicit PID (`launch.attach_pid` or `DOTNET_ATTACH_PID`)
//!   for local/SSH `netcoredbg` attach, and direct TCP DAP attach when `attach_port` is provided.

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

#[path = "adapters_dotnet_steps.rs"]
mod steps;
pub(crate) use steps::plan_dotnet_test_steps;

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
#[cfg(test)]
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
    if raw_program.eq_ignore_ascii_case("unity://attach") {
        return Err(Error::Invalid(
            "Unity Editor attach is not supported (Unity uses the Mono Soft Debugger protocol rather than DAP)."
                .into(),
        ));
    }

    // Check for explicit direct TCP DAP attach port (e.g. remote netcoredbg --server=PORT).
    // netcoredbg requires a target `processId` on `attach` requests (API-6).
    if let Some(port) = req.launch.attach_port {
        let pid = extract_dotnet_attach_pid(req, &env, true).ok_or_else(|| {
            Error::Invalid(
                "Direct TCP .NET attach requires a target process ID (set attach_pid, DOTNET_ATTACH_PID, or pass --pid <PID>)."
                    .into(),
            )
        })?;
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
                "processId": pid,
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

    // Explicit local PID attach via `launch.attach_pid` or `DOTNET_ATTACH_PID` only —
    // never interpret an app's own `--pid` / `-p` argv flags as attach mode (BUG3-7).
    if let Some(pid) = extract_dotnet_attach_pid(req, &env, false) {
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

    let norm_program = resolve_dotnet_program_path(&req.target, raw_program, cwd);
    let mut initial_output: Vec<IdeDebugOutputEntry> = Vec::new();

    // Test debugging mode (`xUnit` / `NUnit` / `MSTest`) — covers both filtered single-test
    // gutter actions and unfiltered whole-project `.NET Test` runs (BUG-25).
    // A Microsoft.Testing.Platform Debug All step launches its built test app.
    let mtp_step = req.launch.debug_step && norm_program.ends_with(".dll");
    if let Some(opt_test_name) =
        detect_dotnet_test_debug_intent(req, ctx, &norm_program, cwd).filter(|_| !mtp_step)
    {
        return resolve_dotnet_test_debug(
            language,
            adapter_name,
            adapter_bin,
            dotnet_bin,
            &norm_program,
            opt_test_name.as_deref(),
            req,
            ctx,
            cwd,
            env,
            initial_output,
        )
        .await;
    }

    // Application launch mode: if `program` is already a `.dll`, launch directly;
    // otherwise locate nearest `.csproj`/`.fsproj`, build Debug, and resolve TargetPath / Debug DLL.
    let mut built_proj_path: Option<PathBuf> = None;
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
            IdeTargetKind::Ssh => select_ssh_dotnet_build_target(&norm_program, cwd),
        };
        built_proj_path = Some(proj_path.clone());
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
        let msbuild_target = query_msbuild_target_path(
            &req.target,
            ctx.ssh.as_ref(),
            &dotnet_bin,
            &build_target,
            cwd,
            &env,
        )
        .await;
        match req.target.kind {
            IdeTargetKind::Local => msbuild_target
                .filter(|p| Path::new(p).is_file())
                .or_else(|| {
                    locate_dotnet_debug_dll(&proj_path).map(|p| p.to_string_lossy().into_owned())
                })
                .ok_or_else(|| {
                    Error::Invalid(format!(
                        "built '{}' but could not locate Debug output DLL",
                        proj_path.display()
                    ))
                })?,
            IdeTargetKind::Ssh => {
                if let Some(tp) = msbuild_target {
                    tp
                } else {
                    let search_dir = proj_path
                        .parent()
                        .filter(|_| {
                            proj_path
                                .extension()
                                .and_then(|e| e.to_str())
                                .is_some_and(|e| {
                                    e.eq_ignore_ascii_case("csproj")
                                        || e.eq_ignore_ascii_case("fsproj")
                                })
                        })
                        .map(|p| p.to_string_lossy().into_owned())
                        .unwrap_or_else(|| cwd.to_string());
                    let find_args = vec![
                        "-lc".into(),
                        "find bin/Debug -maxdepth 2 -name '*.dll' ! -name 'ref*' | head -n 1"
                            .into(),
                    ];
                    let out = run_target_command(
                        &req.target,
                        ctx.ssh.as_ref(),
                        "sh",
                        &find_args,
                        &search_dir,
                        &env,
                        Duration::from_secs(10),
                    )
                    .await?;
                    let rel = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if rel.is_empty() {
                        return Err(Error::Invalid(format!(
                            "remote dotnet build of '{build_target}' succeeded but no TargetPath or bin/Debug/<tfm>/*.dll was found"
                        )));
                    }
                    if rel.starts_with('/') {
                        rel
                    } else {
                        format!("{}/{}", search_dir.trim_end_matches('/'), rel)
                    }
                }
            }
        }
    };

    // BUG2-10: Preserve the project / launch cwd instead of deriving cwd from TargetPath
    // (which breaks .NET 8+ Artifacts output layouts like `repo/artifacts/bin/App/debug/App.dll`).
    let launch_cwd = compute_dotnet_launch_cwd(built_proj_path.as_deref(), cwd);

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
    test_name: Option<&str>,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    env: BTreeMap<String, String>,
    initial_output: Vec<IdeDebugOutputEntry>,
) -> Result<ResolvedDapAdapter> {
    // A Debug All step's project was built by the run's plan.
    if req.launch.debug_step {
        return attach_vstest_host(
            language,
            adapter_name,
            adapter_bin,
            dotnet_bin,
            (norm_program, test_name),
            req,
            ctx,
            cwd,
            env,
            initial_output,
        )
        .await;
    }
    let proj_path = match req.target.kind {
        IdeTargetKind::Local => select_local_dotnet_test_project(norm_program, cwd)?,
        IdeTargetKind::Ssh => {
            select_ssh_dotnet_test_project(req, ctx, norm_program, cwd, &env).await?
        }
    }
    .map(|p| p.to_string_lossy().into_owned())
    .unwrap_or_else(|| cwd.to_string());

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
    attach_vstest_host(
        language,
        adapter_name,
        adapter_bin,
        dotnet_bin,
        (&proj_path, test_name),
        req,
        ctx,
        cwd,
        env,
        initial_output,
    )
    .await
}

/// Run `dotnet test` on the built `proj_path` with `VSTEST_HOST_DEBUG=1` and attach
/// to the one test host it pauses (`-f` keeps a multi-targeted project to one host).
#[allow(clippy::too_many_arguments)]
async fn attach_vstest_host(
    language: &str,
    adapter_name: &str,
    adapter_bin: String,
    dotnet_bin: String,
    (proj_path, test_name): (&str, Option<&str>),
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    env: BTreeMap<String, String>,
    mut initial_output: Vec<IdeDebugOutputEntry>,
) -> Result<ResolvedDapAdapter> {
    let filter_expr = test_name.map(|t| {
        if t.contains('=') || t.contains('~') {
            t.to_string()
        } else {
            format!("FullyQualifiedName~{t}")
        }
    });
    let mut test_args = vec![
        "test".to_string(),
        "--no-build".to_string(),
        "-c".to_string(),
        "Debug".to_string(),
    ];
    if let Some(ref f) = filter_expr {
        test_args.push("--filter".into());
        test_args.push(f.clone());
    }
    if let Some(tfm) = req
        .launch
        .test_framework
        .as_deref()
        .filter(|f| !f.is_empty())
    {
        test_args.push("-f".into());
        test_args.push(tfm.to_string());
    }
    // Per-test `Passed` / `Failed` lines for the console and the step result.
    test_args.push("--logger".into());
    test_args.push("console;verbosity=normal".into());
    test_args.push(proj_path.to_string());
    let label = test_name.map(str::to_string).unwrap_or_else(|| {
        let proj = Path::new(proj_path);
        let name = proj
            .file_name()
            .unwrap_or(proj.as_os_str())
            .to_string_lossy();
        format!("all tests in {name}")
    });

    {
        // Use official VSTest debug handshake (`VSTEST_HOST_DEBUG=1`) on both targets:
        // `dotnet test` pauses `testhost` and prints `Process Id: <PID>`, then exits once
        // `netcoredbg` attaches and the test completes (debugging `dotnet test` itself never
        // reaches the tests). Bind its process group to `BridgeGuard` so Stop or adapter init
        // failure always cleans up `testhost` (OBS-1).
        use tokio::io::{AsyncBufReadExt, BufReader};
        let mut test_env = env.clone();
        test_env.insert("VSTEST_HOST_DEBUG".into(), "1".into());
        let mut child = super::spawn_target_process(
            &req.target,
            ctx.ssh.as_ref(),
            &dotnet_bin,
            &test_args,
            cwd,
            &test_env,
        )
        .await?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Internal("missing dotnet test stdout".into()))?;
        if let Some(stderr) = child.stderr.take() {
            crate::line_reader::spawn_line_drain(
                stderr,
                crate::line_reader::MAX_OUTPUT_LINE_BYTES,
                |_| {},
            );
        }
        // OBS-1: Transfer child + its process group to `BridgeGuard` BEFORE awaiting
        // `Process Id:` so cancelling/stopping the resolver mid-wait kills both
        // `dotnet test` and any spawned `testhost` process immediately on drop.
        let companion = super::BridgeGuard::new();
        companion.push_child(child);
        let mut lines = BufReader::new(stdout).lines();
        let mut host_pid: Option<u32> = None;
        let mut captured = String::new();
        // A cold test host on a large solution can take longer than 20 s.
        let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
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
            // `dotnet test`'s own output (per-test lines, the summary) goes to the
            // console; reading it to the end also keeps its pipe from filling up.
            let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
            companion.set_output(rx);
            tokio::spawn(async move {
                while let Ok(Some(line)) = lines.next_line().await {
                    let _ = tx.send(line);
                }
            });
            if !captured.trim().is_empty() {
                initial_output.push(IdeDebugOutputEntry {
                    seq: 0,
                    category: "stdout".into(),
                    text: captured.clone(),
                });
            }
            initial_output.push(IdeDebugOutputEntry {
                seq: 0,
                category: "console".into(),
                text: format!(
                    "VSTest host paused at PID {pid}; attaching {adapter_name} ({})\n",
                    filter_expr
                        .as_deref()
                        .map(|f| format!("filter: {f}"))
                        .unwrap_or_else(|| "all tests".into())
                ),
            });
            return Ok(ResolvedDapAdapter {
                adapter_name: adapter_name.into(),
                language: language.to_string(),
                transport: DapAdapterTransport::StdioWithCompanion {
                    command: adapter_bin,
                    args: vec!["--interpreter=vscode".into()],
                    cwd: cwd.to_string(),
                    env,
                    companion,
                },
                initialize_args: dotnet_initialize_args(),
                launch_command: "attach".into(),
                launch_args: json!({
                    "name": format!(".NET Test: {label}"),
                    "type": "coreclr",
                    "request": "attach",
                    "processId": pid,
                    "justMyCode": false
                }),
                normalized_cwd: cwd.to_string(),
                initial_output,
            });
        }
        companion.kill_all();
        Err(Error::Invalid(format!(
            "dotnet test did not emit VSTEST_HOST_DEBUG Process Id for '{label}': {}",
            captured.trim()
        )))
    }
}

/// Compute launch working directory for a `.NET` app session, preserving the project
/// directory (or requested `cwd`) even when MSBuild `TargetPath` points into a `.NET 8+`
/// Artifacts output layout (BUG2-10).
pub(crate) fn compute_dotnet_launch_cwd(built_proj_path: Option<&Path>, cwd: &str) -> String {
    if let Some(proj) = built_proj_path {
        if proj
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("csproj") || e.eq_ignore_ascii_case("fsproj"))
        {
            if let Some(parent) = proj.parent().filter(|p| !p.as_os_str().is_empty()) {
                return parent.to_string_lossy().into_owned();
            }
        }
    }
    cwd.to_string()
}

fn is_dotnet_test_project_with_props(proj: &Path, stop_root: &Path) -> bool {
    let xml = std::fs::read_to_string(proj).unwrap_or_default();
    if xml.contains("<IsTestProject>false</IsTestProject>") {
        return false;
    }
    if xml.contains("Microsoft.NET.Test.Sdk") || xml.contains("<IsTestProject>true</IsTestProject>")
    {
        return true;
    }
    let has_test_framework =
        xml.contains("xunit") || xml.contains("NUnit") || xml.contains("MSTest");
    let mut cur = proj.parent();
    for _ in 0..5 {
        let Some(dir) = cur else { break };
        let props = dir.join("Directory.Build.props");
        if props.is_file() {
            let pxml = std::fs::read_to_string(&props).unwrap_or_default();
            if pxml.contains("Microsoft.NET.Test.Sdk")
                || pxml.contains("<IsTestProject>true</IsTestProject>")
            {
                return has_test_framework
                    || pxml.contains("xunit")
                    || pxml.contains("NUnit")
                    || pxml.contains("MSTest");
            }
        }
        if dir == stop_root {
            break;
        }
        cur = dir.parent();
    }
    false
}

/// Test projects a `.sln` (`"path\\App.Tests.csproj"`) or `.slnx` (`Path="..."`) references.
#[allow(clippy::disallowed_methods)] // one small solution file read at debug session start
fn dotnet_solution_test_projects(sln: &Path) -> Vec<PathBuf> {
    let dir = sln.parent().unwrap_or(Path::new(""));
    std::fs::read_to_string(sln)
        .unwrap_or_default()
        .split('"')
        .filter(|t| {
            let low = t.to_ascii_lowercase();
            low.ends_with(".csproj") || low.ends_with(".fsproj")
        })
        .map(|t| dir.join(t.replace('\\', "/")))
        .filter(|p| is_dotnet_test_project_with_props(p, dir))
        .collect()
}

/// The test projects a `.NET` "Debug Tests" of `norm_program` covers.
#[derive(Debug, Default)]
pub(crate) struct DotnetTestSelection {
    /// Test projects, sorted (an explicitly requested project is the one entry).
    pub(crate) tests: Vec<PathBuf>,
    /// The `.sln`/`.slnx` asked for, if any.
    pub(crate) solution: Option<PathBuf>,
    /// What a single session targets when no test project was found.
    pub(crate) fallback: Option<PathBuf>,
    /// Where the scan started (names are shown relative to it).
    pub(crate) root: PathBuf,
}

/// Select the local `.csproj`/`.fsproj` (or `.sln`) targets for `.NET` test debugging:
/// - An explicitly requested `.csproj`/`.fsproj` is used as-is, even when its
///   `IsTestProject`/`Microsoft.NET.Test.Sdk` comes from `Directory.Build.props` (BUG3-8).
/// - A `.sln`/`.slnx` gives the test projects it references.
/// - A directory/repo root is scanned (depth <= 3) for test projects.
#[allow(clippy::disallowed_methods)] // bounded depth-3 scan at debug session start
pub(crate) fn select_local_dotnet_test_projects(
    norm_program: &str,
    cwd: &str,
) -> DotnetTestSelection {
    let prog_path = Path::new(norm_program);
    let ext = prog_path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    let is_file = !norm_program.is_empty() && prog_path.is_file();
    if is_file && matches!(ext.as_str(), "csproj" | "fsproj") {
        return DotnetTestSelection {
            tests: vec![prog_path.to_path_buf()],
            ..DotnetTestSelection::default()
        };
    }
    if is_file && matches!(ext.as_str(), "sln" | "slnx") {
        return DotnetTestSelection {
            tests: dotnet_solution_test_projects(prog_path),
            solution: Some(prog_path.to_path_buf()),
            fallback: Some(prog_path.to_path_buf()),
            root: prog_path.parent().unwrap_or(Path::new("")).to_path_buf(),
        };
    }
    if is_file {
        if let Some(direct) = find_local_dotnet_project(norm_program, cwd) {
            return DotnetTestSelection {
                tests: vec![direct],
                ..DotnetTestSelection::default()
            };
        }
    }
    let root = if !norm_program.is_empty() && prog_path.is_dir() {
        prog_path.to_path_buf()
    } else {
        PathBuf::from(cwd)
    };
    let mut test_projects: Vec<PathBuf> = Vec::new();
    let mut sln_fallback: Option<PathBuf> = None;
    let mut stack = vec![(root.clone(), 0usize)];
    while let Some((dir, depth)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<PathBuf> = rd.flatten().map(|e| e.path()).collect();
        entries.sort();
        for p in entries {
            let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
                continue;
            };
            if p.is_file() {
                let low = name.to_ascii_lowercase();
                if (low.ends_with(".csproj") || low.ends_with(".fsproj"))
                    && is_dotnet_test_project_with_props(&p, &root)
                {
                    test_projects.push(p);
                } else if sln_fallback.is_none()
                    && (low.ends_with(".sln") || low.ends_with(".slnx"))
                {
                    sln_fallback = Some(p);
                }
            } else if p.is_dir()
                && depth < 3
                && !matches!(
                    name,
                    ".git" | "bin" | "obj" | "node_modules" | "artifacts" | ".vs" | "packages"
                )
            {
                stack.push((p, depth + 1));
            }
        }
    }
    test_projects.sort();
    DotnetTestSelection {
        tests: test_projects,
        solution: None,
        fallback: sln_fallback.or_else(|| find_local_dotnet_project(norm_program, cwd)),
        root,
    }
}

/// The one target of a single `.NET` test session. A folder or solution holding
/// several test projects is refused with their names: an unfiltered run plans
/// one Debug All step per project instead, and one filtered `dotnet test`
/// over several projects would pause one host per project (BUG-25).
pub(crate) fn select_local_dotnet_test_project(
    norm_program: &str,
    cwd: &str,
) -> Result<Option<PathBuf>> {
    single_dotnet_test_target(select_local_dotnet_test_projects(norm_program, cwd))
}

fn single_dotnet_test_target(mut sel: DotnetTestSelection) -> Result<Option<PathBuf>> {
    match sel.tests.len() {
        0 => Ok(sel.fallback),
        1 if sel.solution.is_some() => Ok(sel.solution),
        1 => Ok(sel.tests.pop()),
        _ => Err(multiple_dotnet_test_projects_err(&sel.tests, &sel.root)),
    }
}

fn multiple_dotnet_test_projects_err(tests: &[PathBuf], root: &Path) -> Error {
    let names = tests
        .iter()
        .map(|p| p.strip_prefix(root).unwrap_or(p).display().to_string())
        .collect::<Vec<_>>()
        .join(", ");
    Error::Invalid(format!(
        "Found {} .NET test projects ({names}). A debug session attaches to one test host: debug one test from the editor gutter, or debug a .NET Test run configuration whose target is one of these projects (add one in Launch settings if none is listed).",
        tests.len()
    ))
}

/// Prints the SSH target's `.csproj`/`.fsproj`/`.sln`/`.slnx`/`Directory.Build.props` files that
/// [`select_local_dotnet_test_project`] reads for `$1` (program) / `$2` (cwd): `D`/`F`/`N`
/// (program kind), then per file `\x1e<path>\n<content>`.
const SSH_DOTNET_TEST_SNAPSHOT_SH: &str = r#"p=$1; c=$2; s=
if [ -d "$p" ]; then echo D; s=$p
elif [ -f "$p" ]; then echo F; case "$p" in *.sln|*.SLN|*.slnx|*.SLNX) s=$(dirname -- "$p");; esac
else echo N; fi
{
find "$c" ${s:+"$s"} -maxdepth 6 \( -name .git -o -name bin -o -name obj -o -name node_modules -o -name artifacts -o -name .vs -o -name packages \) -prune -o -type f \( -iname '*.csproj' -o -iname '*.fsproj' -o -iname '*.sln' -o -iname '*.slnx' -o -name Directory.Build.props \) -print 2>/dev/null | head -n 500
d=$(dirname -- "$p"); i=0
while [ "$i" -lt 8 ]; do
  for f in "$d"/*.csproj "$d"/*.fsproj "$d"/Directory.Build.props; do [ -f "$f" ] && printf '%s\n' "$f"; done
  [ "$d" = / ] && break
  d=$(dirname -- "$d"); i=$((i+1))
done
} | sort -u | while IFS= read -r f; do printf '\036%s\n' "$f"; head -c 65536 -- "$f"; printf '\n'; done
"#;

/// SSH twin of [`select_local_dotnet_test_project`] (BUG-25, BUG3-8): the files it reads are
/// copied from the target into a local mirror and the same selection runs there, so an SSH
/// "Debug Tests" picks (or refuses) exactly what a local one does.
async fn select_ssh_dotnet_test_project(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    norm_program: &str,
    cwd: &str,
    env: &BTreeMap<String, String>,
) -> Result<Option<PathBuf>> {
    single_dotnet_test_target(
        select_ssh_dotnet_test_projects(req, ctx, norm_program, cwd, env).await?,
    )
}

/// SSH twin of [`select_local_dotnet_test_projects`], on a read-only local mirror.
pub(crate) async fn select_ssh_dotnet_test_projects(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    norm_program: &str,
    cwd: &str,
    env: &BTreeMap<String, String>,
) -> Result<DotnetTestSelection> {
    let low = norm_program.to_ascii_lowercase();
    if low.ends_with(".csproj") || low.ends_with(".fsproj") {
        return Ok(DotnetTestSelection {
            tests: vec![PathBuf::from(norm_program)],
            ..DotnetTestSelection::default()
        });
    }
    let program = if norm_program.is_empty() {
        cwd
    } else {
        norm_program
    };
    let args = ["-c", SSH_DOTNET_TEST_SNAPSHOT_SH, "sh", program, cwd].map(String::from);
    let out = run_target_command(
        &req.target,
        ctx.ssh.as_ref(),
        "sh",
        &args,
        cwd,
        env,
        Duration::from_secs(20),
    )
    .await?;
    let mirror = tempfile::tempdir()
        .map_err(|e| Error::Internal(format!("dotnet test project mirror: {e}")))?;
    select_dotnet_test_projects_from_snapshot(
        mirror.path(),
        &String::from_utf8_lossy(&out.stdout),
        program,
        cwd,
    )
}

/// Write an [`SSH_DOTNET_TEST_SNAPSHOT_SH`] snapshot under `mirror`, run the local selection
/// on it and map the picks back to remote paths. Paths with `..` never leave the mirror.
#[allow(clippy::disallowed_methods)] // small temp mirror written at debug session start
fn select_dotnet_test_projects_from_snapshot(
    mirror: &Path,
    snapshot: &str,
    program: &str,
    cwd: &str,
) -> Result<DotnetTestSelection> {
    let local = |remote: &str| {
        let rel = Path::new(remote).strip_prefix("/").ok()?;
        rel.components()
            .all(|c| matches!(c, std::path::Component::Normal(_)))
            .then(|| mirror.join(rel))
    };
    let io = |e: std::io::Error| Error::Internal(format!("dotnet test project mirror: {e}"));
    let mut parts = snapshot.split('\u{1e}');
    let kind = parts.next().unwrap_or_default().trim();
    for chunk in parts {
        let (path, body) = chunk.split_once('\n').unwrap_or((chunk, ""));
        if let Some(dst) = local(path) {
            std::fs::create_dir_all(dst.parent().unwrap_or(mirror)).map_err(io)?;
            std::fs::write(&dst, body).map_err(io)?;
        }
    }
    let (Some(prog), Some(cwd)) = (local(program), local(cwd)) else {
        return Ok(DotnetTestSelection::default());
    };
    std::fs::create_dir_all(&cwd).map_err(io)?;
    if kind == "D" {
        std::fs::create_dir_all(&prog).map_err(io)?;
    } else if kind == "F" && !prog.exists() {
        std::fs::create_dir_all(prog.parent().unwrap_or(mirror)).map_err(io)?;
        std::fs::write(&prog, "").map_err(io)?;
    }
    let sel = select_local_dotnet_test_projects(&prog.to_string_lossy(), &cwd.to_string_lossy());
    let remote = |p: PathBuf| Path::new("/").join(p.strip_prefix(mirror).unwrap_or(&p));
    Ok(DotnetTestSelection {
        tests: sel.tests.into_iter().map(remote).collect(),
        solution: sel.solution.map(remote),
        fallback: sel.fallback.map(remote),
        root: remote(sel.root),
    })
}

#[cfg(test)]
fn select_dotnet_test_project_from_snapshot(
    mirror: &Path,
    snapshot: &str,
    program: &str,
    cwd: &str,
) -> Result<Option<PathBuf>> {
    single_dotnet_test_target(select_dotnet_test_projects_from_snapshot(
        mirror, snapshot, program, cwd,
    )?)
}

/// Extract an explicit `.NET` attach PID from `launch.attach_pid` or `DOTNET_ATTACH_PID`
/// (and `--pid <n>` / `-p <n>` argv aliases ONLY when `allow_argv_pid_flags` is true for
/// explicit TCP attach mode so app flags are never misread as attach mode, BUG3-7).
pub(crate) fn extract_dotnet_attach_pid(
    req: &IdeStartDebugReq,
    env: &BTreeMap<String, String>,
    allow_argv_pid_flags: bool,
) -> Option<u32> {
    if let Some(pid) = req.launch.attach_pid.filter(|&p| p > 0) {
        return Some(pid);
    }
    if let Some(pid) = req
        .launch
        .env
        .get("DOTNET_ATTACH_PID")
        .or_else(|| env.get("DOTNET_ATTACH_PID"))
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&p| p > 0)
    {
        return Some(pid);
    }
    if !allow_argv_pid_flags {
        return None;
    }
    let mut i = 0;
    while i < req.launch.args.len() {
        let a = req.launch.args[i].trim();
        if let Some(rest) = a.strip_prefix("--pid=") {
            if let Ok(pid) = rest.trim().parse::<u32>() {
                if pid > 0 {
                    return Some(pid);
                }
            }
        } else if (a == "--pid" || a == "-p") && i + 1 < req.launch.args.len() {
            if let Ok(pid) = req.launch.args[i + 1].trim().parse::<u32>() {
                if pid > 0 {
                    return Some(pid);
                }
            }
        }
        i += 1;
    }
    None
}

/// Resolve a `.NET` launch program path against `cwd` first (for attached-root configs)
/// before falling back to `normalize_target_path` (BUG-24).
pub(crate) fn resolve_dotnet_program_path(
    target: &crate::types::IdeTargetSpec,
    raw_program: &str,
    cwd: &str,
) -> String {
    let trimmed = raw_program.trim();
    if trimmed.is_empty() {
        return String::new();
    }
    if !trimmed.starts_with('/') && !cwd.trim().is_empty() {
        let joined = format!(
            "{}/{}",
            cwd.trim_end_matches('/'),
            trimmed.trim_start_matches("./")
        );
        if target.kind == IdeTargetKind::Ssh || Path::new(&joined).exists() {
            return joined;
        }
    }
    normalize_target_path(target, trimmed)
}

/// Choose the `.csproj`/`.fsproj`/`.sln` target to build on SSH when a nested project is selected (BUG-24).
pub(crate) fn select_ssh_dotnet_build_target(norm_program: &str, cwd: &str) -> PathBuf {
    let low = norm_program.to_ascii_lowercase();
    if low.ends_with(".csproj")
        || low.ends_with(".fsproj")
        || low.ends_with(".sln")
        || low.ends_with(".slnx")
    {
        return PathBuf::from(norm_program);
    }
    PathBuf::from(cwd)
}

/// Detect whether a `.NET` debug request targets unit tests (`Some(opt_filter)`)
/// vs an application launch (`None`), covering whole-project unfiltered test runs (BUG-25).
pub(crate) fn detect_dotnet_test_debug_intent(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    norm_program: &str,
    cwd: &str,
) -> Option<Option<String>> {
    if let Some(test_name) = req
        .launch
        .test_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        return Some(Some(test_name.to_string()));
    }
    if req.launch.is_test
        || ctx
            .run_config
            .as_ref()
            .is_some_and(|c| c.runner == crate::types::IdeRunnerKind::DotnetTest)
        || req
            .launch
            .env
            .get("OTTO_DOTNET_TEST_DEBUG")
            .map(String::as_str)
            == Some("1")
    {
        return Some(None);
    }
    if req.target.kind == IdeTargetKind::Local && !norm_program.ends_with(".dll") {
        if let Some(proj) = find_local_dotnet_project(norm_program, cwd) {
            if is_dotnet_test_project_with_props(&proj, Path::new(&req.target.root)) {
                return Some(None);
            }
        }
    }
    None
}

async fn query_msbuild_target_path(
    target: &crate::types::IdeTargetSpec,
    ssh: Option<&crate::types::IdeSshSpec>,
    dotnet_bin: &str,
    build_target: &str,
    cwd: &str,
    env: &BTreeMap<String, String>,
) -> Option<String> {
    let low = build_target.to_ascii_lowercase();
    if !low.ends_with(".csproj") && !low.ends_with(".fsproj") {
        return None;
    }
    let args = vec![
        "msbuild".into(),
        "-nologo".into(),
        "-v:q".into(),
        "-p:Configuration=Debug".into(),
        "-getProperty:TargetPath".into(),
        build_target.to_string(),
    ];
    let out = run_target_command(
        target,
        ssh,
        dotnet_bin,
        &args,
        cwd,
        env,
        Duration::from_secs(15),
    )
    .await
    .ok()?;
    if !out.status.success() {
        return None;
    }
    parse_msbuild_target_path(&String::from_utf8_lossy(&out.stdout))
}

pub(crate) fn parse_msbuild_target_path(stdout: &str) -> Option<String> {
    stdout
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty() && l.to_ascii_lowercase().ends_with(".dll"))
        .map(str::to_string)
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

    #[tokio::test]
    async fn dotnet_review_fixes_unity_attach_pid_test_intent_and_ssh_nested_project() {
        use std::os::unix::fs::PermissionsExt;
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        let data_dir = tmp.path().join("data");
        let target = crate::types::IdeTargetSpec::local(root.to_string_lossy().into_owned());
        let ctx = DapStartContext {
            caller_is_root: true,
            ..Default::default()
        };

        // API-4: Unity attach is removed/rejected with clear Mono Soft Debugger message.
        let unity_req = IdeStartDebugReq {
            target: target.clone(),
            language: "csharp".into(),
            config_id: None,
            launch: crate::types::IdeDebugLaunchSpec {
                program: "unity://attach".into(),
                attach_port: Some(56123),
                ..Default::default()
            },
            ..Default::default()
        };
        let err = resolve_dotnet_adapter(
            &data_dir,
            "csharp",
            &unity_req,
            &ctx,
            &root.to_string_lossy(),
            BTreeMap::new(),
        )
        .await
        .expect_err("unity://attach must error");
        assert!(err.to_string().contains("Mono Soft Debugger"));

        // API-6: Direct TCP attach uses launch.attach_pid (or DOTNET_ATTACH_PID / --pid)
        // and emits `processId` in the real DAP `attach` envelope, failing when missing.
        let mut req_tcp = unity_req.clone();
        req_tcp.launch.program = "App.csproj".into();
        req_tcp.launch.attach_port = Some(4711);
        assert!(resolve_dotnet_adapter(
            &data_dir,
            "csharp",
            &req_tcp,
            &ctx,
            &root.to_string_lossy(),
            BTreeMap::new(),
        )
        .await
        .is_err());
        req_tcp.launch.attach_pid = Some(4242);
        let tcp_res = resolve_dotnet_adapter(
            &data_dir,
            "csharp",
            &req_tcp,
            &ctx,
            &root.to_string_lossy(),
            BTreeMap::new(),
        )
        .await
        .expect("direct TCP attach with launch.attach_pid");
        assert_eq!(tcp_res.launch_command, "attach");
        assert_eq!(tcp_res.launch_args["processId"], json!(4242));

        // BUG-25: Whole-project Debug Tests from a solution root (`is_test: true`) selects
        // nested `tests/App.Tests/App.Tests.csproj` and routes to VSTest debug intent.
        let sln_root = root.join("MultiProjSln");
        let app_dir = sln_root.join("src/App");
        let test_dir = sln_root.join("tests/App.Tests");
        std::fs::create_dir_all(&app_dir).unwrap();
        std::fs::create_dir_all(&test_dir).unwrap();
        std::fs::write(
            app_dir.join("App.csproj"),
            r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><OutputType>Exe</OutputType></PropertyGroup></Project>"#,
        )
        .unwrap();
        let test_csproj = test_dir.join("App.Tests.csproj");
        std::fs::write(
            &test_csproj,
            r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><PackageReference Include="Microsoft.NET.Test.Sdk" Version="17.11.0" /></ItemGroup></Project>"#,
        )
        .unwrap();
        let mut repo_test_req = IdeStartDebugReq {
            target: crate::types::IdeTargetSpec::local(sln_root.to_string_lossy().into_owned()),
            language: "csharp".into(),
            launch: crate::types::IdeDebugLaunchSpec {
                program: sln_root.to_string_lossy().into_owned(),
                cwd: Some(sln_root.to_string_lossy().into_owned()),
                is_test: true,
                ..Default::default()
            },
            ..Default::default()
        };
        assert_eq!(
            detect_dotnet_test_debug_intent(
                &repo_test_req,
                &ctx,
                &sln_root.to_string_lossy(),
                &sln_root.to_string_lossy()
            ),
            Some(None)
        );
        assert_eq!(
            select_local_dotnet_test_project(
                &sln_root.to_string_lossy(),
                &sln_root.to_string_lossy(),
            )
            .unwrap(),
            Some(test_csproj.clone())
        );

        // BUG3-8: An explicitly requested test .csproj whose `Microsoft.NET.Test.Sdk`
        // comes from `Directory.Build.props` (while only listing xUnit locally) MUST be
        // honored as-is and NEVER replaced by another test project in the repo.
        let props_test_dir = sln_root.join("tests/PropsXunit.Tests");
        std::fs::create_dir_all(&props_test_dir).unwrap();
        std::fs::write(
            sln_root.join("Directory.Build.props"),
            r#"<Project><PropertyGroup><IsTestProject>true</IsTestProject></PropertyGroup><ItemGroup><PackageReference Include="Microsoft.NET.Test.Sdk" Version="17.11.0" /></ItemGroup></Project>"#,
        )
        .unwrap();
        let props_csproj = props_test_dir.join("PropsXunit.Tests.csproj");
        std::fs::write(
            &props_csproj,
            r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><PackageReference Include="xunit" Version="2.9.0" /></ItemGroup></Project>"#,
        )
        .unwrap();
        assert_eq!(
            select_local_dotnet_test_project(
                &props_csproj.to_string_lossy(),
                &sln_root.to_string_lossy(),
            )
            .unwrap(),
            Some(props_csproj.clone()),
            "BUG3-8: explicitly requested test .csproj must be preserved verbatim"
        );

        // BUG-25: Unfiltered Debug All Tests on a repo root with MULTIPLE test projects must
        // return an actionable error instead of silently running only the first test project.
        let multi_err = select_local_dotnet_test_project(
            &sln_root.to_string_lossy(),
            &sln_root.to_string_lossy(),
        )
        .expect_err("BUG-25: multiple test projects for unfiltered Debug All Tests must error");
        let multi_msg = multi_err.to_string();
        assert!(
            multi_msg.contains("App.Tests.csproj") && multi_msg.contains("PropsXunit.Tests.csproj"),
            "BUG-25: error should name all discovered test projects: {multi_msg}"
        );

        // BUG-25: an explicit solution referencing both test projects is refused the same way
        // (VSTest pauses one test host per project); one referencing a single test project
        // is used as-is.
        let both_sln = sln_root.join("All.sln");
        std::fs::write(
            &both_sln,
            "Project(\"{FAE04EC0}\") = \"App.Tests\", \"tests\\App.Tests\\App.Tests.csproj\", \"{1}\"\nProject(\"{FAE04EC0}\") = \"PropsXunit.Tests\", \"tests\\PropsXunit.Tests\\PropsXunit.Tests.csproj\", \"{2}\"\n",
        )
        .unwrap();
        let sln_err = select_local_dotnet_test_project(
            &both_sln.to_string_lossy(),
            &sln_root.to_string_lossy(),
        )
        .expect_err("BUG-25: a solution with two test projects must be refused");
        assert!(
            sln_err.to_string().contains("PropsXunit.Tests.csproj"),
            "{sln_err}"
        );
        let one_slnx = sln_root.join("One.slnx");
        std::fs::write(
            &one_slnx,
            r#"<Solution><Project Path="src/App/App.csproj" /><Project Path="tests/App.Tests/App.Tests.csproj" /></Solution>"#,
        )
        .unwrap();
        assert_eq!(
            select_local_dotnet_test_project(
                &one_slnx.to_string_lossy(),
                &sln_root.to_string_lossy(),
            )
            .unwrap(),
            Some(one_slnx.clone())
        );
        std::fs::remove_file(&both_sln).unwrap();
        std::fs::remove_file(&one_slnx).unwrap();

        repo_test_req.launch.is_test = false;
        repo_test_req.launch.program = test_csproj.to_string_lossy().into_owned();
        assert_eq!(
            detect_dotnet_test_debug_intent(
                &repo_test_req,
                &ctx,
                &test_csproj.to_string_lossy(),
                &test_dir.to_string_lossy()
            ),
            Some(None)
        );

        // BUG2-10 + BUG3-7: MSBuild TargetPath pointing to .NET 8+ Artifacts output layout
        // (`repo/artifacts/bin/App/debug/App.dll`) must keep launch cwd at `src/App`, AND
        // passing `--pid 9999` in normal app args must NOT switch Launch into Attach mode!
        let artifacts_dll = sln_root.join("artifacts/bin/App/debug/App.dll");
        std::fs::create_dir_all(artifacts_dll.parent().unwrap()).unwrap();
        std::fs::write(&artifacts_dll, "fake-dll").unwrap();
        let fake_nc = data_dir.join("toolchains/dotnet/netcoredbg/netcoredbg");
        let fake_dotnet = data_dir.join("toolchains/dotnet/current/dotnet");
        for p in [&fake_nc, &fake_dotnet] {
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        }
        std::fs::write(&fake_nc, "#!/bin/sh\nexit 0\n").unwrap();
        std::fs::write(
            &fake_dotnet,
            format!(
                "#!/bin/sh\nif [ \"$1\" = \"msbuild\" ]; then echo '{}'; exit 0; fi\nexit 0\n",
                artifacts_dll.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&fake_nc, std::fs::Permissions::from_mode(0o755)).unwrap();
        std::fs::set_permissions(&fake_dotnet, std::fs::Permissions::from_mode(0o755)).unwrap();
        let app_req = IdeStartDebugReq {
            target: crate::types::IdeTargetSpec::local(sln_root.to_string_lossy().into_owned()),
            language: "csharp".into(),
            launch: crate::types::IdeDebugLaunchSpec {
                program: app_dir.join("App.csproj").to_string_lossy().into_owned(),
                args: vec!["--pid".into(), "9999".into(), "-p".into(), "8888".into()],
                cwd: Some(app_dir.to_string_lossy().into_owned()),
                ..Default::default()
            },
            ..Default::default()
        };
        let app_res = resolve_dotnet_adapter(
            &data_dir,
            "csharp",
            &app_req,
            &ctx,
            &app_dir.to_string_lossy(),
            BTreeMap::new(),
        )
        .await
        .expect("artifacts TargetPath launch with --pid app args");
        assert_eq!(
            app_res.launch_command, "launch",
            "BUG3-7: app --pid/-p args without attach_port/attach_pid must stay in launch mode"
        );
        let launch_args_arr = app_res.launch_args["args"].as_array().unwrap();
        assert!(
            launch_args_arr.contains(&json!("--pid")) && launch_args_arr.contains(&json!("9999")),
            "BUG3-7: app --pid 9999 args must be preserved for launch: {launch_args_arr:?}"
        );
        let expected_app_cwd = std::fs::canonicalize(&app_dir).unwrap_or_else(|_| app_dir.clone());
        assert_eq!(
            app_res.launch_args["cwd"].as_str(),
            Some(expected_app_cwd.to_string_lossy().as_ref()),
            "BUG2-10: launch cwd must stay at project dir, not artifacts dir: {}",
            app_res.launch_args
        );

        // OBS-1: Cancelling the REAL `resolve_dotnet_adapter` future while it is awaiting
        // `Process Id:` from `dotnet test` drops the resolver's `BridgeGuard` and kills
        // the entire process group (both `dotnet test` and its spawned child `testhost`).
        #[cfg(unix)]
        {
            let pid_file = root.join("resolver_testhost.pid");
            let hang_dotnet = data_dir.join("toolchains/dotnet/current/dotnet");
            std::fs::write(
                &hang_dotnet,
                format!(
                    "#!/bin/sh\nif [ \"$1\" = \"build\" ]; then exit 0; fi\nif [ \"$1\" = \"test\" ]; then\n  sleep 30 >/dev/null 2>&1 & echo $! > '{}'\n  sleep 30\n  exit 0\nfi\nexit 0\n",
                    pid_file.display()
                ),
            )
            .unwrap();
            std::fs::set_permissions(&hang_dotnet, std::fs::Permissions::from_mode(0o755)).unwrap();

            let obs_req = IdeStartDebugReq {
                target: crate::types::IdeTargetSpec::local(sln_root.to_string_lossy().into_owned()),
                language: "csharp".into(),
                launch: crate::types::IdeDebugLaunchSpec {
                    program: test_csproj.to_string_lossy().into_owned(),
                    cwd: Some(test_dir.to_string_lossy().into_owned()),
                    is_test: true,
                    ..Default::default()
                },
                ..Default::default()
            };
            let data_dir_cl = data_dir.clone();
            let ctx_cl = ctx.clone();
            let cwd_cl = test_dir.to_string_lossy().into_owned();
            let resolver_task = tokio::spawn(async move {
                resolve_dotnet_adapter(
                    &data_dir_cl,
                    "csharp",
                    &obs_req,
                    &ctx_cl,
                    &cwd_cl,
                    BTreeMap::new(),
                )
                .await
            });
            for _ in 0..80 {
                if pid_file.is_file()
                    && !std::fs::read_to_string(&pid_file)
                        .unwrap_or_default()
                        .trim()
                        .is_empty()
                {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            let child_pid: i32 = std::fs::read_to_string(&pid_file)
                .expect("resolver should have spawned dotnet test + child testhost")
                .trim()
                .parse()
                .unwrap();
            resolver_task.abort();
            let _ = resolver_task.await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            extern "C" {
                fn kill(pid: std::ffi::c_int, sig: std::ffi::c_int) -> std::ffi::c_int;
            }
            #[allow(unsafe_code)]
            let alive = unsafe { kill(child_pid, 0) == 0 };
            assert!(
                !alive,
                "OBS-1: cancelling real resolve_dotnet_adapter mid-PID-wait must kill child testhost {child_pid}"
            );
        }

        // BUG-24: Nested SSH project path & MSBuild TargetPath parsing
        let ssh_target = crate::types::IdeTargetSpec {
            kind: IdeTargetKind::Ssh,
            workspace_id: None,
            root: "/srv/repo".into(),
            connection_id: Some("ssh1".into()),
            additional_roots: Vec::new(),
            project_folders: Vec::new(),
        };
        let resolved_ssh =
            resolve_dotnet_program_path(&ssh_target, "src/App/App.csproj", "/srv/repo");
        assert_eq!(resolved_ssh, "/srv/repo/src/App/App.csproj");
        assert_eq!(
            select_ssh_dotnet_build_target(&resolved_ssh, "/srv/repo"),
            PathBuf::from("/srv/repo/src/App/App.csproj")
        );
        assert_eq!(
            parse_msbuild_target_path("\n/srv/repo/artifacts/bin/App/Debug/App.dll\n").as_deref(),
            Some("/srv/repo/artifacts/bin/App/Debug/App.dll")
        );

        // BUG-21 & API-4: Fresh Unreal (no .sln/.csproj) & Unity emit no broken auto-configs
        let ureal = root.join("UnrealOnly");
        std::fs::create_dir_all(&ureal).unwrap();
        std::fs::write(ureal.join("Game.uproject"), "{}").unwrap();
        let ureal_cfgs = crate::environments::discover_run_configs(
            &crate::types::IdeTargetSpec::local(ureal.to_string_lossy().into_owned()),
            &[],
        );
        assert!(
            !ureal_cfgs.iter().any(|c| c.name.contains("Build Rules")),
            "Unreal without .sln/.csproj must not emit bare dotnet build: {ureal_cfgs:?}"
        );
        let unity_dir = root.join("UnityOnly");
        std::fs::create_dir_all(unity_dir.join("Assets")).unwrap();
        std::fs::create_dir_all(unity_dir.join("ProjectSettings")).unwrap();
        let unity_cfgs = crate::environments::discover_run_configs(
            &crate::types::IdeTargetSpec::local(unity_dir.to_string_lossy().into_owned()),
            &[],
        );
        assert!(
            !unity_cfgs
                .iter()
                .any(|c| c.target_path.as_deref() == Some("unity://attach")),
            "Unity must not advertise broken Mono SDB DAP attach: {unity_cfgs:?}"
        );
    }

    /// BUG-25 / BUG3-8 (SSH): the snapshot script runs on the target (here `sh` on a temp
    /// layout) and the local selection runs on its mirror, so SSH Debug Tests selects or refuses
    /// exactly like a local one; a snapshot path never escapes the mirror.
    #[test]
    #[allow(clippy::disallowed_methods)] // runs the snapshot script synchronously in a unit test
    fn ssh_dotnet_test_project_selection_reads_the_target() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        let write = |rel: &str, body: &str| {
            let p = repo.join(rel);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, body).unwrap();
        };
        let test_sdk = r#"<Project Sdk="Microsoft.NET.Sdk"><ItemGroup><PackageReference Include="Microsoft.NET.Test.Sdk" /></ItemGroup></Project>"#;
        write("tests/A.Tests/A.Tests.csproj", test_sdk);
        write("tests/B.Tests/B.Tests.csproj", test_sdk);
        write(
            "src/App/App.csproj",
            r#"<Project Sdk="Microsoft.NET.Sdk" />"#,
        );
        write(
            "All.sln",
            "Project(\"{FAE04EC0}\") = \"A.Tests\", \"tests\\A.Tests\\A.Tests.csproj\", \"{1}\"\nProject(\"{FAE04EC0}\") = \"B.Tests\", \"tests\\B.Tests\\B.Tests.csproj\", \"{2}\"\n",
        );
        write(
            "tests/A.Tests/Unit/FooTests.cs",
            "public class FooTests {}\n",
        );
        let select = |program: &Path| {
            let out = std::process::Command::new("sh")
                .args(["-c", SSH_DOTNET_TEST_SNAPSHOT_SH, "sh"])
                .arg(program)
                .arg(&repo)
                .output()
                .unwrap();
            let mirror = tempfile::tempdir().unwrap();
            select_dotnet_test_project_from_snapshot(
                mirror.path(),
                &String::from_utf8_lossy(&out.stdout),
                &program.to_string_lossy(),
                &repo.to_string_lossy(),
            )
        };

        // Repo-level Debug Tests with two test projects is refused, naming both and saying what to do.
        let err = select(&repo).unwrap_err().to_string();
        assert!(
            err.contains("tests/A.Tests/A.Tests.csproj")
                && err.contains("tests/B.Tests/B.Tests.csproj"),
            "{err}"
        );
        assert!(
            err.contains("editor gutter") && err.contains("Launch settings"),
            "{err}"
        );
        // So is an explicit solution referencing both.
        let err = select(&repo.join("All.sln")).unwrap_err().to_string();
        assert!(err.contains("B.Tests.csproj"), "{err}");
        // A gutter test in a nested folder resolves to its own project on the target.
        assert_eq!(
            select(&repo.join("tests/A.Tests/Unit/FooTests.cs")).unwrap(),
            Some(repo.join("tests/A.Tests/A.Tests.csproj"))
        );
        // One test project's folder resolves to that project.
        assert_eq!(
            select(&repo.join("tests/B.Tests")).unwrap(),
            Some(repo.join("tests/B.Tests/B.Tests.csproj"))
        );

        // A snapshot path climbing out of the mirror is never written.
        let mirror = tempfile::tempdir().unwrap();
        let snapshot = "N\n\u{1e}/../otto-mirror-escape.csproj\n<Project />\n";
        let _ = select_dotnet_test_project_from_snapshot(mirror.path(), snapshot, "/srv", "/srv");
        assert!(!mirror
            .path()
            .parent()
            .unwrap()
            .join("otto-mirror-escape.csproj")
            .exists());
    }
}
