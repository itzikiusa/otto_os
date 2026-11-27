//! Otto's built-in zero-install Python kernel (frozen fallback): a persistent
//! Python subprocess spoken to over newline-delimited JSON on stdin/stdout.
//! Used only for a Python env without ipykernel; no new features land here.

use std::collections::BTreeMap;
use std::time::{Duration, Instant};

use otto_core::{Error, Result};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

use super::MAX_OUTPUT_LINE_BYTES;

// ---------------------------------------------------------------------------
// Embedded Zero-Install Persistent Python Kernel Script
// ---------------------------------------------------------------------------

const OTTO_KERNEL_PY: &str = r#"
import ast, base64, io, json, os, signal, sys, tempfile, traceback

os.environ.setdefault("MPLBACKEND", "Agg")

# Move the JSON protocol channel onto a private, non-inheritable fd so
# subprocesses and C extensions writing to OS fd 1/2 never corrupt protocol lines.
_PROTO_FD = os.dup(1)
try:
    os.set_inheritable(_PROTO_FD, False)
except Exception:
    pass
_PROTO_OUT = os.fdopen(_PROTO_FD, "w", buffering=1, encoding="utf-8", errors="replace")
_DEVNULL_FD = os.open(os.devnull, os.O_WRONLY)
try:
    os.dup2(_DEVNULL_FD, 1)
    os.dup2(_DEVNULL_FD, 2)
except Exception:
    pass

# Keep stable Python stdout/stderr wrappers across cells (BUG2-12, BUG3-11) so libraries and
# logging.StreamHandler instances that retain a reference to sys.stdout/sys.stderr never
# hit "ValueError: I/O operation on closed file" in later cells, AND if a cell temporarily
# reassigns sys.stdout/sys.stderr (e.g. to a file closed by a `with` block), _exec_cell
# restores both bindings to these persistent wrappers without closing them.
_OTTO_STDOUT = io.TextIOWrapper(
    os.fdopen(1, "wb", buffering=0, closefd=False),
    encoding="utf-8",
    errors="replace",
    line_buffering=True,
)
_OTTO_STDERR = io.TextIOWrapper(
    os.fdopen(2, "wb", buffering=0, closefd=False),
    encoding="utf-8",
    errors="replace",
    line_buffering=True,
)
sys.stdout = _OTTO_STDOUT
sys.stderr = _OTTO_STDERR

_MAX_STREAM_BYTES = 2 * 1024 * 1024

_NS = {"__name__": "__main__", "__doc__": None}
_EXEC_COUNT = 0

def _emit(obj):
    _PROTO_OUT.write(json.dumps(obj, ensure_ascii=False) + "\n")
    _PROTO_OUT.flush()

def _repr_bundle(val):
    data = {}
    try:
        if hasattr(val, "_repr_png_"):
            png = val._repr_png_()
            if isinstance(png, bytes):
                data["image/png"] = base64.b64encode(png).decode("ascii")
            elif isinstance(png, str):
                data["image/png"] = png
    except Exception:
        pass
    try:
        if hasattr(val, "_repr_jpeg_"):
            jpg = val._repr_jpeg_()
            if isinstance(jpg, bytes):
                data["image/jpeg"] = base64.b64encode(jpg).decode("ascii")
            elif isinstance(jpg, str):
                data["image/jpeg"] = jpg
    except Exception:
        pass
    try:
        if hasattr(val, "_repr_svg_"):
            svg = val._repr_svg_()
            if isinstance(svg, str):
                data["image/svg+xml"] = svg
    except Exception:
        pass
    try:
        if hasattr(val, "_repr_html_"):
            html = val._repr_html_()
            if isinstance(html, str):
                data["text/html"] = html[:200000]
    except Exception:
        pass
    try:
        data["text/plain"] = repr(val)[:32768]
    except Exception:
        data["text/plain"] = f"<{type(val).__name__}>"
    return data

def _capture_matplotlib(outputs):
    plt = sys.modules.get("matplotlib.pyplot")
    if plt is None:
        return
    try:
        fignums = list(plt.get_fignums())
        for num in fignums:
            fig = plt.figure(num)
            buf = io.BytesIO()
            fig.savefig(buf, format="png", bbox_inches="tight")
            b64 = base64.b64encode(buf.getvalue()).decode("ascii")
            outputs.append({
                "output_type": "display_data",
                "data": {
                    "image/png": b64,
                    "text/plain": [f"<Figure size {fig.get_size_inches()[0]:.1f}x{fig.get_size_inches()[1]:.1f}>"],
                },
                "metadata": {},
            })
        if fignums:
            plt.close("all")
    except Exception:
        pass

def _collect_vars():
    items = []
    for k, v in _NS.items():
        if k.startswith("_") or callable(v) or isinstance(v, type(sys)):
            continue
        tname = type(v).__name__
        shape = None
        if hasattr(v, "shape"):
            try:
                shape = str(tuple(v.shape))
            except Exception:
                pass
        elif isinstance(v, (list, tuple, dict, set, bytes)):
            shape = f"len={len(v)}"
        try:
            prev = repr(v)
        except Exception:
            prev = f"<{tname}>"
        if len(prev) > 160:
            prev = prev[:157] + "..."
        items.append({"name": k, "var_type": tname, "shape": shape, "preview": prev})
        if len(items) >= 120:
            break
    items.sort(key=lambda x: x["name"])
    return items

def _exec_cell(code):
    global _EXEC_COUNT
    _EXEC_COUNT += 1
    outputs = []
    status = "ok"
    tmp_out = tempfile.TemporaryFile(mode="w+b")
    tmp_err = tempfile.TemporaryFile(mode="w+b")
    try:
        sys.stdout.flush()
    except Exception:
        pass
    try:
        sys.stderr.flush()
    except Exception:
        pass
    os.dup2(tmp_out.fileno(), 1)
    os.dup2(tmp_err.fileno(), 2)
    result_val = None
    has_result = False
    try:
        tree = ast.parse(code, filename=f"<cell-{_EXEC_COUNT}>", mode="exec")
        last_expr = None
        if tree.body and isinstance(tree.body[-1], ast.Expr):
            last_expr = tree.body.pop()
        if tree.body:
            exec(compile(tree, f"<cell-{_EXEC_COUNT}>", "exec"), _NS, _NS)
        if last_expr is not None:
            expr_mod = ast.Expression(last_expr.value)
            result_val = eval(compile(expr_mod, f"<cell-{_EXEC_COUNT}>", "eval"), _NS, _NS)
            if result_val is not None:
                has_result = True
    except KeyboardInterrupt:
        status = "error"
        outputs.append({
            "output_type": "error",
            "ename": "KeyboardInterrupt",
            "evalue": "Cell execution interrupted",
            "traceback": ["KeyboardInterrupt: Cell execution interrupted"],
        })
    except BaseException as exc:
        status = "error"
        tb = traceback.format_exc().splitlines()
        outputs.append({
            "output_type": "error",
            "ename": type(exc).__name__,
            "evalue": str(exc),
            "traceback": tb[-40:],
        })
    finally:
        for _s in (sys.stdout, sys.stderr, _OTTO_STDOUT, _OTTO_STDERR):
            try:
                _s.flush()
            except Exception:
                pass
        # BUG3-11: Restore harness-owned wrappers after each cell without closing them so
        # a temporary `sys.stdout = f` inside a `with` block cannot break later cells.
        sys.stdout = _OTTO_STDOUT
        sys.stderr = _OTTO_STDERR
        try:
            os.dup2(_DEVNULL_FD, 1)
            os.dup2(_DEVNULL_FD, 2)
        except Exception:
            pass

    tmp_out.seek(0)
    tmp_err.seek(0)
    out_txt = tmp_out.read(_MAX_STREAM_BYTES).decode("utf-8", errors="replace")
    err_txt = tmp_err.read(_MAX_STREAM_BYTES).decode("utf-8", errors="replace")
    tmp_out.close()
    tmp_err.close()
    if out_txt:
        outputs.insert(0, {"output_type": "stream", "name": "stdout", "text": out_txt.splitlines(True)})
    if err_txt:
        outputs.append({"output_type": "stream", "name": "stderr", "text": err_txt.splitlines(True)})
    _capture_matplotlib(outputs)
    if has_result:
        bundle = _repr_bundle(result_val)
        outputs.append({
            "output_type": "execute_result",
            "execution_count": _EXEC_COUNT,
            "data": bundle,
            "metadata": {},
        })
    return {
        "ok": True,
        "execution_count": _EXEC_COUNT,
        "status": status,
        "outputs": outputs,
        "variables": _collect_vars(),
    }

_emit({"ready": True, "python": sys.version.split()[0]})

for _raw in sys.stdin:
    _line = _raw.strip()
    if not _line:
        continue
    try:
        _req = json.loads(_line)
    except Exception as _e:
        _emit({"ok": False, "error": f"bad json: {_e}"})
        continue
    _cmd = _req.get("cmd")
    if _cmd == "exec":
        _emit(_exec_cell(_req.get("code", "")))
    elif _cmd == "vars":
        _emit({"ok": True, "execution_count": _EXEC_COUNT, "variables": _collect_vars()})
    elif _cmd == "ping":
        _emit({"ok": True, "execution_count": _EXEC_COUNT})
    elif _cmd == "quit":
        _emit({"ok": True})
        break
"#;

pub(super) struct KernelProc {
    pub(super) session_id: String,
    pub(super) interpreter: String,
    pub(super) cwd: String,
    pub(super) pid: Option<u32>,
    pub(super) child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    pub(super) execution_count: u32,
    pub(super) last_used: Instant,
}

impl KernelProc {
    pub(super) fn terminate_tree(&mut self) {
        #[cfg(unix)]
        if let Some(pid) = self.pid {
            send_unix_signal_group(pid, 9 /* SIGKILL */);
        }
        let _ = self.child.start_kill();
    }
}

impl Drop for KernelProc {
    fn drop(&mut self) {
        self.terminate_tree();
    }
}

#[cfg(unix)]
#[allow(unsafe_code)]
pub(super) fn send_unix_signal_pid(pid: u32, sig: std::ffi::c_int) -> bool {
    extern "C" {
        fn kill(pid: std::ffi::c_int, sig: std::ffi::c_int) -> std::ffi::c_int;
    }
    let Ok(p) = std::ffi::c_int::try_from(pid) else {
        return false;
    };
    if p <= 1 {
        return false;
    }
    // SAFETY: Signaling positive `p > 1` targets only the kernel process itself.
    unsafe { kill(p, sig) == 0 }
}

#[cfg(unix)]
#[allow(unsafe_code)]
pub(super) fn send_unix_signal_group(pid: u32, sig: std::ffi::c_int) -> bool {
    extern "C" {
        fn kill(pid: std::ffi::c_int, sig: std::ffi::c_int) -> std::ffi::c_int;
    }
    let Ok(p) = std::ffi::c_int::try_from(pid) else {
        return false;
    };
    if p <= 1 {
        return false;
    }
    // SAFETY: The kernel is spawned in its own process group (`process_group(0)`),
    // so signaling `-p` targets only the kernel and its spawned worker children.
    unsafe { kill(-p, sig) == 0 || kill(p, sig) == 0 }
}

pub(super) async fn spawn_python_kernel(
    interpreter: &str,
    cwd: &str,
    env: &BTreeMap<String, String>,
) -> Result<KernelProc> {
    let mut cmd = Command::new(interpreter);
    cmd.args(["-u", "-c", OTTO_KERNEL_PY])
        .current_dir(cwd)
        .env_clear()
        .envs(env)
        .env("PYTHONUNBUFFERED", "1")
        .env("MPLBACKEND", "Agg")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    #[cfg(unix)]
    cmd.process_group(0);

    let mut child = cmd.spawn().map_err(|e| {
        Error::Upstream(format!(
            "failed to start Python notebook kernel '{interpreter}': {e}"
        ))
    })?;
    let pid = child.id();
    let stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::Internal("notebook kernel missing stdin".into()))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::Internal("notebook kernel missing stdout".into()))?;
    let mut reader = BufReader::new(stdout);

    let mut ready_line = String::new();
    let n = tokio::time::timeout(Duration::from_secs(10), reader.read_line(&mut ready_line))
        .await
        .map_err(|_| Error::Upstream("Python notebook kernel timed out on startup".into()))?
        .map_err(|e| Error::Upstream(format!("read kernel banner: {e}")))?;
    if n == 0 {
        return Err(Error::Upstream(
            "Python notebook kernel exited before ready".into(),
        ));
    }

    Ok(KernelProc {
        session_id: format!("nbk_{}", otto_core::new_id()),
        interpreter: interpreter.to_string(),
        cwd: cwd.to_string(),
        pid,
        child,
        stdin,
        stdout: reader,
        execution_count: 0,
        last_used: Instant::now(),
    })
}

pub(super) async fn send_kernel_rpc(
    proc: &mut KernelProc,
    payload: &Value,
    timeout: Duration,
) -> Result<Value> {
    let mut line = serde_json::to_string(payload)
        .map_err(|e| Error::Internal(format!("serialize kernel request: {e}")))?;
    line.push('\n');
    proc.stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|e| Error::Upstream(format!("write to notebook kernel: {e}")))?;
    proc.stdin
        .flush()
        .await
        .map_err(|e| Error::Upstream(format!("flush notebook kernel: {e}")))?;

    let mut buf = Vec::new();
    let n = tokio::time::timeout(timeout, proc.stdout.read_until(b'\n', &mut buf))
        .await
        .map_err(|_| Error::Upstream("notebook cell timed out".into()))?
        .map_err(|e| Error::Upstream(format!("read notebook kernel output: {e}")))?;
    if n == 0 {
        return Err(Error::Upstream(
            "notebook kernel terminated unexpectedly".into(),
        ));
    }
    if buf.len() > MAX_OUTPUT_LINE_BYTES {
        return Err(Error::PayloadTooLarge(
            "notebook cell output exceeded 8 MiB limit".into(),
        ));
    }
    serde_json::from_slice(&buf)
        .map_err(|e| Error::Upstream(format!("parse notebook kernel response: {e}")))
}
