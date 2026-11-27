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
    let want_test = req.launch.is_test
        || req.launch.test_name.is_some()
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
        let build_zig_src = if has_build_zig {
            std::fs::read_to_string(Path::new(&zig_cwd).join("build.zig")).ok()
        } else {
            None
        };
        let build_graph = build_zig_src
            .as_deref()
            .map(|src| parse_build_zig_module_graph(src, Path::new(&zig_cwd)))
            .unwrap_or_default();
        let has_test_step = build_zig_src.as_deref().is_some_and(|src| {
            crate::environments::discovery::parse_zig_build_steps(src)
                .iter()
                .any(|s| s == "test")
        });
        let selected_filter = req
            .launch
            .test_name
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty());

        // Project-wide Debug All Tests (`!is_zig_file`) or single-test Debug inside a
        // `build.zig` project with a `test` step: compile via Zig's `--build-runner` hook
        // with `OTTO_ZIG_TEST_NO_EXEC=1` so the real `build.zig` module graph compiles
        // WITHOUT executing `addRunArtifact` (BUG2-1), resolves warm-cached binaries of
        // any age (BUG-5), and applies `compile.filters` when debugging a single test (BUG-35).
        let try_build_runner = (!is_zig_file
            && (has_build_zig || req.target.kind == IdeTargetKind::Ssh))
            || (is_zig_file && has_build_zig && has_test_step);
        if try_build_runner {
            if let Some(runner_path) =
                prepare_zig_build_runner_for_target(req, ctx, &zig_bin, &zig_cwd, env, data_dir)
                    .await
            {
                let mut runner_env = env.clone();
                runner_env.insert("OTTO_ZIG_TEST_NO_EXEC".into(), "1".into());
                if let Some(tname) = selected_filter {
                    runner_env.insert("OTTO_ZIG_TEST_FILTER".into(), tname.to_string());
                }
                if is_zig_file {
                    runner_env.insert("OTTO_ZIG_TEST_FILE".into(), raw_prog.to_string());
                }
                let mut build_args = vec![
                    "build".to_string(),
                    "--build-runner".to_string(),
                    runner_path,
                    "test".to_string(),
                ];
                build_args.extend(extra_flags.clone());
                match run_zig_build_with_macos_fallback(
                    req,
                    ctx,
                    &zig_bin,
                    build_args,
                    &zig_cwd,
                    &runner_env,
                    true,
                    initial_output,
                )
                .await
                {
                    Ok(stdout) => {
                        let bins = parse_otto_zig_test_bins(&stdout);
                        if let Some(bin) = bins.into_iter().next() {
                            let abs = if bin.starts_with('/') {
                                bin
                            } else {
                                format!("{}/{}", zig_cwd.trim_end_matches('/'), bin)
                            };
                            return Ok(abs);
                        }
                        if !is_zig_file {
                            return Err(Error::Upstream(format!(
                                "zig build test did not emit a test executable in '{zig_cwd}'"
                            )));
                        }
                    }
                    Err(err) if !is_zig_file => return Err(err),
                    Err(_) => {
                        // Single-file test in a project whose build.zig test step didn't
                        // cover this file (or macOS host linker mismatch): fall through to
                        // direct `zig test --test-no-exec` with the full transitive module graph.
                    }
                }
            } else if !is_zig_file {
                return Err(Error::Upstream(format!(
                    "could not prepare Zig no-exec test build runner for '{zig_bin}'; cannot debug project test suite without executing tests first"
                )));
            }
        }

        if !is_zig_file {
            return Err(Error::Invalid(
                "Zig test debugging requires a build.zig test step or a .zig source file".into(),
            ));
        }

        let test_file = raw_prog.to_string();
        let out_bin =
            zig_debug_cache_bin_path(data_dir, req.target.kind, &zig_cwd, &test_file, true)?;
        let mut args = vec![
            "test".to_string(),
            "--test-no-exec".to_string(),
            format!("-femit-bin={out_bin}"),
        ];
        if let Some(tname) = selected_filter {
            args.push("--test-filter".into());
            args.push(tname.to_string());
        }
        args.extend(extra_flags);
        append_zig_source_and_modules(&mut args, &test_file, &build_graph);

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
            Ok(_) => {
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
) -> Result<String> {
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
    let stdout_txt = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr_txt = String::from_utf8_lossy(&out.stderr).to_string();
    if out.status.success() {
        if !stderr_txt.trim().is_empty() {
            initial_output.push(IdeDebugOutputEntry {
                category: "console".into(),
                text: stderr_txt,
                seq: 0,
            });
        }
        return Ok(stdout_txt);
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
            // Insert before the first `--dep` / `-M` / trailing source file path.
            let insert_at = retry_args
                .iter()
                .position(|a| a == "--dep" || a.starts_with("-M"))
                .unwrap_or_else(|| retry_args.len().saturating_sub(1));
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
        let retry_out = String::from_utf8_lossy(&retry.stdout).to_string();
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
            return Ok(retry_out);
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
    if req.target.kind == IdeTargetKind::Ssh {
        candidates.push("$HOME/.otto/toolchains/zig/current/zig".to_string());
        candidates.push("$HOME/.otto/toolchains/zig/current/bin/zig".to_string());
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

/// Transitive module-graph node extracted from `build.zig` (BUG-35).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ZigModuleNode {
    pub name: String,
    pub path: String,
    pub deps: Vec<String>,
}

/// Extracted `build.zig` module graph (`root_deps` for `-Mroot`, plus each child module
/// with its own transitive `deps` so `--dep` flags precede the owning `-M<mod>`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct ZigModuleGraph {
    pub root_deps: Vec<String>,
    pub modules: Vec<ZigModuleNode>,
}

/// Extract named local Zig modules and transitive module-to-module imports declared in
/// `build.zig` (`b.createModule`, `b.addModule`, `.addImport`, inline `.imports = &.{...}`)
/// so single-file `zig test` builds preserve multi-hop module graphs (BUG-35).
#[allow(clippy::disallowed_methods)]
pub(crate) fn parse_build_zig_module_graph(build_zig: &str, zig_cwd: &Path) -> ZigModuleGraph {
    use regex::Regex;
    use std::sync::OnceLock;
    static RE_CREATE: OnceLock<Regex> = OnceLock::new();
    static RE_ADD_MOD_VAR: OnceLock<Regex> = OnceLock::new();
    static RE_ADD_MOD: OnceLock<Regex> = OnceLock::new();
    static RE_ADD_IMPORT: OnceLock<Regex> = OnceLock::new();
    static RE_INLINE_IMPORT: OnceLock<Regex> = OnceLock::new();

    let re_create = RE_CREATE.get_or_init(|| {
        Regex::new(r#"(?s)const\s+([A-Za-z_]\w*)\s*=\s*b\.createModule\s*\(\.\{(.*?)\}\s*\)"#)
            .unwrap()
    });
    let re_add_mod_var = RE_ADD_MOD_VAR.get_or_init(|| {
        Regex::new(
            r#"(?s)const\s+([A-Za-z_]\w*)\s*=\s*b\.addModule\(\s*"([^"]+)"\s*,\s*\.\{(.*?)\}\s*\)"#,
        )
        .unwrap()
    });
    let re_add_mod = RE_ADD_MOD.get_or_init(|| {
        Regex::new(r#"(?s)b\.addModule\(\s*"([^"]+)"\s*,\s*\.\{[^}]*?\.root_source_file\s*=\s*b\.path\(\s*"([^"]+)"\s*\)"#)
            .unwrap()
    });
    let re_root_src = Regex::new(r#"\.root_source_file\s*=\s*b\.path\(\s*"([^"]+)"\s*\)"#).unwrap();
    let re_add_import = RE_ADD_IMPORT.get_or_init(|| {
        Regex::new(r#"([A-Za-z_]\w*(?:\.[A-Za-z_]\w*)*)\.addImport\(\s*"([^"]+)"\s*,\s*([A-Za-z_]\w*)\s*\)"#)
            .unwrap()
    });
    let re_inline_import = RE_INLINE_IMPORT.get_or_init(|| {
        Regex::new(r#"(?s)\.\{\s*\.name\s*=\s*"([^"]+)"\s*,\s*\.module\s*=\s*([A-Za-z_]\w*)"#)
            .unwrap()
    });

    // var_name -> (abs_path, canonical_import_name_opt)
    let mut vars: BTreeMap<String, (String, Option<String>)> = BTreeMap::new();
    // (owner_var_opt, import_name, target_var)
    let mut raw_edges: Vec<(Option<String>, String, String)> = Vec::new();

    for cap in re_create.captures_iter(build_zig) {
        let var_name = cap[1].to_string();
        let body = &cap[2];
        if let Some(src_cap) = re_root_src.captures(body) {
            let abs = zig_cwd.join(&src_cap[1]);
            if abs.is_file() {
                vars.insert(var_name.clone(), (abs.to_string_lossy().into_owned(), None));
            }
        }
        for icap in re_inline_import.captures_iter(body) {
            raw_edges.push((
                Some(var_name.clone()),
                icap[1].to_string(),
                icap[2].to_string(),
            ));
        }
    }
    for cap in re_add_mod_var.captures_iter(build_zig) {
        let var_name = cap[1].to_string();
        let mod_name = cap[2].to_string();
        let body = &cap[3];
        if let Some(src_cap) = re_root_src.captures(body) {
            let abs = zig_cwd.join(&src_cap[1]);
            if abs.is_file() {
                vars.insert(
                    var_name.clone(),
                    (abs.to_string_lossy().into_owned(), Some(mod_name)),
                );
            }
        }
        for icap in re_inline_import.captures_iter(body) {
            raw_edges.push((
                Some(var_name.clone()),
                icap[1].to_string(),
                icap[2].to_string(),
            ));
        }
    }

    for cap in re_add_import.captures_iter(build_zig) {
        let recv = &cap[1];
        let imp_name = cap[2].to_string();
        let target_var = cap[3].to_string();
        let owner = if !recv.contains('.') && vars.contains_key(recv) {
            Some(recv.to_string())
        } else {
            None
        };
        raw_edges.push((owner, imp_name, target_var));
    }

    // Assign each var its registered import name (defaulting to var_name if unnamed).
    let mut var_to_mod_name: BTreeMap<String, String> = BTreeMap::new();
    for (v, (_, explicit_name)) in &vars {
        if let Some(n) = explicit_name {
            var_to_mod_name.insert(v.clone(), n.clone());
        }
    }
    for (_, imp_name, target_var) in &raw_edges {
        if vars.contains_key(target_var) {
            var_to_mod_name
                .entry(target_var.clone())
                .or_insert_with(|| imp_name.clone());
        }
    }

    let mut graph = ZigModuleGraph::default();
    let ensure_node = |modules: &mut Vec<ZigModuleNode>, name: &str, path: &str| {
        if !name.is_empty() && !modules.iter().any(|m| m.name == name) {
            modules.push(ZigModuleNode {
                name: name.to_string(),
                path: path.to_string(),
                deps: Vec::new(),
            });
        }
    };

    for cap in re_add_mod.captures_iter(build_zig) {
        let name = &cap[1];
        let abs = zig_cwd.join(&cap[2]);
        if abs.is_file() {
            ensure_node(&mut graph.modules, name, &abs.to_string_lossy());
            if !graph.root_deps.iter().any(|d| d == name) {
                graph.root_deps.push(name.to_string());
            }
        }
    }

    for (owner_opt, imp_name, target_var) in raw_edges {
        let Some((target_path, _)) = vars.get(&target_var) else {
            continue;
        };
        let target_mod_name = var_to_mod_name
            .get(&target_var)
            .cloned()
            .unwrap_or_else(|| imp_name.clone());
        ensure_node(&mut graph.modules, &target_mod_name, target_path);
        let dep_spec = if imp_name == target_mod_name {
            target_mod_name.clone()
        } else {
            format!("{imp_name}={target_mod_name}")
        };
        if let Some(owner_var) = owner_opt {
            if let Some((owner_path, _)) = vars.get(&owner_var) {
                let owner_mod_name = var_to_mod_name
                    .get(&owner_var)
                    .cloned()
                    .unwrap_or(owner_var);
                ensure_node(&mut graph.modules, &owner_mod_name, owner_path);
                if let Some(node) = graph.modules.iter_mut().find(|m| m.name == owner_mod_name) {
                    if !node.deps.iter().any(|d| d == &dep_spec) {
                        node.deps.push(dep_spec);
                    }
                }
            }
        } else if !graph.root_deps.iter().any(|d| d == &dep_spec) {
            graph.root_deps.push(dep_spec);
        }
    }

    // If a build.zig created modules and added imports only between them (or on a root_mod var
    // whose root_source_file matches the main test file), ensure root_deps isn't left empty when
    // modules exist.
    if graph.root_deps.is_empty() && !graph.modules.is_empty() {
        for m in &graph.modules {
            graph.root_deps.push(m.name.clone());
        }
    }

    graph
}

/// Append Zig source and full transitive `-M` / `--dep` flags (`--dep` immediately
/// precedes the `-M<name>` module that owns those imports) (BUG-35).
pub(crate) fn append_zig_source_and_modules(
    args: &mut Vec<String>,
    root_file: &str,
    graph: &ZigModuleGraph,
) {
    if graph.modules.is_empty() {
        args.push(root_file.to_string());
        return;
    }
    for dep in &graph.root_deps {
        args.push("--dep".into());
        args.push(dep.clone());
    }
    args.push(format!("-Mroot={root_file}"));
    for module in &graph.modules {
        for dep in &module.deps {
            args.push("--dep".into());
            args.push(dep.clone());
        }
        args.push(format!("-M{}={}", module.name, module.path));
    }
}

const OTTO_ZIG_RUNNER_PRE_HOOK: &str = r#"
    var otto_test_compiles = ArrayList(*Step.Compile).init(arena);
    const otto_no_exec = if (graph.env_map.get("OTTO_ZIG_TEST_NO_EXEC")) |v| mem.eql(u8, v, "1") else false;
    const otto_filter = graph.env_map.get("OTTO_ZIG_TEST_FILTER");
    const otto_file = graph.env_map.get("OTTO_ZIG_TEST_FILE");
    if (otto_no_exec or (otto_filter != null and otto_filter.?.len > 0)) {
        const OttoWalker = struct {
            seen: std.AutoHashMap(*Step, void),
            out: *ArrayList(*Step.Compile),
            fn visit(self: *@This(), s: *Step) !void {
                if (self.seen.contains(s)) return;
                try self.seen.put(s, {});
                if (s.cast(Step.Compile)) |c| {
                    if (c.kind == .@"test") {
                        try self.out.append(c);
                    }
                }
                for (s.dependencies.items) |dep| {
                    try self.visit(dep);
                }
            }
        };
        var walker = OttoWalker{
            .seen = std.AutoHashMap(*Step, void).init(arena),
            .out = &otto_test_compiles,
        };
        if (targets.items.len > 0) {
            for (targets.items) |tname| {
                if (builder.top_level_steps.get(tname)) |tls| {
                    try walker.visit(&tls.step);
                }
            }
        } else if (builder.top_level_steps.get("test")) |tls| {
            try walker.visit(&tls.step);
        }
        if (otto_file) |want_file| {
            if (want_file.len > 0 and otto_test_compiles.items.len > 1) {
                var matched = ArrayList(*Step.Compile).init(arena);
                for (otto_test_compiles.items) |c| {
                    if (c.root_module.root_source_file) |lp| {
                        const rp = lp.getPath(builder);
                        if (mem.eql(u8, rp, want_file) or mem.endsWith(u8, want_file, rp) or mem.endsWith(u8, rp, want_file)) {
                            try matched.append(c);
                        }
                    }
                }
                if (matched.items.len > 0) {
                    otto_test_compiles = matched;
                }
            }
        }
        if (otto_filter) |f| {
            if (f.len > 0) {
                const slice = try arena.dupe([]const u8, &.{f});
                for (otto_test_compiles.items) |c| {
                    c.filters = slice;
                }
            }
        }
        if (otto_no_exec) {
            if (otto_test_compiles.items.len == 0) {
                fatal("otto: no Zig test compile step found in build.zig", .{});
            }
            const compile_only = builder.step("__otto_compile_tests", "Compile Zig tests without running");
            for (otto_test_compiles.items) |c| {
                compile_only.dependOn(&c.step);
            }
            targets.clearRetainingCapacity();
            try targets.append("__otto_compile_tests");
        }
    }
"#;

const OTTO_ZIG_RUNNER_POST_HOOK: &str = r#"
        if (otto_no_exec) {
            for (otto_test_compiles.items) |c| {
                const bp = c.getEmittedBin().getPath(builder);
                try stdout_writer.print("OTTO_ZIG_TEST_BIN:{s}\n", .{bp});
            }
        }
"#;

/// Patch Zig's own `compiler/build_runner.zig` so `zig build --build-runner <path> test`
/// compiles the project's test `Step.Compile` artifact(s) with optional `compile.filters`
/// WITHOUT executing `Step.Run` (`addRunArtifact`) and prints `OTTO_ZIG_TEST_BIN:<path>`
/// even on warm cache hits (BUG-5, BUG2-1, BUG-35, BUG2-2).
pub(crate) fn patch_zig_build_runner(orig_source: &str) -> Option<String> {
    const PRE_ANCHOR: &str = "if (steps_menu)";
    const POST_ANCHOR: &str = "if (!watch) return cleanExit();";
    if !orig_source.contains(PRE_ANCHOR) || !orig_source.contains(POST_ANCHOR) {
        return None;
    }
    let with_pre = orig_source.replacen(
        PRE_ANCHOR,
        &format!("{OTTO_ZIG_RUNNER_PRE_HOOK}\n    {PRE_ANCHOR}"),
        1,
    );
    Some(with_pre.replacen(
        POST_ANCHOR,
        &format!("{OTTO_ZIG_RUNNER_POST_HOOK}\n        {POST_ANCHOR}"),
        1,
    ))
}

/// Extract emitted test binary paths from `OTTO_ZIG_TEST_BIN:<path>` lines printed by
/// our patched Zig build runner.
pub(crate) fn parse_otto_zig_test_bins(stdout: &str) -> Vec<String> {
    stdout
        .lines()
        .filter_map(|line| line.trim().strip_prefix("OTTO_ZIG_TEST_BIN:"))
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .map(String::from)
        .collect()
}

fn parse_zig_env_lib_dir(zig_env_stdout: &str) -> Option<String> {
    for line in zig_env_stdout.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed
            .strip_prefix("\"lib_dir\":")
            .or_else(|| trimmed.strip_prefix(".lib_dir ="))
        {
            let val = rest.trim().trim_end_matches(',').trim();
            if let Some(inner) = val.strip_prefix('"').and_then(|s| s.strip_suffix('"')) {
                if !inner.is_empty() {
                    return Some(inner.to_string());
                }
            }
        }
    }
    None
}

/// Resolve and cache the patched `otto_build_runner.zig` for a local Zig binary.
#[allow(clippy::disallowed_methods)]
pub(crate) fn prepare_local_zig_build_runner(zig_bin: &str, cache_dir: &Path) -> Option<PathBuf> {
    let mut runner_src_path: Option<PathBuf> = None;
    let bin_p = Path::new(zig_bin);
    if bin_p.is_absolute() {
        let resolved = std::fs::canonicalize(bin_p).unwrap_or_else(|_| bin_p.to_path_buf());
        if let Some(dir) = resolved.parent() {
            for cand in [
                dir.join("lib/zig/compiler/build_runner.zig"),
                dir.join("lib/compiler/build_runner.zig"),
                dir.parent()
                    .map(|p| p.join("lib/zig/compiler/build_runner.zig"))
                    .unwrap_or_default(),
            ] {
                if cand.is_file() {
                    runner_src_path = Some(cand);
                    break;
                }
            }
        }
    }
    if runner_src_path.is_none() {
        if let Ok(out) = std::process::Command::new(zig_bin).arg("env").output() {
            if out.status.success() {
                if let Some(lib_dir) = parse_zig_env_lib_dir(&String::from_utf8_lossy(&out.stdout))
                {
                    let cand = PathBuf::from(lib_dir).join("compiler/build_runner.zig");
                    if cand.is_file() {
                        runner_src_path = Some(cand);
                    }
                }
            }
        }
    }
    let orig = std::fs::read_to_string(runner_src_path?).ok()?;
    let patched = patch_zig_build_runner(&orig)?;
    let mut h = Sha256::new();
    h.update(patched.as_bytes());
    let key = hex::encode(h.finalize());
    let dir = cache_dir.join("zig-build-runners");
    let _ = std::fs::create_dir_all(&dir);
    let dest = dir.join(format!("runner-{}.zig", &key[..16]));
    if !dest.is_file() {
        let tmp = dir.join(format!(".runner-{}.{}.tmp", &key[..16], std::process::id()));
        if std::fs::write(&tmp, patched.as_bytes()).is_ok() {
            let _ = std::fs::rename(&tmp, &dest);
        }
    }
    dest.is_file().then_some(dest)
}

async fn prepare_zig_build_runner_for_target(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    zig_bin: &str,
    zig_cwd: &str,
    env: &BTreeMap<String, String>,
    data_dir: &Path,
) -> Option<String> {
    if req.target.kind == IdeTargetKind::Local {
        let zb = zig_bin.to_string();
        let dd = data_dir.to_path_buf();
        return tokio::task::spawn_blocking(move || prepare_local_zig_build_runner(&zb, &dd))
            .await
            .ok()
            .flatten()
            .map(|p| p.to_string_lossy().into_owned());
    }
    prepare_ssh_zig_build_runner(&req.target, ctx.ssh.as_ref(), zig_bin, zig_cwd, env).await
}

/// Prepare and cache the patched `otto-zig-runner-*.zig` on an SSH host so remote
/// `zig build --build-runner` supports both no-exec debug builds (BUG-5/BUG2-1) and
/// filtered single-test gutter runs (BUG2-2/BUG-35).
pub(crate) async fn prepare_ssh_zig_build_runner(
    target: &IdeTargetSpec,
    ssh: Option<&crate::types::IdeSshSpec>,
    zig_bin: &str,
    zig_cwd: &str,
    env: &BTreeMap<String, String>,
) -> Option<String> {
    // Remote SSH: read `<lib_dir>/compiler/build_runner.zig`, patch in Rust, write to remote /tmp.
    let env_out = run_target_command(
        target,
        ssh,
        zig_bin,
        &["env".into()],
        zig_cwd,
        env,
        crate::proc_group::BUILD_STEP_TIMEOUT,
    )
    .await
    .ok()?;
    if !env_out.status.success() {
        return None;
    }
    let lib_dir = parse_zig_env_lib_dir(&String::from_utf8_lossy(&env_out.stdout))?;
    let remote_orig = format!(
        "{}/compiler/build_runner.zig",
        lib_dir.trim_end_matches('/')
    );
    let cat_out = run_target_command(
        target,
        ssh,
        "cat",
        &[remote_orig],
        zig_cwd,
        env,
        crate::proc_group::BUILD_STEP_TIMEOUT,
    )
    .await
    .ok()?;
    if !cat_out.status.success() {
        return None;
    }
    let orig = String::from_utf8_lossy(&cat_out.stdout);
    let patched = patch_zig_build_runner(&orig)?;
    let mut h = Sha256::new();
    h.update(patched.as_bytes());
    let key = hex::encode(h.finalize());
    let remote_dest = format!("/tmp/otto-zig-runner-{}.zig", &key[..16]);
    let write_script = format!(
        "if [ ! -f {d} ]; then cat <<'__OTTO_ZIG_RUNNER__' > {d}.$$\n{patched}\n__OTTO_ZIG_RUNNER__\nmv -f {d}.$$ {d}; fi",
        d = shell_words::quote(&remote_dest),
    );
    let wr = run_target_command(
        target,
        ssh,
        "sh",
        &["-c".into(), write_script],
        zig_cwd,
        env,
        crate::proc_group::BUILD_STEP_TIMEOUT,
    )
    .await
    .ok()?;
    wr.status.success().then_some(remote_dest)
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
        // BUG-1: Must not recommend `brew install zls` (installs incompatible 0.17.x).
        assert_eq!(zls.install_command, None);
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

    #[test]
    fn zig_bug5_bug35_bug36_bug7_bug2_2_regression_suite() {
        use std::collections::BTreeMap;

        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("my_zig_app");
        std::fs::create_dir_all(proj.join("src")).unwrap();

        let build_zig = r#"
const std = @import("std");
pub fn build(b: *std.Build) void {
    const leaf_mod = b.createModule(.{
        .root_source_file = b.path("src/leaf.zig"),
    });
    const mid_mod = b.createModule(.{
        .root_source_file = b.path("src/mid.zig"),
    });
    mid_mod.addImport("leaf", leaf_mod);
    const exe = b.addExecutable(.{ .name = "my_zig_app", .root_source_file = b.path("src/main.zig") });
    exe.root_module.addImport("mid", mid_mod);
    const run_step = b.step("run", "Run the app");
    const test_step = b.step("test", "Run unit tests");
    _ = .{ run_step, test_step };
}
"#;
        std::fs::write(proj.join("build.zig"), build_zig).unwrap();
        std::fs::write(
            proj.join("src/leaf.zig"),
            "pub fn val() i32 { return 42; }\n",
        )
        .unwrap();
        std::fs::write(
            proj.join("src/mid.zig"),
            "const leaf = @import(\"leaf\");\npub fn answer() i32 { return leaf.val(); }\n",
        )
        .unwrap();
        let main_src = r#"const std = @import("std");
const mid = @import("mid");
pub fn main() void { _ = mid.answer(); }
test "uses mid" { try std.testing.expectEqual(@as(i32, 42), mid.answer()); }
test "failing sibling" { try std.testing.expectEqual(@as(i32, 1), 2); }
"#;
        let main_path = proj.join("src/main.zig");
        std::fs::write(&main_path, main_src).unwrap();

        // BUG-35: Gutter actions inside a build.zig project use `zig build run` / `zig build test`
        // while standalone files retain direct `zig run` / `zig test`.
        let proj_gutters =
            crate::gutter::detect_gutter_items(&main_path.to_string_lossy(), main_src, &[]);
        let main_g = proj_gutters.iter().find(|g| g.label == "main").unwrap();
        assert_eq!(
            main_g.run_spec.runner,
            crate::types::IdeRunnerKind::ZigBuild
        );
        assert_eq!(main_g.run_spec.args, vec!["build", "run"]);
        let test_g = proj_gutters.iter().find(|g| g.label == "uses mid").unwrap();
        assert_eq!(test_g.run_spec.runner, crate::types::IdeRunnerKind::ZigTest);
        assert_eq!(test_g.run_spec.args, vec!["build", "test"]);
        assert_eq!(test_g.run_spec.test_filter.as_deref(), Some("uses mid"));

        let solo_dir = tmp.path().join("standalone");
        std::fs::create_dir_all(&solo_dir).unwrap();
        let solo_path = solo_dir.join("solo.zig");
        std::fs::write(&solo_path, main_src).unwrap();
        let solo_gutters =
            crate::gutter::detect_gutter_items(&solo_path.to_string_lossy(), main_src, &[]);
        let solo_main = solo_gutters.iter().find(|g| g.label == "main").unwrap();
        assert_eq!(
            solo_main.run_spec.runner,
            crate::types::IdeRunnerKind::ZigRun
        );
        assert_eq!(solo_main.run_spec.args[0], "run");

        // BUG-35: Multi-hop transitive module graph (`root -> mid -> leaf`) emits each module's
        // own `--dep` immediately before its `-M<name>=<path>` (never all `--dep` on root).
        let graph = super::parse_build_zig_module_graph(build_zig, &proj);
        assert_eq!(graph.root_deps, vec!["mid"]);
        let mid_node = graph.modules.iter().find(|m| m.name == "mid").unwrap();
        assert_eq!(mid_node.deps, vec!["leaf"]);
        let mut argv = vec!["test".to_string(), "--test-no-exec".to_string()];
        super::append_zig_source_and_modules(&mut argv, &main_path.to_string_lossy(), &graph);
        let root_pos = argv.iter().position(|a| a.starts_with("-Mroot=")).unwrap();
        let mid_pos = argv.iter().position(|a| a.starts_with("-Mmid=")).unwrap();
        let leaf_dep_pos = argv.iter().position(|a| a == "leaf").unwrap();
        assert!(
            leaf_dep_pos > root_pos && leaf_dep_pos < mid_pos,
            "leaf dep must belong to mid (between -Mroot and -Mmid), got: {argv:?}"
        );

        // BUG2-2: Running a single gutter test in a build.zig project must NOT drop the
        // selected test case into unfiltered `zig build test` (which would run failing siblings).
        let gutter_cfg = crate::types::IdeRunConfig {
            id: "gutter-zig-test".into(),
            name: "uses mid".into(),
            runner: crate::types::IdeRunnerKind::ZigTest,
            args: vec!["build".into(), "test".into()],
            cwd: Some(proj.to_string_lossy().into_owned()),
            target_path: Some(main_path.to_string_lossy().into_owned()),
            test_filter: Some("uses mid".into()),
            ..Default::default()
        };
        let (g_prog, g_args) = crate::environments::runner_cmd::build_zig_runner_cmd(
            crate::types::IdeRunnerKind::ZigTest,
            &gutter_cfg,
            &proj.to_string_lossy(),
            None,
            gutter_cfg.args.clone(),
            vec![],
            Some("uses mid"),
        );
        assert_eq!(g_prog, "zig");
        assert!(
            g_args.contains(&"--test-filter".to_string())
                && g_args.contains(&"uses mid".to_string())
                && g_args.iter().any(|a| a.starts_with("-Mmid="))
                && g_args.iter().any(|a| a.starts_with("-Mleaf=")),
            "BUG2-2: single gutter test must preserve --test-filter and transitive modules, got: {g_args:?}"
        );

        // When runner.rs injects `--build-runner` (with OTTO_ZIG_TEST_FILTER in env), keep `zig build`.
        let (_, br_args) = crate::environments::runner_cmd::build_zig_runner_cmd(
            crate::types::IdeRunnerKind::ZigTest,
            &gutter_cfg,
            &proj.to_string_lossy(),
            None,
            vec![
                "build".into(),
                "--build-runner".into(),
                "/tmp/runner.zig".into(),
                "test".into(),
            ],
            vec![],
            Some("uses mid"),
        );
        assert_eq!(
            br_args,
            vec!["build", "--build-runner", "/tmp/runner.zig", "test"]
        );

        // BUG-36: SSH Run All Tests (remote cwd absent on local disk, no `.zig` file, no filter)
        // must emit `zig build test` rather than bare `zig test`.
        let cfg = crate::types::IdeRunConfig {
            id: "ssh-zig-test".into(),
            name: "Test All".into(),
            runner: crate::types::IdeRunnerKind::ZigTest,
            cwd: Some("/nonexistent/remote/zig-project".into()),
            ..Default::default()
        };
        let (prog, cmd_args) = crate::environments::runner_cmd::build_zig_runner_cmd(
            crate::types::IdeRunnerKind::ZigTest,
            &cfg,
            "/nonexistent/remote/zig-project",
            None,
            vec![],
            vec![],
            None,
        );
        assert_eq!(prog, "zig");
        assert_eq!(cmd_args, vec!["build", "test"]);

        // BUG-7: Shared SSH exec wrapper exposes managed Zig toolchain dirs on PATH.
        let ssh_cmd = crate::target::ssh_exec::build_remote_exec_line(
            "/remote/zig",
            "zig",
            &["version".into()],
            &BTreeMap::new(),
        );
        assert!(
            ssh_cmd.contains("\"$HOME\"/.otto/toolchains/zig/current"),
            "SSH exec script missing managed zig/current PATH: {ssh_cmd}"
        );
    }

    #[tokio::test]
    async fn zig_debug_all_tests_uses_no_exec_build_runner_and_resolves_warm_cached_binary() {
        use std::collections::BTreeMap;
        use std::os::unix::fs::PermissionsExt;

        // BUG-5 + BUG2-1: Project-wide Debug All Tests must compile via `--build-runner` with
        // `OTTO_ZIG_TEST_NO_EXEC=1` (never running `addRunArtifact` before DAP) and return the
        // emitted test executable even when it is a warm cache hit older than 2 seconds, never
        // falling back to standalone `src/main.zig` or `zig-out/bin/` app.
        let tmp = tempfile::tempdir().unwrap();
        let data_dir = tmp.path().join("data");
        let fake_zig_dir = tmp.path().join("fake-zig");
        let fake_lib = fake_zig_dir.join("lib/zig/compiler");
        std::fs::create_dir_all(&data_dir).unwrap();
        std::fs::create_dir_all(&fake_lib).unwrap();
        std::fs::write(
            fake_lib.join("build_runner.zig"),
            "pub fn main() !void {\n    if (steps_menu) return;\n    if (!watch) return cleanExit();\n}\n",
        )
        .unwrap();

        let proj = tmp.path().join("proj");
        std::fs::create_dir_all(proj.join("zig-out/bin")).unwrap();
        std::fs::create_dir_all(proj.join(".zig-cache/o/warm123")).unwrap();
        std::fs::write(
            proj.join("build.zig"),
            "pub fn build(b: *std.Build) void { _ = b.step(\"test\", \"tests\"); }\n",
        )
        .unwrap();
        let app_bin = proj.join("zig-out/bin/proj");
        let warm_test_bin = proj.join(".zig-cache/o/warm123/test");
        for p in [&app_bin, &warm_test_bin] {
            std::fs::write(p, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // Make the cached test binary old (1 hour ago) — old 2s mtime scan would reject it!
        #[allow(clippy::disallowed_methods)]
        let _ = std::process::Command::new("touch")
            .args(["-t", "202001010000", &warm_test_bin.to_string_lossy()])
            .status();

        let fake_zig = fake_zig_dir.join("zig");
        let marker = tmp.path().join("invoked.txt");
        let fake_zig_script = format!(
            "#!/bin/sh\n\
             printf '%s|%s\\n' \"$OTTO_ZIG_TEST_NO_EXEC\" \"$*\" > \"{}\"\n\
             if [ \"$OTTO_ZIG_TEST_NO_EXEC\" != \"1\" ]; then echo 'missing NO_EXEC' >&2; exit 2; fi\n\
             case \"$*\" in *--build-runner*) ;; *) echo 'missing --build-runner' >&2; exit 3 ;; esac\n\
             echo 'OTTO_ZIG_TEST_BIN:{}'\n",
            marker.display(),
            warm_test_bin.display()
        );
        std::fs::write(&fake_zig, fake_zig_script).unwrap();
        std::fs::set_permissions(&fake_zig, std::fs::Permissions::from_mode(0o755)).unwrap();

        let req = crate::types::IdeStartDebugReq {
            target: crate::types::IdeTargetSpec::local(proj.to_string_lossy().into_owned()),
            language: "zig".into(),
            launch: crate::types::IdeDebugLaunchSpec {
                program: proj.to_string_lossy().into_owned(),
                cwd: Some(proj.to_string_lossy().into_owned()),
                is_test: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut env = BTreeMap::new();
        env.insert(
            "ZIG_HOME".into(),
            fake_zig_dir.to_string_lossy().into_owned(),
        );
        env.insert(
            "PATH".into(),
            format!("{}:/usr/bin:/bin", fake_zig_dir.display()),
        );
        let ctx = super::DapStartContext {
            caller_is_root: true,
            ..Default::default()
        };
        let mut out = Vec::new();
        let resolved_bin = super::compile_zig_debug_artifact(
            &data_dir,
            &req,
            &ctx,
            &proj.to_string_lossy(),
            &proj.to_string_lossy(),
            true,
            &env,
            &mut out,
        )
        .await
        .expect("no-exec build runner must return warm-cached test binary");
        assert_eq!(resolved_bin, warm_test_bin.to_string_lossy());
        assert_ne!(resolved_bin, app_bin.to_string_lossy());

        let invoked = std::fs::read_to_string(&marker).unwrap();
        assert!(invoked.starts_with("1|build --build-runner "));
        // Verify the generated runner actually strips Step.Run and emits OTTO_ZIG_TEST_BIN.
        let runner_path = invoked
            .split_whitespace()
            .skip_while(|t| *t != "--build-runner")
            .nth(1)
            .expect("runner path in argv");
        let runner_src = std::fs::read_to_string(runner_path).unwrap();
        assert!(runner_src.contains("__otto_compile_tests"));
        assert!(runner_src.contains("OTTO_ZIG_TEST_BIN:"));
    }
}
