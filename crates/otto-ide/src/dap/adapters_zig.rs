//! Zig (`codelldb`, `lldb-dap`, and built-in `lldb -P` `SBDebugger` Python DAP bridge)
//! DAP adapter resolver with `zig build` / `zig build-exe` / `zig test --test-no-exec`
//! compilation and Zig DWARF LLDB pretty-printers (slices, strings, optionals,
//! error unions, and tagged unions).

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
use crate::types::{
    IdeDebugOutputEntry, IdeRunnerKind, IdeStartDebugReq, IdeTargetKind, IdeTargetSpec,
};

/// LLDB Python type summary providers for Zig 0.12–0.15 DWARF types:
/// - `[]u8` / `[]const u8` string slices -> `"hello" (len=5)`
/// - generic `[]T` slices -> `[len=N] {e0, e1, ...}`
/// - `?T` optionals -> `null` or inner value
/// - `E!T` / `anyerror!T` error unions -> `ok(val)` or `error.<name>`
const ZIG_LLDB_FORMATTERS_PY: &str = r#"# Otto IDE Zig LLDB pretty-printers (auto-generated)
import lldb

def _val_text(v):
    if not v or not v.IsValid():
        return ''
    s = v.GetSummary()
    if s:
        return s
    val = v.GetValue()
    if val is not None:
        return val
    n = v.GetNumChildren(8)
    if n > 0:
        parts = []
        for i in range(min(n, 4)):
            c = v.GetChildAtIndex(i)
            cn = c.GetName() or str(i)
            cv = c.GetSummary() or c.GetValue() or '{...}'
            parts.append(f'{cn}={cv}')
        return '{' + ', '.join(parts) + (', ...' if n > 4 else '') + '}'
    return '{...}'

def zig_slice_summary(valobj, internal_dict):
    try:
        ptr = valobj.GetChildMemberWithName('ptr')
        ln_val = valobj.GetChildMemberWithName('len')
        if not ptr or not ptr.IsValid() or not ln_val or not ln_val.IsValid():
            return ''
        length = ln_val.GetValueAsUnsigned(0)
        if length == 0:
            tname = valobj.GetDisplayTypeName() or valobj.GetTypeName() or ''
            return '"" (len=0)' if 'u8' in tname else '[] (len=0)'
        addr = ptr.GetValueAsUnsigned(0)
        if addr == 0:
            return f'null (len={length})'
        proc = valobj.GetProcess()
        tname = valobj.GetDisplayTypeName() or valobj.GetTypeName() or ''
        elem_type = ptr.GetType().GetPointeeType()
        elem_name = elem_type.GetName() or ''
        is_byte_slice = elem_name in ('unsigned char', 'u8', 'const unsigned char', 'const u8')
        if is_byte_slice and proc and proc.IsValid():
            read_len = min(length, 256)
            err = lldb.SBError()
            data = proc.ReadMemory(addr, read_len, err)
            if err.Success() and data is not None:
                try:
                    text = data.decode('utf-8')
                    if all(ch.isprintable() or ch in '\t\r\n' for ch in text):
                        esc = text.replace('\\', '\\\\').replace('"', '\\"').replace('\n', '\\n').replace('\r', '\\r').replace('\t', '\\t')
                        suffix = '...' if length > read_len else ''
                        return f'"{esc}{suffix}" (len={length})'
                except Exception:
                    pass
        target = valobj.GetTarget()
        elem_size = elem_type.GetByteSize()
        if target and target.IsValid() and elem_size > 0:
            shown = min(length, 8)
            elems = []
            for i in range(shown):
                ev = target.CreateValueFromAddress(f'[{i}]', lldb.SBAddress(addr + i * elem_size, target), elem_type)
                elems.append(_val_text(ev))
            more = ', ...' if length > shown else ''
            return f'[len={length}] {{{", ".join(elems)}{more}}}'
        return f'[len={length}]'
    except Exception:
        return ''

def zig_optional_summary(valobj, internal_dict):
    try:
        some = valobj.GetChildMemberWithName('some')
        data = valobj.GetChildMemberWithName('data')
        if not some or not some.IsValid():
            return ''
        if some.GetValueAsUnsigned(0) == 0:
            return 'null'
        return _val_text(data) if data and data.IsValid() else 'some'
    except Exception:
        return ''

def zig_error_union_summary(valobj, internal_dict):
    try:
        tag = valobj.GetChildMemberWithName('tag')
        value = valobj.GetChildMemberWithName('value')
        if not tag or not tag.IsValid():
            return ''
        code = tag.GetValueAsUnsigned(0)
        if code == 0:
            if value and value.IsValid():
                return f'ok({_val_text(value)})'
            return 'ok'
        tag_str = tag.GetValue() or str(code)
        if tag_str.isdigit():
            return f'error(#{code})'
        return f'error.{tag_str}'
    except Exception:
        return ''

def __lldb_init_module(debugger, internal_dict):
    debugger.HandleCommand('type summary add -x "^(const )?\\[\\].*" -F zig_lldb_formatters.zig_slice_summary -w zig')
    debugger.HandleCommand('type summary add -x "^(const )?\\?.*" -F zig_lldb_formatters.zig_optional_summary -w zig')
    debugger.HandleCommand('type summary add -x "^(const )?(anyerror|error\\{[^}]*\\}|[A-Za-z_][A-Za-z0-9_.]*)!.*" -F zig_lldb_formatters.zig_error_union_summary -w zig')
    debugger.HandleCommand('type category enable zig')
"#;

#[allow(clippy::disallowed_methods)]
fn ensure_zig_formatters_script(data_dir: &Path) -> Option<PathBuf> {
    let dir = data_dir.join("ide-build-cache").join("zig");
    std::fs::create_dir_all(&dir).ok()?;
    let script_path = dir.join("zig_lldb_formatters.py");
    let up_to_date = std::fs::read_to_string(&script_path)
        .map(|s| s == ZIG_LLDB_FORMATTERS_PY)
        .unwrap_or(false);
    if !up_to_date {
        std::fs::write(&script_path, ZIG_LLDB_FORMATTERS_PY).ok()?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&script_path, std::fs::Permissions::from_mode(0o600));
        }
    }
    Some(script_path)
}

pub(super) async fn resolve_zig_adapter(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    env: BTreeMap<String, String>,
) -> Result<ResolvedDapAdapter> {
    let mut initial_output = Vec::new();
    let raw_prog = normalize_target_path(&req.target, &req.launch.program);
    let want_test = req.launch.test_name.is_some()
        || ctx
            .run_config
            .as_ref()
            .is_some_and(|rc| rc.runner == IdeRunnerKind::ZigTest);

    let executable = if req.launch.attach_port.is_some()
        || (!raw_prog.is_empty()
            && !raw_prog.ends_with(".zig")
            && !raw_prog.ends_with(".zon")
            && (req.target.kind == IdeTargetKind::Ssh || Path::new(&raw_prog).is_file()))
    {
        raw_prog
    } else {
        compile_zig_debug_artifact(
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

    let prog_args = req.launch.args.clone();
    let init_commands: Vec<String> = if req.target.kind == IdeTargetKind::Local {
        ensure_zig_formatters_script(data_dir)
            .map(|p| {
                let escaped = p.to_string_lossy().replace('\'', "'\\''");
                vec![format!("command script import '{escaped}'")]
            })
            .unwrap_or_default()
    } else {
        Vec::new()
    };

    let mut codelldb_candidates = Vec::new();
    if req.target.kind == IdeTargetKind::Local {
        for rel in [
            "toolchains/zig/dap/extension/adapter/codelldb",
            "toolchains/rust/dap/extension/adapter/codelldb",
            "toolchains/rust/codelldb/adapter/codelldb",
            "toolchains/codelldb/adapter/codelldb",
        ] {
            codelldb_candidates.push(data_dir.join(rel).to_string_lossy().into_owned());
        }
    } else {
        codelldb_candidates
            .push("$HOME/.otto/toolchains/zig/dap/extension/adapter/codelldb".into());
        codelldb_candidates
            .push("$HOME/.otto/toolchains/rust/dap/extension/adapter/codelldb".into());
    }
    codelldb_candidates.push("codelldb".into());

    if let Some(codelldb_bin) = locate_binary(&req.target, ctx, &env, &codelldb_candidates).await {
        let (launch_command, launch_args) = if let Some(port) = req.launch.attach_port {
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
            language: "zig".into(),
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

    let lldb_dap_candidates = vec![
        "lldb-dap".to_string(),
        "lldb-vscode".to_string(),
        "/Library/Developer/CommandLineTools/usr/bin/lldb-dap".to_string(),
        "/Applications/Xcode.app/Contents/Developer/usr/bin/lldb-dap".to_string(),
        "/opt/homebrew/opt/llvm/bin/lldb-dap".to_string(),
        "/usr/bin/lldb-dap".to_string(),
    ];
    if let Some(lldb_bin) = locate_binary(&req.target, ctx, &env, &lldb_dap_candidates).await {
        let (launch_command, launch_args) = if let Some(port) = req.launch.attach_port {
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
            language: "zig".into(),
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

    if let Some((py_bin, lldb_py_dir)) = locate_lldb_python_bridge(&req.target, ctx, &env).await {
        let mut bridge_env = env;
        let merged_pypath = match bridge_env.get("PYTHONPATH") {
            Some(prev) if !prev.is_empty() => format!("{lldb_py_dir}:{prev}"),
            _ => lldb_py_dir,
        };
        bridge_env.insert("PYTHONPATH".into(), merged_pypath);
        return Ok(ResolvedDapAdapter {
            adapter_name: "lldb-python-dap".into(),
            language: "zig".into(),
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
            launch_command: "launch".into(),
            launch_args: json!({
                "program": executable,
                "args": prog_args,
                "cwd": cwd,
                "stopOnEntry": false,
                "language": "zig"
            }),
            normalized_cwd: cwd.to_string(),
            initial_output,
        });
    }

    Err(Error::NotFound(
        "Zig debugger ('codelldb', 'lldb-dap', or Xcode 'lldb') not found; install Xcode Command Line Tools or CodeLLDB via IDE → Toolchains".into(),
    ))
}

#[allow(clippy::too_many_arguments, clippy::disallowed_methods)]
async fn compile_zig_debug_artifact(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    raw_prog: &str,
    want_test: bool,
    env: &BTreeMap<String, String>,
    initial_output: &mut Vec<IdeDebugOutputEntry>,
) -> Result<String> {
    let zig_bin = resolve_zig_binary(req, ctx, cwd, env).await?;
    let allowed_roots = if ctx.caller_is_root {
        Vec::new()
    } else {
        canonical_target_roots(&req.target)
    };
    let zig_cwd = find_zig_dir(&req.target, raw_prog, cwd, &allowed_roots);
    let extra_flags: Vec<String> = ctx
        .run_config
        .iter()
        .flat_map(|rc| rc.build_flags.iter())
        .chain(req.launch.build_flags.iter())
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();

    let has_build_zig =
        req.target.kind == IdeTargetKind::Local && Path::new(&zig_cwd).join("build.zig").is_file();
    let is_zig_file = raw_prog.ends_with(".zig") && !raw_prog.ends_with("/build.zig");

    if want_test {
        let test_file = if is_zig_file {
            raw_prog.to_string()
        } else if req.target.kind == IdeTargetKind::Local {
            let cwd_p = Path::new(&zig_cwd);
            ["src/main.zig", "src/root.zig", "main.zig"]
                .iter()
                .map(|r| cwd_p.join(r))
                .find(|p| p.is_file())
                .map(|p| p.to_string_lossy().into_owned())
                .ok_or_else(|| {
                    Error::Invalid(
                        "Zig test debugging requires a .zig file (or src/main.zig / src/root.zig)"
                            .into(),
                    )
                })?
        } else {
            raw_prog.to_string()
        };
        let out_bin =
            zig_debug_cache_bin_path(data_dir, req.target.kind, &zig_cwd, &test_file, true)?;
        let mut args = vec![
            "test".to_string(),
            "--test-no-exec".to_string(),
            format!("-femit-bin={out_bin}"),
        ];
        if let Some(tname) = req.launch.test_name.as_deref().filter(|s| !s.is_empty()) {
            args.push("--test-filter".into());
            args.push(tname.to_string());
        }
        args.extend(extra_flags);
        args.push(test_file);

        run_zig_build_with_macos_fallback(
            req,
            ctx,
            &zig_bin,
            args,
            &zig_cwd,
            env,
            false,
            initial_output,
        )
        .await?;
        return Ok(out_bin);
    }

    // Prefer `zig build` when a `build.zig` project exists and the user did not
    // explicitly target a standalone `.zig` file via `ZigRun`.
    let force_single_file = ctx
        .run_config
        .as_ref()
        .is_some_and(|rc| rc.runner == IdeRunnerKind::ZigRun);
    if has_build_zig && !force_single_file {
        let mut args = vec!["build".to_string()];
        args.extend(extra_flags.clone());
        let build_res = run_zig_build_with_macos_fallback(
            req,
            ctx,
            &zig_bin,
            args,
            &zig_cwd,
            env,
            true,
            initial_output,
        )
        .await;
        match build_res {
            Ok(()) => {
                if let Some(bin) = find_zig_out_executable(&zig_cwd, raw_prog) {
                    return Ok(bin);
                }
                if !is_zig_file {
                    return Err(Error::Upstream(format!(
                        "zig build succeeded in '{zig_cwd}' but produced no executable under zig-out/bin"
                    )));
                }
            }
            Err(err) if !is_zig_file => return Err(err),
            Err(_) => {
                // Fall through to direct `zig build-exe` when a single `.zig` file was launched.
            }
        }
    }

    if !is_zig_file {
        return Err(Error::Invalid(
            "Zig debugging requires a build.zig project or a .zig source file".into(),
        ));
    }

    let out_bin = zig_debug_cache_bin_path(data_dir, req.target.kind, &zig_cwd, raw_prog, false)?;
    let mut args = vec!["build-exe".to_string(), format!("-femit-bin={out_bin}")];
    args.extend(extra_flags);
    args.push(raw_prog.to_string());

    run_zig_build_with_macos_fallback(
        req,
        ctx,
        &zig_bin,
        args,
        &zig_cwd,
        env,
        false,
        initial_output,
    )
    .await?;
    Ok(out_bin)
}

#[allow(clippy::too_many_arguments)]
async fn run_zig_build_with_macos_fallback(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    zig_bin: &str,
    args: Vec<String>,
    zig_cwd: &str,
    env: &BTreeMap<String, String>,
    is_zig_build_subcommand: bool,
    initial_output: &mut Vec<IdeDebugOutputEntry>,
) -> Result<()> {
    let out = run_target_command(
        &req.target,
        ctx.ssh.as_ref(),
        zig_bin,
        &args,
        zig_cwd,
        env,
        crate::proc_group::BUILD_STEP_TIMEOUT,
    )
    .await?;
    let stderr_txt = String::from_utf8_lossy(&out.stderr).to_string();
    if out.status.success() {
        if !stderr_txt.trim().is_empty() {
            initial_output.push(IdeDebugOutputEntry {
                category: "console".into(),
                text: stderr_txt,
                seq: 0,
            });
        }
        return Ok(());
    }

    // Zig 0.14.x does not ship libc stubs for macOS 26/27 SDKs, causing linker failure
    // `undefined symbol: __availability_version_check`. Retry once pinned to macOS 15.0
    // unless the user already specified an explicit `-target` / `-Dtarget`.
    let has_explicit_target = args
        .iter()
        .any(|a| a == "-target" || a.starts_with("-target=") || a.starts_with("-Dtarget"));
    if cfg!(target_os = "macos")
        && req.target.kind == IdeTargetKind::Local
        && !has_explicit_target
        && stderr_txt.contains("__availability_version_check")
    {
        let mut retry_args = args;
        if is_zig_build_subcommand {
            retry_args.push("-Dtarget=native-macos.15.0".into());
        } else {
            // Insert before the trailing source file path.
            let insert_at = retry_args.len().saturating_sub(1);
            retry_args.insert(insert_at, "native-macos.15.0".into());
            retry_args.insert(insert_at, "-target".into());
        }
        let retry = run_target_command(
            &req.target,
            ctx.ssh.as_ref(),
            zig_bin,
            &retry_args,
            zig_cwd,
            env,
            crate::proc_group::BUILD_STEP_TIMEOUT,
        )
        .await?;
        let retry_err = String::from_utf8_lossy(&retry.stderr).to_string();
        if retry.status.success() {
            initial_output.push(IdeDebugOutputEntry {
                category: "console".into(),
                text: "zig debug build: retried with macOS 15.0 libc target for host SDK compatibility\n"
                    .into(),
                seq: 0,
            });
            if !retry_err.trim().is_empty() {
                initial_output.push(IdeDebugOutputEntry {
                    category: "console".into(),
                    text: retry_err,
                    seq: 0,
                });
            }
            return Ok(());
        }
        return Err(Error::Upstream(format!(
            "zig debug build failed: {}",
            retry_err.trim()
        )));
    }

    Err(Error::Upstream(format!(
        "zig debug build failed: {}",
        stderr_txt.trim()
    )))
}

async fn resolve_zig_binary(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    env: &BTreeMap<String, String>,
) -> Result<String> {
    let mut candidates = Vec::new();
    let sdk_override = super::adapters::resolve_debug_sdk_override(req, ctx, cwd);
    let zig_home = sdk_override
        .as_deref()
        .or_else(|| ctx.project_env.as_ref().and_then(|p| p.zig_home.as_deref()));
    if let Some(home) = zig_home.map(str::trim).filter(|s| !s.is_empty()) {
        let trimmed = home.trim_end_matches('/');
        if trimmed.ends_with("/zig") {
            candidates.push(trimmed.to_string());
        } else {
            candidates.push(format!("{trimmed}/bin/zig"));
            candidates.push(format!("{trimmed}/zig"));
        }
    }
    candidates.push("zig".to_string());
    locate_binary(&req.target, ctx, env, &candidates)
        .await
        .ok_or_else(|| {
            Error::NotFound(
                "Zig compiler ('zig') not found; install Zig via IDE → Toolchains & SDKs".into(),
            )
        })
}

#[allow(clippy::disallowed_methods)]
fn zig_debug_cache_bin_path(
    data_dir: &Path,
    target_kind: IdeTargetKind,
    cwd: &str,
    source: &str,
    is_test: bool,
) -> Result<String> {
    let mut hasher = Sha256::new();
    hasher.update(cwd.as_bytes());
    hasher.update(b"::");
    hasher.update(source.as_bytes());
    hasher.update(if is_test {
        b"::test".as_slice()
    } else {
        b"::exe".as_slice()
    });
    let digest = hex::encode(hasher.finalize());
    let stem = Path::new(source)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or("zig_app");
    let suffix = if is_test { "test" } else { "bin" };
    if target_kind == IdeTargetKind::Local {
        let dir = data_dir
            .join("ide-build-cache")
            .join("zig")
            .join(&digest[..12]);
        std::fs::create_dir_all(&dir)
            .map_err(|e| Error::Internal(format!("create zig debug cache dir: {e}")))?;
        Ok(dir
            .join(format!("{stem}-{suffix}"))
            .to_string_lossy()
            .into_owned())
    } else {
        Ok(format!(
            "/tmp/otto-zig-dbg-{}-{stem}-{suffix}",
            &digest[..10]
        ))
    }
}

#[allow(clippy::disallowed_methods)]
fn find_zig_out_executable(zig_cwd: &str, raw_prog: &str) -> Option<String> {
    let bin_dir = Path::new(zig_cwd).join("zig-out").join("bin");
    let entries = std::fs::read_dir(&bin_dir).ok()?;
    let mut bins: Vec<(std::time::SystemTime, String, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let p = entry.path();
        if !p.is_file() {
            continue;
        }
        let name = p
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("")
            .to_string();
        if name.ends_with(".pdb") || name.ends_with(".dSYM") || name.starts_with('.') {
            continue;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let is_exec = std::fs::metadata(&p)
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false);
            if !is_exec {
                continue;
            }
        }
        let mtime = std::fs::metadata(&p)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
        bins.push((mtime, name, p));
    }
    if bins.is_empty() {
        return None;
    }
    let proj_name = Path::new(zig_cwd)
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");
    let prog_stem = Path::new(raw_prog)
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| *s != "main" && *s != "root" && *s != "build");
    if let Some(stem) = prog_stem {
        if let Some((_, _, p)) = bins.iter().find(|(_, n, _)| n == stem) {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    if let Some((_, _, p)) = bins.iter().find(|(_, n, _)| n == proj_name) {
        return Some(p.to_string_lossy().into_owned());
    }
    bins.sort_by_key(|(mtime, _, _)| std::cmp::Reverse(*mtime));
    bins.into_iter()
        .next()
        .map(|(_, _, p)| p.to_string_lossy().into_owned())
}

#[allow(clippy::disallowed_methods)]
fn find_zig_dir(
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
        loop {
            let check_cur = std::fs::canonicalize(cur).unwrap_or_else(|_| cur.to_path_buf());
            if !allowed_roots.is_empty()
                && !allowed_roots
                    .iter()
                    .any(|r| cur.starts_with(r) || check_cur.starts_with(r))
            {
                break;
            }
            if cur.join("build.zig").is_file() {
                return cur.to_string_lossy().into_owned();
            }
            let Some(parent) = cur.parent() else { break };
            cur = parent;
        }
    }
    fallback_cwd.to_string()
}

#[cfg(test)]
mod zig_support_tests {
    use crate::deps_inspector::parse_build_zig_zon;
    use crate::environments::discovery::parse_zig_build_steps;
    use crate::environments::parse_zig_test_line;
    use crate::lsp_host::servers::{normalize_lsp_lang, resolve_lsp_module_root, REGISTRY};
    use crate::lsp_service::{parse_document_symbols, parse_zig_ast_check_stderr};
    use crate::toolchains_catalog::{parse_zig_index_json, EcosystemKind, TargetPlatform};
    use crate::types::{detect_language, IdeTestCaseStatus};

    #[test]
    fn zig_language_detection_and_lsp_registry() {
        assert_eq!(detect_language("src/main.zig"), Some("zig"));
        assert_eq!(detect_language("build.zig"), Some("zig"));
        assert_eq!(detect_language("build.zig.zon"), Some("zig"));
        assert_eq!(normalize_lsp_lang("zon"), "zig");
        let zls = REGISTRY
            .iter()
            .find(|d| d.lang == "zig")
            .expect("zls entry");
        assert_eq!(zls.cmd, "zls");
        assert!(zls.extensions.contains(&".zig"));
        assert!(zls.extensions.contains(&".zon"));
        assert_eq!(EcosystemKind::parse("zig"), Some(EcosystemKind::Zig));
        assert_eq!(EcosystemKind::Zig.lsp_name(), "zls");
        assert_eq!(EcosystemKind::Zig.dap_name(), "codelldb");
    }

    #[test]
    fn zig_build_steps_and_test_output_parsing() {
        let build_zig = r#"
const std = @import("std");
pub fn build(b: *std.Build) void {
    const run_step = b.step("run", "Run the app");
    const test_step = b.step("test", "Run unit tests");
    const fmt_step = b.step("fmt-check", "Check formatting");
    _ = .{ run_step, test_step, fmt_step };
}
"#;
        let steps = parse_zig_build_steps(build_zig);
        assert_eq!(steps, vec!["run", "test", "fmt-check"]);

        let ok = parse_zig_test_line("1/3 main.test.addition works...OK").unwrap();
        assert_eq!(ok.name, "addition works");
        assert_eq!(ok.status, IdeTestCaseStatus::Passed);

        let fail = parse_zig_test_line("2/3 math.test.overflow...FAIL (Overflow)").unwrap();
        assert_eq!(fail.name, "overflow");
        assert_eq!(fail.status, IdeTestCaseStatus::Failed);

        let skip = parse_zig_test_line("3/3 net.test.offline...SKIP").unwrap();
        assert_eq!(skip.name, "offline");
        assert_eq!(skip.status, IdeTestCaseStatus::Skipped);
    }

    #[test]
    fn zig_zon_dependency_parser_and_symbols_and_ast_check() {
        let zon = r#".{
    .name = .my_app,
    .version = "0.1.0",
    .dependencies = .{
        .zap = .{
            .url = "https://github.com/zigzap/zap/archive/refs/tags/v0.9.1.tar.gz",
            .hash = "1220abcdef1234567890",
        },
        .local_lib = .{
            .path = "../local_lib",
        },
    },
}"#;
        let deps = parse_build_zig_zon(zon);
        assert_eq!(deps.len(), 2);
        assert_eq!(deps[0].name, "zap");
        assert_eq!(deps[0].current_version, "v0.9.1");
        assert_eq!(deps[1].name, "local_lib");
        assert_eq!(deps[1].current_version, "path:../local_lib");

        let zig_src = r#"const std = @import("std");
pub const Point = struct {
    x: i32,
    y: i32,
    pub fn sum(self: Point) i32 {
        return self.x + self.y;
    }
};
pub fn main() !void {}
test "point sum" {
    try std.testing.expectEqual(@as(i32, 3), (Point{ .x = 1, .y = 2 }).sum());
}
"#;
        let syms = parse_document_symbols("src/main.zig", "zig", zig_src);
        let names: Vec<&str> = syms.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Point"), "syms={names:?}");
        assert!(names.contains(&"main"), "syms={names:?}");
        assert!(names.contains(&"point sum"), "syms={names:?}");
        let point = syms.iter().find(|s| s.name == "Point").unwrap();
        assert!(
            point
                .children
                .iter()
                .any(|c| c.name == "sum" && c.kind == "method"),
            "point children={:?}",
            point.children
        );

        let ast_err = "<stdin>:2:13: error: expected expression, found ';'\n";
        let diags = parse_zig_ast_check_stderr(ast_err);
        assert_eq!(diags.len(), 1);
        assert_eq!(diags[0].line, 2);
        assert_eq!(diags[0].col, 13);
        assert_eq!(diags[0].severity, "error");
        assert!(diags[0].message.contains("expected expression"));
    }

    #[test]
    fn zig_index_json_and_module_root_resolution() {
        let index = r#"{
            "master": { "version": "0.15.0-dev.1" },
            "0.14.0": {
                "date": "2025-03-04",
                "aarch64-macos": {
                    "tarball": "https://ziglang.org/download/0.14.0/zig-macos-aarch64-0.14.0.tar.xz",
                    "shasum": "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"
                }
            },
            "0.13.0": {
                "date": "2024-06-06",
                "aarch64-macos": {
                    "tarball": "https://ziglang.org/download/0.13.0/zig-macos-aarch64-0.13.0.tar.xz",
                    "shasum": "fedcba9876543210fedcba9876543210fedcba9876543210fedcba9876543210"
                }
            }
        }"#;
        let plat = TargetPlatform::from_uname("Darwin", "arm64").unwrap();
        let parsed = parse_zig_index_json(index, plat);
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].version, "0.14.0");
        assert_eq!(
            parsed[0].sha256.as_deref(),
            Some("0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef")
        );
        assert_eq!(parsed[1].version, "0.13.0");

        let tmp = tempfile::tempdir().unwrap();
        let ws = tmp.path().join("zig-app");
        let dep = tmp.path().join(".cache/zig/p/1220deadbeef");
        std::fs::create_dir_all(ws.join("src")).unwrap();
        std::fs::write(ws.join("build.zig"), "").unwrap();
        std::fs::create_dir_all(&dep).unwrap();
        std::fs::write(dep.join("build.zig"), "").unwrap();
        std::fs::write(dep.join("lib.zig"), "").unwrap();

        let root_dep = resolve_lsp_module_root(
            "zig",
            &ws.to_string_lossy(),
            Some(&dep.join("lib.zig").to_string_lossy()),
            &[],
        )
        .unwrap();
        assert_eq!(root_dep, ws.to_string_lossy());
    }
}
