//! C / C++ and Unreal Engine (`codelldb`, `lldb-dap`, and built-in `lldb -P`
//! `SBDebugger` Python DAP bridge) DAP adapter resolver.
//!
//! Supports:
//! - Automatic build-before-debug for CMake (`CMakeLists.txt`), Meson (`meson.build`),
//!   Make (`Makefile`), and standalone `.c`/`.cpp` files (cached under
//!   `<data_dir>/ide-build-cache/cpp/`).
//! - GoogleTest (`--gtest_filter=...`) and Catch2 test filtering.
//! - Live process attach by PID (`attach_pid`), process name (`attach_process_name`,
//!   e.g. `UnrealEditor`), GDB-remote port (`attach_port`), and core-dump inspection
//!   (`core_file`).
//! - LLDB Python type summaries so C++ STL containers (`std::vector`, `std::map`,
//!   `std::unordered_map`, `std::optional`) and Unreal Engine types (`FString`,
//!   `TArray`, `FVector`) display their values inline without hiding synthetic children.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use otto_core::{Error, Result};
use serde_json::json;
use sha2::{Digest, Sha256};

use super::adapters::{
    locate_binary, normalize_target_path, DapAdapterTransport, ResolvedDapAdapter,
};
use super::adapters_env::canonical_target_roots;
use super::adapters_rust_sh::locate_lldb_python_bridge;
use super::lldb_bridge::LLDB_PYTHON_DAP_BRIDGE;
use super::{run_target_command, DapStartContext};
use crate::environments::discovery_cpp::parse_cmake_executables;
use crate::types::{
    IdeDebugOutputEntry, IdeRunnerKind, IdeStartDebugReq, IdeTargetKind, IdeTargetSpec,
};

/// LLDB Python type summary providers for C++ STL containers (libc++ & libstdc++)
/// and Unreal Engine core types (`FString`, `TArray`, `FVector`).
const CPP_LLDB_FORMATTERS_PY: &str = r#"# Otto IDE C/C++ & Unreal Engine LLDB pretty-printers (auto-generated)
import lldb

def _short_val(v):
    if not v or not v.IsValid():
        return '?'
    s = v.GetSummary()
    if s:
        return s
    val = v.GetValue()
    if val is not None:
        return val
    nc = v.GetNumChildren(6)
    if nc > 0:
        parts = []
        for i in range(min(nc, 3)):
            c = v.GetChildAtIndex(i)
            cn = c.GetName() or str(i)
            cv = c.GetSummary() or c.GetValue() or '{...}'
            parts.append(f'{cn}={cv}')
        return '{' + ', '.join(parts) + (', ...' if nc > 3 else '') + '}'
    return '{...}'

def cpp_vector_summary(valobj, internal_dict):
    try:
        n = valobj.GetNumChildren(16)
        if n == 0:
            return '[] (size=0)'
        shown = min(n, 8)
        elems = [_short_val(valobj.GetChildAtIndex(i)) for i in range(shown)]
        more = ', ...' if n > shown else ''
        return f'[{", ".join(elems)}{more}] (size={n})'
    except Exception:
        return ''

def cpp_map_summary(valobj, internal_dict):
    try:
        n = valobj.GetNumChildren(16)
        if n == 0:
            return '{} (size=0)'
        shown = min(n, 6)
        pairs = []
        for i in range(shown):
            entry = valobj.GetChildAtIndex(i)
            first = entry.GetChildMemberWithName('first')
            second = entry.GetChildMemberWithName('second')
            if first and first.IsValid() and second and second.IsValid():
                pairs.append(f'{_short_val(first)}: {_short_val(second)}')
            else:
                pairs.append(_short_val(entry))
        more = ', ...' if n > shown else ''
        return '{' + ', '.join(pairs) + more + f'}} (size={n})'
    except Exception:
        return ''

def cpp_optional_summary(valobj, internal_dict):
    try:
        if valobj.GetNumChildren(2) == 0:
            return 'std::nullopt'
        val = valobj.GetChildMemberWithName('Value') or valobj.GetChildAtIndex(0)
        if val and val.IsValid():
            return _short_val(val)
        return 'std::nullopt'
    except Exception:
        return ''

def ue_fstring_summary(valobj, internal_dict):
    try:
        data = valobj.GetChildMemberWithName('Data')
        if not data or not data.IsValid():
            return '""'
        num = data.GetChildMemberWithName('ArrayNum')
        n = num.GetValueAsUnsigned(0) if num and num.IsValid() else 0
        if n <= 1:
            return '""'
        alloc = data.GetChildMemberWithName('AllocatorInstance')
        ptr = alloc.GetChildMemberWithName('Data') if alloc and alloc.IsValid() else None
        addr = ptr.GetValueAsUnsigned(0) if ptr and ptr.IsValid() else 0
        proc = valobj.GetProcess()
        if addr and proc and proc.IsValid():
            err = lldb.SBError()
            raw = proc.ReadMemory(addr, min(n - 1, 256) * 2, err)
            if err.Success() and raw:
                return '"' + raw.decode('utf-16-le', 'replace') + '"'
        return f'FString(len={n - 1})'
    except Exception:
        return ''

def ue_tarray_summary(valobj, internal_dict):
    try:
        num = valobj.GetChildMemberWithName('ArrayNum')
        mx = valobj.GetChildMemberWithName('ArrayMax')
        n = num.GetValueAsUnsigned(0) if num and num.IsValid() else 0
        cap = mx.GetValueAsUnsigned(0) if mx and mx.IsValid() else 0
        return f'TArray(num={n}, max={cap})'
    except Exception:
        return ''

def ue_fvector_summary(valobj, internal_dict):
    try:
        x = valobj.GetChildMemberWithName('X')
        y = valobj.GetChildMemberWithName('Y')
        z = valobj.GetChildMemberWithName('Z')
        if x and y and z and x.IsValid() and y.IsValid() and z.IsValid():
            return f'({x.GetValue()}, {y.GetValue()}, {z.GetValue()})'
        return ''
    except Exception:
        return ''

def __lldb_init_module(debugger, internal_dict):
    debugger.HandleCommand('type summary add -x "^std::(__1::|__cxx11::)?vector<.*>$" -F cpp_lldb_formatters.cpp_vector_summary -w otto_cpp')
    debugger.HandleCommand('type summary add -x "^std::(__1::|__cxx11::)?(unordered_)?map<.*>$" -F cpp_lldb_formatters.cpp_map_summary -w otto_cpp')
    debugger.HandleCommand('type summary add -x "^std::(__1::)?optional<.*>$" -F cpp_lldb_formatters.cpp_optional_summary -w otto_cpp')
    debugger.HandleCommand('type summary add "^FString$" -F cpp_lldb_formatters.ue_fstring_summary -w otto_cpp')
    debugger.HandleCommand('type summary add -x "^TArray<.*>$" -F cpp_lldb_formatters.ue_tarray_summary -w otto_cpp')
    debugger.HandleCommand('type summary add -x "^(FVector|UE::Math::TVector<.*>)$" -F cpp_lldb_formatters.ue_fvector_summary -w otto_cpp')
    debugger.HandleCommand('type category enable otto_cpp')
"#;

#[allow(clippy::disallowed_methods)]
fn ensure_cpp_formatters_script(data_dir: &Path) -> Option<PathBuf> {
    let dir = data_dir.join("ide-build-cache").join("cpp");
    std::fs::create_dir_all(&dir).ok()?;
    let script_path = dir.join("cpp_lldb_formatters.py");
    let up_to_date = std::fs::read_to_string(&script_path)
        .map(|s| s == CPP_LLDB_FORMATTERS_PY)
        .unwrap_or(false);
    if !up_to_date {
        std::fs::write(&script_path, CPP_LLDB_FORMATTERS_PY).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o600));
        }
    }
    Some(script_path)
}

fn is_cpp_source_file(path: &str) -> bool {
    let lower = path.to_ascii_lowercase();
    lower.ends_with(".c")
        || lower.ends_with(".cpp")
        || lower.ends_with(".cc")
        || lower.ends_with(".cxx")
        || lower.ends_with(".c++")
        || lower.ends_with(".h")
        || lower.ends_with(".hpp")
        || lower.ends_with(".hh")
        || lower.ends_with(".hxx")
        || lower.ends_with(".inl")
}

pub(super) async fn resolve_cpp_adapter(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    let mut initial_output = Vec::new();
    let raw_prog = normalize_target_path(&req.target, &req.launch.program);
    let attach_pid = req.launch.attach_pid;
    let attach_process_name = req
        .launch
        .attach_process_name
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from);
    let core_file = req
        .launch
        .core_file
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|cf| normalize_target_path(&req.target, cf));
    let is_attach_mode = req.launch.attach_port.is_some()
        || attach_pid.is_some()
        || attach_process_name.is_some()
        || core_file.is_some();

    let want_test = req.launch.test_name.is_some()
        || ctx
            .run_config
            .as_ref()
            .is_some_and(|rc| rc.runner == IdeRunnerKind::Ctest);

    let is_prebuilt_binary = !raw_prog.is_empty()
        && !is_cpp_source_file(&raw_prog)
        && !raw_prog.ends_with("CMakeLists.txt")
        && !raw_prog.ends_with("meson.build")
        && !raw_prog.ends_with(".uproject")
        && (req.target.kind == IdeTargetKind::Ssh || Path::new(&raw_prog).is_file());
    let executable = if is_attach_mode || is_prebuilt_binary {
        raw_prog
    } else {
        compile_cpp_debug_artifact(
            data_dir,
            req,
            ctx,
            cwd,
            &raw_prog,
            want_test,
            &env,
            &mut initial_output,
        )
        .await?
    };

    let mut prog_args = Vec::new();
    if let Some(tname) = req.launch.test_name.as_deref().filter(|s| !s.is_empty()) {
        // GoogleTest uses `Suite.Test` / `--gtest_filter=...`; Catch2 accepts the test name directly.
        if tname.contains('.')
            && !req
                .launch
                .args
                .iter()
                .any(|a| a.starts_with("--gtest_filter"))
        {
            prog_args.push(format!("--gtest_filter={tname}"));
        } else if !req.launch.args.iter().any(|a| a == tname) {
            prog_args.push(tname.to_string());
        }
    }
    prog_args.extend(req.launch.args.clone());

    let init_commands: Vec<String> = if req.target.kind == IdeTargetKind::Local {
        ensure_cpp_formatters_script(data_dir)
            .map(|p| {
                let escaped = p.to_string_lossy().replace('\'', "'\\''");
                vec![format!("command script import '{escaped}'")]
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    // 1. Prefer CodeLLDB if installed.
    let mut codelldb_candidates = Vec::new();
    if req.target.kind == IdeTargetKind::Local {
        for rel in [
            "toolchains/cpp/dap/extension/adapter/codelldb",
            "toolchains/rust/dap/extension/adapter/codelldb",
            "toolchains/zig/dap/extension/adapter/codelldb",
            "toolchains/rust/codelldb/adapter/codelldb",
            "toolchains/codelldb/adapter/codelldb",
        ] {
            codelldb_candidates.push(data_dir.join(rel).to_string_lossy().into_owned());
        }
    } else {
        codelldb_candidates
            .push("$HOME/.otto/toolchains/cpp/dap/extension/adapter/codelldb".into());
        codelldb_candidates
            .push("$HOME/.otto/toolchains/rust/dap/extension/adapter/codelldb".into());
    }
    codelldb_candidates.push("codelldb".into());

    if let Some(codelldb_bin) = locate_binary(&req.target, ctx, &env, &codelldb_candidates).await {
        let (launch_command, launch_args) = if let Some(core) = core_file.as_deref() {
            let mut cmds = init_commands.clone();
            cmds.push(format!(
                "target create -c '{}'",
                core.replace('\'', "'\\''")
            ));
            (
                "attach".to_string(),
                json!({
                    "program": executable,
                    "cwd": cwd,
                    "initCommands": cmds
                }),
            )
        } else if let Some(pid) = attach_pid {
            (
                "attach".to_string(),
                json!({
                    "pid": pid,
                    "program": executable,
                    "cwd": cwd,
                    "initCommands": init_commands
                }),
            )
        } else if let Some(pname) = attach_process_name.as_deref() {
            (
                "attach".to_string(),
                json!({
                    "program": pname,
                    "cwd": cwd,
                    "initCommands": init_commands
                }),
            )
        } else if let Some(port) = req.launch.attach_port {
            (
                "attach".to_string(),
                json!({
                    "gdbRemotePort": port,
                    "cwd": cwd,
                    "initCommands": init_commands
                }),
            )
        } else {
            (
                "launch".to_string(),
                json!({
                    "program": executable,
                    "args": prog_args,
                    "cwd": cwd,
                    "env": env,
                    "stopOnEntry": false,
                    "initCommands": init_commands
                }),
            )
        };
        return Ok(ResolvedDapAdapter {
            adapter_name: "codelldb".into(),
            language: "cpp".into(),
            transport: DapAdapterTransport::TcpListen {
                command: codelldb_bin,
                args: vec!["--port".into(), "0".into()],
                cwd: cwd.to_string(),
                env,
            },
            initialize_args: json!({
                "adapterID": "lldb",
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "pathFormat": "path"
            }),
            launch_command,
            launch_args,
            normalized_cwd: cwd.to_string(),
            initial_output,
        });
    }

    // 2. Native LLVM / Xcode `lldb-dap`.
    let lldb_dap_candidates = vec![
        "lldb-dap".to_string(),
        "lldb-vscode".to_string(),
        "/Library/Developer/CommandLineTools/usr/bin/lldb-dap".to_string(),
        "/Applications/Xcode.app/Contents/Developer/usr/bin/lldb-dap".to_string(),
        "/opt/homebrew/opt/llvm/bin/lldb-dap".to_string(),
        "/usr/bin/lldb-dap".to_string(),
    ];
    if let Some(lldb_bin) = locate_binary(&req.target, ctx, &env, &lldb_dap_candidates).await {
        let (launch_command, launch_args) = if let Some(core) = core_file.as_deref() {
            (
                "attach".to_string(),
                json!({
                    "program": executable,
                    "coreFile": core,
                    "cwd": cwd,
                    "initCommands": init_commands
                }),
            )
        } else if let Some(pid) = attach_pid {
            let mut obj = json!({
                "pid": pid,
                "cwd": cwd,
                "initCommands": init_commands
            });
            if !executable.is_empty() {
                obj["program"] = json!(executable);
            }
            ("attach".to_string(), obj)
        } else if let Some(pname) = attach_process_name.as_deref() {
            (
                "attach".to_string(),
                json!({
                    "program": pname,
                    "waitFor": false,
                    "cwd": cwd,
                    "initCommands": init_commands
                }),
            )
        } else if let Some(port) = req.launch.attach_port {
            (
                "attach".to_string(),
                json!({
                    "gdb-remote-port": port,
                    "cwd": cwd,
                    "initCommands": init_commands
                }),
            )
        } else {
            let lldb_env: Vec<String> = env.iter().map(|(k, v)| format!("{k}={v}")).collect();
            (
                "launch".to_string(),
                json!({
                    "program": executable,
                    "args": prog_args,
                    "cwd": cwd,
                    "env": lldb_env,
                    "stopOnEntry": false,
                    "initCommands": init_commands
                }),
            )
        };
        return Ok(ResolvedDapAdapter {
            adapter_name: "lldb-dap".into(),
            language: "cpp".into(),
            transport: DapAdapterTransport::Stdio {
                command: lldb_bin,
                args: Vec::new(),
                cwd: cwd.to_string(),
                env,
            },
            initialize_args: json!({
                "adapterID": "lldb-dap",
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "pathFormat": "path"
            }),
            launch_command,
            launch_args,
            normalized_cwd: cwd.to_string(),
            initial_output,
        });
    }

    // 3. Fallback: Built-in `lldb -P` Python DAP bridge.
    if let Some((py_bin, lldb_py_dir)) = locate_lldb_python_bridge(&req.target, ctx, &env).await {
        let mut bridge_env = env;
        let merged_pypath = match bridge_env.get("PYTHONPATH") {
            Some(prev) if !prev.is_empty() => format!("{lldb_py_dir}:{prev}"),
            _ => lldb_py_dir,
        };
        bridge_env.insert("PYTHONPATH".into(), merged_pypath);
        let (launch_command, mut launch_args) = if is_attach_mode {
            (
                "attach".to_string(),
                json!({
                    "program": executable,
                    "cwd": cwd,
                    "language": "cpp"
                }),
            )
        } else {
            (
                "launch".to_string(),
                json!({
                    "program": executable,
                    "args": prog_args,
                    "cwd": cwd,
                    "stopOnEntry": false,
                    "language": "cpp"
                }),
            )
        };
        if let Some(pid) = attach_pid {
            launch_args["pid"] = json!(pid);
        }
        if let Some(pname) = attach_process_name {
            launch_args["processName"] = json!(pname);
        }
        if let Some(core) = core_file {
            launch_args["coreFile"] = json!(core);
        }
        return Ok(ResolvedDapAdapter {
            adapter_name: "lldb-python-dap".into(),
            language: "cpp".into(),
            transport: DapAdapterTransport::Stdio {
                command: py_bin,
                args: vec!["-u".into(), "-c".into(), LLDB_PYTHON_DAP_BRIDGE.into()],
                cwd: cwd.to_string(),
                env: bridge_env,
            },
            initialize_args: json!({
                "adapterID": "lldb",
                "linesStartAt1": true,
                "columnsStartAt1": true,
                "pathFormat": "path"
            }),
            launch_command,
            launch_args,
            normalized_cwd: cwd.to_string(),
            initial_output,
        });
    }

    Err(Error::NotFound(
        "C/C++ debugger ('codelldb', 'lldb-dap', or Xcode 'lldb') not found; install Xcode Command Line Tools or CodeLLDB via IDE → Toolchains".into(),
    ))
}

#[allow(clippy::too_many_arguments, clippy::disallowed_methods)]
async fn compile_cpp_debug_artifact(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    raw_prog: &str,
    want_test: bool,
    env: &BTreeMap<String, String>,
    initial_output: &mut Vec<IdeDebugOutputEntry>,
) -> Result<String> {
    let allowed_roots = if ctx.caller_is_root {
        Vec::new()
    } else {
        canonical_target_roots(&req.target)
    };
    let project_dir = find_cpp_build_root(&req.target, raw_prog, cwd, &allowed_roots);
    let extra_flags: Vec<String> = ctx
        .run_config
        .iter()
        .flat_map(|rc| rc.build_flags.iter())
        .chain(req.launch.build_flags.iter())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    let is_local = req.target.kind == IdeTargetKind::Local;
    let project_path = Path::new(&project_dir);

    // 1. CMake project: configure Debug + compile_commands.json if needed, then build.
    //    If the user launched a specific standalone `.cpp`/`.c` file that is NOT
    //    referenced in `CMakeLists.txt` (and CMake doesn't glob sources), fall
    //    through to standalone compiler build so clicking Debug on a scratch or
    //    ad-hoc file inside a CMake repo still debugs that exact file.
    let cmake_file = project_path.join("CMakeLists.txt");
    let use_cmake = if is_local && cmake_file.is_file() {
        if is_cpp_source_file(raw_prog) {
            let cmake_text = std::fs::read_to_string(&cmake_file).unwrap_or_default();
            let base = Path::new(raw_prog)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or("");
            let lower = cmake_text.to_ascii_lowercase();
            (!base.is_empty() && cmake_text.contains(base))
                || lower.contains("file(glob")
                || lower.contains("aux_source_directory")
                || lower.contains("add_subdirectory")
        } else {
            true
        }
    } else {
        false
    };
    if use_cmake {
        let cmake_bin = locate_binary(&req.target, ctx, env, &["cmake".into()])
            .await
            .ok_or_else(|| {
                Error::NotFound("CMake ('cmake') not found on PATH for C/C++ debug build".into())
            })?;
        let build_dir = crate::environments::runner_cmd::find_cpp_build_dir(&project_dir)
            .unwrap_or_else(|| "build".into());
        let build_path = project_path.join(&build_dir);
        if !build_path.join("CMakeCache.txt").is_file() {
            let cfg_args = vec![
                "-S".to_string(),
                ".".to_string(),
                "-B".to_string(),
                build_dir.clone(),
                "-DCMAKE_BUILD_TYPE=Debug".to_string(),
                "-DCMAKE_EXPORT_COMPILE_COMMANDS=ON".to_string(),
            ];
            let cfg_out = run_target_command(
                &req.target,
                ctx.ssh.as_ref(),
                &cmake_bin,
                &cfg_args,
                &project_dir,
                env,
                crate::proc_group::BUILD_STEP_TIMEOUT,
            )
            .await?;
            if !cfg_out.status.success() {
                let err = String::from_utf8_lossy(&cfg_out.stderr);
                return Err(Error::Upstream(format!(
                    "cmake configure failed: {}",
                    err.trim()
                )));
            }
        }
        let mut build_args = vec!["--build".to_string(), build_dir.clone()];
        let target_hint = infer_cmake_target_hint(project_path, raw_prog, want_test);
        if let Some(ref th) = target_hint {
            build_args.push("--target".into());
            build_args.push(th.clone());
        }
        build_args.extend(extra_flags.clone());
        let build_out = run_target_command(
            &req.target,
            ctx.ssh.as_ref(),
            &cmake_bin,
            &build_args,
            &project_dir,
            env,
            crate::proc_group::BUILD_STEP_TIMEOUT,
        )
        .await?;
        let stderr_txt = String::from_utf8_lossy(&build_out.stderr).to_string();
        if !build_out.status.success() {
            let stdout_txt = String::from_utf8_lossy(&build_out.stdout);
            return Err(Error::Upstream(format!(
                "cmake debug build failed: {}{}",
                stdout_txt.trim(),
                stderr_txt.trim()
            )));
        }
        if !stderr_txt.trim().is_empty() {
            initial_output.push(IdeDebugOutputEntry {
                category: "console".into(),
                text: stderr_txt,
                seq: 0,
            });
        }
        if let Some(exe) = find_cmake_executable(&build_path, target_hint.as_deref(), want_test) {
            return Ok(exe);
        }
        if !is_cpp_source_file(raw_prog) {
            return Err(Error::Upstream(format!(
                "cmake build succeeded in '{project_dir}' but no debug executable was found under '{build_dir}'"
            )));
        }
    }

    // 2. Meson project: setup + compile, then find built executable in builddir/.
    if is_local && project_path.join("meson.build").is_file() {
        if let Some(meson_bin) = locate_binary(&req.target, ctx, env, &["meson".into()]).await {
            let build_path = project_path.join("builddir");
            if !build_path.join("build.ninja").is_file() {
                let _ = run_target_command(
                    &req.target,
                    ctx.ssh.as_ref(),
                    &meson_bin,
                    &[
                        "setup".into(),
                        "builddir".into(),
                        "--buildtype=debug".into(),
                    ],
                    &project_dir,
                    env,
                    crate::proc_group::BUILD_STEP_TIMEOUT,
                )
                .await?;
            }
            let compile_out = run_target_command(
                &req.target,
                ctx.ssh.as_ref(),
                &meson_bin,
                &["compile".into(), "-C".into(), "builddir".into()],
                &project_dir,
                env,
                crate::proc_group::BUILD_STEP_TIMEOUT,
            )
            .await?;
            if compile_out.status.success() {
                if let Some(exe) = find_cmake_executable(&build_path, None, want_test) {
                    return Ok(exe);
                }
            }
        }
    }

    // 3. Standalone C / C++ file compile via clang/clang++ (or configured CC/CXX).
    let src_file = if is_cpp_source_file(raw_prog) {
        raw_prog.to_string()
    } else {
        return Err(Error::Invalid(
            "C/C++ debugging requires a CMake/Meson project, a compiled binary, or a .c/.cpp source file".into(),
        ));
    };

    let is_c = src_file.to_ascii_lowercase().ends_with(".c");
    let compiler = resolve_cpp_compiler(req, ctx, cwd, env, is_c).await?;
    let out_bin = cpp_debug_cache_bin_path(data_dir, req.target.kind, &project_dir, &src_file)?;

    let mut args = vec![
        "-g".to_string(),
        "-O0".to_string(),
        if is_c {
            "-std=c17".to_string()
        } else {
            "-std=c++20".to_string()
        },
    ];
    args.extend(extra_flags);
    args.push(src_file);
    args.push("-o".to_string());
    args.push(out_bin.clone());

    let out = run_target_command(
        &req.target,
        ctx.ssh.as_ref(),
        &compiler,
        &args,
        &project_dir,
        env,
        crate::proc_group::BUILD_STEP_TIMEOUT,
    )
    .await?;
    let stderr_txt = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() {
        return Err(Error::Upstream(format!(
            "C/C++ debug compile failed ({compiler}): {}",
            stderr_txt.trim()
        )));
    }
    if !stderr_txt.trim().is_empty() {
        initial_output.push(IdeDebugOutputEntry {
            category: "console".into(),
            text: stderr_txt,
            seq: 0,
        });
    }
    Ok(out_bin)
}

async fn resolve_cpp_compiler(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    env: &BTreeMap<String, String>,
    is_c: bool,
) -> Result<String> {
    let mut candidates = Vec::new();
    let bin_name = if is_c { "clang" } else { "clang++" };
    let gnu_name = if is_c { "gcc" } else { "g++" };
    let sdk_override = super::adapters::resolve_debug_sdk_override(req, ctx, cwd);
    let tc_root = sdk_override.as_deref().or_else(|| {
        ctx.project_env
            .as_ref()
            .and_then(|p| p.cpp_toolchain.as_deref())
    });
    if let Some(root) = tc_root.map(str::trim).filter(|s| !s.is_empty()) {
        let trimmed = root.trim_end_matches('/');
        if trimmed.ends_with("/clang") || trimmed.ends_with("/clang++") {
            if is_c {
                candidates.push(trimmed.trim_end_matches("++").to_string());
            } else if trimmed.ends_with("/clang") {
                candidates.push(format!("{trimmed}++"));
            } else {
                candidates.push(trimmed.to_string());
            }
        } else {
            candidates.push(format!("{trimmed}/bin/{bin_name}"));
            candidates.push(format!("{trimmed}/{bin_name}"));
        }
    }
    let env_key = if is_c { "CC" } else { "CXX" };
    if let Some(env_cc) = env
        .get(env_key)
        .map(String::as_str)
        .filter(|s| !s.trim().is_empty())
    {
        candidates.push(env_cc.trim().to_string());
    }
    candidates.push(bin_name.to_string());
    candidates.push(gnu_name.to_string());
    locate_binary(&req.target, ctx, env, &candidates)
        .await
        .ok_or_else(|| {
            Error::NotFound(format!(
                "C/C++ compiler ('{bin_name}' / '{gnu_name}') not found; install Xcode Command Line Tools or LLVM via IDE → Toolchains"
            ))
        })
}

#[allow(clippy::disallowed_methods)]
fn cpp_debug_cache_bin_path(
    data_dir: &Path,
    target_kind: IdeTargetKind,
    cwd: &str,
    source: &str,
) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(cwd.as_bytes());
    hasher.update(b"::");
    hasher.update(source.as_bytes());
    let digest = hex::encode(hasher.finalize());
    let stem = Path::new(source)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("cpp_app");
    if target_kind == IdeTargetKind::Local {
        let dir = data_dir
            .join("ide-build-cache")
            .join("cpp")
            .join(&digest[..12]);
        std::fs::create_dir_all(&dir)
            .map_err(|e| Error::Internal(format!("create cpp debug cache dir: {e}")))?;
        Ok(dir
            .join(format!("{stem}-bin"))
            .to_string_lossy()
            .into_owned())
    } else {
        Ok(format!("/tmp/otto-cpp-dbg-{}-{stem}-bin", &digest[..10]))
    }
}

#[allow(clippy::disallowed_methods)]
fn infer_cmake_target_hint(project_dir: &Path, raw_prog: &str, want_test: bool) -> Option<String> {
    let cmake_txt = std::fs::read_to_string(project_dir.join("CMakeLists.txt")).ok()?;
    let targets = parse_cmake_executables(&cmake_txt);
    if targets.is_empty() {
        return None;
    }
    // Direct target name (e.g. from CppRun auto-config).
    let base_name = Path::new(raw_prog)
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("");
    if let Some(hit) = targets
        .iter()
        .find(|t| t.as_str() == raw_prog || t.as_str() == base_name)
    {
        return Some(hit.clone());
    }
    // Match `add_executable(target ... <filename>)` when launching from a source file.
    if !base_name.is_empty() {
        for line in cmake_txt.lines() {
            if line.contains(base_name) {
                for t in &targets {
                    if line.contains(t.as_str()) {
                        return Some(t.clone());
                    }
                }
            }
        }
    }
    if want_test {
        if let Some(t) = targets
            .iter()
            .find(|t| t.to_ascii_lowercase().contains("test"))
        {
            return Some(t.clone());
        }
    }
    targets.into_iter().next()
}

#[allow(clippy::disallowed_methods)]
fn find_cmake_executable(
    build_dir: &Path,
    target_hint: Option<&str>,
    want_test: bool,
) -> Option<String> {
    if let Some(hint) = target_hint {
        for sub in ["", "Debug", "bin", "tests", "test"] {
            let cand = if sub.is_empty() {
                build_dir.join(hint)
            } else {
                build_dir.join(sub).join(hint)
            };
            if is_native_executable(&cand) {
                return Some(cand.to_string_lossy().into_owned());
            }
        }
    }
    let mut found: Vec<(std::time::SystemTime, String, PathBuf)> = Vec::new();
    collect_executables_shallow(build_dir, 0, &mut found);
    if found.is_empty() {
        return None;
    }
    if want_test {
        if let Some((_, _, p)) = found
            .iter()
            .find(|(_, name, _)| name.to_ascii_lowercase().contains("test"))
        {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    found.sort_by_key(|(mtime, _, _)| std::cmp::Reverse(*mtime));
    found
        .into_iter()
        .next()
        .map(|(_, _, p)| p.to_string_lossy().into_owned())
}

#[allow(clippy::disallowed_methods)]
fn collect_executables_shallow(
    dir: &Path,
    depth: u8,
    out: &mut Vec<(std::time::SystemTime, String, PathBuf)>,
) {
    if depth > 2 {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten().take(128) {
        let p = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.starts_with('.')
            || matches!(
                name.as_str(),
                "CMakeFiles" | "_deps" | "meson-private" | "meson-info" | "meson-logs"
            )
            || name.ends_with(".dSYM")
        {
            continue;
        }
        if p.is_dir() {
            collect_executables_shallow(&p, depth + 1, out);
        } else if is_native_executable(&p) {
            let mtime = std::fs::metadata(&p)
                .and_then(|m| m.modified())
                .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
            out.push((mtime, name, p));
        }
    }
}

#[allow(clippy::disallowed_methods)]
fn is_native_executable(p: &Path) -> bool {
    if !p.is_file() {
        return false;
    }
    let name = p.file_name().and_then(|n| n.to_str()).unwrap_or("");
    if name.starts_with('.')
        || name.ends_with(".a")
        || name.ends_with(".so")
        || name.ends_with(".dylib")
        || name.ends_with(".o")
        || name.ends_with(".cmake")
        || name.ends_with(".sh")
        || name.ends_with(".py")
        || name.ends_with(".txt")
        || name.ends_with(".json")
    {
        return false;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::metadata(p)
            .map(|m| m.permissions().mode() & 0o111 != 0)
            .unwrap_or(false)
    }
    #[cfg(not(unix))]
    {
        true
    }
}

#[allow(clippy::disallowed_methods)]
fn find_cpp_build_root(
    target: &IdeTargetSpec,
    program: &str,
    fallback_cwd: &str,
    allowed_roots: &[PathBuf],
) -> String {
    if target.kind == IdeTargetKind::Local && !program.is_empty() {
        let mut cur = Path::new(program);
        if cur.is_file() {
            cur = cur.parent().unwrap_or(cur);
        }
        let mut best: Option<String> = None;
        loop {
            let check_cur = std::fs::canonicalize(cur).unwrap_or_else(|_| cur.to_path_buf());
            if !allowed_roots.is_empty()
                && !allowed_roots
                    .iter()
                    .any(|r| cur.starts_with(r) || check_cur.starts_with(r))
            {
                break;
            }
            if cur.join("CMakePresets.json").is_file()
                || cur.join("CMakeLists.txt").is_file()
                || cur.join("meson.build").is_file()
            {
                best = Some(cur.to_string_lossy().into_owned());
            }
            let Some(parent) = cur.parent() else { break };
            cur = parent;
        }
        if let Some(root) = best {
            return root;
        }
    }
    fallback_cwd.to_string()
}

#[cfg(test)]
mod cpp_support_tests {
    use crate::deps_inspector::{
        parse_cmake_deps, parse_conanfile_txt, parse_unreal_plugins, parse_vcpkg_json,
    };
    use crate::environments::parse_test_output;
    use crate::gutter::detect_gutter_items;
    use crate::lsp_host::servers::{normalize_lsp_lang, resolve_lsp_module_root, REGISTRY};
    use crate::lsp_service::{parse_clang_stderr, parse_document_symbols};
    use crate::toolchains_catalog::EcosystemKind;
    use crate::types::{detect_language, IdeGutterKind, IdeRunnerKind, IdeTestCaseStatus};

    #[test]
    fn cpp_language_detection_and_lsp_registry() {
        assert_eq!(detect_language("src/main.cpp"), Some("cpp"));
        assert_eq!(detect_language("include/engine.hpp"), Some("cpp"));
        assert_eq!(detect_language("src/util.inl"), Some("cpp"));
        assert_eq!(detect_language("src/lib.c"), Some("c"));
        assert_eq!(detect_language("CMakeLists.txt"), Some("cmake"));
        assert_eq!(detect_language("meson.build"), Some("meson"));
        assert_eq!(detect_language("MyGame.uproject"), Some("json"));
        assert_eq!(normalize_lsp_lang("c"), "cpp");
        assert_eq!(normalize_lsp_lang("hpp"), "cpp");
        let clangd = REGISTRY
            .iter()
            .find(|d| d.lang == "cpp")
            .expect("clangd entry");
        assert_eq!(clangd.cmd, "clangd");
        assert!(clangd.extensions.contains(&".cpp"));
        assert!(clangd.extensions.contains(&".h"));
        assert_eq!(EcosystemKind::parse("cpp"), Some(EcosystemKind::Cpp));
        assert_eq!(EcosystemKind::Cpp.lsp_name(), "clangd");
        assert_eq!(EcosystemKind::Cpp.dap_name(), "codelldb");
    }

    #[test]
    fn cpp_gutter_detects_main_gtest_and_catch2() {
        let src = r#"
#include <gtest/gtest.h>

TEST(MathSuite, AddsNumbers) {
    EXPECT_EQ(1 + 1, 2);
}

TEST_F(MathSuite, SubtractsNumbers) {
    EXPECT_EQ(3 - 1, 2);
}

TEST_CASE("vector grows", "[vec]") {
    REQUIRE(true);
}

int main(int argc, char** argv) {
    return 0;
}
"#;
        let items = detect_gutter_items("/proj/tests/math_test.cpp", src, &[]);
        assert!(items
            .iter()
            .any(|i| i.kind == IdeGutterKind::Suite && i.label == "MathSuite"));
        assert!(items.iter().any(|i| {
            i.kind == IdeGutterKind::Test
                && i.label == "MathSuite.AddsNumbers"
                && i.run_spec.runner == IdeRunnerKind::Ctest
        }));
        assert!(items
            .iter()
            .any(|i| i.kind == IdeGutterKind::Test && i.label == "vector grows"));
        assert!(items.iter().any(|i| {
            i.kind == IdeGutterKind::Main
                && i.label == "main"
                && i.run_spec.runner == IdeRunnerKind::CppRun
        }));
    }

    #[test]
    fn cpp_test_parsers_handle_ctest_gtest_and_meson() {
        let gtest_lines = vec![
            "[       OK ] MathSuite.Adds (3 ms)".into(),
            "tests/math_test.cpp:14: Failure: Expected 4".into(),
            "[  FAILED  ] MathSuite.Divides (1 ms)".into(),
            "[  SKIPPED ] MathSuite.Pending (0 ms)".into(),
        ];
        let gtest = parse_test_output(IdeRunnerKind::Ctest, &gtest_lines);
        assert_eq!(gtest.len(), 3);
        assert_eq!(gtest[0].status, IdeTestCaseStatus::Passed);
        assert_eq!(gtest[0].suite.as_deref(), Some("MathSuite"));
        assert_eq!(gtest[0].name, "Adds");
        assert_eq!(gtest[0].duration_ms, Some(3));
        assert_eq!(gtest[1].status, IdeTestCaseStatus::Failed);
        assert_eq!(gtest[2].status, IdeTestCaseStatus::Skipped);

        let ctest_lines = vec![
            "1/2 Test #1: MathTest .......................   Passed    0.02 sec".into(),
            "2/2 Test #2: FailTest .......................***Failed    0.05 sec".into(),
        ];
        let ctest = parse_test_output(IdeRunnerKind::Ctest, &ctest_lines);
        assert_eq!(ctest.len(), 2);
        assert_eq!(ctest[0].name, "MathTest");
        assert_eq!(ctest[0].status, IdeTestCaseStatus::Passed);
        assert_eq!(ctest[0].duration_ms, Some(20));
        assert_eq!(ctest[1].name, "FailTest");
        assert_eq!(ctest[1].status, IdeTestCaseStatus::Failed);

        let meson_lines = vec![
            "1/2 unit_math        OK              0.01s".into(),
            "2/2 unit_bad         FAIL            0.03s   exit status 1".into(),
        ];
        let meson = parse_test_output(IdeRunnerKind::Meson, &meson_lines);
        assert_eq!(meson.len(), 2);
        assert_eq!(meson[0].status, IdeTestCaseStatus::Passed);
        assert_eq!(meson[1].status, IdeTestCaseStatus::Failed);
    }

    #[test]
    fn cpp_deps_symbols_and_clang_diagnostics_parse_cleanly() {
        let vcpkg = r#"{"dependencies": ["fmt", {"name": "spdlog", "version>=": "1.14.0"}]}"#;
        let vd = parse_vcpkg_json(vcpkg);
        assert_eq!(vd.len(), 2);
        assert_eq!(vd[0].name, "fmt");
        assert_eq!(vd[1].name, "spdlog");
        assert_eq!(vd[1].current_version, "1.14.0");

        let conan = "[requires]\nfmt/10.2.1\nspdlog/1.14.1@user/stable\n";
        let cd = parse_conanfile_txt(conan);
        assert_eq!(cd.len(), 2);
        assert_eq!(cd[0].name, "fmt");
        assert_eq!(cd[0].current_version, "10.2.1");

        let cmake = "find_package(Boost 1.84 REQUIRED)\nFetchContent_Declare(googletest GIT_REPOSITORY https://example.com)\n";
        let cmd = parse_cmake_deps(cmake);
        assert_eq!(cmd.len(), 2);
        assert_eq!(cmd[0].name, "Boost");
        assert_eq!(cmd[0].current_version, "1.84");
        assert_eq!(cmd[1].name, "googletest");

        let uproj = r#"{"Plugins": [{"Name": "EnhancedInput", "Enabled": true}, {"Name": "OldPlugin", "Enabled": false}]}"#;
        let up = parse_unreal_plugins(uproj);
        assert_eq!(up.len(), 2);
        assert_eq!(up[0].name, "EnhancedInput");
        assert_eq!(up[0].current_version, "enabled");

        let src =
            "namespace engine {\nclass DEMO_API Actor {\n};\nint compute(int x) { return x; }\n}\n";
        let syms = parse_document_symbols("engine.cpp", "cpp", src);
        fn collect_names(nodes: &[crate::types::IdeSymbolNode], out: &mut Vec<String>) {
            for n in nodes {
                out.push(n.name.clone());
                collect_names(&n.children, out);
            }
        }
        let mut sym_names = Vec::new();
        collect_names(&syms, &mut sym_names);
        assert!(
            sym_names.iter().any(|n| n == "engine"),
            "syms={sym_names:?}"
        );
        assert!(sym_names.iter().any(|n| n == "Actor"), "syms={sym_names:?}");
        assert!(
            sym_names.iter().any(|n| n == "compute"),
            "syms={sym_names:?}"
        );

        let diags =
            parse_clang_stderr("/proj/main.cpp:7:12: error: use of undeclared identifier 'foo'\n");
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 7);
        assert_eq!(diags[0].col, 12);

        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("CMakeLists.txt"), "project(demo)\n").unwrap();
        let src_dir = tmp.path().join("src");
        std::fs::create_dir_all(&src_dir).unwrap();
        let main_cpp = src_dir.join("main.cpp");
        std::fs::write(&main_cpp, "int main() { return 0; }\n").unwrap();
        let canon_tmp = tmp.path().canonicalize().unwrap();
        let mod_root = resolve_lsp_module_root(
            "cpp",
            &canon_tmp.to_string_lossy(),
            Some(&main_cpp.to_string_lossy()),
            std::slice::from_ref(&canon_tmp),
        )
        .unwrap();
        assert_eq!(
            std::path::Path::new(&mod_root).canonicalize().unwrap(),
            canon_tmp,
            "CMake root should be resolved from nested src/main.cpp"
        );
    }
}
