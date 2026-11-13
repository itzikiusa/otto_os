//! Shared language-server pool: ONE server process per `(owner, lang, root, ssh)`,
//! multiplexed across every `/ws/lsp` socket of that owner that asks for it.
//!
//! Memory, performance & process-hygiene invariants:
//! - Zero pre-spawning: servers launch only when a document of that language opens.
//! - Concurrent servers per owner are capped at [`MAX_SERVERS_PER_OWNER`] (`2` LRU).
//! - Explicit child heap caps (`jdtls` `-Xmx256m`, `gopls` `GOMEMLIMIT=384MiB` +
//!   `GOGC=75`, Node-based LSPs `--max-old-space-size=256`).
//! - Every local server spawns in its own Unix process group (`process_group(0)`)
//!   and is cleaned up via `shutdown` -> `exit` -> `killpg` (`-SIGTERM`/`-SIGKILL`).
//! - Live build-manifest edits (`pom.xml`, `build.gradle*`, `go.mod`, `Cargo.toml`,
//!   `package.json`, `pyproject.toml`) push `workspace/didChangeWatchedFiles` so
//!   servers refresh without "Invalidate Caches".

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Duration;

use serde_json::{json, Value};
use tokio::io::{AsyncBufRead, AsyncWrite, BufReader};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::{framing, servers};
use crate::types::IdeSshSpec;

/// How long an unused server outlives its last socket.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(45);
/// Fast grace period after the last open document + socket of a server closes.
const POST_DOC_IDLE_GRACE: Duration = Duration::from_secs(3);
/// Maximum concurrent pooled language servers per user (`<= 2` LRU).
pub const MAX_SERVERS_PER_OWNER: usize = 2;
/// Queued client → server messages per pooled server (backpressure).
const DATA_CAPACITY: usize = 1024;
/// Bytes queued for a server's stdin before the actor stops accepting client messages.
pub const WRITE_QUEUE_MAX_BYTES: usize = 8 * 1024 * 1024;
/// Queued server → client messages per socket before the socket is dropped.
const CLIENT_CAPACITY: usize = 1024;
/// Parsed server messages buffered between the stdout reader and the actor.
const SERVER_CAPACITY: usize = 256;

/// Identity of a pooled server.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct PoolKey {
    pub owner: String,
    pub lang: String,
    pub root: String,
    pub ssh: Option<IdeSshSpec>,
}

/// Optional fallback command + args (used by Remote SSH spawning).
#[derive(Clone, Debug, Default)]
pub struct SpawnLaunch {
    pub cmd: String,
    pub args: Vec<String>,
    pub fallback_cmd: Option<String>,
    pub fallback_args: Vec<String>,
}

impl SpawnLaunch {
    pub fn primary(cmd: impl Into<String>, args: Vec<String>) -> Self {
        Self {
            cmd: cmd.into(),
            args,
            fallback_cmd: None,
            fallback_args: Vec::new(),
        }
    }
}

/// A started server's framed-stdio ends.
pub struct ServerIo {
    pub reader: Box<dyn AsyncBufRead + Send + Unpin>,
    pub writer: Box<dyn AsyncWrite + Send + Unpin>,
    pub child: Option<tokio::process::Child>,
}

/// Starts a server for `key` with `launch`.
pub type Spawner = Arc<dyn Fn(&PoolKey, &SpawnLaunch) -> std::io::Result<ServerIo> + Send + Sync>;

static ACTIVE_POOLS: OnceLock<Mutex<Vec<Weak<LspPool>>>> = OnceLock::new();

fn register_pool(pool: &Arc<LspPool>) {
    let reg = ACTIVE_POOLS.get_or_init(|| Mutex::new(Vec::new()));
    let mut guard = reg.lock().unwrap_or_else(|e| e.into_inner());
    guard.retain(|w| w.strong_count() > 0);
    guard.push(Arc::downgrade(pool));
}

/// Notify active pooled language servers when a project build manifest changes
/// (`1` = Created, `2` = Changed, `3` = Deleted) so `jdtls`, `gopls`,
/// `rust-analyzer`, `pyright`, and `tsserver` refresh immediately without stale caches.
pub fn notify_watched_file_if_build_manifest(path: &Path, change_type: u8) {
    let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
        return;
    };
    let is_manifest = matches!(
        name,
        "pom.xml"
            | "build.gradle"
            | "build.gradle.kts"
            | "settings.gradle"
            | "settings.gradle.kts"
            | "gradle.properties"
            | "go.mod"
            | "go.sum"
            | "go.work"
            | "Cargo.toml"
            | "package.json"
            | "tsconfig.json"
            | "jsconfig.json"
            | "pyproject.toml"
            | "requirements.txt"
    );
    if !is_manifest {
        return;
    }
    let Some(reg) = ACTIVE_POOLS.get() else {
        return;
    };
    let pools: Vec<Arc<LspPool>> = {
        let mut guard = reg.lock().unwrap_or_else(|e| e.into_inner());
        guard.retain(|w| w.strong_count() > 0);
        guard.iter().filter_map(Weak::upgrade).collect()
    };
    if pools.is_empty() {
        return;
    }
    let uri = path_to_file_uri(path);
    let path_str = path.to_string_lossy();
    for pool in pools {
        pool.notify_watched_path(&path_str, &uri, change_type);
    }
}

fn path_to_file_uri(path: &Path) -> String {
    let s = path.to_string_lossy();
    if s.starts_with('/') {
        format!("file://{s}")
    } else {
        format!("file:///{s}")
    }
}

/// POSIX single-quote escaping (`'abc'` -> `'abc'`, `a'b` -> `'a'\''b'`).
pub fn posix_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

fn reject_ssh_option_like(field: &str, val: &str) -> std::io::Result<()> {
    if val.starts_with('-') || val.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            format!("ssh {field} must not start with '-' or contain whitespace"),
        ));
    }
    Ok(())
}

/// Build the remote POSIX shell script that enters `root`, augments `PATH`
/// with `~/.otto/toolchains/**/bin` + standard SDK dirs, and `exec`s the LSP.
pub fn build_remote_lsp_script(root: &str, launch: &SpawnLaunch) -> String {
    let quoted_root = posix_quote(root);
    let invoke_primary = std::iter::once(posix_quote(&launch.cmd))
        .chain(launch.args.iter().map(|a| posix_quote(a)))
        .collect::<Vec<_>>()
        .join(" ");

    let path_setup = concat!(
        "for d in ",
        "\"$HOME\"/.otto/toolchains/bin ",
        "\"$HOME\"/.otto/toolchains/*/bin ",
        "\"$HOME\"/.otto/toolchains/*/*/bin ",
        "\"$HOME\"/.otto/toolchains/*/*/*/bin ",
        "\"$HOME\"/.otto/toolchains/*/*/Contents/Home/bin ",
        "\"$HOME\"/.otto/toolchains/*/*/node_modules/.bin ",
        "\"$HOME\"/.otto/toolchains/*/*/lib/node_modules/.bin ",
        "\"$HOME/go/bin\" \"$HOME/.cargo/bin\" \"$HOME/.local/bin\"; ",
        "do [ -d \"$d\" ] && PATH=\"$d:$PATH\"; done; export PATH"
    );

    if let Some(fb) = &launch.fallback_cmd {
        let invoke_fb = std::iter::once(posix_quote(fb))
            .chain(launch.fallback_args.iter().map(|a| posix_quote(a)))
            .collect::<Vec<_>>()
            .join(" ");
        let p_check = posix_quote(&launch.cmd);
        let fb_check = posix_quote(fb);
        format!(
            "cd {quoted_root} && {path_setup}; \
             if command -v {p_check} >/dev/null 2>&1; then exec {invoke_primary}; \
             elif command -v {fb_check} >/dev/null 2>&1; then exec {invoke_fb}; \
             else echo 'otto-lsp: language server not found on remote host' >&2; exit 127; fi"
        )
    } else {
        format!("cd {quoted_root} && {path_setup}; exec {invoke_primary}")
    }
}

fn apply_language_memory_caps(
    command: &mut tokio::process::Command,
    lang: &str,
    cmd_name: &str,
    inherited: &BTreeMap<String, String>,
) {
    let norm = servers::normalize_lsp_lang(lang);
    match norm {
        "go" => {
            if !inherited.contains_key("GOGC") && std::env::var_os("GOGC").is_none() {
                command.env("GOGC", "75");
            }
            if !inherited.contains_key("GOMEMLIMIT") && std::env::var_os("GOMEMLIMIT").is_none() {
                command.env("GOMEMLIMIT", "384MiB");
            }
        }
        "java" => {
            let jvm_flags = [
                "-Xmx256m",
                "-Xms64m",
                "-XX:+UseG1GC",
                "-XX:MaxGCPauseMillis=50",
                "-XX:MinHeapFreeRatio=10",
                "-XX:MaxHeapFreeRatio=20",
            ];
            for f in jvm_flags {
                command.arg(format!("--jvm-arg={f}"));
            }
            if !inherited.contains_key("JAVA_TOOL_OPTIONS")
                && std::env::var_os("JAVA_TOOL_OPTIONS").is_none()
            {
                command.env("JAVA_TOOL_OPTIONS", jvm_flags.join(" "));
            }
        }
        "typescript" | "javascript" | "shell" | "json" | "css" | "html" => {
            set_node_heap_cap(command, inherited);
        }
        "python" if cmd_name.contains("pyright") => {
            set_node_heap_cap(command, inherited);
        }
        _ => {}
    }
}

fn set_node_heap_cap(command: &mut tokio::process::Command, inherited: &BTreeMap<String, String>) {
    let existing = inherited
        .get("NODE_OPTIONS")
        .cloned()
        .or_else(|| std::env::var("NODE_OPTIONS").ok())
        .unwrap_or_default();
    if !existing.contains("--max-old-space-size") {
        let merged = if existing.trim().is_empty() {
            "--max-old-space-size=256".to_string()
        } else {
            format!("{} --max-old-space-size=256", existing.trim())
        };
        command.env("NODE_OPTIONS", merged);
    }
}

#[cfg(unix)]
fn terminate_process_group(pgid: u32) {
    if pgid <= 1 {
        return;
    }
    std::thread::spawn(move || {
        let neg = format!("-{pgid}");
        #[allow(clippy::disallowed_methods)] // dedicated cleanup OS thread
        let _ = std::process::Command::new("/bin/kill")
            .args(["-TERM", "--", &neg])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
        #[allow(clippy::disallowed_methods)] // dedicated cleanup OS thread
        let _ = std::process::Command::new("/bin/kill")
            .args(["-KILL", "--", &neg])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    });
}

/// Production spawner supporting both Local (with inherited user login-shell env,
/// SDK `PATH`, and strict child heap caps) and Remote SSH (`ssh -T`) workspaces.
pub fn process_spawner_with_data_dir(data_dir: PathBuf) -> Spawner {
    Arc::new(move |key: &PoolKey, launch: &SpawnLaunch| {
        let mut command = if let Some(ssh) = &key.ssh {
            reject_ssh_option_like("host", &ssh.host)?;
            if !ssh.user.is_empty() {
                reject_ssh_option_like("user", &ssh.user)?;
            }
            if let Some(jump) = &ssh.proxy_jump {
                reject_ssh_option_like("proxy_jump", jump)?;
            }
            let mut c = tokio::process::Command::new("ssh");
            c.arg("-T")
                .arg("-o")
                .arg("BatchMode=yes")
                .arg("-o")
                .arg("StrictHostKeyChecking=accept-new")
                .arg("-o")
                .arg("LogLevel=ERROR")
                .arg("-o")
                .arg("ConnectTimeout=10")
                .arg("-o")
                .arg("ServerAliveInterval=15")
                .arg("-o")
                .arg("ServerAliveCountMax=3");
            if ssh.port > 0 {
                c.arg("-p").arg(ssh.port.to_string());
            }
            if let Some(identity) = &ssh.identity_file {
                if !identity.is_empty() {
                    let expanded = otto_core::paths::expand_home_strict(identity)
                        .unwrap_or_else(|_| PathBuf::from(identity));
                    c.arg("-i").arg(expanded);
                }
            }
            if let Some(jump) = &ssh.proxy_jump {
                if !jump.is_empty() {
                    c.arg("-J").arg(jump);
                }
            }
            let target = if ssh.user.is_empty() {
                ssh.host.clone()
            } else {
                format!("{}@{}", ssh.user, ssh.host)
            };
            let remote_script = build_remote_lsp_script(&key.root, launch);
            c.arg("--").arg(target).arg(remote_script);
            #[cfg(unix)]
            c.process_group(0);
            c
        } else {
            let effective_env =
                crate::environments::cached_effective_env_for_local_root(&data_dir, &key.root);
            let mut c = tokio::process::Command::new(&launch.cmd);
            c.args(&launch.args)
                .current_dir(&key.root)
                .envs(&effective_env);

            let aug_path = servers::augmented_local_path(&data_dir);
            let merged_path = match effective_env.get("PATH") {
                Some(ep) if !ep.is_empty() => format!("{ep}:{aug_path}"),
                _ => aug_path,
            };
            c.env("PATH", merged_path);

            if !effective_env.contains_key("JAVA_HOME") && std::env::var_os("JAVA_HOME").is_none() {
                if let Some(java_home) = servers::detect_toolchain_java_home(&data_dir) {
                    c.env("JAVA_HOME", java_home);
                }
            }
            apply_language_memory_caps(&mut c, &key.lang, &launch.cmd, &effective_env);
            #[cfg(unix)]
            c.process_group(0);
            c
        };

        let mut child = command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()?;
        let stdin = child.stdin.take().expect("piped stdin");
        let stdout = child.stdout.take().expect("piped stdout");
        let stderr = child.stderr.take().expect("piped stderr");
        let lang = key.lang.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut lines = BufReader::new(stderr).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                tracing::debug!(lang = lang.as_str(), "lsp stderr: {line}");
            }
        });
        Ok(ServerIo {
            reader: Box::new(BufReader::new(stdout)),
            writer: Box::new(stdin),
            child: Some(child),
        })
    })
}

/// Default spawner when no explicit `data_dir` is supplied.
pub fn process_spawner() -> Spawner {
    process_spawner_with_data_dir(std::env::temp_dir())
}

enum Ctl {
    Attach {
        client: u64,
        tx: mpsc::Sender<String>,
    },
    Detach {
        client: u64,
    },
    WatchedFile {
        uri: String,
        change_type: u8,
    },
    Evict,
}

struct Data {
    client: u64,
    text: String,
}

struct Entry {
    gen: u64,
    ctl: mpsc::UnboundedSender<Ctl>,
    data: mpsc::Sender<Data>,
    open_docs: Arc<AtomicUsize>,
    active_clients: Arc<AtomicUsize>,
    last_active: Arc<AtomicU64>,
}

/// The pool: one entry (actor task + server) per [`PoolKey`].
pub struct LspPool {
    entries: Mutex<HashMap<PoolKey, Entry>>,
    spawner: Spawner,
    idle: Duration,
    write_cap: usize,
    next_client: AtomicU64,
    next_gen: AtomicU64,
    activity_seq: Arc<AtomicU64>,
}

/// One socket's attachment. Dropping it detaches the socket.
pub struct Attached {
    client: u64,
    pub rx: mpsc::Receiver<String>,
    ctl: mpsc::UnboundedSender<Ctl>,
    data: mpsc::Sender<Data>,
}

impl Attached {
    /// Queue one client → server message. `false` once the server is gone.
    pub async fn send(&self, text: String) -> bool {
        self.data
            .send(Data {
                client: self.client,
                text,
            })
            .await
            .is_ok()
    }
}

impl Drop for Attached {
    fn drop(&mut self) {
        let _ = self.ctl.send(Ctl::Detach {
            client: self.client,
        });
    }
}

impl LspPool {
    pub fn new(spawner: Spawner, idle: Duration) -> Arc<Self> {
        Self::with_write_cap(spawner, idle, WRITE_QUEUE_MAX_BYTES)
    }

    /// [`Self::new`] with an explicit stdin backlog bound (tests).
    pub fn with_write_cap(spawner: Spawner, idle: Duration, write_cap: usize) -> Arc<Self> {
        let pool = Arc::new(Self {
            entries: Mutex::new(HashMap::new()),
            spawner,
            idle,
            write_cap,
            next_client: AtomicU64::new(1),
            next_gen: AtomicU64::new(1),
            activity_seq: Arc::new(AtomicU64::new(1)),
        });
        register_pool(&pool);
        pool
    }

    /// Number of live pooled servers.
    pub fn live_servers(&self) -> usize {
        self.lock().len()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PoolKey, Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn notify_watched_path(&self, path_str: &str, uri: &str, change_type: u8) {
        let watched_path = Path::new(path_str);
        let entries = self.lock();
        for (k, e) in entries.iter() {
            if k.ssh.is_some() {
                continue;
            }
            let in_primary = watched_path.starts_with(Path::new(&k.root));
            let in_attached = !in_primary
                && crate::environments::cached_additional_roots(&format!("local:{}", k.root))
                    .into_iter()
                    .chain(crate::environments::cached_additional_roots(&k.root))
                    .any(|ar| watched_path.starts_with(Path::new(&ar)));
            if in_primary || in_attached {
                let _ = e.ctl.send(Ctl::WatchedFile {
                    uri: uri.to_string(),
                    change_type,
                });
            }
        }
    }

    /// Attach a socket to the server for `key`, starting it when none is running.
    pub fn attach(
        self: &Arc<Self>,
        key: PoolKey,
        cmd: &str,
        args: &[String],
    ) -> std::io::Result<Attached> {
        self.attach_launch(key, &SpawnLaunch::primary(cmd, args.to_vec()))
    }

    /// Attach a socket with a [`SpawnLaunch`] (including optional remote fallback).
    pub fn attach_launch(
        self: &Arc<Self>,
        key: PoolKey,
        launch: &SpawnLaunch,
    ) -> std::io::Result<Attached> {
        let client = self.next_client.fetch_add(1, Ordering::Relaxed);
        let (out_tx, rx) = mpsc::channel(CLIENT_CAPACITY);
        let mut entries = self.lock();
        if let Some(e) = entries.get(&key) {
            if e.ctl
                .send(Ctl::Attach {
                    client,
                    tx: out_tx.clone(),
                })
                .is_ok()
            {
                let tick = self.activity_seq.fetch_add(1, Ordering::Relaxed);
                e.last_active.store(tick, Ordering::Release);
                return Ok(Attached {
                    client,
                    rx,
                    ctl: e.ctl.clone(),
                    data: e.data.clone(),
                });
            }
            entries.remove(&key);
        }

        // Immediately reap detached 0-doc servers from a previous workspace when switching roots.
        let stale_other_roots: Vec<PoolKey> = entries
            .iter()
            .filter(|(k, e)| {
                k.owner == key.owner
                    && k.root != key.root
                    && e.open_docs.load(Ordering::Acquire) == 0
                    && e.active_clients.load(Ordering::Acquire) == 0
            })
            .map(|(k, _)| k.clone())
            .collect();
        for rk in stale_other_roots {
            if let Some(evicted) = entries.remove(&rk) {
                let _ = evicted.ctl.send(Ctl::Evict);
            }
        }

        // Enforce strict `<= MAX_SERVERS_PER_OWNER` (`2`) LRU cap per user:
        // prefer evicting 0-open-doc servers first, then fallback to oldest overall.
        while entries.keys().filter(|k| k.owner == key.owner).count() >= MAX_SERVERS_PER_OWNER {
            let zero_doc_victim = entries
                .iter()
                .filter(|(k, e)| k.owner == key.owner && e.open_docs.load(Ordering::Acquire) == 0)
                .min_by_key(|(_, e)| e.last_active.load(Ordering::Acquire))
                .map(|(k, _)| k.clone());
            let victim = zero_doc_victim.or_else(|| {
                entries
                    .iter()
                    .filter(|(k, _)| k.owner == key.owner)
                    .min_by_key(|(_, e)| e.last_active.load(Ordering::Acquire))
                    .map(|(k, _)| k.clone())
            });
            let Some(victim_key) = victim else {
                break;
            };
            if let Some(evicted) = entries.remove(&victim_key) {
                let _ = evicted.ctl.send(Ctl::Evict);
            }
        }

        let io = (self.spawner)(&key, launch)?;
        let gen = self.next_gen.fetch_add(1, Ordering::Relaxed);
        let init_tick = self.activity_seq.fetch_add(1, Ordering::Relaxed);
        let open_docs = Arc::new(AtomicUsize::new(0));
        let active_clients = Arc::new(AtomicUsize::new(1));
        let last_active = Arc::new(AtomicU64::new(init_tick));
        let (ctl, ctl_rx) = mpsc::unbounded_channel();
        let (data, data_rx) = mpsc::channel(DATA_CAPACITY);
        let _ = ctl.send(Ctl::Attach { client, tx: out_tx });
        entries.insert(
            key.clone(),
            Entry {
                gen,
                ctl: ctl.clone(),
                data: data.clone(),
                open_docs: Arc::clone(&open_docs),
                active_clients: Arc::clone(&active_clients),
                last_active: Arc::clone(&last_active),
            },
        );
        drop(entries);
        tokio::spawn(run_actor(
            Arc::clone(self),
            key,
            gen,
            io,
            ctl_rx,
            data_rx,
            open_docs,
            active_clients,
            last_active,
        ));
        Ok(Attached {
            client,
            rx,
            ctl,
            data,
        })
    }

    fn remove_if(&self, key: &PoolKey, gen: u64) {
        let mut entries = self.lock();
        if entries.get(key).is_some_and(|e| e.gen == gen) {
            entries.remove(key);
        }
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_actor(
    pool: Arc<LspPool>,
    key: PoolKey,
    gen: u64,
    io: ServerIo,
    mut ctl_rx: mpsc::UnboundedReceiver<Ctl>,
    mut data_rx: mpsc::Receiver<Data>,
    open_docs: Arc<AtomicUsize>,
    active_clients: Arc<AtomicUsize>,
    last_active: Arc<AtomicU64>,
) {
    let ServerIo {
        reader,
        writer,
        child,
    } = io;
    let child_pgid = child.as_ref().and_then(tokio::process::Child::id);

    let (w_tx, mut w_rx) = mpsc::unbounded_channel::<Vec<u8>>();
    let backlog = Arc::new(AtomicUsize::new(0));
    let drained = Arc::new(tokio::sync::Notify::new());
    let writer_task = {
        let (backlog, drained) = (Arc::clone(&backlog), Arc::clone(&drained));
        tokio::spawn(async move {
            let mut writer = writer;
            while let Some(body) = w_rx.recv().await {
                let res = framing::write_message(&mut writer, &body).await;
                backlog.fetch_sub(body.len(), Ordering::AcqRel);
                drained.notify_one();
                if let Err(e) = res {
                    tracing::debug!("lsp: stdin write closed: {e}");
                    break;
                }
            }
        })
    };
    let (r_tx, mut r_rx) = mpsc::channel::<Vec<u8>>(SERVER_CAPACITY);
    let reader_task = tokio::spawn(async move {
        let mut reader = reader;
        loop {
            match framing::read_message(&mut reader).await {
                Ok(Some(body)) => {
                    if r_tx.send(body).await.is_err() {
                        break;
                    }
                }
                Ok(None) => {
                    tracing::debug!("lsp: server stdout closed");
                    break;
                }
                Err(e) => {
                    tracing::warn!("lsp: stdout framing error: {e}");
                    break;
                }
            }
        }
    });

    let mut mux = Mux::new(
        key.clone(),
        w_tx,
        Arc::clone(&backlog),
        open_docs,
        active_clients,
        last_active,
        Arc::clone(&pool.activity_seq),
    );
    let write_cap = pool.write_cap;
    let mut idle_at: Option<Instant> = None;
    loop {
        let has_room = backlog.load(Ordering::Acquire) < write_cap;
        tokio::select! {
            biased;
            c = ctl_rx.recv() => match c {
                Some(Ctl::Attach { client, tx }) => mux.attach(client, tx),
                Some(Ctl::Detach { client }) => mux.detach(client),
                Some(Ctl::WatchedFile { uri, change_type }) => mux.on_watched_file(&uri, change_type),
                Some(Ctl::Evict) | None => break,
            },
            d = data_rx.recv(), if has_room => match d {
                Some(Data { client, text }) => mux.on_client(client, &text),
                None => break,
            },
            _ = drained.notified(), if !has_room => {}
            s = r_rx.recv() => match s {
                Some(body) => {
                    if !mux.on_server(&body) {
                        break;
                    }
                }
                None => break,
            },
            _ = tokio::time::sleep_until(idle_at.unwrap_or_else(Instant::now)), if idle_at.is_some() => {
                let mut entries = pool.lock();
                if mux.clients.is_empty() && ctl_rx.is_empty() {
                    if entries.get(&key).is_some_and(|e| e.gen == gen) {
                        entries.remove(&key);
                    }
                    ctl_rx.close();
                    tracing::debug!(lang = key.lang.as_str(), root = key.root.as_str(), "lsp: idle server reaped");
                    break;
                }
                idle_at = None;
                continue;
            }
        }
        if mux.clients.is_empty() {
            let delay = if mux.ever_opened_doc && mux.docs.is_empty() {
                pool.idle.min(POST_DOC_IDLE_GRACE)
            } else {
                pool.idle
            };
            idle_at.get_or_insert_with(|| Instant::now() + delay);
        } else {
            idle_at = None;
        }
    }

    pool.remove_if(&key, gen);
    mux.clients.clear();
    reader_task.abort();
    mux.to_server(&json!({"jsonrpc": "2.0", "id": "otto-pool-shutdown", "method": "shutdown"}));
    mux.to_server(&json!({"jsonrpc": "2.0", "method": "exit"}));
    drop(mux);
    let _ = tokio::time::timeout(Duration::from_millis(800), writer_task).await;
    if let Some(mut child) = child {
        if tokio::time::timeout(Duration::from_millis(600), child.wait())
            .await
            .is_err()
        {
            #[cfg(unix)]
            if let Some(pgid) = child_pgid {
                terminate_process_group(pgid);
            }
            let _ = child.kill().await;
        } else {
            #[cfg(unix)]
            if let Some(pgid) = child_pgid {
                terminate_process_group(pgid);
            }
        }
    }
}

struct Client {
    tx: mpsc::Sender<String>,
    open: HashSet<String>,
}

enum Init {
    NotStarted,
    Pending {
        pool_id: u64,
        waiters: Vec<(u64, Value)>,
    },
    Done(Value),
}

struct Mux {
    key: PoolKey,
    to_server_tx: mpsc::UnboundedSender<Vec<u8>>,
    backlog: Arc<AtomicUsize>,
    open_docs: Arc<AtomicUsize>,
    active_clients: Arc<AtomicUsize>,
    last_active: Arc<AtomicU64>,
    activity_seq: Arc<AtomicU64>,
    clients: BTreeMap<u64, Client>,
    pending: HashMap<u64, (u64, Value)>,
    server_reqs: HashMap<String, (u64, Value)>,
    init: Init,
    initialized_sent: bool,
    docs: HashMap<String, usize>,
    ever_opened_doc: bool,
    next_id: u64,
}

fn str_at<'a>(v: &'a Value, path: &[&str]) -> Option<&'a str> {
    let mut cur = v;
    for p in path {
        cur = cur.get(*p)?;
    }
    cur.as_str()
}

fn default_gopls_settings() -> Value {
    json!({
        "directoryFilters": [
            "-**/node_modules",
            "-**/target",
            "-**/.git",
            "-**/dist",
            "-**/build",
            "-**/.svelte-kit",
            "-**/__pycache__"
        ],
        "completeUnimported": true
    })
}

fn detect_local_python_path(root: &str) -> Option<String> {
    let r = Path::new(root);
    for rel in [".venv/bin/python", "venv/bin/python", ".env/bin/python"] {
        let p = r.join(rel);
        if p.is_file() {
            return Some(p.to_string_lossy().into_owned());
        }
    }
    None
}

impl Mux {
    fn new(
        key: PoolKey,
        to_server_tx: mpsc::UnboundedSender<Vec<u8>>,
        backlog: Arc<AtomicUsize>,
        open_docs: Arc<AtomicUsize>,
        active_clients: Arc<AtomicUsize>,
        last_active: Arc<AtomicU64>,
        activity_seq: Arc<AtomicU64>,
    ) -> Self {
        Self {
            key,
            to_server_tx,
            backlog,
            open_docs,
            active_clients,
            last_active,
            activity_seq,
            clients: BTreeMap::new(),
            pending: HashMap::new(),
            server_reqs: HashMap::new(),
            init: Init::NotStarted,
            initialized_sent: false,
            docs: HashMap::new(),
            ever_opened_doc: false,
            next_id: 1,
        }
    }

    fn touch(&self) {
        let tick = self.activity_seq.fetch_add(1, Ordering::Relaxed);
        self.last_active.store(tick, Ordering::Release);
    }

    fn alloc_id(&mut self) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn to_server(&self, v: &Value) {
        if let Ok(bytes) = serde_json::to_vec(v) {
            self.queue_write(bytes);
        }
    }

    fn to_server_raw(&self, text: &str) {
        self.queue_write(text.as_bytes().to_vec());
    }

    fn queue_write(&self, bytes: Vec<u8>) {
        let n = bytes.len();
        self.backlog.fetch_add(n, Ordering::AcqRel);
        if self.to_server_tx.send(bytes).is_err() {
            self.backlog.fetch_sub(n, Ordering::AcqRel);
        }
    }

    fn send_client(&mut self, client: u64, text: String) {
        let full = match self.clients.get(&client) {
            Some(c) => c.tx.try_send(text).is_err(),
            None => false,
        };
        if full {
            tracing::warn!(client, "lsp: socket fell behind; detaching it");
            self.detach(client);
        }
    }

    fn attach(&mut self, client: u64, tx: mpsc::Sender<String>) {
        self.touch();
        self.clients.insert(
            client,
            Client {
                tx,
                open: HashSet::new(),
            },
        );
        self.active_clients
            .store(self.clients.len(), Ordering::Release);
    }

    fn detach(&mut self, client: u64) {
        let Some(c) = self.clients.remove(&client) else {
            return;
        };
        self.active_clients
            .store(self.clients.len(), Ordering::Release);
        self.touch();
        for uri in c.open {
            self.release_doc(&uri);
        }
        let orphaned: Vec<String> = self
            .server_reqs
            .iter()
            .filter(|(_, (c, _))| *c == client)
            .map(|(k, _)| k.clone())
            .collect();
        for k in orphaned {
            if let Some((_, id)) = self.server_reqs.remove(&k) {
                self.to_server(&json!({"jsonrpc": "2.0", "id": id, "result": null}));
            }
        }
        let stale: Vec<u64> = self
            .pending
            .iter()
            .filter(|(_, (c, _))| *c == client)
            .map(|(pid, _)| *pid)
            .collect();
        for pid in stale {
            self.pending.remove(&pid);
            self.to_server(
                &json!({"jsonrpc": "2.0", "method": "$/cancelRequest", "params": {"id": pid}}),
            );
        }
        if let Init::Pending { waiters, .. } = &mut self.init {
            waiters.retain(|(c, _)| *c != client);
        }
    }

    fn release_doc(&mut self, uri: &str) {
        let Some(n) = self.docs.get_mut(uri) else {
            return;
        };
        *n -= 1;
        if *n == 0 {
            self.docs.remove(uri);
            self.open_docs.store(self.docs.len(), Ordering::Release);
            self.to_server(&json!({
                "jsonrpc": "2.0",
                "method": "textDocument/didClose",
                "params": {"textDocument": {"uri": uri}},
            }));
        }
    }

    fn on_watched_file(&mut self, uri: &str, change_type: u8) {
        if !matches!(self.init, Init::Done(_)) {
            return;
        }
        self.to_server(&json!({
            "jsonrpc": "2.0",
            "method": "workspace/didChangeWatchedFiles",
            "params": {
                "changes": [{ "uri": uri, "type": change_type }]
            }
        }));
    }

    fn workspace_folders_json(&self) -> Vec<Value> {
        let mut folders = Vec::new();
        let primary_uri = if self.key.root.starts_with("file://") {
            self.key.root.clone()
        } else {
            format!("file://{}", self.key.root)
        };
        let primary_name = Path::new(&self.key.root)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("workspace")
            .to_string();
        folders.push(json!({ "uri": primary_uri, "name": primary_name }));

        if self.key.ssh.is_none() {
            let pkey = format!("local:{}", self.key.root.trim_end_matches('/'));
            for extra in crate::environments::cached_additional_roots(&pkey) {
                let extra_trimmed = extra.trim();
                if extra_trimmed.is_empty() || extra_trimmed == self.key.root {
                    continue;
                }
                let name = Path::new(extra_trimmed)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("attached");
                folders.push(json!({
                    "uri": format!("file://{extra_trimmed}"),
                    "name": name,
                }));
            }
        }
        folders
    }

    fn enrich_initialize_params(&self, msg: &mut Value) {
        let Some(params) = msg.get_mut("params").and_then(Value::as_object_mut) else {
            return;
        };
        let ws_folders = self.workspace_folders_json();
        if ws_folders.len() > 1 || !params.contains_key("workspaceFolders") {
            params.insert("workspaceFolders".into(), Value::Array(ws_folders));
        }

        let norm_lang = servers::normalize_lsp_lang(&self.key.lang);
        if norm_lang == "go" {
            let defaults = default_gopls_settings();
            match params.get_mut("initializationOptions") {
                Some(Value::Object(existing)) => {
                    if let Value::Object(def_map) = defaults {
                        for (k, v) in def_map {
                            existing.entry(k).or_insert(v);
                        }
                    }
                }
                None | Some(Value::Null) => {
                    params.insert("initializationOptions".into(), defaults);
                }
                _ => {}
            }
        } else if norm_lang == "python" && self.key.ssh.is_none() {
            if let Some(py_path) = detect_local_python_path(&self.key.root) {
                if params.get("initializationOptions").is_none() {
                    params.insert(
                        "initializationOptions".into(),
                        json!({ "pythonPath": py_path }),
                    );
                }
            }
        }
    }

    fn build_configuration_reply(&self, msg: &Value) -> Value {
        let items = msg
            .pointer("/params/items")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default();
        let norm_lang = servers::normalize_lsp_lang(&self.key.lang);
        let py_path = if norm_lang == "python" && self.key.ssh.is_none() {
            detect_local_python_path(&self.key.root)
        } else {
            None
        };
        let replies: Vec<Value> = items
            .iter()
            .map(|it| {
                let section = it.get("section").and_then(Value::as_str).unwrap_or("");
                if section == "gopls" || (section.is_empty() && norm_lang == "go") {
                    default_gopls_settings()
                } else if (section == "python" || section == "python.analysis") && py_path.is_some()
                {
                    json!({ "pythonPath": py_path })
                } else {
                    Value::Null
                }
            })
            .collect();
        Value::Array(replies)
    }

    fn on_client(&mut self, client: u64, text: &str) {
        if !self.clients.contains_key(&client) {
            return;
        }
        self.touch();
        let Ok(mut msg) = serde_json::from_str::<Value>(text) else {
            return;
        };
        let method = msg.get("method").and_then(Value::as_str).map(str::to_owned);
        let id = msg.get("id").filter(|v| !v.is_null()).cloned();
        match (method.as_deref(), id) {
            (Some("initialize"), Some(id)) => {
                if let Init::Done(result) = &self.init {
                    let reply = json!({"jsonrpc": "2.0", "id": id, "result": result}).to_string();
                    self.send_client(client, reply);
                } else if let Init::Pending { waiters, .. } = &mut self.init {
                    waiters.push((client, id));
                } else {
                    let pool_id = self.alloc_id();
                    msg["id"] = json!(pool_id);
                    self.enrich_initialize_params(&mut msg);
                    self.init = Init::Pending {
                        pool_id,
                        waiters: vec![(client, id)],
                    };
                    self.to_server(&msg);
                }
            }
            (Some("shutdown"), Some(id)) => {
                let reply = json!({"jsonrpc": "2.0", "id": id, "result": null});
                self.send_client(client, reply.to_string());
            }
            (Some(_), Some(id)) => {
                let pool_id = self.alloc_id();
                self.pending.insert(pool_id, (client, id));
                msg["id"] = json!(pool_id);
                self.to_server(&msg);
            }
            (Some("initialized"), None) => {
                if !self.initialized_sent {
                    self.initialized_sent = true;
                    self.to_server_raw(text);
                }
            }
            (Some("exit"), None) => {}
            (Some("textDocument/didOpen"), None) => {
                let Some(uri) = str_at(&msg, &["params", "textDocument", "uri"]) else {
                    return;
                };
                let uri = uri.to_owned();
                self.ever_opened_doc = true;
                let newly = self
                    .clients
                    .get_mut(&client)
                    .is_some_and(|c| c.open.insert(uri.clone()));
                let holders = if newly {
                    let n = self.docs.entry(uri.clone()).or_insert(0);
                    *n += 1;
                    let count = *n;
                    self.open_docs.store(self.docs.len(), Ordering::Release);
                    count
                } else {
                    self.docs.get(&uri).copied().unwrap_or(1)
                };
                if newly && holders == 1 {
                    self.to_server_raw(text);
                } else {
                    self.to_server(&json!({
                        "jsonrpc": "2.0",
                        "method": "textDocument/didClose",
                        "params": {"textDocument": {"uri": uri}},
                    }));
                    self.to_server_raw(text);
                }
            }
            (Some("textDocument/didClose"), None) => {
                let Some(uri) = str_at(&msg, &["params", "textDocument", "uri"]) else {
                    return;
                };
                let uri = uri.to_owned();
                if self
                    .clients
                    .get_mut(&client)
                    .is_some_and(|c| c.open.remove(&uri))
                {
                    self.release_doc(&uri);
                }
            }
            (Some("$/cancelRequest"), None) => {
                let Some(target) = msg.pointer("/params/id").cloned() else {
                    return;
                };
                let pool_id = self
                    .pending
                    .iter()
                    .find(|(_, (c, id))| *c == client && *id == target)
                    .map(|(pid, _)| *pid);
                if let Some(pid) = pool_id {
                    msg["params"]["id"] = json!(pid);
                    self.to_server(&msg);
                }
            }
            (Some(_), None) => self.to_server_raw(text),
            (None, Some(id)) => {
                if self.server_reqs.remove(&id.to_string()).is_some() {
                    self.to_server_raw(text);
                }
            }
            (None, None) => {}
        }
    }

    fn on_server(&mut self, body: &[u8]) -> bool {
        let Ok(text) = std::str::from_utf8(body) else {
            tracing::debug!("lsp: server sent non-UTF-8 body, dropping");
            return true;
        };
        let Ok(mut msg) = serde_json::from_str::<Value>(text) else {
            return true;
        };
        let method = msg.get("method").and_then(Value::as_str).map(str::to_owned);
        let id = msg.get("id").filter(|v| !v.is_null()).cloned();
        match (method.as_deref(), id) {
            (Some(m), Some(id)) => {
                // Handle protocol housekeeping requests directly inside the pool host so
                // `gopls`, `jdtls`, `pyright`, and `rust-analyzer` never block on browser tabs.
                match m {
                    "workspace/configuration" => {
                        let result = self.build_configuration_reply(&msg);
                        self.to_server(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
                    }
                    "workspace/workspaceFolders" => {
                        let result = Value::Array(self.workspace_folders_json());
                        self.to_server(&json!({"jsonrpc": "2.0", "id": id, "result": result}));
                    }
                    "window/workDoneProgress/create"
                    | "client/registerCapability"
                    | "client/unregisterCapability" => {
                        self.to_server(&json!({"jsonrpc": "2.0", "id": id, "result": null}));
                    }
                    _ => match self.clients.keys().next().copied() {
                        Some(target) => {
                            self.server_reqs.insert(id.to_string(), (target, id));
                            self.send_client(target, text.to_owned());
                        }
                        None => {
                            self.to_server(&json!({"jsonrpc": "2.0", "id": id, "result": null}));
                        }
                    },
                }
            }
            (Some("textDocument/publishDiagnostics"), None) => {
                let Some(uri) = str_at(&msg, &["params", "uri"]) else {
                    return true;
                };
                let targets: Vec<u64> = self
                    .clients
                    .iter()
                    .filter(|(_, c)| c.open.contains(uri))
                    .map(|(id, _)| *id)
                    .collect();
                for c in targets {
                    self.send_client(c, text.to_owned());
                }
            }
            (Some(_), None) => {
                let all: Vec<u64> = self.clients.keys().copied().collect();
                for c in all {
                    self.send_client(c, text.to_owned());
                }
            }
            (None, Some(id)) => {
                let Some(pid) = id.as_u64() else {
                    return true;
                };
                if let Init::Pending { pool_id, .. } = &self.init {
                    if *pool_id == pid {
                        let Init::Pending { waiters, .. } =
                            std::mem::replace(&mut self.init, Init::NotStarted)
                        else {
                            unreachable!()
                        };
                        let ok = msg.get("result").cloned();
                        let err = msg.get("error").cloned();
                        for (c, wid) in waiters {
                            let reply = match (&ok, &err) {
                                (Some(r), _) => json!({"jsonrpc": "2.0", "id": wid, "result": r}),
                                (None, Some(e)) => json!({"jsonrpc": "2.0", "id": wid, "error": e}),
                                (None, None) => {
                                    json!({"jsonrpc": "2.0", "id": wid, "result": null})
                                }
                            };
                            self.send_client(c, reply.to_string());
                        }
                        return match ok {
                            Some(r) => {
                                self.init = Init::Done(r);
                                true
                            }
                            None => false,
                        };
                    }
                }
                if let Some((c, orig)) = self.pending.remove(&pid) {
                    msg["id"] = orig;
                    self.send_client(c, msg.to_string());
                }
            }
            (None, None) => {}
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake {
        spawns: Arc<AtomicUsize>,
        inits: Arc<AtomicUsize>,
        doc_log: Arc<Mutex<Vec<String>>>,
    }

    fn fake_pool(idle: Duration) -> (Arc<LspPool>, Fake) {
        let spawns = Arc::new(AtomicUsize::new(0));
        let inits = Arc::new(AtomicUsize::new(0));
        let doc_log = Arc::new(Mutex::new(Vec::new()));
        let (s, i, d) = (spawns.clone(), inits.clone(), doc_log.clone());
        let spawner: Spawner = Arc::new(move |_key: &PoolKey, _launch: &SpawnLaunch| {
            s.fetch_add(1, Ordering::SeqCst);
            let (pool_side_w, server_r) = tokio::io::duplex(1 << 16);
            let (server_w, pool_side_r) = tokio::io::duplex(1 << 16);
            let (inits, doc_log) = (i.clone(), d.clone());
            tokio::spawn(async move {
                let mut r = BufReader::new(server_r);
                let mut w = server_w;
                while let Ok(Some(body)) = framing::read_message(&mut r).await {
                    let msg: Value = serde_json::from_slice(&body).unwrap();
                    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
                    let reply = match method {
                        "initialize" => {
                            inits.fetch_add(1, Ordering::SeqCst);
                            Some(
                                json!({"jsonrpc":"2.0","id":msg["id"],"result":{"capabilities":{"hoverProvider":true}}}),
                            )
                        }
                        "textDocument/hover" => {
                            Some(json!({"jsonrpc":"2.0","id":msg["id"],"result":{"contents":"hi"}}))
                        }
                        "textDocument/didOpen" | "textDocument/didClose" => {
                            let uri = msg["params"]["textDocument"]["uri"].as_str().unwrap();
                            doc_log.lock().unwrap().push(format!("{method} {uri}"));
                            (method == "textDocument/didOpen").then(|| {
                                json!({
                                    "jsonrpc":"2.0","method":"textDocument/publishDiagnostics",
                                    "params":{"uri":uri,"diagnostics":[]}
                                })
                            })
                        }
                        "workspace/didChangeWatchedFiles" => {
                            let uri = msg["params"]["changes"][0]["uri"]
                                .as_str()
                                .unwrap_or_default();
                            doc_log.lock().unwrap().push(format!("watched {uri}"));
                            None
                        }
                        _ => None,
                    };
                    if let Some(reply) = reply {
                        let bytes = serde_json::to_vec(&reply).unwrap();
                        if framing::write_message(&mut w, &bytes).await.is_err() {
                            break;
                        }
                    }
                }
            });
            Ok(ServerIo {
                reader: Box::new(BufReader::new(pool_side_r)),
                writer: Box::new(pool_side_w),
                child: None,
            })
        });
        (
            LspPool::new(spawner, idle),
            Fake {
                spawns,
                inits,
                doc_log,
            },
        )
    }

    fn key(root: &str) -> PoolKey {
        key_for("u1", root)
    }

    fn key_for(owner: &str, root: &str) -> PoolKey {
        PoolKey {
            owner: owner.into(),
            lang: "typescript".into(),
            root: root.into(),
            ssh: None,
        }
    }

    async fn recv(a: &mut Attached) -> Value {
        let text = tokio::time::timeout(Duration::from_secs(2), a.rx.recv())
            .await
            .expect("timed out waiting for a server message")
            .expect("socket closed");
        serde_json::from_str(&text).unwrap()
    }

    async fn nothing(a: &mut Attached) {
        let got = tokio::time::timeout(Duration::from_millis(100), a.rx.recv()).await;
        assert!(got.is_err(), "unexpected message: {got:?}");
    }

    async fn init(a: &mut Attached, id: i64) {
        let req =
            json!({"jsonrpc":"2.0","id":id,"method":"initialize","params":{"rootUri":"file:///r"}});
        assert!(a.send(req.to_string()).await);
        let resp = recv(a).await;
        assert_eq!(resp["id"], json!(id));
        assert_eq!(resp["result"]["capabilities"]["hoverProvider"], json!(true));
        a.send(json!({"jsonrpc":"2.0","method":"initialized","params":{}}).to_string())
            .await;
    }

    fn open(uri: &str) -> String {
        json!({"jsonrpc":"2.0","method":"textDocument/didOpen",
            "params":{"textDocument":{"uri":uri,"languageId":"typescript","version":1,"text":"x"}}})
        .to_string()
    }

    fn close(uri: &str) -> String {
        json!({"jsonrpc":"2.0","method":"textDocument/didClose",
            "params":{"textDocument":{"uri":uri}}})
        .to_string()
    }

    #[tokio::test]
    async fn two_sockets_for_the_same_lang_and_root_share_one_server() {
        let (pool, fake) = fake_pool(IDLE_TIMEOUT);
        let mut a = pool.attach(key("/r"), "tsls", &[]).unwrap();
        let mut b = pool.attach(key("/r"), "tsls", &[]).unwrap();
        assert_eq!(fake.spawns.load(Ordering::SeqCst), 1);
        assert_eq!(pool.live_servers(), 1);

        init(&mut a, 1).await;
        init(&mut b, 1).await;
        assert_eq!(fake.inits.load(Ordering::SeqCst), 1);

        let hover = |id: i64| {
            json!({"jsonrpc":"2.0","id":id,"method":"textDocument/hover","params":{}}).to_string()
        };
        assert!(b.send(hover(7)).await);
        assert!(a.send(hover(7)).await);
        let rb = recv(&mut b).await;
        let ra = recv(&mut a).await;
        assert_eq!((rb["id"].clone(), ra["id"].clone()), (json!(7), json!(7)));
        assert_eq!(rb["result"]["contents"], json!("hi"));
        nothing(&mut a).await;
        nothing(&mut b).await;

        let _c = pool.attach(key("/other"), "tsls", &[]).unwrap();
        assert_eq!(fake.spawns.load(Ordering::SeqCst), 2);
        assert_eq!(pool.live_servers(), 2);
    }

    #[tokio::test]
    async fn local_and_ssh_roots_never_share_a_pooled_server() {
        let (pool, fake) = fake_pool(IDLE_TIMEOUT);
        let _local = pool.attach(key("/srv/app"), "gopls", &[]).unwrap();
        let mut remote_key = key("/srv/app");
        remote_key.ssh = Some(IdeSshSpec {
            connection_id: "conn_1".into(),
            name: "box".into(),
            host: "127.0.0.1".into(),
            port: 2222,
            user: "dev".into(),
            identity_file: None,
            proxy_jump: None,
        });
        let _remote = pool.attach(remote_key, "gopls", &[]).unwrap();
        assert_eq!(fake.spawns.load(Ordering::SeqCst), 2);
        assert_eq!(pool.live_servers(), 2);
    }

    #[test]
    fn remote_script_escapes_paths_and_refuses_option_injection() {
        assert!(reject_ssh_option_like("host", "-oProxyCommand=evil").is_err());
        assert!(reject_ssh_option_like("user", "dev\nroot").is_err());
        assert!(reject_ssh_option_like("host", "dev.example.com").is_ok());

        let launch = SpawnLaunch {
            cmd: "pyright-langserver".into(),
            args: vec!["--stdio".into()],
            fallback_cmd: Some("pylsp".into()),
            fallback_args: vec![],
        };
        let script = build_remote_lsp_script("/srv/my 'app'", &launch);
        assert!(script.contains("cd '/srv/my '\\''app'\\'''"));
        assert!(script.contains("exec 'pyright-langserver' '--stdio'"));
        assert!(script.contains("elif command -v 'pylsp'"));
    }

    #[tokio::test]
    async fn shared_documents_are_ref_counted_and_diagnostics_routed() {
        let (pool, fake) = fake_pool(IDLE_TIMEOUT);
        let mut a = pool.attach(key("/r"), "tsls", &[]).unwrap();
        let mut b = pool.attach(key("/r"), "tsls", &[]).unwrap();
        init(&mut a, 1).await;
        init(&mut b, 1).await;

        a.send(open("file:///r/x.ts")).await;
        let diag = recv(&mut a).await;
        assert_eq!(diag["params"]["uri"], json!("file:///r/x.ts"));
        nothing(&mut b).await;

        b.send(open("file:///r/x.ts")).await;
        let _ = recv(&mut a).await;
        let _ = recv(&mut b).await;
        a.send(close("file:///r/x.ts")).await;
        tokio::task::yield_now().await;
        drop(b);
        let expected = vec![
            "textDocument/didOpen file:///r/x.ts".to_string(),
            "textDocument/didClose file:///r/x.ts".to_string(),
            "textDocument/didOpen file:///r/x.ts".to_string(),
            "textDocument/didClose file:///r/x.ts".to_string(),
        ];
        let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
        loop {
            let log = fake.doc_log.lock().unwrap().clone();
            if log.len() >= expected.len() || tokio::time::Instant::now() >= deadline {
                assert_eq!(log, expected);
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    #[tokio::test]
    async fn idle_server_is_reaped_and_a_quick_reattach_reuses_it() {
        let (pool, fake) = fake_pool(Duration::from_millis(80));
        let mut a = pool.attach(key("/r"), "tsls", &[]).unwrap();
        init(&mut a, 1).await;
        drop(a);
        tokio::time::sleep(Duration::from_millis(20)).await;
        let mut b = pool.attach(key("/r"), "tsls", &[]).unwrap();
        init(&mut b, 3).await;
        assert_eq!(fake.spawns.load(Ordering::SeqCst), 1);
        assert_eq!(fake.inits.load(Ordering::SeqCst), 1);
        drop(b);
        tokio::time::sleep(Duration::from_millis(300)).await;
        assert_eq!(pool.live_servers(), 0);
        let mut c = pool.attach(key("/r"), "tsls", &[]).unwrap();
        init(&mut c, 1).await;
        assert_eq!(fake.spawns.load(Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn different_owners_never_share_a_server() {
        let (pool, fake) = fake_pool(IDLE_TIMEOUT);
        let _a = pool.attach(key_for("alice", "/r"), "tsls", &[]).unwrap();
        let _b = pool.attach(key_for("bob", "/r"), "tsls", &[]).unwrap();
        let _a2 = pool.attach(key_for("alice", "/r"), "tsls", &[]).unwrap();
        assert_eq!(fake.spawns.load(Ordering::SeqCst), 2);
        assert_eq!(pool.live_servers(), 2);
    }

    #[tokio::test]
    async fn a_stalled_server_backpressures_client_sends() {
        let spawner: Spawner = Arc::new(|_key: &PoolKey, _launch: &SpawnLaunch| {
            let (pool_side_w, server_r) = tokio::io::duplex(4 * 1024);
            let (_server_w, pool_side_r) = tokio::io::duplex(1024);
            tokio::spawn(async move {
                let _hold = (server_r, _server_w);
                std::future::pending::<()>().await;
            });
            Ok(ServerIo {
                reader: Box::new(BufReader::new(pool_side_r)),
                writer: Box::new(pool_side_w),
                child: None,
            })
        });
        let pool = LspPool::with_write_cap(spawner, IDLE_TIMEOUT, 16 * 1024);
        let a = pool.attach(key("/r"), "tsls", &[]).unwrap();
        let body = "x".repeat(1024);
        let mut accepted = 0usize;
        let mut blocked = false;
        for i in 0..(DATA_CAPACITY * 3) {
            let msg = json!({"jsonrpc":"2.0","method":"textDocument/didChange",
                "params":{"n": i, "text": body}})
            .to_string();
            match tokio::time::timeout(Duration::from_millis(200), a.send(msg)).await {
                Ok(true) => accepted += 1,
                Ok(false) => panic!("server went away"),
                Err(_) => {
                    blocked = true;
                    break;
                }
            }
        }
        assert!(blocked, "sends never blocked: {accepted} accepted");
        assert!(accepted <= DATA_CAPACITY + 64, "{accepted} accepted");
    }

    #[tokio::test]
    async fn max_servers_per_owner_lru_and_workspace_switch_eviction() {
        assert_eq!(IDLE_TIMEOUT, Duration::from_secs(45));
        assert_eq!(MAX_SERVERS_PER_OWNER, 2);

        let (pool, fake) = fake_pool(IDLE_TIMEOUT);
        let lang_key = |lang: &str, root: &str| PoolKey {
            owner: "u1".into(),
            lang: lang.into(),
            root: root.into(),
            ssh: None,
        };

        let mut s_rust = pool.attach(lang_key("rust", "/repo"), "ra", &[]).unwrap();
        init(&mut s_rust, 1).await;
        s_rust.send(open("file:///repo/main.rs")).await;
        let _ = recv(&mut s_rust).await;

        let mut s_go = pool.attach(lang_key("go", "/repo"), "gopls", &[]).unwrap();
        init(&mut s_go, 1).await;

        assert_eq!(pool.live_servers(), 2);
        assert_eq!(fake.spawns.load(Ordering::SeqCst), 2);

        // Spawning a 3rd server (`typescript`) for `u1` must evict `go` (`open_docs == 0`) while keeping `rust` (`open_docs == 1`).
        let mut s_ts = pool
            .attach(lang_key("typescript", "/repo"), "tsls", &[])
            .unwrap();
        init(&mut s_ts, 1).await;

        assert_eq!(pool.live_servers(), MAX_SERVERS_PER_OWNER);
        let go_closed = tokio::time::timeout(Duration::from_secs(1), s_go.rx.recv())
            .await
            .expect("evicted server channel should close promptly");
        assert!(go_closed.is_none());

        // Manifest file notification reaches active `/repo` servers immediately,
        // while sibling prefix `/repo-other` is strictly excluded by `Path::starts_with`:
        notify_watched_file_if_build_manifest(Path::new("/repo-other/pom.xml"), 2);
        notify_watched_file_if_build_manifest(Path::new("/repo/pom.xml"), 2);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
        loop {
            let log = fake.doc_log.lock().unwrap().clone();
            if log.iter().any(|l| l == "watched file:///repo/pom.xml")
                || tokio::time::Instant::now() >= deadline
            {
                assert!(log.iter().any(|l| l == "watched file:///repo/pom.xml"));
                assert!(!log
                    .iter()
                    .any(|l| l == "watched file:///repo-other/pom.xml"));
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }

        // Close all tabs/sockets on `/repo` and switch workspace to `/repo2`: `/repo` 0-doc servers evict immediately!
        s_rust.send(close("file:///repo/main.rs")).await;
        drop(s_rust);
        drop(s_ts);
        tokio::time::sleep(Duration::from_millis(50)).await;

        let _s_new = pool.attach(lang_key("go", "/repo2"), "gopls", &[]).unwrap();
        assert_eq!(pool.live_servers(), 1);
    }
}
