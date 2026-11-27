//! Jupyter / Python Notebook kernel execution engine, kernelspec & interpreter
//! discovery (venv / uv / conda / poetry / Jupyter kernelspecs), live variable
//! inspection, interrupt / restart lifecycle, and GPU / MPS accelerator detection.
//!
//! Runs a lightweight, zero-install persistent Python kernel subprocess over
//! newline-delimited JSON on stdin/stdout using the project's selected Python
//! interpreter (`venv`, `uv`, `conda`, `poetry`, or system `python3`). Captures
//! stdout/stderr streams, AST-evaluated final expressions, `_repr_html_()`
//! tables (pandas/polars), `_repr_png_()`/`_repr_svg_()` and `matplotlib` plots
//! as `image/png`, formatted tracebacks, and live global variables.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
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
    /// `"venv"` | `"uv"` | `"conda"` | `"poetry"` | `"kernelspec"` | `"system"`.
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
import ast, base64, io, json, os, signal, sys, traceback

os.environ.setdefault("MPLBACKEND", "Agg")

_ORIG_OUT = sys.stdout
_ORIG_ERR = sys.stderr

class _StreamCapture(io.StringIO):
    pass

_NS = {"__name__": "__main__", "__doc__": None}
_EXEC_COUNT = 0

def _emit(obj):
    _ORIG_OUT.write(json.dumps(obj, ensure_ascii=False) + "\n")
    _ORIG_OUT.flush()

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
    cap_out = _StreamCapture()
    cap_err = _StreamCapture()
    sys.stdout = cap_out
    sys.stderr = cap_err
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
        sys.stdout = _ORIG_OUT
        sys.stderr = _ORIG_ERR

    out_txt = cap_out.getvalue()
    err_txt = cap_err.getvalue()
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
// Kernel Discovery (venv / uv / conda / poetry / Jupyter kernelspecs / system)
// ---------------------------------------------------------------------------

#[allow(clippy::disallowed_methods)] // bounded manifest reads executed inside spawn_blocking
pub fn discover_notebook_kernels(
    root: &Path,
    data_dir: &Path,
    project_env: Option<&IdeProjectEnvironment>,
) -> Vec<IdeNotebookKernelInfo> {
    let mut kernels = Vec::new();
    let mut seen: Vec<String> = Vec::new();
    let active_py = project_env
        .and_then(|e| e.python_interpreter.as_deref())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| {
            std::fs::canonicalize(s)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|_| s.to_string())
        });

    let mut push_kernel = |id: String,
                           display_name: String,
                           interpreter: String,
                           kind: &str,
                           version: Option<String>| {
        let canon = std::fs::canonicalize(&interpreter)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|_| interpreter.clone());
        if seen.contains(&canon) {
            return;
        }
        seen.push(canon.clone());
        let active = active_py.as_deref() == Some(canon.as_str())
            || active_py.as_deref() == Some(interpreter.as_str());
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

    // 2. Installed Jupyter kernelspecs (`kernels/*/kernel.json`)
    let mut spec_dirs: Vec<PathBuf> = Vec::new();
    if !otto_core::e2e::isolated() {
        if let Some(home) = dirs::home_dir() {
            spec_dirs.push(home.join(".local/share/jupyter/kernels"));
            spec_dirs.push(home.join("Library/Jupyter/kernels"));
        }
        spec_dirs.push(PathBuf::from("/usr/local/share/jupyter/kernels"));
        spec_dirs.push(PathBuf::from("/opt/homebrew/share/jupyter/kernels"));
    }
    for sub in [".venv", "venv"] {
        spec_dirs.push(root.join(sub).join("share/jupyter/kernels"));
    }
    for sdir in spec_dirs {
        #[allow(clippy::disallowed_methods)]
        let Ok(rd) = std::fs::read_dir(&sdir) else {
            continue;
        };
        for entry in rd.flatten().take(24) {
            let kjson = entry.path().join("kernel.json");
            let Ok(raw) = std::fs::read_to_string(&kjson) else {
                continue;
            };
            let Ok(val) = serde_json::from_str::<Value>(&raw) else {
                continue;
            };
            let lang = val
                .get("language")
                .and_then(Value::as_str)
                .unwrap_or("python");
            if !lang.eq_ignore_ascii_case("python") {
                continue;
            }
            let Some(argv0) = val
                .get("argv")
                .and_then(Value::as_array)
                .and_then(|a| a.first())
                .and_then(Value::as_str)
            else {
                continue;
            };
            if !Path::new(argv0).is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let display = val
                .get("display_name")
                .and_then(Value::as_str)
                .unwrap_or(&name)
                .to_string();
            push_kernel(
                format!("kernelspec:{name}"),
                format!("{display} (Jupyter)"),
                argv0.to_string(),
                "kernelspec",
                None,
            );
        }
    }

    // 3. System python3 fallback
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

impl Drop for KernelProc {
    fn drop(&mut self) {
        let _ = self.child.start_kill();
    }
}

#[derive(Clone, Hash, PartialEq, Eq)]
struct KernelKey {
    owner: String,
    project_key: String,
    notebook_path: String,
}

/// Manages persistent per-notebook Python kernel processes for the IDE.
#[derive(Clone, Default)]
pub struct NotebookManager {
    sessions: Arc<StdMutex<HashMap<KernelKey, Arc<TokioMutex<KernelProc>>>>>,
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
        let arc = guard.get(&key)?.clone();
        drop(guard);
        arc.try_lock().ok().map(|k| {
            (
                k.session_id.clone(),
                k.interpreter.clone(),
                k.execution_count,
            )
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
        let kernel_arc = self
            .get_or_spawn(owner, project_key, notebook_path, cwd, interpreter, env)
            .await?;
        let mut proc = kernel_arc.lock().await;
        proc.last_used = Instant::now();

        let payload = json!({ "cmd": "exec", "code": code });
        let resp = send_kernel_rpc(&mut proc, &payload, timeout).await;
        let val = match resp {
            Ok(v) => v,
            Err(e) => {
                // Evict broken kernel so the next run spawns cleanly.
                drop(proc);
                self.remove_session(owner, project_key, notebook_path);
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
        let Some(kernel_arc) = self.get_existing(owner, project_key, notebook_path) else {
            return Ok(IdeNotebookVariablesResp {
                session_id: None,
                execution_count: 0,
                variables: Vec::new(),
            });
        };
        let mut proc = kernel_arc.lock().await;
        let val =
            send_kernel_rpc(&mut proc, &json!({ "cmd": "vars" }), Duration::from_secs(5)).await?;
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

    /// Send `SIGINT` to the active kernel process without destroying the session.
    pub fn interrupt(&self, owner: &str, project_key: &str, notebook_path: &str) -> bool {
        let Some(kernel_arc) = self.get_existing(owner, project_key, notebook_path) else {
            return false;
        };
        // Read PID without waiting for an executing cell's lock? Store PID safely via try_lock
        // or inspect child PID — actually we keep a parallel PID map so interrupt works DURING exec!
        let _ = kernel_arc;
        self.send_sigint_by_key(&KernelKey {
            owner: owner.to_string(),
            project_key: project_key.to_string(),
            notebook_path: notebook_path.to_string(),
        })
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
        self.shutdown_session(owner, project_key, notebook_path)
            .await;
        let arc = self
            .get_or_spawn(owner, project_key, notebook_path, cwd, interpreter, env)
            .await?;
        let proc = arc.lock().await;
        Ok(proc.session_id.clone())
    }

    pub async fn shutdown_session(&self, owner: &str, project_key: &str, notebook_path: &str) {
        if let Some(arc) = self.remove_session(owner, project_key, notebook_path) {
            let mut proc = arc.lock().await;
            let _ = proc.child.start_kill();
        }
    }

    fn get_existing(
        &self,
        owner: &str,
        project_key: &str,
        notebook_path: &str,
    ) -> Option<Arc<TokioMutex<KernelProc>>> {
        let key = KernelKey {
            owner: owner.to_string(),
            project_key: project_key.to_string(),
            notebook_path: notebook_path.to_string(),
        };
        self.sessions.lock().ok()?.get(&key).cloned()
    }

    fn remove_session(
        &self,
        owner: &str,
        project_key: &str,
        notebook_path: &str,
    ) -> Option<Arc<TokioMutex<KernelProc>>> {
        let key = KernelKey {
            owner: owner.to_string(),
            project_key: project_key.to_string(),
            notebook_path: notebook_path.to_string(),
        };
        let removed = self.sessions.lock().ok()?.remove(&key);
        if let Ok(mut pids) = kernel_pids().lock() {
            pids.remove(&key);
        }
        removed
    }

    fn send_sigint_by_key(&self, key: &KernelKey) -> bool {
        let pid = kernel_pids().lock().ok().and_then(|m| m.get(key).copied());
        let Some(pid) = pid else {
            return false;
        };
        #[cfg(unix)]
        {
            send_unix_sigint(pid)
        }
        #[cfg(not(unix))]
        {
            let _ = pid;
            false
        }
    }

    async fn get_or_spawn(
        &self,
        owner: &str,
        project_key: &str,
        notebook_path: &str,
        cwd: &str,
        interpreter: &str,
        env: &BTreeMap<String, String>,
    ) -> Result<Arc<TokioMutex<KernelProc>>> {
        let key = KernelKey {
            owner: owner.to_string(),
            project_key: project_key.to_string(),
            notebook_path: notebook_path.to_string(),
        };

        if let Some(existing) = self.get_existing(owner, project_key, notebook_path) {
            let reuse = {
                let mut proc = existing.lock().await;
                let alive = proc.child.try_wait().ok().flatten().is_none();
                alive && proc.interpreter == interpreter && proc.cwd == cwd
            };
            if reuse {
                return Ok(existing);
            }
            self.shutdown_session(owner, project_key, notebook_path)
                .await;
        }

        let proc = spawn_python_kernel(interpreter, cwd, env).await?;
        let pid = proc.pid;
        let arc = Arc::new(TokioMutex::new(proc));

        let mut evicted: Option<Arc<TokioMutex<KernelProc>>> = None;
        {
            let mut guard = self
                .sessions
                .lock()
                .map_err(|_| Error::Internal("notebook session lock poisoned".into()))?;
            if guard.len() >= MAX_ACTIVE_KERNELS {
                if let Some(oldest_key) = guard.keys().next().cloned() {
                    evicted = guard.remove(&oldest_key);
                    if let Ok(mut pids) = kernel_pids().lock() {
                        pids.remove(&oldest_key);
                    }
                }
            }
            guard.insert(key.clone(), Arc::clone(&arc));
            if let Some(p) = pid {
                if let Ok(mut pids) = kernel_pids().lock() {
                    pids.insert(key, p);
                }
            }
        }
        if let Some(old) = evicted {
            let mut g = old.lock().await;
            let _ = g.child.start_kill();
        }
        Ok(arc)
    }
}

fn kernel_pids() -> &'static StdMutex<HashMap<KernelKey, u32>> {
    static PIDS: std::sync::OnceLock<StdMutex<HashMap<KernelKey, u32>>> =
        std::sync::OnceLock::new();
    PIDS.get_or_init(|| StdMutex::new(HashMap::new()))
}

#[cfg(unix)]
#[allow(unsafe_code)]
fn send_unix_sigint(pid: u32) -> bool {
    extern "C" {
        fn kill(pid: std::ffi::c_int, sig: std::ffi::c_int) -> std::ffi::c_int;
    }
    const SIGINT: std::ffi::c_int = 2;
    let Ok(p) = std::ffi::c_int::try_from(pid) else {
        return false;
    };
    if p <= 1 {
        return false;
    }
    // SAFETY: Sending SIGINT to a child kernel PID we spawned is OS-safe.
    unsafe { kill(p, SIGINT) == 0 }
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

        // 3. Rich HTML table & PNG display via _repr_html_ / _repr_png_
        let r3 = mgr
            .execute_cell(
                "u1",
                "local:/tmp/nb",
                "demo.ipynb",
                &cwd,
                &py_str,
                &env,
                "class FakeDF:\n  def _repr_html_(self): return '<table><tr><th>a</th></tr><tr><td>1</td></tr></table>'\nFakeDF()",
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

        mgr.shutdown_session("u1", "local:/tmp/nb", "demo.ipynb")
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
        let kernels = discover_notebook_kernels(tmp.path(), tmp.path(), None);
        assert!(
            !kernels.is_empty(),
            "expected at least system python3 kernel on host"
        );

        let ipynb = r##"{"cells":[{"cell_type":"markdown","source":["# hi"]},{"cell_type":"code","source":["a = 1\n","b = 2\n"]}],"metadata":{},"nbformat":4,"nbformat_minor":5}"##;
        let cell1 = extract_ipynb_cell_python(ipynb, Some(1)).unwrap();
        assert_eq!(cell1, "a = 1\nb = 2\n");

        // Verify ML training, TensorBoard, and Jupyter Lab auto-discovery
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
        assert!(
            discovered.iter().any(|c| c.name.contains("ML Train")
                && c.env_overrides.get("PYTHONUNBUFFERED").map(String::as_str) == Some("1")),
            "expected ML Train auto-config with PYTHONUNBUFFERED=1: {discovered:?}"
        );
        assert!(
            discovered.iter().any(|c| c.name.contains("TensorBoard")
                && c.args.iter().any(|a| a == "tensorboard.main")),
            "expected TensorBoard auto-config: {discovered:?}"
        );
        assert!(
            discovered.iter().any(|c| c.name == "Jupyter Lab"),
            "expected Jupyter Lab auto-config: {discovered:?}"
        );
    }
}
