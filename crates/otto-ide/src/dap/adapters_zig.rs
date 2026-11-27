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
use super::{
    run_target_command, ssh_private_cache_dir, ssh_private_cache_dir_script, DapStartContext,
};
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
            None,
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

/// Debug All Tests of a `build.zig` project with several test suites: every
/// suite is compiled once (nothing runs, BUG2-1) and debugged in its own step,
/// `src/main.zig`'s suite last. `None` for one suite, a single file or test,
/// and an attach.
pub(crate) async fn plan_zig_test_steps(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    cwd: &str,
    env: BTreeMap<String, String>,
) -> Result<Option<super::session::DebugPlan>> {
    use super::session::{DebugPlan, PlannedStep, StepKind};
    let raw_prog = normalize_target_path(&req.target, &req.launch.program);
    let want_test = req.launch.is_test
        || ctx
            .run_config
            .as_ref()
            .is_some_and(|rc| rc.runner == IdeRunnerKind::ZigTest);
    let filtered = req
        .launch
        .test_name
        .as_deref()
        .is_some_and(|t| !t.trim().is_empty());
    let is_zig_file = raw_prog.ends_with(".zig") && !raw_prog.ends_with("/build.zig");
    let prebuilt = !raw_prog.is_empty()
        && !raw_prog.ends_with(".zig")
        && !raw_prog.ends_with(".zon")
        && (req.target.kind == IdeTargetKind::Ssh || Path::new(&raw_prog).is_file());
    if !want_test || filtered || is_zig_file || prebuilt {
        return Ok(None);
    }
    let mut output = Vec::new();
    let mut bins = Vec::new();
    compile_zig_debug_artifact(
        data_dir,
        req,
        ctx,
        cwd,
        &raw_prog,
        true,
        &env,
        &mut output,
        Some(&mut bins),
    )
    .await?;
    if bins.len() < 2 {
        return Ok(None);
    }
    bins.sort_by_key(|b| b.source.ends_with("src/main.zig"));
    // Zig prints canonical paths (`/private/var/…` for `/var/…`).
    let root = req.target.root.trim_end_matches('/').to_string();
    let canon_root = std::fs::canonicalize(&root)
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| root.clone());
    let steps: Vec<PlannedStep> = bins
        .into_iter()
        .map(|b| {
            let src = b
                .source
                .strip_prefix(canon_root.as_str())
                .or_else(|| b.source.strip_prefix(root.as_str()))
                .map_or(b.source.as_str(), |s| s.trim_start_matches('/'));
            let label = if src.is_empty() {
                b.name.clone()
            } else {
                format!("{} ({src})", b.name)
            };
            let mut step_req = req.clone();
            step_req.launch.program = b.path;
            step_req.launch.debug_step = true;
            PlannedStep {
                label,
                req: step_req,
                kind: StepKind::Zig,
                error: None,
                program: b.source,
            }
        })
        .collect();
    let title = format!("Debug all tests · {} test suites", steps.len());
    let output = output
        .into_iter()
        .flat_map(|o| o.text.lines().map(str::to_string).collect::<Vec<_>>())
        .collect();
    Ok(Some(DebugPlan {
        title,
        steps,
        output,
    }))
}

/// `zig version` of `zig_bin` on the target, for the "supports" message.
async fn zig_version(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    zig_bin: &str,
    cwd: &str,
    env: &BTreeMap<String, String>,
) -> String {
    let out = run_target_command(
        &req.target,
        ctx.ssh.as_ref(),
        zig_bin,
        &["version".to_string()],
        cwd,
        env,
        std::time::Duration::from_secs(10),
    )
    .await;
    out.ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|v| !v.is_empty())
        .unwrap_or_else(|| "a Zig version Otto can't read".into())
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
    all_bins: Option<&mut Vec<ZigEmittedTestBin>>,
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
        // SSH: the remote `build.zig` above the file decides the project graph, as locally.
        let remote_build = if req.target.kind == IdeTargetKind::Ssh && is_zig_file {
            find_remote_build_zig(req, ctx, raw_prog, env).await
        } else {
            None
        };
        let (zig_cwd, build_zig_src) = match remote_build {
            Some((dir, src)) => (dir, Some(src)),
            None if has_build_zig => {
                let src = std::fs::read_to_string(Path::new(&zig_cwd).join("build.zig")).ok();
                (zig_cwd.clone(), src)
            }
            None => (zig_cwd.clone(), None),
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
            || (is_zig_file && has_test_step);
        if try_build_runner {
            let Some(runner_path) =
                prepare_zig_build_runner_for_target(req, ctx, &zig_bin, &zig_cwd, env, data_dir)
                    .await
            else {
                let version = zig_version(req, ctx, &zig_bin, &zig_cwd, env).await;
                return Err(Error::Upstream(format!(
                    "Debug of build.zig tests supports Zig {OTTO_ZIG_HOOK_VERSIONS}; this project uses {version}. Run tests works for every version."
                )));
            };
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
            // BUG-35: a failing build keeps the real build.zig compiler graph's error
            // rather than silently substituting a partial source-only compile.
            let stdout = run_zig_build_with_macos_fallback(
                req,
                ctx,
                &zig_bin,
                build_args,
                &zig_cwd,
                &runner_env,
                true,
                initial_output,
            )
            .await?;
            let abs = |p: String| {
                if p.starts_with('/') {
                    p
                } else {
                    format!("{}/{}", zig_cwd.trim_end_matches('/'), p)
                }
            };
            let bins: Vec<ZigEmittedTestBin> = parse_otto_zig_test_bins(&stdout)
                .into_iter()
                .map(|b| ZigEmittedTestBin {
                    path: abs(b.path),
                    ..b
                })
                .collect();
            let Some(bin) = select_zig_test_bin(&bins, raw_prog, is_zig_file) else {
                return Err(Error::Upstream(format!(
                    "zig build test did not emit a test executable in '{zig_cwd}'"
                )));
            };
            if let Some(all) = all_bins {
                *all = bins;
            }
            return Ok(bin);
        }

        if !is_zig_file {
            return Err(Error::Invalid(
                "Zig test debugging requires a build.zig test step or a .zig source file".into(),
            ));
        }

        let test_file = raw_prog.to_string();
        let out_bin =
            zig_debug_out_bin(data_dir, req, ctx, &zig_cwd, env, &test_file, true).await?;
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

    let out_bin = zig_debug_out_bin(data_dir, req, ctx, &zig_cwd, env, raw_prog, false).await?;
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

/// Shell walk from `$1`'s directory up to the bound `$2` (`""` = `/`): prints
/// `ROOT:<dir>` and the nearest `build.zig`, or exits 1 when none is inside the bound.
const ZIG_FIND_BUILD_SH: &str = r#"d=$(dirname "$1")
while :; do
  case "$d" in "$2"|"$2"/*) ;; *) exit 1 ;; esac
  if [ -f "$d/build.zig" ]; then printf 'ROOT:%s\n' "$d"; exec cat "$d/build.zig"; fi
  [ "$d" = / ] && exit 1
  d=$(dirname "$d")
done"#;

/// SSH: the nearest remote `build.zig` above `file` inside the target root — its
/// directory and source (BUG-35).
async fn find_remote_build_zig(
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    file: &str,
    env: &BTreeMap<String, String>,
) -> Option<(String, String)> {
    let bound = req.target.root.trim_end_matches('/').to_string();
    let args = ["-c", ZIG_FIND_BUILD_SH, "sh", file, &bound].map(String::from);
    let out = run_target_command(
        &req.target,
        ctx.ssh.as_ref(),
        "sh",
        &args,
        &req.target.root,
        env,
        crate::proc_group::BUILD_STEP_TIMEOUT,
    )
    .await
    .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    let (first, src) = text.split_once('\n')?;
    let dir = first.strip_prefix("ROOT:")?;
    out.status
        .success()
        .then(|| (dir.to_string(), src.to_string()))
}

/// Debug executable path: Otto's data dir locally, the SSH user's private
/// `$HOME/.otto/cache/zig-debug` remotely (never the shared `/tmp`, SEC3-1).
async fn zig_debug_out_bin(
    data_dir: &Path,
    req: &IdeStartDebugReq,
    ctx: &DapStartContext,
    zig_cwd: &str,
    env: &BTreeMap<String, String>,
    source: &str,
    is_test: bool,
) -> Result<String> {
    let remote_dir = if req.target.kind == IdeTargetKind::Local {
        None
    } else {
        Some(ssh_private_cache_dir(&req.target, ctx.ssh.as_ref(), "zig-debug", zig_cwd, env).await?)
    };
    zig_debug_cache_bin_path(data_dir, remote_dir.as_deref(), zig_cwd, source, is_test)
}

#[allow(clippy::disallowed_methods)]
fn zig_debug_cache_bin_path(
    data_dir: &Path,
    remote_dir: Option<&str>,
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
    if let Some(dir) = remote_dir {
        Ok(format!("{dir}/{}-{stem}-{suffix}", &digest[..12]))
    } else {
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

/// Otto's no-exec / filtered-run hook, inserted before `if (steps_menu)` of Zig's own
/// `compiler/build_runner.zig`, one per build-runner layout it was written and run
/// against, each with the markers that layout must show.
const OTTO_ZIG_RUNNER_HOOKS: &[(&str, &[&str])] = &[
    (
        include_str!("zig_runner_hooks/v0_14.zig"),
        &[
            "const io = std.io;",
            "const Step = std.Build.Step;",
            "var targets = ArrayList([]const u8).init(arena);",
        ],
    ),
    (
        include_str!("zig_runner_hooks/v0_16.zig"),
        &[
            "const Io = std.Io;",
            "const Step = std.Build.Step;",
            "var targets = std.array_list.Managed([]const u8).init(arena);",
            "graph.environ_map",
        ],
    ),
];

/// The Zig minors whose build runner takes the hook (for the "supports" message).
const OTTO_ZIG_HOOK_VERSIONS: &str = "0.14 and 0.16";

/// Emitted test suite executable reported by `OttoEmitBinStep.make` before `run.cleanExit()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ZigEmittedTestBin {
    pub name: String,
    pub source: String,
    pub path: String,
}

/// Patch Zig's own `compiler/build_runner.zig` (the 0.14 or 0.16 layout) so
/// `zig build --build-runner <path> test`:
/// - In no-exec debug mode (`OTTO_ZIG_TEST_NO_EXEC=1`): compiles test `Step.Compile` artifacts
///   without running `Step.Run` (`addRunArtifact`), and runs `OttoEmitBinStep.make` inside the
///   step graph BEFORE `run.cleanExit()` calls `process.exit(0)` (BUG3-1, BUG-5, BUG2-1, BUG-35).
/// - In filtered single-file run mode (`OTTO_ZIG_TEST_FILTER` + `OTTO_ZIG_TEST_FILE`): routes
///   execution through `__otto_run_filtered_tests` depending ONLY on the matched file's `Step.Run`
///   so unrelated suites in dual-suite templates never execute (BUG2-2).
/// - Any other layout (0.13, 0.15, 0.17 has no build runner to replace) gives `None`.
pub(crate) fn patch_zig_build_runner(orig_source: &str) -> Option<String> {
    const PRE_ANCHOR: &str = "if (steps_menu)";
    if !orig_source.contains(PRE_ANCHOR) {
        return None;
    }
    let (hook, _) = OTTO_ZIG_RUNNER_HOOKS
        .iter()
        .find(|(_, markers)| markers.iter().all(|m| orig_source.contains(m)))?;
    Some(orig_source.replacen(PRE_ANCHOR, &format!("{hook}\n    {PRE_ANCHOR}"), 1))
}

/// Extract emitted test suite binaries from `OTTO_ZIG_TEST_BIN:<name>|<src>|<path>`
/// (or legacy `OTTO_ZIG_TEST_BIN:<path>`) lines printed by `OttoEmitBinStep.make`.
pub(crate) fn parse_otto_zig_test_bins(stdout: &str) -> Vec<ZigEmittedTestBin> {
    stdout
        .lines()
        .filter_map(|line| line.trim().strip_prefix("OTTO_ZIG_TEST_BIN:"))
        .filter_map(|rest| {
            let trimmed = rest.trim();
            if trimmed.is_empty() {
                return None;
            }
            let parts: Vec<&str> = trimmed.splitn(3, '|').collect();
            if parts.len() == 3 && !parts[2].trim().is_empty() {
                Some(ZigEmittedTestBin {
                    name: parts[0].trim().to_string(),
                    source: parts[1].trim().to_string(),
                    path: parts[2].trim().to_string(),
                })
            } else {
                Some(ZigEmittedTestBin {
                    name: "test".into(),
                    source: String::new(),
                    path: trimmed.to_string(),
                })
            }
        })
        .collect()
}

/// Select the test binary of a single-session Debug: the only suite, the suite
/// whose root is `raw_prog` or imports it (BUG-5), else `src/main.zig`'s suite (or
/// the first). A project-wide Debug of several suites is a Debug All run instead
/// (one step per suite, see [`plan_zig_test_steps`]).
pub(crate) fn select_zig_test_bin(
    bins: &[ZigEmittedTestBin],
    raw_prog: &str,
    is_zig_file: bool,
) -> Option<String> {
    if bins.len() <= 1 {
        return bins.first().map(|b| b.path.clone());
    }
    if is_zig_file && !raw_prog.is_empty() {
        if let Some(matched) = bins.iter().find(|b| {
            !b.source.is_empty()
                && (b.source == raw_prog
                    || raw_prog.ends_with(&format!("/{}", b.source))
                    || b.source.ends_with(&format!("/{raw_prog}")))
        }) {
            return Some(matched.path.clone());
        }
        // A test in a non-root file lives in the suite whose root imports that file.
        if let Some(owner) = bins
            .iter()
            .find(|b| zig_file_imports_reach(Path::new(&b.source), Path::new(raw_prog)))
        {
            return Some(owner.path.clone());
        }
    }
    let chosen = bins
        .iter()
        .find(|b| b.source.ends_with("src/main.zig") || b.source == "src/main.zig")
        .unwrap_or(&bins[0]);
    Some(chosen.path.clone())
}

/// Whether `target` is reachable from the local suite root `root` through relative
/// `@import("x.zig")` chains (named-module imports belong to other suites).
#[allow(clippy::disallowed_methods)]
fn zig_file_imports_reach(root: &Path, target: &Path) -> bool {
    static RE_IMPORT: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
    let re = RE_IMPORT.get_or_init(|| regex::Regex::new(r#"@import\("([^"]+\.zig)"\)"#).unwrap());
    let Ok(target) = std::fs::canonicalize(target) else {
        return false;
    };
    let mut seen = std::collections::HashSet::new();
    let mut queue = vec![root.to_path_buf()];
    while let Some(p) = queue.pop() {
        let Ok(p) = std::fs::canonicalize(&p) else {
            continue;
        };
        if p == target {
            return true;
        }
        if seen.len() >= 4096 || !seen.insert(p.clone()) {
            continue;
        }
        let (Ok(src), Some(dir)) = (std::fs::read_to_string(&p), p.parent()) else {
            continue;
        };
        queue.extend(re.captures_iter(&src).map(|c| dir.join(&c[1])));
    }
    false
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

/// Build a POSIX shell script that stages the patched Zig build runner inside the
/// SSH user's private `$HOME/.otto/cache/zig-build-runners` directory (`0700` dir,
/// `0600` file), rejecting symlinks or foreign ownership and verifying SHA-256
/// content before reusing any cached runner — never using shared `/tmp` (SEC3-1).
pub(crate) fn build_ssh_zig_runner_install_script(
    key_prefix: &str,
    expected_sha256: &str,
    patched: &str,
) -> String {
    format!(
        "{}\
         DEST=\"$DIR/runner-{key_prefix}.zig\"\n\
         WANT='{expected_sha256}'\n\
         NEED_WRITE=1\n\
         if [ -f \"$DEST\" ] && [ ! -L \"$DEST\" ] && [ -O \"$DEST\" ]; then\n\
           GOT=\"\"\n\
           if command -v sha256sum >/dev/null 2>&1; then GOT=$(sha256sum \"$DEST\" | awk '{{print $1}}');\n\
           elif command -v shasum >/dev/null 2>&1; then GOT=$(shasum -a 256 \"$DEST\" | awk '{{print $1}}'); fi\n\
           if [ \"$GOT\" = \"$WANT\" ]; then chmod 600 \"$DEST\"; NEED_WRITE=0; fi\n\
         fi\n\
         if [ \"$NEED_WRITE\" = \"1\" ]; then\n\
           TMP=\"$DIR/.runner-{key_prefix}.$$.tmp\"\n\
           rm -f \"$TMP\"\n\
           cat <<'__OTTO_ZIG_RUNNER__' > \"$TMP\"\n\
{patched}\n\
__OTTO_ZIG_RUNNER__\n\
           chmod 600 \"$TMP\"\n\
           mv -f \"$TMP\" \"$DEST\"\n\
         fi\n\
         printf 'RUNNER_PATH:%s\\n' \"$DEST\"\n",
        ssh_private_cache_dir_script("zig-build-runners")
    )
}

/// Prepare and cache the patched `runner-*.zig` inside the SSH user's private
/// `$HOME/.otto/cache/zig-build-runners` directory so remote `zig build --build-runner`
/// supports both no-exec debug builds (BUG-5/BUG2-1) and filtered gutter runs (BUG2-2/BUG-35).
pub(crate) async fn prepare_ssh_zig_build_runner(
    target: &IdeTargetSpec,
    ssh: Option<&crate::types::IdeSshSpec>,
    zig_bin: &str,
    zig_cwd: &str,
    env: &BTreeMap<String, String>,
) -> Option<String> {
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
    // Account for trailing newline added by heredoc when computing expected SHA-256.
    let patched_with_nl = if patched.ends_with('\n') {
        patched
    } else {
        format!("{patched}\n")
    };
    let mut h = Sha256::new();
    h.update(patched_with_nl.as_bytes());
    let full_sha = hex::encode(h.finalize());
    let body_for_heredoc = patched_with_nl.trim_end_matches('\n');
    let write_script =
        build_ssh_zig_runner_install_script(&full_sha[..16], &full_sha, body_for_heredoc);
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
    if !wr.status.success() {
        return None;
    }
    String::from_utf8_lossy(&wr.stdout)
        .lines()
        .find_map(|l| l.trim().strip_prefix("RUNNER_PATH:"))
        .map(str::trim)
        .filter(|p| !p.is_empty() && p.starts_with('/'))
        .map(String::from)
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
    use std::path::Path;

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
            "const std = @import(\"std\");\nconst io = std.io;\nconst Step = std.Build.Step;\npub fn main() !void {\n    var targets = ArrayList([]const u8).init(arena);\n    if (steps_menu) return;\n}\n",
        )
        .unwrap();

        let proj = tmp.path().join("proj");
        std::fs::create_dir_all(proj.join("zig-out/bin")).unwrap();
        std::fs::create_dir_all(proj.join(".zig-cache/o/warm123")).unwrap();
        std::fs::create_dir_all(proj.join(".zig-cache/o/warm456")).unwrap();
        std::fs::write(
            proj.join("build.zig"),
            "pub fn build(b: *std.Build) void { _ = b.step(\"test\", \"tests\"); }\n",
        )
        .unwrap();
        let app_bin = proj.join("zig-out/bin/proj");
        let warm_lib_test_bin = proj.join(".zig-cache/o/warm123/test");
        let warm_exe_test_bin = proj.join(".zig-cache/o/warm456/test");
        for p in [&app_bin, &warm_lib_test_bin, &warm_exe_test_bin] {
            std::fs::write(p, "#!/bin/sh\nexit 0\n").unwrap();
            std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
        // Make cached test binaries old (2020) using async tokio::process::Command (TEST3-1).
        let _ = tokio::process::Command::new("touch")
            .args([
                "-t",
                "202001010000",
                &warm_lib_test_bin.to_string_lossy(),
                &warm_exe_test_bin.to_string_lossy(),
            ])
            .status()
            .await;

        let fake_zig = fake_zig_dir.join("zig");
        let marker = tmp.path().join("invoked.txt");
        let fake_zig_script = format!(
            "#!/bin/sh\n\
             printf '%s|%s\\n' \"$OTTO_ZIG_TEST_NO_EXEC\" \"$*\" > \"{}\"\n\
             if [ \"$OTTO_ZIG_TEST_NO_EXEC\" != \"1\" ]; then echo 'missing NO_EXEC' >&2; exit 2; fi\n\
             case \"$*\" in *--build-runner*) ;; *) echo 'missing --build-runner' >&2; exit 3 ;; esac\n\
             echo 'OTTO_ZIG_TEST_BIN:lib_test|src/root.zig|{}'\n\
             echo 'OTTO_ZIG_TEST_BIN:exe_test|src/main.zig|{}'\n",
            marker.display(),
            warm_lib_test_bin.display(),
            warm_exe_test_bin.display()
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
        let mut all_bins = Vec::new();
        let resolved_bin = super::compile_zig_debug_artifact(
            &data_dir,
            &req,
            &ctx,
            &proj.to_string_lossy(),
            &proj.to_string_lossy(),
            true,
            &env,
            &mut out,
            Some(&mut all_bins),
        )
        .await
        .expect("no-exec build runner must return warm-cached test binary");
        assert_eq!(resolved_bin, warm_exe_test_bin.to_string_lossy());
        assert_ne!(resolved_bin, app_bin.to_string_lossy());
        // Debug All: every suite the runner emitted, in its order (one step each).
        let names: Vec<&str> = all_bins.iter().map(|b| b.name.as_str()).collect();
        assert_eq!(names, ["lib_test", "exe_test"]);
        assert_eq!(all_bins[0].path, warm_lib_test_bin.to_string_lossy());

        let invoked = std::fs::read_to_string(&marker).unwrap();
        assert!(invoked.starts_with("1|build --build-runner "));
        // BUG3-1 + BUG2-2: Verify generated runner uses OttoEmitBinStep inside the step graph
        // (runs before run.cleanExit()!) and routes filtered single-file runs through
        // __otto_run_filtered_tests (never running unrelated suites).
        let runner_path = invoked
            .split_whitespace()
            .skip_while(|t| *t != "--build-runner")
            .nth(1)
            .expect("runner path in argv");
        let runner_src = std::fs::read_to_string(runner_path).unwrap();
        assert!(runner_src.contains("OttoEmitBinStep"));
        assert!(runner_src.contains("__otto_compile_tests"));
        assert!(runner_src.contains("__otto_run_filtered_tests"));
        assert!(runner_src.contains("OTTO_ZIG_TEST_BIN:"));

        // BUG3-1: only the 0.14 and 0.16 runner layouts take the hook; 0.13 and 0.15
        // (`const io = std.io;` with `std.array_list.Managed`) are gated.
        assert!(super::patch_zig_build_runner(
            "const std = @import(\"std\");\npub fn main() !void { if (steps_menu) return; }\n"
        )
        .is_none());
        let v0_15 = "const io = std.io;\nconst Step = std.Build.Step;\nvar targets = std.array_list.Managed([]const u8).init(arena);\nif (steps_menu) {}\n";
        assert!(super::patch_zig_build_runner(v0_15).is_none());
        let v0_16 = "const Io = std.Io;\nconst Step = std.Build.Step;\nvar targets = std.array_list.Managed([]const u8).init(arena);\nconst x = graph.environ_map;\n    if (steps_menu) {}\n";
        let patched = super::patch_zig_build_runner(v0_16).expect("0.16 layout takes the hook");
        assert!(patched.contains("graph.environ_map.get(\"OTTO_ZIG_TEST_NO_EXEC\")"));
        assert!(patched.contains("writerStreaming(step.owner.graph.io"));
        assert!(
            patched.find("OttoEmitBinStep").unwrap() < patched.find("if (steps_menu)").unwrap()
        );

        // BUG-35: Single-file Debug inside a build.zig project with a `test` step must fail
        // clearly when the build runner fails rather than silently substituting a partial
        // regex-only `zig test` compile.
        let fail_zig_dir = tmp.path().join("fail-zig");
        let fail_lib = fail_zig_dir.join("lib/zig/compiler");
        std::fs::create_dir_all(&fail_lib).unwrap();
        std::fs::write(
            fail_lib.join("build_runner.zig"),
            "const std = @import(\"std\");\nconst io = std.io;\nconst Step = std.Build.Step;\npub fn main() !void {\n    var targets = ArrayList([]const u8).init(arena);\n    if (steps_menu) return;\n}\n",
        )
        .unwrap();
        let fail_zig = fail_zig_dir.join("zig");
        std::fs::write(
            &fail_zig,
            "#!/bin/sh\necho 'build.zig dependency failed' >&2\nexit 1\n",
        )
        .unwrap();
        std::fs::set_permissions(&fail_zig, std::fs::Permissions::from_mode(0o755)).unwrap();
        let main_zig = proj.join("src/main.zig");
        std::fs::create_dir_all(main_zig.parent().unwrap()).unwrap();
        std::fs::write(&main_zig, "test \"t\" {}\n").unwrap();
        let mut fail_env = BTreeMap::new();
        fail_env.insert(
            "ZIG_HOME".into(),
            fail_zig_dir.to_string_lossy().into_owned(),
        );
        fail_env.insert(
            "PATH".into(),
            format!("{}:/usr/bin:/bin", fail_zig_dir.display()),
        );
        let single_req = crate::types::IdeStartDebugReq {
            target: crate::types::IdeTargetSpec::local(proj.to_string_lossy().into_owned()),
            language: "zig".into(),
            launch: crate::types::IdeDebugLaunchSpec {
                program: main_zig.to_string_lossy().into_owned(),
                cwd: Some(proj.to_string_lossy().into_owned()),
                test_name: Some("t".into()),
                is_test: true,
                ..Default::default()
            },
            ..Default::default()
        };
        let err = super::compile_zig_debug_artifact(
            &data_dir,
            &single_req,
            &ctx,
            &proj.to_string_lossy(),
            &main_zig.to_string_lossy(),
            true,
            &fail_env,
            &mut Vec::new(),
            None,
        )
        .await
        .expect_err("BUG-35: must fail clearly instead of partial source-only fallback");
        assert!(
            err.to_string().contains("build.zig dependency failed"),
            "unexpected error: {err}"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ssh_zig_runner_uses_private_home_dir_and_rejects_tampered_file_and_symlinks() {
        use sha2::{Digest, Sha256};
        use std::os::unix::fs::PermissionsExt;

        // SEC3-1: SSH Zig runner must NEVER use shared /tmp; it must stage in private
        // `$HOME/.otto/cache/zig-build-runners` (0700/0600), verify SHA-256 + ownership
        // before reuse, overwrite tampered files, and refuse symlinked cache dirs.
        let tmp = tempfile::tempdir().unwrap();
        let fake_home = tmp.path().join("ssh-user-home");
        std::fs::create_dir_all(&fake_home).unwrap();

        let body = "// verified otto zig runner\nconst x = 1;";
        let body_with_nl = format!("{body}\n");
        let mut h = Sha256::new();
        h.update(body_with_nl.as_bytes());
        let full_sha = hex::encode(h.finalize());
        let key_prefix = &full_sha[..16];

        let script = super::build_ssh_zig_runner_install_script(key_prefix, &full_sha, body);
        assert!(
            !script.contains("/tmp/"),
            "SEC3-1: SSH runner script must never use shared /tmp: {script}"
        );

        // Pre-plant a tampered file at the exact destination pathname.
        let cache_dir = fake_home.join(".otto/cache/zig-build-runners");
        std::fs::create_dir_all(&cache_dir).unwrap();
        let dest = cache_dir.join(format!("runner-{key_prefix}.zig"));
        std::fs::write(&dest, "// TAMPERED MALICIOUS RUNNER\n").unwrap();

        let out = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&script)
            .env("HOME", &fake_home)
            .env("PATH", "/usr/bin:/bin")
            .output()
            .await
            .unwrap();
        assert!(
            out.status.success(),
            "runner install script failed: {:?}",
            out
        );
        let stdout = String::from_utf8_lossy(&out.stdout);
        let reported = stdout
            .lines()
            .find_map(|l| l.trim().strip_prefix("RUNNER_PATH:"))
            .expect("RUNNER_PATH output");
        assert_eq!(reported, dest.to_string_lossy());
        assert_eq!(
            std::fs::read_to_string(&dest).unwrap(),
            body_with_nl,
            "SEC3-1: tampered file at runner path must be overwritten with verified content"
        );
        let dir_mode = std::fs::metadata(&cache_dir).unwrap().permissions().mode() & 0o777;
        let file_mode = std::fs::metadata(&dest).unwrap().permissions().mode() & 0o777;
        assert_eq!(dir_mode, 0o700);
        assert_eq!(file_mode, 0o600);

        // Symlinked cache directory must be refused (non-zero exit).
        let sym_home = tmp.path().join("sym-home");
        let outside = tmp.path().join("outside");
        std::fs::create_dir_all(sym_home.join(".otto/cache")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::os::unix::fs::symlink(&outside, sym_home.join(".otto/cache/zig-build-runners"))
            .unwrap();
        let sym_out = tokio::process::Command::new("sh")
            .arg("-c")
            .arg(&script)
            .env("HOME", &sym_home)
            .env("PATH", "/usr/bin:/bin")
            .output()
            .await
            .unwrap();
        assert!(
            !sym_out.status.success(),
            "SEC3-1: symlinked cache directory must be refused"
        );
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn real_zig_014_build_runner_emits_bins_before_clean_exit_and_isolates_filtered_suite() {
        // BUG3-1 + BUG-5 + BUG2-2 live proof against the host's real Zig 0.14.x compiler
        // when installed: compiles the patched `compiler/build_runner.zig` against a stock
        // dual-suite `build.zig` (`lib_unit_tests` + `exe_unit_tests`) and executes it live.
        let Ok(env_out) = tokio::process::Command::new("zig")
            .arg("env")
            .output()
            .await
        else {
            return;
        };
        if !env_out.status.success() {
            return;
        }
        let env_str = String::from_utf8_lossy(&env_out.stdout);
        let Some(lib_dir) = super::parse_zig_env_lib_dir(&env_str) else {
            return;
        };
        let orig_path = Path::new(&lib_dir).join("compiler/build_runner.zig");
        let Ok(orig) = std::fs::read_to_string(&orig_path) else {
            return;
        };
        let Some(patched) = super::patch_zig_build_runner(&orig) else {
            return;
        };
        let zig_exe = env_str
            .lines()
            .find_map(|l| {
                l.trim()
                    .strip_prefix("\"zig_exe\":")
                    .or_else(|| l.trim().strip_prefix(".zig_exe ="))
                    .and_then(|r| {
                        r.trim()
                            .trim_end_matches(',')
                            .trim()
                            .strip_prefix('"')
                            .and_then(|s| s.strip_suffix('"'))
                    })
            })
            .unwrap_or("zig")
            .to_string();

        let tmp = tempfile::tempdir().unwrap();
        let proj = tmp.path().join("dual_proj");
        std::fs::create_dir_all(proj.join("src")).unwrap();
        let lib_marker = tmp.path().join("lib_ran.marker");
        let exe_fail_marker = tmp.path().join("exe_fail_ran.marker");

        std::fs::write(
            proj.join("src/root.zig"),
            format!(
                "const std = @import(\"std\");\n\
                 pub fn add(a: i32, b: i32) i32 {{ return a + b; }}\n\
                 test \"failing lib suite\" {{\n\
                     var f = try std.fs.createFileAbsolute(\"{}\", .{{}});\n\
                     f.close();\n\
                     try std.testing.expectEqual(@as(i32, 1), 999);\n\
                 }}\n",
                lib_marker.display()
            ),
        )
        .unwrap();
        std::fs::write(
            proj.join("src/main.zig"),
            format!(
                "const std = @import(\"std\");\n\
                 const lib = @import(\"dual_lib\");\n\
                 pub fn main() void {{ _ = lib.add(1, 2); }}\n\
                 test \"passing exe\" {{ try std.testing.expectEqual(@as(i32, 3), lib.add(1, 2)); }}\n\
                 test \"failing exe sibling\" {{\n\
                     var f = try std.fs.createFileAbsolute(\"{}\", .{{}});\n\
                     f.close();\n\
                     try std.testing.expectEqual(@as(i32, 1), 2);\n\
                 }}\n",
                exe_fail_marker.display()
            ),
        )
        .unwrap();
        std::fs::write(
            proj.join("build.zig"),
            r#"const std = @import("std");
pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const optimize = b.standardOptimizeOption(.{});
    const lib_mod = b.createModule(.{
        .root_source_file = b.path("src/root.zig"),
        .target = target,
        .optimize = optimize,
    });
    const exe_mod = b.createModule(.{
        .root_source_file = b.path("src/main.zig"),
        .target = target,
        .optimize = optimize,
    });
    exe_mod.addImport("dual_lib", lib_mod);
    const lib_unit_tests = b.addTest(.{ .root_module = lib_mod });
    const run_lib_unit_tests = b.addRunArtifact(lib_unit_tests);
    const exe_unit_tests = b.addTest(.{ .root_module = exe_mod });
    const run_exe_unit_tests = b.addRunArtifact(exe_unit_tests);
    const test_step = b.step("test", "Run unit tests");
    test_step.dependOn(&run_lib_unit_tests.step);
    test_step.dependOn(&run_exe_unit_tests.step);
}
"#,
        )
        .unwrap();

        let runner_src = tmp.path().join("otto_build_runner.zig");
        let deps_src = tmp.path().join("deps.zig");
        let runner_bin = tmp.path().join("otto_runner_bin");
        std::fs::write(&runner_src, patched).unwrap();
        std::fs::write(
            &deps_src,
            "pub const packages = struct {}; pub const root_deps: []const struct { []const u8, []const u8 } = &.{};\n",
        )
        .unwrap();

        let mut compile_cmd = tokio::process::Command::new(&zig_exe);
        compile_cmd.arg("build-exe");
        if cfg!(target_os = "macos") {
            compile_cmd.args(["-target", "native-macos.15.0"]);
        }
        let compile_out = compile_cmd
            .args([
                &format!("-femit-bin={}", runner_bin.display()),
                "--dep",
                "@build",
                "--dep",
                "@dependencies",
                &format!("-Mroot={}", runner_src.display()),
                &format!("-M@build={}", proj.join("build.zig").display()),
                &format!("-M@dependencies={}", deps_src.display()),
            ])
            .output()
            .await
            .unwrap();
        assert!(
            compile_out.status.success(),
            "patched build_runner.zig failed to compile: {}",
            String::from_utf8_lossy(&compile_out.stderr)
        );

        let cache_dir = proj.join(".zig-cache");
        let global_cache = tmp.path().join("global-cache");
        let mut runner_args = vec![
            zig_exe.clone(),
            lib_dir.clone(),
            proj.to_string_lossy().into_owned(),
            cache_dir.to_string_lossy().into_owned(),
            global_cache.to_string_lossy().into_owned(),
        ];
        if cfg!(target_os = "macos") {
            runner_args.push("-Dtarget=native-macos.15.0".into());
        }
        runner_args.push("test".into());

        // 1. BUG3-1 + BUG-5 + BUG2-1: No-exec mode emits BOTH suite binaries BEFORE
        //    run.cleanExit() and never executes either test binary.
        let no_exec_out = tokio::process::Command::new(&runner_bin)
            .args(&runner_args)
            .current_dir(&proj)
            .env("OTTO_ZIG_TEST_NO_EXEC", "1")
            .output()
            .await
            .unwrap();
        assert!(
            no_exec_out.status.success(),
            "no-exec runner failed: {}",
            String::from_utf8_lossy(&no_exec_out.stderr)
        );
        assert!(!lib_marker.exists() && !exe_fail_marker.exists());
        let bins = super::parse_otto_zig_test_bins(&String::from_utf8_lossy(&no_exec_out.stdout));
        assert_eq!(
            bins.len(),
            2,
            "expected both suites emitted before cleanExit: {bins:?}"
        );
        for b in &bins {
            assert!(
                Path::new(&b.path).is_file(),
                "emitted binary missing: {}",
                b.path
            );
        }

        // 2. BUG2-2: Filtered single-file Run on src/main.zig executes ONLY `passing exe`,
        //    skipping both failing `lib_unit_tests` (src/root.zig) and failing exe sibling.
        let filtered_out = tokio::process::Command::new(&runner_bin)
            .args(&runner_args)
            .current_dir(&proj)
            .env("OTTO_ZIG_TEST_FILTER", "passing exe")
            .env("OTTO_ZIG_TEST_FILE", proj.join("src/main.zig"))
            .output()
            .await
            .unwrap();
        assert!(
            filtered_out.status.success(),
            "BUG2-2: filtered run should pass without executing failing lib suite or sibling: {}",
            String::from_utf8_lossy(&filtered_out.stderr)
        );
        assert!(!lib_marker.exists(), "BUG2-2: unrelated lib_unit_tests ran");
        assert!(!exe_fail_marker.exists(), "BUG2-2: failing exe sibling ran");

        // 3. BUG3-1: a `test` step that only compiles (no run or install step) never asks
        //    for the binary; the runner must request it or getPath panics with no marker.
        let co_proj = tmp.path().join("compile_only");
        std::fs::create_dir_all(co_proj.join("src")).unwrap();
        std::fs::write(co_proj.join("src/root.zig"), "test \"t\" {}\n").unwrap();
        std::fs::write(
            co_proj.join("build.zig"),
            r#"const std = @import("std");
pub fn build(b: *std.Build) void {
    const target = b.standardTargetOptions(.{});
    const mod = b.createModule(.{ .root_source_file = b.path("src/root.zig"), .target = target });
    const t = b.addTest(.{ .root_module = mod });
    b.step("test", "Compile tests").dependOn(&t.step);
}
"#,
        )
        .unwrap();
        let co_runner_bin = tmp.path().join("otto_runner_co_bin");
        let mut co_compile = tokio::process::Command::new(&zig_exe);
        co_compile.arg("build-exe");
        if cfg!(target_os = "macos") {
            co_compile.args(["-target", "native-macos.15.0"]);
        }
        let co_compile_out = co_compile
            .args([
                &format!("-femit-bin={}", co_runner_bin.display()),
                "--dep",
                "@build",
                "--dep",
                "@dependencies",
                &format!("-Mroot={}", runner_src.display()),
                &format!("-M@build={}", co_proj.join("build.zig").display()),
                &format!("-M@dependencies={}", deps_src.display()),
            ])
            .output()
            .await
            .unwrap();
        assert!(co_compile_out.status.success());
        runner_args[2] = co_proj.to_string_lossy().into_owned();
        runner_args[3] = co_proj.join(".zig-cache").to_string_lossy().into_owned();
        let co_out = tokio::process::Command::new(&co_runner_bin)
            .args(&runner_args)
            .current_dir(&co_proj)
            .env("OTTO_ZIG_TEST_NO_EXEC", "1")
            .output()
            .await
            .unwrap();
        assert!(
            co_out.status.success(),
            "compile-only test step: {}",
            String::from_utf8_lossy(&co_out.stderr)
        );
        let co_bins = super::parse_otto_zig_test_bins(&String::from_utf8_lossy(&co_out.stdout));
        assert_eq!(co_bins.len(), 1, "{co_bins:?}");
        assert!(Path::new(&co_bins[0].path).is_file());
    }

    #[test]
    fn zig_single_file_debug_selects_the_suite_that_imports_the_file() {
        // BUG-35/BUG-5: a test in src/math.zig (imported by the lib root) is in the lib
        // suite; the exe suite reaches the lib only through a named module.
        let tmp = tempfile::tempdir().unwrap();
        let src = tmp.path().join("src");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("root.zig"),
            "pub const m = @import(\"math.zig\");\n",
        )
        .unwrap();
        std::fs::write(src.join("math.zig"), "test \"adds\" {}\n").unwrap();
        std::fs::write(src.join("main.zig"), "const lib = @import(\"lib\");\n").unwrap();
        let bin = |name: &str, file: &str| super::ZigEmittedTestBin {
            name: name.into(),
            source: src.join(file).to_string_lossy().into_owned(),
            path: format!("/cache/{name}"),
        };
        let bins = [bin("exe", "main.zig"), bin("lib", "root.zig")];
        let math = src.join("math.zig").to_string_lossy().into_owned();
        assert_eq!(
            super::select_zig_test_bin(&bins, &math, true),
            Some("/cache/lib".to_string())
        );
        // A file no suite reaches gets the `src/main.zig` suite.
        let other = tmp.path().join("other.zig");
        std::fs::write(&other, "test {}\n").unwrap();
        let chosen = super::select_zig_test_bin(&bins, &other.to_string_lossy(), true).unwrap();
        assert_eq!(chosen, "/cache/exe");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn remote_build_zig_walk_finds_the_project_inside_the_target_root() {
        // BUG-35 (SSH): single-test Debug must find the build.zig above the file.
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path().join("ws");
        let proj = root.join("proj");
        std::fs::create_dir_all(proj.join("src/deep")).unwrap();
        std::fs::write(proj.join("build.zig"), "// project build\n").unwrap();
        std::fs::write(tmp.path().join("build.zig"), "// outside root\n").unwrap();
        let run = |file: std::path::PathBuf, bound: std::path::PathBuf| async move {
            tokio::process::Command::new("sh")
                .args(["-c", super::ZIG_FIND_BUILD_SH, "sh"])
                .arg(file)
                .arg(bound)
                .output()
                .await
                .unwrap()
        };
        let out = run(proj.join("src/deep/t.zig"), root.clone()).await;
        assert!(out.status.success());
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("ROOT:{}\n// project build\n", proj.display())
        );
        // Never climbs above the target root to an unrelated build.zig.
        let lone = root.join("lone");
        std::fs::create_dir_all(&lone).unwrap();
        assert!(!run(lone.join("t.zig"), root.clone()).await.status.success());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn ssh_zig_debug_binaries_live_in_the_private_home_cache() {
        use std::os::unix::fs::PermissionsExt;

        // SEC3-1: SSH debug executables go to a 0700 $HOME/.otto/cache/zig-debug, never /tmp.
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        std::fs::create_dir_all(&home).unwrap();
        let script = format!(
            "{}printf 'DIR:%s\\n' \"$DIR\"\n",
            super::ssh_private_cache_dir_script("zig-debug")
        );
        let out = tokio::process::Command::new("sh")
            .args(["-c", &script])
            .env("HOME", &home)
            .output()
            .await
            .unwrap();
        assert!(out.status.success(), "{out:?}");
        let dir = home.join(".otto/cache/zig-debug");
        assert_eq!(
            String::from_utf8_lossy(&out.stdout).trim(),
            format!("DIR:{}", dir.display())
        );
        assert_eq!(
            std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777,
            0o700
        );
        let bin = super::zig_debug_cache_bin_path(
            tmp.path(),
            Some(&dir.to_string_lossy()),
            "/srv/proj",
            "/srv/proj/src/main.zig",
            true,
        )
        .unwrap();
        assert!(bin.starts_with(&format!("{}/", dir.display())) && bin.ends_with("main-test"));
    }
}
