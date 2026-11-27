//! Jupyter / Python Notebook kernel execution engine, Python interpreter
//! discovery (venv / uv / conda / poetry / system Python), live variable
//! inspection, interrupt / restart lifecycle, and GPU / MPS accelerator detection.
//!
//! Runs a lightweight, zero-install persistent Python kernel subprocess over
//! newline-delimited JSON on stdin/stdout using the project's selected Python
//! interpreter (`venv`, `uv`, `conda`, `poetry`, or system `python3`). Captures
//! stdout/stderr streams, AST-evaluated final expressions, `_repr_html_()`
//! tables (pandas/polars), `_repr_png_()`/`_repr_svg_()` and `matplotlib` plots
//! as `image/png`, formatted tracebacks, and live global variables.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::Mutex as TokioMutex;

use crate::environments::db_store::discover_local_python_venvs;
use crate::types::{IdeMlAcceleratorInfo, IdeProjectEnvironment, IdeTargetSpec};

const MAX_ACTIVE_KERNELS: usize = 12;
const DEFAULT_CELL_TIMEOUT_SECS: u64 = 120;
const MAX_OUTPUT_LINE_BYTES: usize = 8 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Public API Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeNotebookKernelInfo {
    pub id: String,
    pub display_name: String,
    pub language: String,
    pub interpreter: String,
    /// `"venv"` | `"uv"` | `"conda"` | `"poetry"` | `"system"`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub active: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeNotebookVariable {
    pub name: String,
    pub var_type: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    pub preview: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeNotebookKernelsReq {
    pub target: IdeTargetSpec,
    #[serde(default)]
    pub notebook_path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeNotebookKernelsResp {
    pub kernels: Vec<IdeNotebookKernelInfo>,
    pub accelerators: Vec<IdeMlAcceleratorInfo>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_interpreter: Option<String>,
    #[serde(default)]
    pub execution_count: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeNotebookExecuteReq {
    pub target: IdeTargetSpec,
    pub notebook_path: String,
    pub code: String,
    #[serde(default)]
    pub interpreter: Option<String>,
    #[serde(default)]
    pub timeout_secs: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeNotebookExecuteResp {
    pub session_id: String,
    pub interpreter: String,
    pub execution_count: u32,
    pub status: String,
    pub elapsed_ms: u64,
    pub outputs: Vec<Value>,
    pub variables: Vec<IdeNotebookVariable>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeNotebookSessionReq {
    pub target: IdeTargetSpec,
    pub notebook_path: String,
    #[serde(default)]
    pub interpreter: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeNotebookVariablesResp {
    pub session_id: Option<String>,
    pub execution_count: u32,
    pub variables: Vec<IdeNotebookVariable>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeNotebookOkResp {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

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

// ---------------------------------------------------------------------------
// GPU / MPS / CUDA Accelerator Detection
// ---------------------------------------------------------------------------

/// Fast (<5ms on macOS) local ML hardware accelerator probe (Apple Silicon MPS,
/// NVIDIA CUDA via `nvidia-smi`, Linux ROCm `/dev/kfd`).
#[allow(clippy::disallowed_methods)] // called inside spawn_blocking
pub fn detect_local_accelerators() -> Vec<IdeMlAcceleratorInfo> {
    let mut out = Vec::new();
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        let brand = std::process::Command::new("sysctl")
            .args(["-n", "machdep.cpu.brand_string"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Apple Silicon".to_string());
        let mem_mb = std::process::Command::new("sysctl")
            .args(["-n", "hw.memsize"])
            .output()
            .ok()
            .filter(|o| o.status.success())
            .and_then(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .parse::<u64>()
                    .ok()
            })
            .map(|bytes| bytes / (1024 * 1024));
        out.push(IdeMlAcceleratorInfo {
            kind: "mps".into(),
            name: format!("{brand} (Metal MPS)"),
            memory_mb: mem_mb,
            available: true,
        });
    }
    if let Ok(o) = std::process::Command::new("nvidia-smi")
        .args([
            "--query-gpu=name,memory.total",
            "--format=csv,noheader,nounits",
        ])
        .output()
    {
        if o.status.success() {
            for line in String::from_utf8_lossy(&o.stdout).lines() {
                let mut parts = line.split(',');
                let name = parts.next().map(str::trim).unwrap_or("");
                if !name.is_empty() {
                    let mem_mb = parts.next().and_then(|s| s.trim().parse::<u64>().ok());
                    out.push(IdeMlAcceleratorInfo {
                        kind: "cuda".into(),
                        name: name.to_string(),
                        memory_mb: mem_mb,
                        available: true,
                    });
                }
            }
        }
    }
    #[cfg(target_os = "linux")]
    {
        if Path::new("/dev/kfd").exists() && out.is_empty() {
            out.push(IdeMlAcceleratorInfo {
                kind: "rocm".into(),
                name: "AMD ROCm GPU".into(),
                memory_mb: None,
                available: true,
            });
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Kernel Discovery (venv / uv / conda / poetry / system Python interpreters)
// ---------------------------------------------------------------------------

/// Compute a stable environment identity key for deduplication without collapsing
/// two distinct virtual environments whose `bin/python` symlinks target the same
/// base Python executable.
fn python_env_dedup_key(interpreter: &str, kind: &str) -> String {
    let p = Path::new(interpreter);
    if kind != "system" {
        if let Some(parent) = p.parent() {
            let env_root = if parent.file_name().and_then(|s| s.to_str()) == Some("bin") {
                parent.parent().unwrap_or(parent)
            } else {
                parent
            };
            let canon_root =
                std::fs::canonicalize(env_root).unwrap_or_else(|_| env_root.to_path_buf());
            return format!("env:{}", canon_root.to_string_lossy());
        }
    }
    let canon = std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    format!("sys:{}", canon.to_string_lossy())
}

#[allow(clippy::disallowed_methods)] // bounded manifest reads executed inside spawn_blocking
pub fn discover_notebook_kernels(
    root: &Path,
    data_dir: &Path,
    project_env: Option<&IdeProjectEnvironment>,
) -> Vec<IdeNotebookKernelInfo> {
    let mut kernels = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let raw_active = project_env
        .and_then(|e| e.python_interpreter.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty());
    let active_env_key = raw_active.map(|s| python_env_dedup_key(s, "venv"));
    let active_sys_key = raw_active.map(|s| python_env_dedup_key(s, "system"));

    let mut push_kernel = |id: String,
                           display_name: String,
                           interpreter: String,
                           kind: &str,
                           version: Option<String>| {
        let dedup = python_env_dedup_key(&interpreter, kind);
        if seen.contains(&dedup) {
            return;
        }
        seen.push(dedup.clone());
        let active = raw_active == Some(interpreter.as_str())
            || (kind != "system" && active_env_key.as_deref() == Some(dedup.as_str()))
            || (kind == "system" && active_sys_key.as_deref() == Some(dedup.as_str()));
        kernels.push(IdeNotebookKernelInfo {
            id,
            display_name,
            language: "python".into(),
            interpreter,
            kind: kind.to_string(),
            version,
            active,
        });
    };

    // 1. Project & host virtualenvs (venv / uv / conda / poetry)
    for venv in discover_local_python_venvs(root, data_dir) {
        let bin = if Path::new(&venv.path).join("bin/python").is_file() {
            format!("{}/bin/python", venv.path.trim_end_matches('/'))
        } else if Path::new(&venv.path).join("bin/python3").is_file() {
            format!("{}/bin/python3", venv.path.trim_end_matches('/'))
        } else {
            continue;
        };
        let folder = Path::new(&venv.path)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("python");
        let label = format!("Python {} ({}: {})", venv.version, venv.kind, folder);
        push_kernel(
            format!("{}:{folder}", venv.kind),
            label,
            bin,
            &venv.kind,
            Some(venv.version),
        );
    }

    // 2. System python3 fallback (standard Python execution)
    let search_dirs = crate::toolchains::system_search_dirs();
    if let Some(py3) = crate::toolchains::find_executable_in(&search_dirs, &["python3", "python"]) {
        let py_str = py3.to_string_lossy().into_owned();
        let ver = std::process::Command::new(&py3)
            .arg("--version")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| {
                String::from_utf8_lossy(&o.stdout)
                    .trim()
                    .trim_start_matches("Python ")
                    .to_string()
            });
        let label = match &ver {
            Some(v) if !v.is_empty() => format!("Python {v} (system)"),
            _ => "Python 3 (system)".into(),
        };
        push_kernel("system:python3".into(), label, py_str, "system", ver);
    }

    if !kernels.iter().any(|k| k.active) {
        if let Some(first) = kernels.first_mut() {
            first.active = true;
        }
    }
    kernels
}

// ---------------------------------------------------------------------------
// Persistent Notebook Kernel Manager
// ---------------------------------------------------------------------------

struct KernelProc {
    session_id: String,
    interpreter: String,
    cwd: String,
    pid: Option<u32>,
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    execution_count: u32,
    last_used: Instant,
}

impl KernelProc {
    fn terminate_tree(&mut self) {
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

#[derive(Clone, Hash, PartialEq, Eq)]
struct KernelKey {
    owner: String,
    project_key: String,
    notebook_path: String,
}

/// Per-notebook kernel slot owning both its async mutex and its own active child PID.
/// Storing the PID per slot (`AtomicU32`, `0` while spawning/stopped) makes cross-slot
/// PID interference structurally impossible across both delayed teardown (`CONC2-1`) and
/// pre-ready replacement startup (`CONC3-1`).
struct KernelSlotInner {
    proc: TokioMutex<Option<KernelProc>>,
    pid: AtomicU32,
}

impl KernelSlotInner {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            proc: TokioMutex::new(None),
            pid: AtomicU32::new(0),
        })
    }
}

type KernelSlot = Arc<KernelSlotInner>;

/// Manages persistent per-notebook Python kernel processes for the IDE.
#[derive(Clone, Default)]
pub struct NotebookManager {
    sessions: Arc<StdMutex<HashMap<KernelKey, KernelSlot>>>,
}

impl NotebookManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn active_session_summary(
        &self,
        owner: &str,
        project_key: &str,
        notebook_path: &str,
    ) -> Option<(String, String, u32)> {
        let key = KernelKey {
            owner: owner.to_string(),
            project_key: project_key.to_string(),
            notebook_path: notebook_path.to_string(),
        };
        let guard = self.sessions.lock().ok()?;
        let slot = guard.get(&key)?.clone();
        drop(guard);
        slot.proc.try_lock().ok().and_then(|opt| {
            opt.as_ref().map(|k| {
                (
                    k.session_id.clone(),
                    k.interpreter.clone(),
                    k.execution_count,
                )
            })
        })
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn execute_cell(
        &self,
        owner: &str,
        project_key: &str,
        notebook_path: &str,
        cwd: &str,
        interpreter: &str,
        env: &BTreeMap<String, String>,
        code: &str,
        timeout: Duration,
    ) -> Result<IdeNotebookExecuteResp> {
        let started = Instant::now();
        let key = KernelKey {
            owner: owner.to_string(),
            project_key: project_key.to_string(),
            notebook_path: notebook_path.to_string(),
        };
        let slot = self.get_or_create_slot(&key).await?;
        let mut slot_guard = slot.proc.lock().await;
        ensure_slot_kernel(&slot, &mut slot_guard, interpreter, cwd, env).await?;
        let proc = slot_guard
            .as_mut()
            .ok_or_else(|| Error::Internal("notebook kernel missing after spawn".into()))?;
        proc.last_used = Instant::now();

        let payload = json!({ "cmd": "exec", "code": code });
        let resp = send_kernel_rpc(proc, &payload, timeout).await;
        let val = match resp {
            Ok(v) => v,
            Err(e) => {
                // Evict broken kernel so the next run spawns cleanly.
                clear_slot_kernel(&slot, &mut slot_guard);
                return Err(e);
            }
        };

        let exec_count = val
            .get("execution_count")
            .and_then(Value::as_u64)
            .unwrap_or((proc.execution_count + 1) as u64) as u32;
        proc.execution_count = exec_count;
        let status = val
            .get("status")
            .and_then(Value::as_str)
            .unwrap_or("ok")
            .to_string();
        let outputs = val
            .get("outputs")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let variables: Vec<IdeNotebookVariable> = val
            .get("variables")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();

        Ok(IdeNotebookExecuteResp {
            session_id: proc.session_id.clone(),
            interpreter: proc.interpreter.clone(),
            execution_count: exec_count,
            status,
            elapsed_ms: started.elapsed().as_millis() as u64,
            outputs,
            variables,
        })
    }

    pub async fn list_variables(
        &self,
        owner: &str,
        project_key: &str,
        notebook_path: &str,
    ) -> Result<IdeNotebookVariablesResp> {
        let Some(slot) = self.get_existing_slot(owner, project_key, notebook_path) else {
            return Ok(IdeNotebookVariablesResp {
                session_id: None,
                execution_count: 0,
                variables: Vec::new(),
            });
        };
        let mut slot_guard = slot.proc.lock().await;
        let Some(proc) = slot_guard.as_mut() else {
            return Ok(IdeNotebookVariablesResp {
                session_id: None,
                execution_count: 0,
                variables: Vec::new(),
            });
        };
        let val = send_kernel_rpc(proc, &json!({ "cmd": "vars" }), Duration::from_secs(5)).await?;
        let variables: Vec<IdeNotebookVariable> = val
            .get("variables")
            .and_then(|v| serde_json::from_value(v.clone()).ok())
            .unwrap_or_default();
        Ok(IdeNotebookVariablesResp {
            session_id: Some(proc.session_id.clone()),
            execution_count: proc.execution_count,
            variables,
        })
    }

    /// Send `SIGINT` only to the current active slot's kernel process without destroying the session.
    pub fn interrupt(&self, owner: &str, project_key: &str, notebook_path: &str) -> bool {
        let Some(slot) = self.get_existing_slot(owner, project_key, notebook_path) else {
            return false;
        };
        let pid = slot.pid.load(Ordering::Acquire);
        if pid == 0 {
            return false;
        }
        // BUG2-11: Interrupt signals only the Python kernel process (SIGINT to `pid`),
        // leaving independent background workers started by earlier cells running.
        // Restart and shutdown continue to terminate the entire process group (`-pid`).
        #[cfg(unix)]
        {
            send_unix_signal_pid(pid, 2 /* SIGINT */)
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            false
        }
    }

    pub async fn restart(
        &self,
        owner: &str,
        project_key: &str,
        notebook_path: &str,
        cwd: &str,
        interpreter: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<String> {
        let key = KernelKey {
            owner: owner.to_string(),
            project_key: project_key.to_string(),
            notebook_path: notebook_path.to_string(),
        };
        let slot = self.get_or_create_slot(&key).await?;
        let mut slot_guard = slot.proc.lock().await;
        clear_slot_kernel(&slot, &mut slot_guard);
        ensure_slot_kernel(&slot, &mut slot_guard, interpreter, cwd, env).await?;
        let proc = slot_guard
            .as_ref()
            .ok_or_else(|| Error::Internal("notebook kernel missing after restart".into()))?;
        Ok(proc.session_id.clone())
    }

    pub async fn shutdown_session(&self, owner: &str, project_key: &str, notebook_path: &str) {
        let key = KernelKey {
            owner: owner.to_string(),
            project_key: project_key.to_string(),
            notebook_path: notebook_path.to_string(),
        };
        if let Some(slot) = self.remove_slot(&key) {
            let mut slot_guard = slot.proc.lock().await;
            clear_slot_kernel(&slot, &mut slot_guard);
        }
    }

    fn get_existing_slot(
        &self,
        owner: &str,
        project_key: &str,
        notebook_path: &str,
    ) -> Option<KernelSlot> {
        let key = KernelKey {
            owner: owner.to_string(),
            project_key: project_key.to_string(),
            notebook_path: notebook_path.to_string(),
        };
        self.sessions.lock().ok()?.get(&key).cloned()
    }

    fn remove_slot(&self, key: &KernelKey) -> Option<KernelSlot> {
        self.sessions.lock().ok()?.remove(key)
    }

    /// Atomically returns the per-notebook slot mutex so concurrent first runs
    /// serialize kernel creation and share a single kernel process.
    async fn get_or_create_slot(&self, key: &KernelKey) -> Result<KernelSlot> {
        let mut evicted: Option<KernelSlot> = None;
        let slot = {
            let mut guard = self
                .sessions
                .lock()
                .map_err(|_| Error::Internal("notebook session lock poisoned".into()))?;
            if let Some(existing) = guard.get(key) {
                Arc::clone(existing)
            } else {
                if guard.len() >= MAX_ACTIVE_KERNELS {
                    if let Some(oldest_key) = guard.keys().next().cloned() {
                        if let Some(old_slot) = guard.remove(&oldest_key) {
                            evicted = Some(old_slot);
                        }
                    }
                }
                let fresh = KernelSlotInner::new();
                guard.insert(key.clone(), Arc::clone(&fresh));
                fresh
            }
        };
        if let Some(old_slot) = evicted {
            let mut g = old_slot.proc.lock().await;
            clear_slot_kernel(&old_slot, &mut g);
        }
        Ok(slot)
    }
}

fn clear_slot_kernel(slot: &KernelSlot, slot_guard: &mut Option<KernelProc>) {
    slot.pid.store(0, Ordering::Release);
    if let Some(mut proc) = slot_guard.take() {
        proc.terminate_tree();
    }
}

async fn ensure_slot_kernel(
    slot: &KernelSlot,
    slot_guard: &mut Option<KernelProc>,
    interpreter: &str,
    cwd: &str,
    env: &BTreeMap<String, String>,
) -> Result<()> {
    let reuse = if let Some(proc) = slot_guard.as_mut() {
        let alive = proc.child.try_wait().ok().flatten().is_none();
        alive && proc.interpreter == interpreter && proc.cwd == cwd
    } else {
        false
    };
    if reuse {
        return Ok(());
    }
    clear_slot_kernel(slot, slot_guard);
    let proc = spawn_python_kernel(interpreter, cwd, env).await?;
    slot.pid.store(proc.pid.unwrap_or(0), Ordering::Release);
    *slot_guard = Some(proc);
    Ok(())
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn send_unix_signal_pid(pid: u32, sig: std::ffi::c_int) -> bool {
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
fn send_unix_signal_group(pid: u32, sig: std::ffi::c_int) -> bool {
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

async fn spawn_python_kernel(
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

async fn send_kernel_rpc(
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

pub fn default_cell_timeout(timeout_secs: Option<u64>) -> Duration {
    Duration::from_secs(
        timeout_secs
            .unwrap_or(DEFAULT_CELL_TIMEOUT_SECS)
            .clamp(1, 3600),
    )
}

/// Extract Python source from a `.ipynb` notebook (single cell by 0-based `cell_idx`, or all
/// code cells joined with `# %%` cell markers) for DAP debugging.
pub fn extract_ipynb_cell_python(raw_ipynb: &str, cell_idx: Option<usize>) -> Result<String> {
    let val: Value = serde_json::from_str(raw_ipynb)
        .map_err(|e| Error::Invalid(format!("invalid .ipynb JSON: {e}")))?;
    let cells = val
        .get("cells")
        .and_then(Value::as_array)
        .ok_or_else(|| Error::Invalid(".ipynb missing 'cells' array".into()))?;
    let join_src = |cell: &Value| -> String {
        match cell.get("source") {
            Some(Value::String(s)) => s.clone(),
            Some(Value::Array(arr)) => arr
                .iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(""),
            _ => String::new(),
        }
    };
    if let Some(idx) = cell_idx {
        let cell = cells
            .get(idx)
            .ok_or_else(|| Error::Invalid(format!("notebook cell index {idx} out of range")))?;
        return Ok(join_src(cell));
    }
    let mut out = String::new();
    for (i, cell) in cells.iter().enumerate() {
        if cell.get("cell_type").and_then(Value::as_str) == Some("code") {
            out.push_str(&format!("# %% cell {i}\n"));
            out.push_str(&join_src(cell));
            out.push_str("\n\n");
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn live_python_notebook_kernel_executes_rich_outputs_vars_interrupt_and_restart() {
        let search = crate::toolchains::system_search_dirs();
        let Some(py3) = crate::toolchains::find_executable_in(&search, &["python3", "python"])
        else {
            return;
        };
        let py_str = py3.to_string_lossy().into_owned();
        let tmp = tempfile::tempdir().unwrap();
        let cwd = tmp.path().to_string_lossy().into_owned();
        let mut env = BTreeMap::new();
        if let Ok(path) = std::env::var("PATH") {
            env.insert("PATH".into(), path);
        }

        let mgr = NotebookManager::new();
        // 1. Define variable + stdout
        let r1 = mgr
            .execute_cell(
                "u1",
                "local:/tmp/nb",
                "demo.ipynb",
                &cwd,
                &py_str,
                &env,
                "x = 21\nprint('hello notebook')\n",
                Duration::from_secs(10),
            )
            .await
            .expect("cell 1");
        assert_eq!(r1.status, "ok");
        assert_eq!(r1.execution_count, 1);
        assert!(r1.outputs.iter().any(|o| o["output_type"] == "stream"));
        assert!(r1
            .variables
            .iter()
            .any(|v| v.name == "x" && v.preview == "21"));

        // 2. Evaluate trailing expression using persisted variable
        let r2 = mgr
            .execute_cell(
                "u1",
                "local:/tmp/nb",
                "demo.ipynb",
                &cwd,
                &py_str,
                &env,
                "x * 2",
                Duration::from_secs(10),
            )
            .await
            .expect("cell 2");
        assert_eq!(r2.execution_count, 2);
        let res = r2
            .outputs
            .iter()
            .find(|o| o["output_type"] == "execute_result")
            .expect("execute_result");
        assert_eq!(res["data"]["text/plain"], "42");

        // 3. Simulated rich HTML (_repr_html_) delivery (not a real pandas DataFrame)
        let r3 = mgr
            .execute_cell(
                "u1",
                "local:/tmp/nb",
                "demo.ipynb",
                &cwd,
                &py_str,
                &env,
                "class SimulatedHtmlObj:\n  def _repr_html_(self): return '<table><tr><th>a</th></tr><tr><td>1</td></tr></table>'\nSimulatedHtmlObj()",
                Duration::from_secs(10),
            )
            .await
            .expect("cell 3");
        let html_res = r3
            .outputs
            .iter()
            .find(|o| o["output_type"] == "execute_result")
            .expect("html execute_result");
        assert!(html_res["data"]["text/html"]
            .as_str()
            .unwrap()
            .contains("<table>"));

        // 3b. Subprocess writing directly to OS fd 1 must not corrupt protocol (BUG-37)
        let r_sub = mgr
            .execute_cell(
                "u1",
                "local:/tmp/nb",
                "demo.ipynb",
                &cwd,
                &py_str,
                &env,
                "import subprocess, sys\nsubprocess.run([sys.executable, '-c', 'print(\"from-subproc\")'], check=True)\n'still-alive'",
                Duration::from_secs(10),
            )
            .await
            .expect("cell with subprocess stdout");
        assert_eq!(r_sub.status, "ok");
        assert_eq!(r_sub.session_id, r1.session_id);
        let sub_stdout = r_sub
            .outputs
            .iter()
            .find(|o| o["output_type"] == "stream" && o["name"] == "stdout")
            .expect("captured subprocess stdout");
        assert!(sub_stdout["text"]
            .as_array()
            .unwrap()
            .iter()
            .any(|l| l.as_str().unwrap_or("").contains("from-subproc")));

        // 4. Runtime error does not kill kernel state
        let r4 = mgr
            .execute_cell(
                "u1",
                "local:/tmp/nb",
                "demo.ipynb",
                &cwd,
                &py_str,
                &env,
                "1 / 0",
                Duration::from_secs(10),
            )
            .await
            .expect("cell 4");
        assert_eq!(r4.status, "error");
        assert_eq!(r4.outputs[0]["ename"], "ZeroDivisionError");

        // 5. Variables endpoint still sees `x`
        let vars = mgr
            .list_variables("u1", "local:/tmp/nb", "demo.ipynb")
            .await
            .expect("vars");
        assert!(vars
            .variables
            .iter()
            .any(|v| v.name == "x" && v.preview == "21"));

        // 6. Interrupt a sleeping cell cleanly via SIGINT
        let mgr_int = mgr.clone();
        let cwd_int = cwd.clone();
        let py_int = py_str.clone();
        let env_int = env.clone();
        let exec_task = tokio::spawn(async move {
            mgr_int
                .execute_cell(
                    "u1",
                    "local:/tmp/nb",
                    "demo.ipynb",
                    &cwd_int,
                    &py_int,
                    &env_int,
                    "import time\ntime.sleep(30)",
                    Duration::from_secs(10),
                )
                .await
        });
        tokio::time::sleep(Duration::from_millis(250)).await;
        assert!(mgr.interrupt("u1", "local:/tmp/nb", "demo.ipynb"));
        let r_int = exec_task.await.unwrap().expect("interrupted cell");
        assert_eq!(r_int.status, "error");
        assert_eq!(r_int.outputs[0]["ename"], "KeyboardInterrupt");

        // 7. Restart clears kernel state
        let new_id = mgr
            .restart("u1", "local:/tmp/nb", "demo.ipynb", &cwd, &py_str, &env)
            .await
            .expect("restart");
        assert_ne!(new_id, r1.session_id);
        let vars_after = mgr
            .list_variables("u1", "local:/tmp/nb", "demo.ipynb")
            .await
            .expect("vars after restart");
        assert!(vars_after.variables.is_empty());

        // 8. Background child worker spawned by a cell is killed with the kernel process group (OBS-3)
        #[cfg(unix)]
        {
            let r_bg = mgr
                .execute_cell(
                    "u1",
                    "local:/tmp/nb",
                    "demo.ipynb",
                    &cwd,
                    &py_str,
                    &env,
                    "import subprocess, sys\n_bg = subprocess.Popen([sys.executable, '-c', 'import time; time.sleep(30)'])\n_bg.pid",
                    Duration::from_secs(10),
                )
                .await
                .expect("spawn bg worker");
            let child_pid: u32 = r_bg
                .outputs
                .iter()
                .find(|o| o["output_type"] == "execute_result")
                .and_then(|o| o["data"]["text/plain"].as_str())
                .and_then(|s| s.trim().parse().ok())
                .expect("child pid");
            mgr.shutdown_session("u1", "local:/tmp/nb", "demo.ipynb")
                .await;
            tokio::time::sleep(Duration::from_millis(150)).await;
            assert!(
                !send_unix_signal_group(child_pid, 0),
                "background worker {child_pid} should be terminated with the kernel process group"
            );
        }
        #[cfg(not(unix))]
        mgr.shutdown_session("u1", "local:/tmp/nb", "demo.ipynb")
            .await;

        // 9. Concurrent first runs on a brand-new notebook share one kernel (CONC-5)
        let (c1, c2) = tokio::join!(
            mgr.execute_cell(
                "u1",
                "local:/tmp/nb-conc",
                "race.ipynb",
                &cwd,
                &py_str,
                &env,
                "first_var = 101",
                Duration::from_secs(10),
            ),
            mgr.execute_cell(
                "u1",
                "local:/tmp/nb-conc",
                "race.ipynb",
                &cwd,
                &py_str,
                &env,
                "second_var = 202",
                Duration::from_secs(10),
            ),
        );
        let c1 = c1.expect("concurrent run 1");
        let c2 = c2.expect("concurrent run 2");
        assert_eq!(
            c1.session_id, c2.session_id,
            "concurrent first runs must share a single kernel session"
        );
        let conc_vars = mgr
            .list_variables("u1", "local:/tmp/nb-conc", "race.ipynb")
            .await
            .expect("concurrent vars");
        assert!(conc_vars.variables.iter().any(|v| v.name == "first_var"));
        assert!(conc_vars.variables.iter().any(|v| v.name == "second_var"));
        mgr.shutdown_session("u1", "local:/tmp/nb-conc", "race.ipynb")
            .await;

        // BUG2-12: Libraries/loggers retaining sys.stderr across cells never hit closed-file errors.
        let r_log_setup = mgr
            .execute_cell(
                "u1",
                "local:/tmp/nb-streams",
                "streams.ipynb",
                &cwd,
                &py_str,
                &BTreeMap::new(),
                "import logging, sys\nsaved_err = sys.stderr\nlogger = logging.getLogger('otto_nb_test')\nlogger.setLevel(logging.WARNING)\nh = logging.StreamHandler(sys.stderr)\nlogger.addHandler(h)\nprint('cell1-ok')\n",
                Duration::from_secs(10),
            )
            .await
            .expect("stream setup cell");
        assert_eq!(r_log_setup.status, "ok");
        let r_log_use = mgr
            .execute_cell(
                "u1",
                "local:/tmp/nb-streams",
                "streams.ipynb",
                &cwd,
                &py_str,
                &BTreeMap::new(),
                "saved_err.write('direct-retained-err\\n')\nlogger.warning('logged-warning-cell2')\n",
                Duration::from_secs(10),
            )
            .await
            .expect("stream reuse cell");
        assert_eq!(
            r_log_use.status, "ok",
            "BUG2-12: retained stream must stay open: {r_log_use:?}"
        );
        let err_stream_text: String = r_log_use
            .outputs
            .iter()
            .filter(|o| {
                o.get("output_type").and_then(|v| v.as_str()) == Some("stream")
                    && o.get("name").and_then(|v| v.as_str()) == Some("stderr")
            })
            .filter_map(|o| o.get("text"))
            .map(|t| match t {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Array(arr) => {
                    arr.iter().filter_map(|v| v.as_str()).collect::<String>()
                }
                _ => String::new(),
            })
            .collect();
        assert!(
            err_stream_text.contains("direct-retained-err")
                && err_stream_text.contains("logged-warning-cell2"),
            "BUG2-12: retained stderr writes must be captured in cell 2: {r_log_use:?}"
        );

        // BUG3-11: A cell that temporarily reassigns sys.stdout to a file closed by a `with`
        // block must NOT break subsequent cells' `print(...)` calls.
        let r_rebind = mgr
            .execute_cell(
                "u1",
                "local:/tmp/nb-streams",
                "streams.ipynb",
                &cwd,
                &py_str,
                &BTreeMap::new(),
                "import os, sys\nwith open(os.devnull, 'w') as _f:\n    sys.stdout = _f\n    print('to-devnull')\n",
                Duration::from_secs(10),
            )
            .await
            .expect("rebind stdout cell");
        assert_eq!(r_rebind.status, "ok", "rebind cell failed: {r_rebind:?}");
        let r_after_rebind = mgr
            .execute_cell(
                "u1",
                "local:/tmp/nb-streams",
                "streams.ipynb",
                &cwd,
                &py_str,
                &BTreeMap::new(),
                "print('restored-stdout-ok')\n",
                Duration::from_secs(10),
            )
            .await
            .expect("cell after stdout rebind");
        assert_eq!(
            r_after_rebind.status, "ok",
            "BUG3-11: cell after temporary sys.stdout reassignment must succeed: {r_after_rebind:?}"
        );
        let restored_out: String = r_after_rebind
            .outputs
            .iter()
            .filter(|o| o.get("output_type").and_then(|v| v.as_str()) == Some("stream"))
            .filter_map(|o| o.get("text"))
            .map(|t| match t {
                serde_json::Value::String(s) => s.clone(),
                serde_json::Value::Array(arr) => {
                    arr.iter().filter_map(|v| v.as_str()).collect::<String>()
                }
                _ => String::new(),
            })
            .collect();
        assert!(
            restored_out.contains("restored-stdout-ok"),
            "BUG3-11: expected captured stdout after rebind restore, got: {r_after_rebind:?}"
        );
        mgr.shutdown_session("u1", "local:/tmp/nb-streams", "streams.ipynb")
            .await;

        // BUG2-11: Interrupting a running cell sends SIGINT only to the kernel, leaving an
        // independent background worker spawned in an earlier cell alive until shutdown.
        #[cfg(unix)]
        {
            let bg_spawn = mgr
                .execute_cell(
                    "u1",
                    "local:/tmp/nb-int-bg",
                    "bg.ipynb",
                    &cwd,
                    &py_str,
                    &BTreeMap::new(),
                    "import subprocess\n_bg = subprocess.Popen(['sleep', '30'], stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)\n_bg.pid\n",
                    Duration::from_secs(10),
                )
                .await
                .expect("bg worker spawn");
            let bg_pid: i32 = bg_spawn
                .outputs
                .iter()
                .find(|o| o.get("output_type").and_then(|v| v.as_str()) == Some("execute_result"))
                .and_then(|o| o.get("data"))
                .and_then(|d| d.get("text/plain"))
                .and_then(|v| v.as_str())
                .and_then(|s| s.trim().parse().ok())
                .expect("bg worker pid");
            let mgr_bg = mgr.clone();
            let cwd_bg = cwd.clone();
            let py_bg = py_str.clone();
            let long_cell = tokio::spawn(async move {
                mgr_bg
                    .execute_cell(
                        "u1",
                        "local:/tmp/nb-int-bg",
                        "bg.ipynb",
                        &cwd_bg,
                        &py_bg,
                        &BTreeMap::new(),
                        "import time\ntime.sleep(10)\n",
                        Duration::from_secs(15),
                    )
                    .await
            });
            tokio::time::sleep(Duration::from_millis(250)).await;
            assert!(mgr.interrupt("u1", "local:/tmp/nb-int-bg", "bg.ipynb"));
            let int_res = long_cell.await.unwrap().expect("interrupted cell result");
            assert_eq!(int_res.status, "error");
            assert!(
                int_res
                    .outputs
                    .iter()
                    .any(|o| o.get("ename").and_then(|v| v.as_str()) == Some("KeyboardInterrupt")),
                "expected KeyboardInterrupt: {int_res:?}"
            );
            extern "C" {
                fn kill(pid: std::ffi::c_int, sig: std::ffi::c_int) -> std::ffi::c_int;
            }
            #[allow(unsafe_code)]
            let worker_alive = unsafe { kill(bg_pid, 0) == 0 };
            assert!(
                worker_alive,
                "BUG2-11: interrupting cell 2 must NOT kill background worker {bg_pid} from cell 1"
            );
            mgr.shutdown_session("u1", "local:/tmp/nb-int-bg", "bg.ipynb")
                .await;
            tokio::time::sleep(Duration::from_millis(120)).await;
            #[allow(unsafe_code)]
            let worker_alive_after_shutdown = unsafe { kill(bg_pid, 0) == 0 };
            assert!(
                !worker_alive_after_shutdown,
                "shutdown_session must still kill background worker {bg_pid}"
            );
        }

        // CONC2-1 + CONC3-1: Per-slot PID ownership guarantees that:
        // (1) while a replacement slot is published before its interpreter finishes spawning
        //     (`pid == 0`), `interrupt()` returns `false` and NEVER signals the detached old kernel;
        // (2) delayed teardown of the old slot NEVER clears the replacement slot's PID.
        let conc_key = KernelKey {
            owner: "u1".into(),
            project_key: "local:/tmp/nb-conc2".into(),
            notebook_path: "replace.ipynb".into(),
        };
        let old_slot = mgr.get_or_create_slot(&conc_key).await.unwrap();
        old_slot.pid.store(std::process::id(), Ordering::Release);
        let detached = mgr.remove_slot(&conc_key).expect("detach old slot");
        let replacement_slot = mgr.get_or_create_slot(&conc_key).await.unwrap();
        assert_eq!(replacement_slot.pid.load(Ordering::Acquire), 0);
        assert!(
            !mgr.interrupt("u1", "local:/tmp/nb-conc2", "replace.ipynb"),
            "CONC3-1: interrupt during replacement startup must return false and never signal detached old kernel"
        );
        replacement_slot.pid.store(999_002, Ordering::Release);
        {
            let mut old_guard = detached.proc.lock().await;
            clear_slot_kernel(&detached, &mut old_guard);
        }
        assert_eq!(
            replacement_slot.pid.load(Ordering::Acquire),
            999_002,
            "CONC2-1: old slot teardown must never clear replacement slot PID"
        );
        mgr.shutdown_session("u1", "local:/tmp/nb-conc2", "replace.ipynb")
            .await;
    }

    #[test]
    fn accelerator_and_kernel_discovery_and_ipynb_extraction_work() {
        let accels = detect_local_accelerators();
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        assert!(accels.iter().any(|a| a.kind == "mps" && a.available));
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        let _ = accels;

        let tmp = tempfile::tempdir().unwrap();
        // RED-5: Even when Python and non-Python Jupyter kernelspec directories exist on disk
        // (including a valid executable Python kernelspec that the old scanner advertised),
        // the built-in standard Python runner must emit zero `kernelspec` entries.
        let search = crate::toolchains::system_search_dirs();
        let real_py = crate::toolchains::find_executable_in(&search, &["python3"])
            .unwrap_or_else(|| "/usr/bin/python3".into());
        let py_kspec = tmp
            .path()
            .join(".venv/share/jupyter/kernels/python3-custom");
        std::fs::create_dir_all(&py_kspec).unwrap();
        std::fs::write(
            py_kspec.join("kernel.json"),
            serde_json::json!({
                "argv": [real_py, "-m", "ipykernel_launcher", "-f", "{connection_file}"],
                "display_name": "Custom Py Kernelspec",
                "language": "python"
            })
            .to_string(),
        )
        .unwrap();
        let fake_kspec = tmp.path().join(".venv/share/jupyter/kernels/ir");
        std::fs::create_dir_all(&fake_kspec).unwrap();
        std::fs::write(
            fake_kspec.join("kernel.json"),
            r#"{"argv":["R","--slave"],"display_name":"R","language":"R"}"#,
        )
        .unwrap();
        let data_kspec = tmp.path().join("share/jupyter/kernels/julia-1.10");
        std::fs::create_dir_all(&data_kspec).unwrap();
        std::fs::write(
            data_kspec.join("kernel.json"),
            r#"{"argv":["julia"],"display_name":"Julia","language":"julia"}"#,
        )
        .unwrap();
        let kernels = discover_notebook_kernels(tmp.path(), tmp.path(), None);
        assert!(
            !kernels.is_empty(),
            "expected at least system python3 kernel on host"
        );
        assert!(
            !kernels.iter().any(|k| {
                k.kind == "kernelspec"
                    || k.id.starts_with("kernelspec:")
                    || k.display_name == "Custom Py Kernelspec"
            }),
            "built-in Python runner must not advertise Jupyter kernelspec protocol: {kernels:?}"
        );

        // Distinct virtualenvs symlinking to the same host Python binary stay distinct (BUG-38)
        #[cfg(unix)]
        {
            let search = crate::toolchains::system_search_dirs();
            if let Some(sys_py) = crate::toolchains::find_executable_in(&search, &["python3"]) {
                let venv_proj = tempfile::tempdir().unwrap();
                for name in [".venv", "venv"] {
                    let vdir = venv_proj.path().join(name);
                    std::fs::create_dir_all(vdir.join("bin")).unwrap();
                    std::fs::write(vdir.join("pyvenv.cfg"), "version = 3.12.0\n").unwrap();
                    std::os::unix::fs::symlink(&sys_py, vdir.join("bin/python")).unwrap();
                }
                let found = discover_notebook_kernels(venv_proj.path(), venv_proj.path(), None);
                let env_kernels: Vec<_> = found.iter().filter(|k| k.kind == "venv").collect();
                assert!(
                    env_kernels.len() >= 2,
                    "two distinct venvs pointing to the same python binary must both remain selectable: {found:?}"
                );
            }
        }

        let ipynb = r##"{"cells":[{"cell_type":"markdown","source":["# hi"]},{"cell_type":"code","source":["a = 1\n","b = 2\n"]}],"metadata":{},"nbformat":4,"nbformat_minor":5}"##;
        let cell1 = extract_ipynb_cell_python(ipynb, Some(1)).unwrap();
        assert_eq!(cell1, "a = 1\nb = 2\n");

        // Verify ML training, TensorBoard, and Jupyter Lab auto-discovery (BUG-39 & SEC-9)
        let ml_dir = tempfile::tempdir().unwrap();
        std::fs::write(ml_dir.path().join("train.py"), "print('epoch 1')\n").unwrap();
        std::fs::write(
            ml_dir.path().join("requirements.txt"),
            "torch==2.5.0\ntensorboard>=2.18\n",
        )
        .unwrap();
        std::fs::create_dir_all(ml_dir.path().join("runs")).unwrap();
        std::fs::write(ml_dir.path().join("experiment.ipynb"), ipynb).unwrap();
        let target = crate::types::IdeTargetSpec {
            kind: crate::types::IdeTargetKind::Local,
            workspace_id: None,
            root: ml_dir.path().to_string_lossy().into_owned(),
            connection_id: None,
            additional_roots: Vec::new(),
            project_folders: Vec::new(),
        };
        let discovered = crate::environments::discover_run_configs(&target, &[]);
        let ml_cfg = discovered
            .iter()
            .find(|c| c.name.contains("ML Train"))
            .expect("expected ML Train auto-config");
        assert_eq!(
            ml_cfg
                .env_overrides
                .get("PYTHONUNBUFFERED")
                .map(String::as_str),
            Some("1")
        );
        assert!(
            !ml_cfg.args.iter().any(|a| a == "-u"),
            "ML Train must not pass -u as a script arg: {:?}",
            ml_cfg.args
        );
        let tb_cfg = discovered
            .iter()
            .find(|c| c.name.contains("TensorBoard"))
            .expect("expected TensorBoard auto-config");
        assert!(tb_cfg.args.iter().any(|a| a == "tensorboard.main"));
        assert!(
            !tb_cfg.args.iter().any(|a| a == "--bind_all"),
            "TensorBoard auto-config must stay on loopback by default: {:?}",
            tb_cfg.args
        );
        assert!(
            discovered.iter().any(|c| c.name == "Jupyter Lab"),
            "expected Jupyter Lab auto-config: {discovered:?}"
        );
    }
}
