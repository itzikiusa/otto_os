//! One running Jupyter kernel spoken to over the real messaging protocol:
//! launch from a kernelspec, the five HMAC-signed ZeroMQ channels, execute with
//! streamed outputs, stdin, interrupt (signal or message), restart, shutdown.

use std::collections::{BTreeMap, HashMap};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::{Arc, Mutex as StdMutex};
use std::time::Duration;

use jupyter_protocol::{
    ConnectionInfo, DebugRequest, ExecuteRequest, Header, InputReply, InterruptRequest,
    JupyterMessage, KernelInfoRequest, ReplyStatus, ShutdownRequest, Transport,
};
use jupyter_zmq_client as jzc;
use otto_core::{Error, Result};
use serde_json::{json, Value};
use tokio::sync::{broadcast, mpsc, watch, Mutex as TokioMutex};
use tokio::task::JoinHandle;
use tokio::time::Instant;
use zeromq::SocketRecv;

use super::builtin_kernel::send_unix_signal_group;
use super::kernelspecs::{InterruptMode, JupyterSpec};

const START_TIMEOUT: Duration = Duration::from_secs(30);
const HEARTBEAT_EVERY: Duration = Duration::from_secs(5);
const HEARTBEAT_MISSES: u32 = 3;
const INTERRUPT_WAIT: Duration = Duration::from_secs(5);
const STDERR_RING_BYTES: usize = 64 * 1024;
/// One output (`MAX_OUTPUT_LINE_BYTES`) and one cell's outputs over these are replaced.
const MAX_OUTPUT_BYTES: usize = 8 * 1024 * 1024;
const MAX_CELL_BYTES: usize = 32 * 1024 * 1024;

fn lock<T>(m: &StdMutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Ch {
    Shell,
    IoPub,
    Stdin,
    Control,
}

/// One verified incoming message, content kept as raw JSON.
#[derive(Debug, Clone)]
struct Incoming {
    channel: Ch,
    header: Header,
    parent_id: Option<String>,
    content: Value,
}

fn parse_raw(raw: jzc::RawMessage, channel: Ch) -> Option<Incoming> {
    let header: Header = serde_json::from_slice(raw.jparts.first()?).ok()?;
    let parent: Option<Header> = serde_json::from_slice(raw.jparts.get(1)?).ok();
    let content: Value = serde_json::from_slice(raw.jparts.get(3)?).ok()?;
    Some(Incoming {
        channel,
        header,
        parent_id: parent.map(|p| p.msg_id),
        content,
    })
}

/// Kernel lifecycle as the status chip shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KernelStatus {
    /// `starting` | `idle` | `busy` | `dead`.
    pub state: String,
    pub exit_code: Option<i32>,
}

/// State shared by the channel reader tasks and the kernel handle.
struct Core {
    kernel_id: String,
    waiters: StdMutex<HashMap<String, mpsc::UnboundedSender<Incoming>>>,
    status: watch::Sender<KernelStatus>,
    events: broadcast::Sender<Value>,
    debugger: AtomicBool,
    stderr: StdMutex<String>,
    stopping: AtomicBool,
}

impl Core {
    fn register(&self, msg_id: &str) -> mpsc::UnboundedReceiver<Incoming> {
        let (tx, rx) = mpsc::unbounded_channel();
        lock(&self.waiters).insert(msg_id.to_string(), tx);
        rx
    }

    fn unregister(&self, msg_id: &str) {
        lock(&self.waiters).remove(msg_id);
    }

    fn dispatch(&self, msg: Incoming) {
        if msg.channel == Ch::IoPub && msg.header.msg_type == "status" {
            if let Some(state) = msg.content.get("execution_state").and_then(Value::as_str) {
                self.set_state(state, None);
            }
        }
        let Some(parent) = msg.parent_id.clone() else {
            return;
        };
        if let Some(tx) = lock(&self.waiters).get(&parent) {
            let _ = tx.send(msg);
        }
    }

    fn set_state(&self, state: &str, exit_code: Option<i32>) {
        let changed = self.status.send_if_modified(|s| {
            if s.state == "dead" || (s.state == state && s.exit_code == exit_code) {
                return false;
            }
            s.state = state.to_string();
            s.exit_code = exit_code;
            true
        });
        if changed {
            let _ = self.events.send(self.status_event());
        }
    }

    fn status_event(&self) -> Value {
        let s = self.status.borrow().clone();
        json!({
            "kind": "status",
            "state": s.state,
            "exit_code": s.exit_code,
            "kernel_id": self.kernel_id,
            "debugger": self.debugger.load(Ordering::Acquire),
        })
    }

    fn push_stderr(&self, line: &str) {
        let mut buf = lock(&self.stderr);
        buf.push_str(line);
        buf.push('\n');
        if buf.len() > STDERR_RING_BYTES {
            let cut = buf.len() - STDERR_RING_BYTES;
            let cut = (cut..buf.len())
                .find(|i| buf.is_char_boundary(*i))
                .unwrap_or(0);
            buf.drain(..cut);
        }
    }

    fn stderr_tail(&self, lines: usize) -> String {
        let buf = lock(&self.stderr);
        let all: Vec<&str> = buf.lines().filter(|l| !l.trim().is_empty()).collect();
        all[all.len().saturating_sub(lines)..].join("\n")
    }
}

fn spawn_reader<S: SocketRecv + Send + 'static>(
    mut conn: jzc::Connection<S>,
    channel: Ch,
    core: Arc<Core>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        while let Ok(zmsg) = conn.socket.recv().await {
            // A message that fails the HMAC check (or is malformed) is dropped.
            let Ok(raw) = jzc::RawMessage::from_multipart(zmsg, &conn.mac) else {
                continue;
            };
            if let Some(msg) = parse_raw(raw, channel) {
                core.dispatch(msg);
            }
        }
    })
}

/// Expand `${VAR}` in a kernelspec `env` value from the kernel's own env only;
/// unknown names stay as written (Python's `Template.safe_substitute`).
pub(crate) fn expand_spec_env(value: &str, env: &BTreeMap<String, String>) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end)
                if !after[..end].is_empty()
                    && after[..end]
                        .chars()
                        .all(|c| c.is_ascii_alphanumeric() || c == '_') =>
            {
                match env.get(&after[..end]) {
                    Some(v) => out.push_str(v),
                    None => out.push_str(&rest[start..start + 3 + end]),
                }
                rest = &after[end + 1..];
            }
            _ => {
                out.push_str("${");
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Cell outputs in nbformat shape, kept in step with the SSE stream.
struct Outputs {
    run_id: String,
    events: broadcast::Sender<Value>,
    items: Vec<Value>,
    display_ids: Vec<Option<String>>,
    clear_pending: bool,
    bytes: usize,
}

fn too_large(bytes: usize) -> Value {
    json!({
        "output_type": "stream",
        "name": "stderr",
        "text": format!("Output too large to show ({:.1} MB)\n", bytes as f64 / (1024.0 * 1024.0)),
    })
}

impl Outputs {
    fn emit_set(&self, index: usize) {
        let _ = self.events.send(json!({
            "kind": "output", "run_id": self.run_id, "index": index, "output": self.items[index],
        }));
    }

    fn clear_now(&mut self) {
        self.items.clear();
        self.display_ids.clear();
        self.bytes = 0;
        let _ = self
            .events
            .send(json!({ "kind": "clear_output", "run_id": self.run_id }));
    }

    fn flush_pending_clear(&mut self) {
        if std::mem::take(&mut self.clear_pending) {
            self.clear_now();
        }
    }

    /// Size-capped copy of `out`: one output over 8 MiB, or a cell past 32 MiB.
    fn capped(&mut self, out: Value) -> Value {
        let size = serde_json::to_string(&out).map(|s| s.len()).unwrap_or(0);
        if size > MAX_OUTPUT_BYTES || self.bytes + size > MAX_CELL_BYTES {
            return too_large(size);
        }
        self.bytes += size;
        out
    }

    fn push(&mut self, out: Value, display_id: Option<String>) {
        self.flush_pending_clear();
        let out = self.capped(out);
        self.items.push(out);
        self.display_ids.push(display_id);
        self.emit_set(self.items.len() - 1);
    }

    fn stream(&mut self, content: &Value) {
        self.flush_pending_clear();
        let name = content
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("stdout");
        let text = content.get("text").and_then(Value::as_str).unwrap_or("");
        // Consecutive chunks of one stream merge into one output, as Jupyter does.
        if let Some(last) = self.items.last_mut() {
            if last["output_type"] == "stream" && last["name"] == name {
                let merged = format!("{}{text}", last["text"].as_str().unwrap_or(""));
                if merged.len() <= MAX_OUTPUT_BYTES && self.bytes + text.len() <= MAX_CELL_BYTES {
                    self.bytes += text.len();
                    last["text"] = Value::String(merged);
                    self.emit_set(self.items.len() - 1);
                    return;
                }
            }
        }
        self.push(
            json!({ "output_type": "stream", "name": name, "text": text }),
            None,
        );
    }

    fn display(&mut self, content: &Value, execution_count: Option<u64>) {
        let mut out = json!({
            "output_type": if execution_count.is_some() { "execute_result" } else { "display_data" },
            "data": content.get("data").cloned().unwrap_or_else(|| json!({})),
            "metadata": content.get("metadata").cloned().unwrap_or_else(|| json!({})),
        });
        if let Some(n) = execution_count {
            out["execution_count"] = json!(n);
        }
        let display_id = content
            .pointer("/transient/display_id")
            .and_then(Value::as_str)
            .map(str::to_string);
        self.push(out, display_id);
    }

    fn update_display(&mut self, content: &Value) {
        let Some(id) = content
            .pointer("/transient/display_id")
            .and_then(Value::as_str)
        else {
            return;
        };
        for i in 0..self.items.len() {
            if self.display_ids[i].as_deref() == Some(id) {
                let mut out = json!({
                    "output_type": self.items[i]["output_type"].clone(),
                    "data": content.get("data").cloned().unwrap_or_else(|| json!({})),
                    "metadata": content.get("metadata").cloned().unwrap_or_else(|| json!({})),
                });
                if let Some(n) = self.items[i].get("execution_count") {
                    out["execution_count"] = n.clone();
                }
                self.items[i] = self.capped(out);
                self.emit_set(i);
            }
        }
    }

    fn error(&mut self, content: &Value) {
        self.push(
            json!({
                "output_type": "error",
                "ename": content.get("ename").cloned().unwrap_or_else(|| json!("Error")),
                "evalue": content.get("evalue").cloned().unwrap_or_else(|| json!("")),
                "traceback": content.get("traceback").cloned().unwrap_or_else(|| json!([])),
            }),
            None,
        );
    }
}

/// The outcome of one `execute_request`.
#[derive(Debug, Clone)]
pub struct ExecOutcome {
    /// `ok` | `error` | `aborted` | `dead`.
    pub status: String,
    pub execution_count: u32,
    pub outputs: Vec<Value>,
}

pub(crate) fn dead_message(exit_code: Option<i32>) -> String {
    match exit_code {
        Some(code) => format!("The kernel stopped (exit code {code}). Restart to continue."),
        None => "The kernel stopped. Restart to continue.".into(),
    }
}

/// What a kernel launch needs besides its spec.
pub struct KernelLaunchCtx<'a> {
    pub kernel_id: &'a str,
    pub cwd: &'a str,
    pub env: &'a BTreeMap<String, String>,
    pub runtime_dir: &'a Path,
    pub events: broadcast::Sender<Value>,
}

pub struct JupyterKernel {
    pub kernel_id: String,
    pub session_id: String,
    pub display_name: String,
    interrupt_mode: InterruptMode,
    pid: u32,
    exited: Arc<AtomicBool>,
    conn_file: PathBuf,
    shell: TokioMutex<jzc::DealerSendConnection>,
    control: TokioMutex<jzc::DealerSendConnection>,
    stdin: TokioMutex<jzc::DealerSendConnection>,
    core: Arc<Core>,
    exec_lock: TokioMutex<()>,
    pending_input: StdMutex<Option<Header>>,
    debug_seq: AtomicI64,
    tasks: StdMutex<Vec<JoinHandle<()>>>,
}

enum StartError {
    PortInUse,
    Other(Error),
}

impl From<Error> for StartError {
    fn from(e: Error) -> Self {
        StartError::Other(e)
    }
}

fn upstream<E: std::fmt::Display>(what: &str) -> impl FnOnce(E) -> Error + '_ {
    move |e| Error::Upstream(format!("{what}: {e}"))
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    std::fs::write(path, bytes)
}

impl JupyterKernel {
    /// Launch `spec` and wait until it answers `kernel_info_request` with IOPub
    /// subscribed. An address-in-use exit during start retries once on new ports.
    pub async fn start(spec: &JupyterSpec, ctx: KernelLaunchCtx<'_>) -> Result<Arc<Self>> {
        match Self::start_once(spec, &ctx).await {
            Ok(k) => Ok(k),
            Err(StartError::PortInUse) => match Self::start_once(spec, &ctx).await {
                Ok(k) => Ok(k),
                Err(StartError::PortInUse) => Err(Error::Upstream(format!(
                    "Kernel '{}' couldn't start: its ports were taken twice.",
                    spec.display_name
                ))),
                Err(StartError::Other(e)) => Err(e),
            },
            Err(StartError::Other(e)) => Err(e),
        }
    }

    async fn start_once(
        spec: &JupyterSpec,
        ctx: &KernelLaunchCtx<'_>,
    ) -> std::result::Result<Arc<Self>, StartError> {
        let name = &spec.display_name;
        let ip: std::net::IpAddr = std::net::Ipv4Addr::LOCALHOST.into();
        let (ports, listeners) = jzc::peek_ports_with_listeners(ip, 5)
            .await
            .map_err(upstream("reserve kernel ports"))?;
        let key: [u8; 32] = rand::random();
        let info = ConnectionInfo {
            ip: "127.0.0.1".into(),
            transport: Transport::TCP,
            shell_port: ports[0],
            iopub_port: ports[1],
            stdin_port: ports[2],
            control_port: ports[3],
            hb_port: ports[4],
            key: hex::encode(key),
            signature_scheme: "hmac-sha256".into(),
            kernel_name: Some(spec.spec_name.clone()),
        };
        let conn_file = ctx
            .runtime_dir
            .join(format!("kernel-{}.json", uuid::Uuid::new_v4()));
        let body = serde_json::to_vec(&info).map_err(|e| Error::Internal(e.to_string()))?;
        write_private(&conn_file, &body).map_err(upstream("write kernel connection file"))?;
        let remove_file = || {
            let _ = std::fs::remove_file(&conn_file);
        };

        let argv: Vec<String> = spec
            .argv
            .iter()
            .map(|a| a.replace("{connection_file}", &conn_file.to_string_lossy()))
            .collect();
        let mut cmd = tokio::process::Command::new(&argv[0]);
        cmd.args(&argv[1..])
            .current_dir(ctx.cwd)
            .env_clear()
            .envs(ctx.env)
            .envs(
                spec.env
                    .iter()
                    .map(|(k, v)| (k, expand_spec_env(v, ctx.env))),
            )
            // ipykernel's parent poller exits the kernel if the daemon dies.
            .env("JPY_PARENT_PID", std::process::id().to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        cmd.process_group(0);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                remove_file();
                return Err(if e.kind() == std::io::ErrorKind::NotFound {
                    Error::Invalid(format!(
                        "Kernel '{name}' couldn't start: {} wasn't found.",
                        argv[0]
                    ))
                } else {
                    Error::Upstream(format!("Kernel '{name}' couldn't start: {e}"))
                }
                .into());
            }
        };
        // The kernel binds these ports first thing; hold them only until spawn.
        drop(listeners);
        let pid = child.id().unwrap_or(0);

        let (status_tx, _) = watch::channel(KernelStatus {
            state: "starting".into(),
            exit_code: None,
        });
        let core = Arc::new(Core {
            kernel_id: ctx.kernel_id.to_string(),
            waiters: StdMutex::new(HashMap::new()),
            status: status_tx,
            events: ctx.events.clone(),
            debugger: AtomicBool::new(false),
            stderr: StdMutex::new(String::new()),
            stopping: AtomicBool::new(false),
        });
        let _ = core.events.send(core.status_event());
        let mut tasks = Vec::new();
        for pipe in [
            child
                .stdout
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
            child
                .stderr
                .take()
                .map(|s| Box::new(s) as Box<dyn tokio::io::AsyncRead + Unpin + Send>),
        ]
        .into_iter()
        .flatten()
        {
            let c = core.clone();
            tasks.push(crate::line_reader::spawn_line_drain(
                pipe,
                crate::line_reader::MAX_OUTPUT_LINE_BYTES,
                move |l| c.push_stderr(&l),
            ));
        }
        let exited = Arc::new(AtomicBool::new(false));
        {
            let (core, exited) = (core.clone(), exited.clone());
            tasks.push(tokio::spawn(async move {
                let code = child.wait().await.ok().and_then(|s| {
                    #[cfg(unix)]
                    {
                        use std::os::unix::process::ExitStatusExt;
                        s.code().or_else(|| s.signal().map(|sig| -sig))
                    }
                    #[cfg(not(unix))]
                    {
                        s.code()
                    }
                });
                exited.store(true, Ordering::Release);
                if !core.stopping.load(Ordering::Acquire) {
                    core.set_state("dead", code);
                }
            }));
        }

        let abort = |tasks: Vec<JoinHandle<()>>| {
            for t in tasks {
                t.abort();
            }
            #[cfg(unix)]
            if pid > 1 {
                send_unix_signal_group(pid, 9);
            }
        };
        let session_id = uuid::Uuid::new_v4().to_string();
        let mut dead_rx = core.status.subscribe();
        let connect = async {
            let identity =
                jzc::peer_identity_for_session(&session_id).map_err(upstream("identity"))?;
            let shell = jzc::create_client_shell_connection_with_identity(
                &info,
                &session_id,
                identity.clone(),
            )
            .await
            .map_err(upstream("connect shell"))?;
            let stdin =
                jzc::create_client_stdin_connection_with_identity(&info, &session_id, identity)
                    .await
                    .map_err(upstream("connect stdin"))?;
            let control = jzc::create_client_control_connection(&info, &session_id)
                .await
                .map_err(upstream("connect control"))?;
            let iopub = jzc::create_client_iopub_connection(&info, "", &session_id)
                .await
                .map_err(upstream("connect iopub"))?;
            let hb = jzc::create_client_heartbeat_connection(&info)
                .await
                .map_err(upstream("connect heartbeat"))?;
            Ok::<_, Error>((shell, stdin, control, iopub, hb))
        };
        let deadline = Instant::now() + START_TIMEOUT;
        let started_late = || {
            let tail = core.stderr_tail(12);
            Error::Upstream(format!(
                "Kernel '{name}' started but didn't answer within 30 s.{}",
                if tail.is_empty() {
                    String::new()
                } else {
                    format!("\n{tail}")
                }
            ))
        };
        let died = |core: &Core| -> StartError {
            let tail = core.stderr_tail(12);
            if tail.contains("Address already in use") {
                return StartError::PortInUse;
            }
            let code = core.status.borrow().exit_code;
            StartError::Other(Error::Upstream(format!(
                "Kernel '{name}' stopped while starting{}.{}",
                code.map(|c| format!(" (exit code {c})"))
                    .unwrap_or_default(),
                if tail.is_empty() {
                    String::new()
                } else {
                    format!("\n{tail}")
                }
            )))
        };
        let wait_dead = |rx: &mut watch::Receiver<KernelStatus>| {
            let mut rx = rx.clone();
            async move {
                let _ = rx.wait_for(|s| s.state == "dead").await;
            }
        };
        let conns = tokio::select! {
            r = tokio::time::timeout_at(deadline, connect) => match r {
                Ok(Ok(c)) => c,
                Ok(Err(e)) => { abort(tasks); remove_file(); return Err(e.into()); }
                Err(_) => { abort(tasks); remove_file(); return Err(started_late().into()); }
            },
            _ = wait_dead(&mut dead_rx) => {
                // Let the stderr drain catch the last lines before reading them.
                tokio::time::sleep(Duration::from_millis(100)).await;
                let err = died(&core);
                abort(tasks);
                remove_file();
                return Err(err);
            }
        };
        let (shell, stdin, control, iopub, mut hb) = conns;
        let (shell_tx, shell_rx) = shell.split();
        let (stdin_tx, stdin_rx) = stdin.split();
        let (control_tx, control_rx) = control.split();
        tasks.push(spawn_reader(shell_rx, Ch::Shell, core.clone()));
        tasks.push(spawn_reader(stdin_rx, Ch::Stdin, core.clone()));
        tasks.push(spawn_reader(control_rx, Ch::Control, core.clone()));
        tasks.push(spawn_reader(iopub, Ch::IoPub, core.clone()));

        let kernel = Arc::new(JupyterKernel {
            kernel_id: ctx.kernel_id.to_string(),
            session_id,
            display_name: name.clone(),
            interrupt_mode: spec.interrupt_mode,
            pid,
            exited,
            conn_file: conn_file.clone(),
            shell: TokioMutex::new(shell_tx),
            control: TokioMutex::new(control_tx),
            stdin: TokioMutex::new(stdin_tx),
            core: core.clone(),
            exec_lock: TokioMutex::new(()),
            pending_input: StdMutex::new(None),
            debug_seq: AtomicI64::new(0),
            tasks: StdMutex::new(tasks),
        });

        // kernel_info until the reply AND an IOPub message for it arrive: the
        // second proves the IOPub subscription is live (no lost first outputs).
        let ready = async {
            loop {
                let msg: JupyterMessage = KernelInfoRequest {}.into();
                let id = msg.header.msg_id.clone();
                let mut rx = core.register(&id);
                kernel
                    .shell
                    .lock()
                    .await
                    .send(msg)
                    .await
                    .map_err(upstream("kernel_info"))?;
                let (mut reply, mut iopub_seen) = (None::<Value>, false);
                let round = Instant::now() + Duration::from_millis(1000);
                while reply.is_none() || !iopub_seen {
                    match tokio::time::timeout_at(round, rx.recv()).await {
                        Ok(Some(m)) if m.channel == Ch::IoPub => iopub_seen = true,
                        Ok(Some(m)) if m.header.msg_type == "kernel_info_reply" => {
                            reply = Some(m.content)
                        }
                        Ok(Some(_)) => {}
                        _ => break,
                    }
                }
                core.unregister(&id);
                if let (Some(reply), true) = (reply, iopub_seen) {
                    return Ok::<Value, Error>(reply);
                }
            }
        };
        let reply = tokio::select! {
            r = tokio::time::timeout_at(deadline, ready) => match r {
                Ok(Ok(reply)) => reply,
                Ok(Err(e)) => { drop(kernel); return Err(e.into()); }
                Err(_) => { let e = started_late(); drop(kernel); return Err(e.into()); }
            },
            _ = wait_dead(&mut dead_rx) => {
                tokio::time::sleep(Duration::from_millis(100)).await;
                let err = died(&core);
                drop(kernel);
                return Err(err);
            }
        };
        let debugger = reply
            .get("supported_features")
            .and_then(Value::as_array)
            .is_some_and(|f| f.iter().any(|x| x == "debugger"))
            || reply.get("debugger").and_then(Value::as_bool) == Some(true);
        core.debugger.store(debugger, Ordering::Release);
        core.set_state("idle", None);

        let hb_core = core.clone();
        let hb_exited = kernel.exited.clone();
        let hb_task = tokio::spawn(async move {
            let mut misses = 0;
            loop {
                tokio::time::sleep(HEARTBEAT_EVERY).await;
                if hb_exited.load(Ordering::Acquire) {
                    return;
                }
                match tokio::time::timeout(Duration::from_secs(3), hb.single_heartbeat()).await {
                    Ok(Ok(())) => misses = 0,
                    _ => {
                        misses += 1;
                        if misses >= HEARTBEAT_MISSES {
                            hb_core.set_state("dead", None);
                            #[cfg(unix)]
                            if pid > 1 && !hb_exited.load(Ordering::Acquire) {
                                send_unix_signal_group(pid, 9);
                            }
                            return;
                        }
                        // A REQ socket that missed its reply can't send again.
                        match jzc::create_client_heartbeat_connection(&info).await {
                            Ok(fresh) => hb = fresh,
                            Err(_) => continue,
                        }
                    }
                }
            }
        });
        lock(&kernel.tasks).push(hb_task);
        Ok(kernel)
    }

    /// The kernel's process (and process-group) id.
    pub fn pid(&self) -> u32 {
        self.pid
    }

    pub fn status(&self) -> KernelStatus {
        self.core.status.borrow().clone()
    }

    pub fn status_event(&self) -> Value {
        self.core.status_event()
    }

    pub fn debugger(&self) -> bool {
        self.core.debugger.load(Ordering::Acquire)
    }

    fn ensure_alive(&self) -> Result<()> {
        let s = self.status();
        if s.state == "dead" {
            return Err(Error::Conflict(dead_message(s.exit_code)));
        }
        Ok(())
    }

    /// Run `code` and collect its outputs, streaming each change to the
    /// notebook's SSE subscribers tagged with `run_id`.
    pub async fn execute(
        &self,
        code: &str,
        run_id: &str,
        timeout: Option<Duration>,
    ) -> Result<ExecOutcome> {
        let _serial = self.exec_lock.lock().await;
        self.ensure_alive()?;
        let msg: JupyterMessage = ExecuteRequest {
            code: code.to_string(),
            silent: false,
            store_history: true,
            user_expressions: None,
            allow_stdin: true,
            stop_on_error: true,
        }
        .into();
        let id = msg.header.msg_id.clone();
        let mut rx = self.core.register(&id);
        let mut status_rx = self.core.status.subscribe();
        let sent = self.shell.lock().await.send(msg).await;
        if let Err(e) = sent {
            self.core.unregister(&id);
            return Err(Error::Upstream(format!("send execute_request: {e}")));
        }
        let mut out = Outputs {
            run_id: run_id.to_string(),
            events: self.core.events.clone(),
            items: Vec::new(),
            display_ids: Vec::new(),
            clear_pending: false,
            bytes: 0,
        };
        let deadline = timeout.map(|t| Instant::now() + t);
        let (mut reply, mut idle) = (None::<Value>, false);
        let result = loop {
            if reply.is_some() && idle {
                break Ok(());
            }
            let m = tokio::select! {
                m = rx.recv() => m,
                _ = status_rx.wait_for(|s| s.state == "dead") => {
                    let code = self.status().exit_code;
                    let text = dead_message(code);
                    out.push(json!({"output_type": "error", "ename": "KernelStopped", "evalue": text, "traceback": [text]}), None);
                    break Err("dead");
                }
                _ = async { tokio::time::sleep_until(deadline.unwrap_or_else(Instant::now)).await }, if deadline.is_some() => {
                    break Err("timeout");
                }
            };
            let Some(m) = m else { break Err("closed") };
            let c = &m.content;
            match m.header.msg_type.as_str() {
                "status" if m.channel == Ch::IoPub => {
                    idle |= c.get("execution_state").and_then(Value::as_str) == Some("idle");
                }
                "stream" => out.stream(c),
                "display_data" => out.display(c, None),
                "execute_result" => out.display(
                    c,
                    Some(
                        c.get("execution_count")
                            .and_then(Value::as_u64)
                            .unwrap_or(0),
                    ),
                ),
                "update_display_data" => out.update_display(c),
                "clear_output" => {
                    if c.get("wait").and_then(Value::as_bool).unwrap_or(false) {
                        out.clear_pending = true;
                    } else {
                        out.clear_now();
                    }
                }
                "error" => out.error(c),
                "execute_reply" => reply = Some(c.clone()),
                "input_request" => {
                    *lock(&self.pending_input) = Some(m.header.clone());
                    let _ = self.core.events.send(json!({
                        "kind": "input_request",
                        "run_id": run_id,
                        "prompt": c.get("prompt").and_then(Value::as_str).unwrap_or(""),
                        "password": c.get("password").and_then(Value::as_bool).unwrap_or(false),
                    }));
                }
                _ => {}
            }
        };
        self.core.unregister(&id);
        self.cancel_input();
        let status = match result {
            Ok(()) => reply
                .as_ref()
                .and_then(|r| r.get("status"))
                .and_then(Value::as_str)
                .unwrap_or("ok")
                .to_string(),
            Err("dead") => "dead".into(),
            Err("timeout") => return Err(Error::Upstream("notebook cell timed out".into())),
            Err(_) => return Err(Error::Upstream("kernel channel closed".into())),
        };
        let execution_count = reply
            .as_ref()
            .and_then(|r| r.get("execution_count"))
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        Ok(ExecOutcome {
            status,
            execution_count,
            outputs: out.items,
        })
    }

    fn cancel_input(&self) {
        if lock(&self.pending_input).take().is_some() {
            let _ = self.core.events.send(json!({ "kind": "input_done" }));
        }
    }

    /// Answer the pending `input_request` (stdin channel). `false` when none waits.
    pub async fn send_input(&self, value: &str) -> Result<bool> {
        let Some(parent) = lock(&self.pending_input).take() else {
            return Ok(false);
        };
        let mut msg: JupyterMessage = InputReply {
            value: value.to_string(),
            status: ReplyStatus::Ok,
            error: None,
        }
        .into();
        msg.parent_header = Some(parent);
        self.stdin
            .lock()
            .await
            .send(msg)
            .await
            .map_err(upstream("send input_reply"))?;
        let _ = self.core.events.send(json!({ "kind": "input_done" }));
        Ok(true)
    }

    /// Interrupt the running cell (`SIGINT` to the process group, or
    /// `interrupt_request` on control) and wait up to 5 s for it to stop.
    pub async fn interrupt(&self) -> (bool, String) {
        self.cancel_input();
        match self.interrupt_mode {
            InterruptMode::Signal =>
            {
                #[cfg(unix)]
                if self.pid > 1 && !self.exited.load(Ordering::Acquire) {
                    send_unix_signal_group(self.pid, 2);
                }
            }
            InterruptMode::Message => {
                let msg: JupyterMessage = InterruptRequest {}.into();
                if let Err(e) = self.control.lock().await.send(msg).await {
                    return (false, format!("Couldn't send interrupt_request: {e}"));
                }
            }
        }
        let mut rx = self.core.status.subscribe();
        let stopped =
            tokio::time::timeout(INTERRUPT_WAIT, rx.wait_for(|s| s.state != "busy")).await;
        match stopped {
            Ok(Ok(_)) => (true, "Interrupted".into()),
            _ => (
                false,
                "The kernel didn't stop. Restart it to stop the cell.".into(),
            ),
        }
    }

    /// One DAP request carried as `debug_request` on control; the DAP response.
    pub async fn debug_request(&self, mut request: Value, timeout: Duration) -> Result<Value> {
        if request.get("seq").is_none() {
            request["seq"] = json!(self.debug_seq.fetch_add(1, Ordering::AcqRel) + 1);
        }
        let msg: JupyterMessage = DebugRequest { content: request }.into();
        let id = msg.header.msg_id.clone();
        let mut rx = self.core.register(&id);
        let sent = self.control.lock().await.send(msg).await;
        let res = match sent {
            Err(e) => Err(Error::Upstream(format!("send debug_request: {e}"))),
            Ok(()) => loop {
                match tokio::time::timeout(timeout, rx.recv()).await {
                    Ok(Some(m)) if m.header.msg_type == "debug_reply" => break Ok(m.content),
                    Ok(Some(_)) => {}
                    Ok(None) => break Err(Error::Upstream("kernel control channel closed".into())),
                    Err(_) => {
                        break Err(Error::Upstream(
                            "the kernel's debugger didn't answer".into(),
                        ));
                    }
                }
            },
        };
        self.core.unregister(&id);
        res
    }

    /// Kernel globals through the debugger's `inspectVariables` (works before the
    /// debugger starts); empty for kernels without debugger support.
    pub async fn inspect_variables(&self) -> Vec<super::IdeNotebookVariable> {
        if !self.debugger() || self.status().state == "dead" {
            return Vec::new();
        }
        let req = json!({"type": "request", "command": "inspectVariables", "arguments": {}});
        let Ok(resp) = self.debug_request(req, Duration::from_secs(5)).await else {
            return Vec::new();
        };
        resp.pointer("/body/variables")
            .and_then(Value::as_array)
            .map(|vars| {
                vars.iter()
                    .map(|v| super::IdeNotebookVariable {
                        name: v["name"].as_str().unwrap_or("").to_string(),
                        var_type: v["type"].as_str().unwrap_or("").to_string(),
                        shape: None,
                        preview: v["value"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(200)
                            .collect(),
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Graceful stop: `shutdown_request` on control, then the process group is
    /// killed after `grace` if the kernel is still running.
    pub async fn shutdown(&self, restart: bool, grace: Duration) {
        self.core.stopping.store(true, Ordering::Release);
        self.cancel_input();
        if !self.exited.load(Ordering::Acquire) {
            let msg: JupyterMessage = ShutdownRequest { restart }.into();
            let _ = self.control.lock().await.send(msg).await;
            let deadline = Instant::now() + grace;
            while Instant::now() < deadline && !self.exited.load(Ordering::Acquire) {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        }
        self.kill();
    }

    fn kill(&self) {
        #[cfg(unix)]
        if self.pid > 1 && !self.exited.load(Ordering::Acquire) {
            send_unix_signal_group(self.pid, 9);
        }
        for t in lock(&self.tasks).drain(..) {
            t.abort();
        }
        let _ = std::fs::remove_file(&self.conn_file);
    }
}

impl Drop for JupyterKernel {
    fn drop(&mut self) {
        self.core.stopping.store(true, Ordering::Release);
        self.kill();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_env_expands_only_from_the_kernel_env() {
        let env = BTreeMap::from([("HOME".to_string(), "/tmp/h".to_string())]);
        assert_eq!(expand_spec_env("${HOME}/x", &env), "/tmp/h/x");
        // Unknown names stay as written, like Template.safe_substitute.
        assert_eq!(expand_spec_env("${NOPE}:${HOME}", &env), "${NOPE}:/tmp/h");
        assert_eq!(expand_spec_env("a${b", &env), "a${b");
        assert_eq!(expand_spec_env("plain", &env), "plain");
    }

    fn outputs() -> (Outputs, broadcast::Receiver<Value>) {
        let (tx, rx) = broadcast::channel(64);
        (
            Outputs {
                run_id: "r".into(),
                events: tx,
                items: Vec::new(),
                display_ids: Vec::new(),
                clear_pending: false,
                bytes: 0,
            },
            rx,
        )
    }

    #[test]
    fn streams_merge_and_clear_wait_defers_to_the_next_output() {
        let (mut o, _rx) = outputs();
        o.stream(&json!({"name": "stdout", "text": "0\n"}));
        o.stream(&json!({"name": "stdout", "text": "1\n"}));
        assert_eq!(o.items.len(), 1);
        assert_eq!(o.items[0]["text"], "0\n1\n");
        o.clear_pending = true;
        assert_eq!(
            o.items.len(),
            1,
            "clear_output(wait=True) waits for new output"
        );
        o.stream(&json!({"name": "stdout", "text": "2\n"}));
        assert_eq!(o.items.len(), 1);
        assert_eq!(o.items[0]["text"], "2\n");
    }

    #[test]
    fn update_display_data_replaces_by_display_id() {
        let (mut o, _rx) = outputs();
        o.display(
            &json!({"data": {"text/plain": "1"}, "transient": {"display_id": "d1"}}),
            None,
        );
        o.stream(&json!({"name": "stdout", "text": "x"}));
        o.update_display(&json!({"data": {"text/plain": "2"}, "transient": {"display_id": "d1"}}));
        assert_eq!(o.items.len(), 2);
        assert_eq!(o.items[0]["data"]["text/plain"], "2");
        assert_eq!(o.items[0]["output_type"], "display_data");
    }

    #[test]
    fn an_output_over_8_mib_is_replaced() {
        let (mut o, _rx) = outputs();
        let big = "x".repeat(MAX_OUTPUT_BYTES + 10);
        o.display(&json!({"data": {"text/plain": big}}), None);
        assert!(o.items[0]["text"]
            .as_str()
            .unwrap()
            .starts_with("Output too large to show"));
    }
}
