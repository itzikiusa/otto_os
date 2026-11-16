//! Multiplexing JSON-RPC actor (`Mux` + `run_actor`) for pooled LSP servers.
//!
//! Scopes `workspaceFolders` and `rootUri` strictly to the server's single
//! module root (`self.key.root`) so multi-repository projects never ask one
//! language server to index dozens of sibling repositories.

#[path = "pool_mux_rpc.rs"]
mod pool_mux_rpc;

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio::time::Instant;

use super::framing;
use super::pool::{Ctl, Data, LspPool, PoolKey, ServerIo, POST_DOC_IDLE_GRACE, SERVER_CAPACITY};
#[cfg(unix)]
use super::pool_spawn::{kill_process_group_sync, terminate_process_group};

#[cfg(unix)]
struct PgidDropGuard(Option<u32>);

#[cfg(unix)]
impl Drop for PgidDropGuard {
    fn drop(&mut self) {
        if let Some(pgid) = self.0 {
            kill_process_group_sync(pgid);
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_actor(
    pool: Arc<LspPool>,
    key: PoolKey,
    gen: u64,
    io: ServerIo,
    mut ctl_rx: mpsc::UnboundedReceiver<Ctl>,
    mut data_rx: mpsc::Receiver<Data>,
    open_docs: Arc<AtomicUsize>,
    _active_clients: Arc<AtomicUsize>,
    last_active: Arc<AtomicU64>,
    owner_is_root: bool,
    go_build_tags: Vec<String>,
) {
    let ServerIo {
        reader,
        writer,
        child,
    } = io;
    #[cfg(unix)]
    let child_pgid = child.as_ref().and_then(tokio::process::Child::id);
    #[cfg(unix)]
    let _pgid_guard = PgidDropGuard(child_pgid);

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
        last_active,
        Arc::clone(&pool.activity_seq),
        owner_is_root,
        go_build_tags,
    );
    let write_cap = pool.write_cap;
    let mut idle_at: Option<Instant> = None;
    let mut was_evicted = false;
    let mut rss_tick = tokio::time::interval(Duration::from_secs(15));
    rss_tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let _ = rss_tick.tick().await;

    loop {
        let has_room = backlog.load(Ordering::Acquire) < write_cap;
        tokio::select! {
            biased;
            c = ctl_rx.recv() => match c {
                Some(Ctl::Attach { client, tx }) => mux.attach(client, tx),
                Some(Ctl::Detach { client }) => mux.detach(client),
                Some(Ctl::WatchedFile { uri, change_type }) => mux.on_watched_file(&uri, change_type),
                Some(Ctl::Evict) => {
                    was_evicted = true;
                    break;
                }
                None => break,
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
            _ = rss_tick.tick() => {
                let protect = (!mux.clients.is_empty()).then(|| key.clone());
                let pool_cl = Arc::clone(&pool);
                let _ = tokio::task::spawn_blocking(move || {
                    pool_cl.enforce_rss_budget(protect.as_ref());
                })
                .await;
            }
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
            let delay = if mux.docs.is_empty() {
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
    if was_evicted {
        writer_task.abort();
        drop(mux);
        if let Some(mut child) = child {
            #[cfg(unix)]
            if let Some(pgid) = child_pgid {
                kill_process_group_sync(pgid);
            }
            let _ = child.kill().await;
            let _ = tokio::time::timeout(Duration::from_millis(150), child.wait()).await;
        }
        return;
    }
    mux.to_server(&json!({"jsonrpc": "2.0", "id": "otto-pool-shutdown", "method": "shutdown"}));
    mux.to_server(&json!({"jsonrpc": "2.0", "method": "exit"}));
    drop(mux);
    let _ = tokio::time::timeout(Duration::from_millis(150), writer_task).await;
    if let Some(mut child) = child {
        if tokio::time::timeout(Duration::from_millis(150), child.wait())
            .await
            .is_err()
        {
            #[cfg(unix)]
            if let Some(pgid) = child_pgid {
                terminate_process_group(pgid);
            }
            let _ = child.kill().await;
            let _ = tokio::time::timeout(Duration::from_millis(150), child.wait()).await;
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
    canon_root: Option<PathBuf>,
    to_server_tx: mpsc::UnboundedSender<Vec<u8>>,
    backlog: Arc<AtomicUsize>,
    open_docs: Arc<AtomicUsize>,
    last_active: Arc<AtomicU64>,
    activity_seq: Arc<AtomicU64>,
    owner_is_root: bool,
    go_build_tags: Vec<String>,
    clients: BTreeMap<u64, Client>,
    pending: HashMap<u64, (u64, Value)>,
    server_reqs: HashMap<String, (u64, Value)>,
    init: Init,
    initialized_sent: bool,
    docs: HashMap<String, usize>,
    next_id: u64,
}

impl Mux {
    #[allow(clippy::too_many_arguments)]
    fn new(
        key: PoolKey,
        to_server_tx: mpsc::UnboundedSender<Vec<u8>>,
        backlog: Arc<AtomicUsize>,
        open_docs: Arc<AtomicUsize>,
        last_active: Arc<AtomicU64>,
        activity_seq: Arc<AtomicU64>,
        owner_is_root: bool,
        go_build_tags: Vec<String>,
    ) -> Self {
        #[allow(clippy::disallowed_methods)]
        let canon_root = key
            .ssh
            .is_none()
            .then(|| {
                std::fs::canonicalize(&key.root).ok().or_else(|| {
                    otto_core::secret_paths::canonicalize_prospective(std::path::Path::new(
                        &key.root,
                    ))
                })
            })
            .flatten();
        Self {
            key,
            canon_root,
            to_server_tx,
            backlog,
            open_docs,
            last_active,
            activity_seq,
            owner_is_root,
            go_build_tags,
            clients: BTreeMap::new(),
            pending: HashMap::new(),
            server_reqs: HashMap::new(),
            init: Init::NotStarted,
            initialized_sent: false,
            docs: HashMap::new(),
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
    }

    fn detach(&mut self, client: u64) {
        let Some(c) = self.clients.remove(&client) else {
            return;
        };
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
}
