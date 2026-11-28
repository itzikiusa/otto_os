//! Notebook kernels for the IDE: real Jupyter kernels spoken to over the
//! Jupyter messaging protocol (any kernelspec: ipykernel, R, Bash, …), with
//! Otto's built-in zero-install Python kernel kept as the frozen fallback for a
//! Python env without ipykernel. One kernel per open notebook, keyed by
//! (owner, project, notebook path), at most [`MAX_ACTIVE_KERNELS`] at once.
//! Also: Python env discovery, live variables and GPU / MPS accelerator detection.

mod builtin_kernel;
pub mod jupyter_kernel;
pub mod kernelspecs;
#[cfg(test)]
mod live_tests;

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::{Duration, Instant};

use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tokio::sync::{broadcast, Mutex as TokioMutex};

#[cfg(unix)]
use self::builtin_kernel::send_unix_signal_pid;
use self::builtin_kernel::{send_kernel_rpc, spawn_python_kernel, KernelProc};
use self::jupyter_kernel::{JupyterKernel, KernelLaunchCtx};
use self::kernelspecs::{JupyterSpec, KernelLaunch};
use crate::types::{IdeMlAcceleratorInfo, IdeTargetSpec};

pub use self::kernelspecs::{discover_kernels, record_kernel_trust, DiscoveredKernel};

const MAX_ACTIVE_KERNELS: usize = 12;
const DEFAULT_CELL_TIMEOUT_SECS: u64 = 120;
const MAX_OUTPUT_LINE_BYTES: usize = 8 * 1024 * 1024;
const STREAM_CAPACITY: usize = 512;

// ---------------------------------------------------------------------------
// Public API Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeNotebookKernelInfo {
    pub id: String,
    pub display_name: String,
    pub language: String,
    /// The kernel's interpreter / command (`argv[0]` for a Jupyter kernel).
    pub interpreter: String,
    /// `"jupyter"` | `"builtin"`.
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default)]
    pub active: bool,
    /// The kernelspec name written to `metadata.kernelspec.name` (Jupyter only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub spec_name: Option<String>,
    /// `"user"` | `"system"` | `"env"` | `"workspace"` (Jupyter only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub origin: Option<String>,
    /// The full launch command, shell-quoted (Jupyter only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub argv_preview: Option<String>,
    /// A workspace kernelspec the person hasn't accepted yet: ask before its first start.
    #[serde(default)]
    pub needs_trust: bool,
    /// The spec declares Jupyter debugger support (`metadata.debugger`); the
    /// running kernel's `kernel_info_reply` is authoritative once it starts.
    #[serde(default)]
    pub debugger: bool,
    /// Python envs: `"venv"` | `"uv"` | `"conda"` | `"poetry"` | `"system"` (picks pip or uv to install ipykernel).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub env_kind: Option<String>,
}

/// The notebook's running kernel, as the status chip shows it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdeNotebookKernelStatus {
    pub kernel_id: String,
    /// `"starting"` | `"idle"` | `"busy"` | `"dead"`.
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit_code: Option<i32>,
    #[serde(default)]
    pub debugger: bool,
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
    /// The notebook's running Jupyter kernel, if any.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel_status: Option<IdeNotebookKernelStatus>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeNotebookExecuteReq {
    pub target: IdeTargetSpec,
    pub notebook_path: String,
    pub code: String,
    /// A kernel `id` from `/ide/notebooks/kernels` (preferred over `interpreter`).
    #[serde(default)]
    pub kernel_id: Option<String>,
    #[serde(default)]
    pub interpreter: Option<String>,
    /// Without it a Jupyter cell waits for the kernel (no limit); the built-in
    /// kernel keeps its 120 s default.
    #[serde(default)]
    pub timeout_secs: Option<u64>,
    /// Tags this run's streamed events on `/ide/notebooks/stream`.
    #[serde(default)]
    pub run_id: Option<String>,
    /// The person accepted starting this workspace kernelspec.
    #[serde(default)]
    pub trust_kernel: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeNotebookExecuteResp {
    pub session_id: String,
    pub interpreter: String,
    pub execution_count: u32,
    /// `ok` | `error` | `aborted` | `dead` (the kernel stopped mid-cell).
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
    pub kernel_id: Option<String>,
    #[serde(default)]
    pub interpreter: Option<String>,
    #[serde(default)]
    pub trust_kernel: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IdeNotebookInputReq {
    pub target: IdeTargetSpec,
    pub notebook_path: String,
    pub value: String,
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
// GPU / MPS / CUDA Accelerator Detection
// ---------------------------------------------------------------------------

/// Fast (<5ms on macOS) local ML hardware accelerator probe (Apple Silicon MPS,
/// NVIDIA CUDA via `nvidia-smi`, Linux ROCm `/dev/kfd`).
#[allow(clippy::disallowed_methods)] // called inside spawn_blocking
fn probe_local_accelerators() -> Vec<IdeMlAcceleratorInfo> {
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

/// Accelerators probed once per daemon: the kernel list is refreshed on every
/// notebook open, and the probe runs `sysctl` / `nvidia-smi`.
pub fn detect_local_accelerators() -> Vec<IdeMlAcceleratorInfo> {
    static CACHE: std::sync::OnceLock<Vec<IdeMlAcceleratorInfo>> = std::sync::OnceLock::new();
    CACHE.get_or_init(probe_local_accelerators).clone()
}

// ---------------------------------------------------------------------------
// Persistent Notebook Kernel Manager
// ---------------------------------------------------------------------------

#[derive(Clone, Hash, PartialEq, Eq)]
struct KernelKey {
    owner: String,
    project_key: String,
    notebook_path: String,
}

impl KernelKey {
    fn new(owner: &str, project_key: &str, notebook_path: &str) -> Self {
        Self {
            owner: owner.to_string(),
            project_key: project_key.to_string(),
            notebook_path: notebook_path.to_string(),
        }
    }
}

/// Per-notebook kernel slot: at most one of the built-in process (`proc`, its
/// PID in `pid`) or a Jupyter kernel (`jupyter`) is live at a time.
/// Storing the PID per slot (`AtomicU32`, `0` while spawning/stopped) makes cross-slot
/// PID interference structurally impossible across both delayed teardown (`CONC2-1`) and
/// pre-ready replacement startup (`CONC3-1`).
struct KernelSlotInner {
    proc: TokioMutex<Option<KernelProc>>,
    pid: AtomicU32,
    jupyter: TokioMutex<Option<Arc<JupyterKernel>>>,
    /// When the notebook last asked for its kernel (the cap evicts the oldest idle one).
    last_used: StdMutex<Instant>,
}

impl KernelSlotInner {
    fn new() -> Arc<Self> {
        Arc::new(Self {
            proc: TokioMutex::new(None),
            pid: AtomicU32::new(0),
            jupyter: TokioMutex::new(None),
            last_used: StdMutex::new(Instant::now()),
        })
    }

    fn last_used(&self) -> Instant {
        *self.last_used.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// No cell runs and no kernel starts: a locked kernel is in use.
    fn is_idle(&self) -> bool {
        let (Ok(_proc), Ok(jupyter)) = (self.proc.try_lock(), self.jupyter.try_lock()) else {
            return false;
        };
        jupyter
            .as_ref()
            .is_none_or(|k| matches!(k.status().state.as_str(), "idle" | "dead"))
    }

    async fn stop(&self) {
        if let Some(k) = self.jupyter.lock().await.take() {
            k.shutdown(false, grace()).await;
        }
        clear_slot_kernel(self, &mut *self.proc.lock().await);
    }
}

type KernelSlot = Arc<KernelSlotInner>;

/// Where and how one notebook's kernel runs.
pub struct NotebookKernelCtx<'a> {
    pub owner: &'a str,
    pub project_key: &'a str,
    pub notebook_path: &'a str,
    pub cwd: &'a str,
    pub env: &'a BTreeMap<String, String>,
    /// Holds Jupyter connection files (`<data_dir>/jupyter-runtime`).
    pub runtime_dir: &'a Path,
}

impl NotebookKernelCtx<'_> {
    fn key(&self) -> KernelKey {
        KernelKey::new(self.owner, self.project_key, self.notebook_path)
    }
}

/// Manages persistent per-notebook kernels for the IDE.
#[derive(Clone, Default)]
pub struct NotebookManager {
    sessions: Arc<StdMutex<HashMap<KernelKey, KernelSlot>>>,
    /// Per-notebook event fan-out for `/ide/notebooks/stream` (outlives kernel restarts).
    streams: Arc<StdMutex<HashMap<KernelKey, broadcast::Sender<Value>>>>,
}

fn grace() -> Duration {
    Duration::from_secs(3)
}

impl NotebookManager {
    pub fn new() -> Self {
        Self::default()
    }

    fn events_for(&self, key: &KernelKey) -> broadcast::Sender<Value> {
        let mut streams = self.streams.lock().unwrap_or_else(|e| e.into_inner());
        streams
            .entry(key.clone())
            .or_insert_with(|| broadcast::channel(STREAM_CAPACITY).0)
            .clone()
    }

    /// Subscribe to a notebook's events; the first value is its kernel status.
    pub fn subscribe(
        &self,
        owner: &str,
        project_key: &str,
        notebook_path: &str,
    ) -> (Value, broadcast::Receiver<Value>) {
        let key = KernelKey::new(owner, project_key, notebook_path);
        let rx = self.events_for(&key).subscribe();
        let initial = match self.kernel_status(owner, project_key, notebook_path) {
            Some(s) => {
                let mut v = serde_json::to_value(s).unwrap_or_else(|_| json!({}));
                v["kind"] = json!("status");
                v
            }
            None => json!({ "kind": "status", "state": "none" }),
        };
        (initial, rx)
    }

    fn current_jupyter(
        &self,
        owner: &str,
        project_key: &str,
        notebook_path: &str,
    ) -> Option<Arc<JupyterKernel>> {
        let slot = self.get_existing_slot(owner, project_key, notebook_path)?;
        let guard = slot.jupyter.try_lock().ok()?;
        guard.clone()
    }

    /// The notebook's running Jupyter kernel, for the status chip.
    pub fn kernel_status(
        &self,
        owner: &str,
        project_key: &str,
        notebook_path: &str,
    ) -> Option<IdeNotebookKernelStatus> {
        let k = self.current_jupyter(owner, project_key, notebook_path)?;
        let s = k.status();
        Some(IdeNotebookKernelStatus {
            kernel_id: k.kernel_id.clone(),
            state: s.state,
            exit_code: s.exit_code,
            debugger: k.debugger(),
        })
    }

    pub fn active_session_summary(
        &self,
        owner: &str,
        project_key: &str,
        notebook_path: &str,
    ) -> Option<(String, String, u32)> {
        if let Some(k) = self.current_jupyter(owner, project_key, notebook_path) {
            return Some((k.session_id.clone(), k.display_name.clone(), 0));
        }
        let slot = self.get_existing_slot(owner, project_key, notebook_path)?;
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

    /// Run one cell on the notebook's kernel, starting (or switching to) `launch`.
    pub async fn execute(
        &self,
        ctx: &NotebookKernelCtx<'_>,
        kernel_id: &str,
        launch: &KernelLaunch,
        code: &str,
        run_id: &str,
        timeout: Option<Duration>,
    ) -> Result<IdeNotebookExecuteResp> {
        match launch {
            KernelLaunch::Builtin { interpreter } => {
                if let Some(slot) =
                    self.get_existing_slot(ctx.owner, ctx.project_key, ctx.notebook_path)
                {
                    if let Some(old) = slot.jupyter.lock().await.take() {
                        old.shutdown(false, grace()).await;
                    }
                }
                let timeout = timeout.unwrap_or_else(|| default_cell_timeout(None));
                self.execute_cell(
                    ctx.owner,
                    ctx.project_key,
                    ctx.notebook_path,
                    ctx.cwd,
                    interpreter,
                    ctx.env,
                    code,
                    timeout,
                )
                .await
            }
            KernelLaunch::Jupyter(spec) => {
                let started = Instant::now();
                let kernel = self.ensure_jupyter(ctx, kernel_id, spec, false).await?;
                let out = kernel.execute(code, run_id, timeout).await?;
                let variables = kernel.inspect_variables().await;
                Ok(IdeNotebookExecuteResp {
                    session_id: kernel.session_id.clone(),
                    interpreter: spec.argv.first().cloned().unwrap_or_default(),
                    execution_count: out.execution_count,
                    status: out.status,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                    outputs: out.outputs,
                    variables,
                })
            }
        }
    }

    /// The slot's Jupyter kernel for `kernel_id`, started when absent, switched
    /// when another kernel runs, restarted when `fresh`. A dead kernel stays dead
    /// (its next execute says so) until Restart.
    async fn ensure_jupyter(
        &self,
        ctx: &NotebookKernelCtx<'_>,
        kernel_id: &str,
        spec: &JupyterSpec,
        fresh: bool,
    ) -> Result<Arc<JupyterKernel>> {
        let slot = self.get_or_create_slot(&ctx.key()).await?;
        let mut guard = slot.jupyter.lock().await;
        if let Some(k) = guard.as_ref() {
            if k.kernel_id == kernel_id && !fresh {
                return Ok(k.clone());
            }
        }
        if let Some(old) = guard.take() {
            old.shutdown(
                fresh,
                if fresh {
                    Duration::from_secs(5)
                } else {
                    grace()
                },
            )
            .await;
        }
        {
            let mut proc = slot.proc.lock().await;
            clear_slot_kernel(&slot, &mut proc);
        }
        let kernel = JupyterKernel::start(
            spec,
            KernelLaunchCtx {
                kernel_id,
                cwd: ctx.cwd,
                env: ctx.env,
                runtime_dir: ctx.runtime_dir,
                events: self.events_for(&ctx.key()),
            },
        )
        .await?;
        *guard = Some(kernel.clone());
        Ok(kernel)
    }

    /// Restart the notebook's kernel on `launch`; returns the new session id.
    pub async fn restart_kernel(
        &self,
        ctx: &NotebookKernelCtx<'_>,
        kernel_id: &str,
        launch: &KernelLaunch,
    ) -> Result<String> {
        match launch {
            KernelLaunch::Builtin { interpreter } => {
                if let Some(slot) =
                    self.get_existing_slot(ctx.owner, ctx.project_key, ctx.notebook_path)
                {
                    if let Some(old) = slot.jupyter.lock().await.take() {
                        old.shutdown(false, grace()).await;
                    }
                }
                self.restart(
                    ctx.owner,
                    ctx.project_key,
                    ctx.notebook_path,
                    ctx.cwd,
                    interpreter,
                    ctx.env,
                )
                .await
            }
            KernelLaunch::Jupyter(spec) => Ok(self
                .ensure_jupyter(ctx, kernel_id, spec, true)
                .await?
                .session_id
                .clone()),
        }
    }

    /// Interrupt the running cell; the message says what happened.
    pub async fn interrupt_kernel(
        &self,
        owner: &str,
        project_key: &str,
        notebook_path: &str,
    ) -> (bool, String) {
        if let Some(slot) = self.get_existing_slot(owner, project_key, notebook_path) {
            let jupyter = slot.jupyter.lock().await.clone();
            if let Some(k) = jupyter {
                return k.interrupt().await;
            }
        }
        if self.interrupt(owner, project_key, notebook_path) {
            (true, "Interrupt signal sent to kernel".into())
        } else {
            (false, "No active kernel for this notebook".into())
        }
    }

    /// Answer the cell's pending `input()`; `false` when nothing waits for input.
    pub async fn send_input(
        &self,
        owner: &str,
        project_key: &str,
        notebook_path: &str,
        value: &str,
    ) -> Result<bool> {
        let Some(slot) = self.get_existing_slot(owner, project_key, notebook_path) else {
            return Ok(false);
        };
        let kernel = slot.jupyter.lock().await.clone();
        match kernel {
            Some(k) => k.send_input(value).await,
            None => Ok(false),
        }
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
        let key = KernelKey::new(owner, project_key, notebook_path);
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
        let empty = IdeNotebookVariablesResp {
            session_id: None,
            execution_count: 0,
            variables: Vec::new(),
        };
        let Some(slot) = self.get_existing_slot(owner, project_key, notebook_path) else {
            return Ok(empty);
        };
        let jupyter = slot.jupyter.lock().await.clone();
        if let Some(k) = jupyter {
            return Ok(IdeNotebookVariablesResp {
                session_id: Some(k.session_id.clone()),
                execution_count: 0,
                variables: k.inspect_variables().await,
            });
        }
        let mut slot_guard = slot.proc.lock().await;
        let Some(proc) = slot_guard.as_mut() else {
            return Ok(empty);
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
        let key = KernelKey::new(owner, project_key, notebook_path);
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
        let key = KernelKey::new(owner, project_key, notebook_path);
        if let Some(slot) = self.remove_slot(&key) {
            slot.stop().await;
        }
        self.streams
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .remove(&key);
    }

    fn get_existing_slot(
        &self,
        owner: &str,
        project_key: &str,
        notebook_path: &str,
    ) -> Option<KernelSlot> {
        let key = KernelKey::new(owner, project_key, notebook_path);
        self.sessions.lock().ok()?.get(&key).cloned()
    }

    fn remove_slot(&self, key: &KernelKey) -> Option<KernelSlot> {
        self.sessions.lock().ok()?.remove(key)
    }

    /// Atomically returns the per-notebook slot so concurrent first runs
    /// serialize kernel creation and share a single kernel process. At the cap
    /// the least recently used idle kernel (or a slot whose kernel never
    /// started) is stopped to make room; a running cell is never stopped, so
    /// with 12 busy kernels the 13th notebook is refused.
    async fn get_or_create_slot(&self, key: &KernelKey) -> Result<KernelSlot> {
        let mut guard = self
            .sessions
            .lock()
            .map_err(|_| Error::Internal("notebook session lock poisoned".into()))?;
        if let Some(existing) = guard.get(key) {
            *existing.last_used.lock().unwrap_or_else(|e| e.into_inner()) = Instant::now();
            return Ok(Arc::clone(existing));
        }
        if guard.len() >= MAX_ACTIVE_KERNELS {
            let oldest_idle = guard
                .iter()
                .filter(|(_, slot)| slot.is_idle())
                .min_by_key(|(_, slot)| slot.last_used())
                .map(|(k, _)| k.clone());
            let Some(evicted) = oldest_idle.and_then(|k| guard.remove(&k)) else {
                return Err(Error::Conflict(format!(
                    "{MAX_ACTIVE_KERNELS} kernels are running. Close a notebook to start another."
                )));
            };
            tokio::spawn(async move { evicted.stop().await });
        }
        let fresh = KernelSlotInner::new();
        guard.insert(key.clone(), Arc::clone(&fresh));
        Ok(fresh)
    }
}

fn clear_slot_kernel(slot: &KernelSlotInner, slot_guard: &mut Option<KernelProc>) {
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
    #[cfg(unix)]
    use super::builtin_kernel::send_unix_signal_group;
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

    async fn twelve_slots(mgr: &NotebookManager) -> Vec<KernelSlot> {
        let mut slots = Vec::new();
        for i in 0..MAX_ACTIVE_KERNELS {
            let key = KernelKey::new("u", "p", &format!("{i}.ipynb"));
            slots.push(mgr.get_or_create_slot(&key).await.unwrap());
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
        slots
    }

    fn has_slot(mgr: &NotebookManager, path: &str) -> bool {
        mgr.get_existing_slot("u", "p", path).is_some()
    }

    #[tokio::test]
    async fn at_the_cap_the_least_recently_used_idle_kernel_makes_room() {
        let mgr = NotebookManager::new();
        let _slots = twelve_slots(&mgr).await;
        // 0.ipynb is used again, so 1.ipynb is now the least recently used.
        mgr.get_or_create_slot(&KernelKey::new("u", "p", "0.ipynb"))
            .await
            .unwrap();
        mgr.get_or_create_slot(&KernelKey::new("u", "p", "13.ipynb"))
            .await
            .expect("an idle kernel makes room for the 13th notebook");
        assert!(has_slot(&mgr, "13.ipynb") && has_slot(&mgr, "0.ipynb"));
        assert!(
            !has_slot(&mgr, "1.ipynb"),
            "the least recently used idle slot goes"
        );
    }

    #[tokio::test]
    async fn the_13th_notebook_is_refused_while_twelve_kernels_are_busy() {
        let mgr = NotebookManager::new();
        let slots = twelve_slots(&mgr).await;
        // A running cell holds its slot's kernel lock.
        let _busy: Vec<_> = slots.iter().map(|s| s.proc.try_lock().unwrap()).collect();
        let err = match mgr
            .get_or_create_slot(&KernelKey::new("u", "p", "13.ipynb"))
            .await
        {
            Err(e) => e,
            Ok(_) => panic!("a busy kernel is never stopped to make room"),
        };
        assert!(err
            .to_string()
            .contains("12 kernels are running. Close a notebook to start another."));
        // A notebook that already has its kernel keeps it.
        assert!(mgr
            .get_or_create_slot(&KernelKey::new("u", "p", "0.ipynb"))
            .await
            .is_ok());
    }

    #[test]
    fn accelerator_detection_and_ipynb_extraction_work() {
        let accels = detect_local_accelerators();
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        assert!(accels.iter().any(|a| a.kind == "mps" && a.available));
        #[cfg(not(all(target_os = "macos", target_arch = "aarch64")))]
        let _ = accels;

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
