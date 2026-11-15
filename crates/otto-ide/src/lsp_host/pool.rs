//! Shared language-server pool: ONE server process per `(owner, lang, root, ssh)`,
//! multiplexed across every `/ws/lsp` socket of that owner that asks for it.
//!
//! Scale, memory & process-hygiene invariants:
//! - Zero pre-spawning: servers launch only when a code document opens, scoped
//!   to its module root (`go.mod`, `pom.xml`, `Cargo.toml`, `tsconfig.json`, `pyproject.toml`).
//! - Lightweight formats (`markdown`, `csv`, `sql`, `json`, `yaml`) never spawn processes.
//! - Concurrent servers are capped at [`MAX_SERVERS_PER_OWNER`] (`3`) and
//!   [`MAX_GLOBAL_SERVERS`] (`3`), with total child RSS bounded by
//!   [`LSP_RSS_BUDGET_BYTES`] (`1.2 GiB`), evicting `active_clients == 0` servers
//!   first and never evicting the active/just-requested server.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use dashmap::DashMap;
use tokio::io::{AsyncBufRead, AsyncWrite};
use tokio::sync::mpsc;

use super::pool_mux::run_actor;
use super::pool_spawn::build_process_spawner;
pub use super::pool_spawn::{build_remote_lsp_script, posix_quote};
use super::servers;
use crate::environments::module_sdks::effective_go_build_tags_for_paths_in_project;
use crate::environments::EnvironmentManager;
use crate::types::{IdeProjectEnvironment, IdeSshSpec};

/// How long an unused server outlives its last socket when documents remain open.
pub const IDLE_TIMEOUT: Duration = Duration::from_secs(45);
/// Fast grace period after a server has 0 attached sockets and 0 open documents.
pub(super) const POST_DOC_IDLE_GRACE: Duration = Duration::from_millis(450);
/// Maximum concurrent pooled language servers per user (`<= 3` LRU).
pub const MAX_SERVERS_PER_OWNER: usize = 3;
/// Maximum concurrent pooled language servers across the entire daemon (`<= 3` LRU).
pub const MAX_GLOBAL_SERVERS: usize = 3;
/// Total child RSS budget across all pooled language servers (`1.2 GiB`).
pub const LSP_RSS_BUDGET_BYTES: u64 = 1_288_490_188;
/// Queued client → server messages per pooled server (backpressure).
pub(super) const DATA_CAPACITY: usize = 1024;
/// Bytes queued for a server's stdin before the actor stops accepting client messages.
pub const WRITE_QUEUE_MAX_BYTES: usize = 8 * 1024 * 1024;
/// Queued server → client messages per socket before the socket is dropped.
const CLIENT_CAPACITY: usize = 1024;
/// Parsed server messages buffered between the stdout reader and the actor.
pub(super) const SERVER_CAPACITY: usize = 256;

/// Identity of a pooled server.
///
/// For local `go` (`gopls`) servers, `go_build_tags` holds the canonical sorted
/// build tags (`(root, sorted_tags)`) so distinct build-tag configurations never
/// collide on the same pool entry.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct PoolKey {
    pub owner: String,
    pub lang: String,
    pub root: String,
    pub ssh: Option<IdeSshSpec>,
    pub go_build_tags: Vec<String>,
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
pub(crate) type RssProbe = Arc<dyn Fn(&PoolKey, Option<u32>) -> u64 + Send + Sync>;

/// Production spawner bound directly to an [`EnvironmentManager`] instance.
pub fn process_spawner_with_engine(data_dir: PathBuf, environments: EnvironmentManager) -> Spawner {
    build_process_spawner(data_dir, Some(environments), Arc::new(DashMap::new()))
}

/// Production spawner supporting both Local and Remote SSH (`ssh -T`) workspaces.
pub fn process_spawner_with_data_dir(data_dir: PathBuf) -> Spawner {
    build_process_spawner(data_dir, None, Arc::new(DashMap::new()))
}

/// Default spawner when no explicit `data_dir` is supplied.
pub fn process_spawner() -> Spawner {
    process_spawner_with_data_dir(std::env::temp_dir())
}

pub(super) enum Ctl {
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

pub(super) struct Data {
    pub(super) client: u64,
    pub(super) text: String,
}

pub(super) struct Entry {
    pub(super) gen: u64,
    pub(super) child_pid: Option<u32>,
    pub(super) ctl: mpsc::UnboundedSender<Ctl>,
    pub(super) data: mpsc::Sender<Data>,
    pub(super) open_docs: Arc<AtomicUsize>,
    pub(super) active_clients: Arc<AtomicUsize>,
    pub(super) last_active: Arc<AtomicU64>,
    pub(super) go_build_tags: Vec<String>,
}

struct InflightGuard<'a> {
    inflight: &'a (Mutex<HashSet<PoolKey>>, Condvar),
    key: PoolKey,
}

impl Drop for InflightGuard<'_> {
    fn drop(&mut self) {
        let mut set = self.inflight.0.lock().unwrap_or_else(|e| e.into_inner());
        set.remove(&self.key);
        self.inflight.1.notify_all();
    }
}

/// The pool: one entry (actor task + server) per [`PoolKey`].
pub struct LspPool {
    pub(super) entries: Mutex<HashMap<PoolKey, Entry>>,
    inflight: (Mutex<HashSet<PoolKey>>, Condvar),
    spawner: Spawner,
    pub(super) idle: Duration,
    pub(super) write_cap: usize,
    next_client: AtomicU64,
    next_gen: AtomicU64,
    pub(super) activity_seq: Arc<AtomicU64>,
    pub(super) environments: Option<EnvironmentManager>,
    owner_privileges: Arc<DashMap<String, bool>>,
    pub(super) go_build_tags_by_root: DashMap<String, Vec<String>>,
    pub(super) project_go_tag_roots: DashMap<String, Vec<String>>,
    pub(super) rss_probe_override: Option<RssProbe>,
}

/// One socket's attachment. Dropping it synchronously decrements `active_clients`
/// and detaches the socket from the server mux.
pub struct Attached {
    client: u64,
    pub root: String,
    pub lang: String,
    pub rx: mpsc::Receiver<String>,
    ctl: mpsc::UnboundedSender<Ctl>,
    data: mpsc::Sender<Data>,
    active_clients: Arc<AtomicUsize>,
}

impl Attached {
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
        self.active_clients.fetch_sub(1, Ordering::AcqRel);
        let _ = self.ctl.send(Ctl::Detach {
            client: self.client,
        });
    }
}

impl LspPool {
    pub fn new(spawner: Spawner, idle: Duration) -> Arc<Self> {
        Self::with_write_cap(spawner, idle, WRITE_QUEUE_MAX_BYTES)
    }

    pub fn new_with_environments(
        spawner: Spawner,
        idle: Duration,
        environments: EnvironmentManager,
    ) -> Arc<Self> {
        Self::new_inner(
            spawner,
            idle,
            WRITE_QUEUE_MAX_BYTES,
            Some(environments),
            None,
        )
    }

    pub fn new_managed(
        data_dir: PathBuf,
        idle: Duration,
        environments: EnvironmentManager,
    ) -> Arc<Self> {
        let owner_privileges = Arc::new(DashMap::new());
        let spawner = build_process_spawner(
            data_dir,
            Some(environments.clone()),
            Arc::clone(&owner_privileges),
        );
        Arc::new(Self {
            entries: Mutex::new(HashMap::new()),
            inflight: (Mutex::new(HashSet::new()), Condvar::new()),
            spawner,
            idle,
            write_cap: WRITE_QUEUE_MAX_BYTES,
            next_client: AtomicU64::new(1),
            next_gen: AtomicU64::new(1),
            activity_seq: Arc::new(AtomicU64::new(1)),
            environments: Some(environments),
            owner_privileges,
            go_build_tags_by_root: DashMap::new(),
            project_go_tag_roots: DashMap::new(),
            rss_probe_override: None,
        })
    }

    pub fn with_write_cap(spawner: Spawner, idle: Duration, write_cap: usize) -> Arc<Self> {
        Self::new_inner(spawner, idle, write_cap, None, None)
    }

    pub(crate) fn new_inner(
        spawner: Spawner,
        idle: Duration,
        write_cap: usize,
        environments: Option<EnvironmentManager>,
        rss_probe_override: Option<RssProbe>,
    ) -> Arc<Self> {
        Arc::new(Self {
            entries: Mutex::new(HashMap::new()),
            inflight: (Mutex::new(HashSet::new()), Condvar::new()),
            spawner,
            idle,
            write_cap,
            next_client: AtomicU64::new(1),
            next_gen: AtomicU64::new(1),
            activity_seq: Arc::new(AtomicU64::new(1)),
            environments,
            owner_privileges: Arc::new(DashMap::new()),
            go_build_tags_by_root: DashMap::new(),
            project_go_tag_roots: DashMap::new(),
            rss_probe_override,
        })
    }

    pub fn record_owner_privilege(&self, owner: &str, is_root: bool) {
        self.owner_privileges.insert(owner.to_string(), is_root);
    }

    pub(super) fn owner_is_root(&self, owner: &str) -> bool {
        self.owner_privileges
            .get(owner)
            .map(|r| *r)
            .unwrap_or(false)
    }

    fn normalize_tag_root(root: &str) -> String {
        let trimmed = root.trim();
        if trimmed == "/" {
            "/".to_string()
        } else {
            trimmed.trim_end_matches('/').to_string()
        }
    }

    /// Update active Go build tags for `root` and evict/restart any local `go`
    /// language servers under `root` whose effective build tags changed.
    pub fn set_go_build_tags(&self, root: &str, tags: Vec<String>) {
        let norm = Self::normalize_tag_root(root);
        if norm.is_empty() {
            return;
        }
        let clean = crate::go_build_tags::validate_go_build_tags(&tags).unwrap_or_default();
        self.go_build_tags_by_root.insert(norm, clean);
        self.evict_stale_go_servers();
    }

    /// Resolve effective Go build tags for `root` from explicit pool overrides
    /// (longest ancestor root match) or the hydrated [`EnvironmentManager`].
    pub fn go_build_tags(&self, root: &str) -> Vec<String> {
        let norm = Self::normalize_tag_root(root);
        if norm.is_empty() {
            return Vec::new();
        }
        let norm_path = Path::new(&norm);
        let mut best: Option<(usize, Vec<String>)> = None;
        for item in self.go_build_tags_by_root.iter() {
            let mr = item.key();
            if (norm == *mr || norm_path.starts_with(Path::new(mr)))
                && best.as_ref().is_none_or(|(len, _)| mr.len() > *len)
            {
                best = Some((mr.len(), item.value().clone()));
            }
        }
        if let Some((_, tags)) = best {
            return tags;
        }
        self.environments
            .as_ref()
            .map(|e| e.effective_go_build_tags_for_local_root(&norm))
            .unwrap_or_default()
    }

    /// Synchronize project-level and per-module Go build tags after `PUT /ide/environments`,
    /// evicting affected local `go` servers (including cleared sibling module roots)
    /// whose effective tags changed.
    pub fn sync_project_go_build_tags(&self, project_root: &str, env: &IdeProjectEnvironment) {
        let norm = Self::normalize_tag_root(project_root);
        if norm.is_empty() {
            return;
        }
        if let Some((_, prev_roots)) = self.project_go_tag_roots.remove(&norm) {
            for r in prev_roots {
                self.go_build_tags_by_root.remove(&r);
            }
        }
        let norm_path = Path::new(&norm);
        let stale_keys: Vec<String> = self
            .go_build_tags_by_root
            .iter()
            .filter(|it| it.key() == &norm || Path::new(it.key()).starts_with(norm_path))
            .map(|it| it.key().clone())
            .collect();
        for k in stale_keys {
            self.go_build_tags_by_root.remove(&k);
        }

        let mut tracked_roots = Vec::new();
        let root_tags = effective_go_build_tags_for_paths_in_project(
            Some(&norm),
            env,
            std::slice::from_ref(&norm),
            true,
        );
        self.go_build_tags_by_root.insert(norm.clone(), root_tags);
        tracked_roots.push(norm.clone());

        for ar in &env.additional_roots {
            let ar_norm = Self::normalize_tag_root(ar);
            if !ar_norm.is_empty() {
                let ar_tags = effective_go_build_tags_for_paths_in_project(
                    Some(&norm),
                    env,
                    std::slice::from_ref(&ar_norm),
                    true,
                );
                self.go_build_tags_by_root.insert(ar_norm.clone(), ar_tags);
                if !tracked_roots.contains(&ar_norm) {
                    tracked_roots.push(ar_norm);
                }
            }
        }
        for mod_root in env.module_go_build_tags.keys() {
            let mr_norm = Self::normalize_tag_root(mod_root);
            if !mr_norm.is_empty() {
                let mr_tags = effective_go_build_tags_for_paths_in_project(
                    Some(&norm),
                    env,
                    std::slice::from_ref(&mr_norm),
                    true,
                );
                self.go_build_tags_by_root.insert(mr_norm.clone(), mr_tags);
                if !tracked_roots.contains(&mr_norm) {
                    tracked_roots.push(mr_norm);
                }
            }
        }
        self.project_go_tag_roots.insert(norm, tracked_roots);
        self.evict_stale_go_servers();
    }

    pub fn live_servers(&self) -> usize {
        self.lock().len()
    }

    pub(super) fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<PoolKey, Entry>> {
        self.entries.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn attach(
        self: &Arc<Self>,
        key: PoolKey,
        cmd: &str,
        args: &[String],
    ) -> std::io::Result<Attached> {
        self.attach_launch(key, &SpawnLaunch::primary(cmd, args.to_vec()))
    }

    pub fn attach_launch(
        self: &Arc<Self>,
        key: PoolKey,
        launch: &SpawnLaunch,
    ) -> std::io::Result<Attached> {
        self.attach_launch_with_file(key, launch, None)
    }

    pub fn attach_launch_with_file(
        self: &Arc<Self>,
        mut key: PoolKey,
        launch: &SpawnLaunch,
        file_hint: Option<&str>,
    ) -> std::io::Result<Attached> {
        if servers::is_lightweight_non_lsp_lang(&key.lang) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Unsupported,
                format!(
                    "language '{}' uses built-in lightweight intelligence without an external LSP process",
                    key.lang
                ),
            ));
        }
        key.lang = servers::normalize_lsp_lang(&key.lang).to_string();
        if key.ssh.is_none() {
            key.root = servers::resolve_lsp_module_root(&key.lang, &key.root, file_hint, &[])?;
        }
        let go_tags = if key.lang == "go" && key.ssh.is_none() {
            let resolved = if key.go_build_tags.is_empty() {
                self.go_build_tags(&key.root)
            } else {
                key.go_build_tags.clone()
            };
            key.go_build_tags = crate::go_build_tags::canonicalize_go_build_tags(&resolved);
            resolved
        } else {
            key.go_build_tags.clear();
            Vec::new()
        };

        let client = self.next_client.fetch_add(1, Ordering::Relaxed);
        let (out_tx, rx) = mpsc::channel(CLIENT_CAPACITY);

        let _inflight_guard = loop {
            {
                let mut entries = self.lock();
                if let Some(e) = entries.get(&key) {
                    if e.ctl
                        .send(Ctl::Attach {
                            client,
                            tx: out_tx.clone(),
                        })
                        .is_ok()
                    {
                        e.active_clients.fetch_add(1, Ordering::AcqRel);
                        let tick = self.activity_seq.fetch_add(1, Ordering::Relaxed);
                        e.last_active.store(tick, Ordering::Release);
                        let active_clients = Arc::clone(&e.active_clients);
                        let ctl = e.ctl.clone();
                        let data = e.data.clone();
                        drop(entries);
                        if self.rss_probe_override.is_some() {
                            self.enforce_rss_budget(Some(&key));
                        }
                        return Ok(Attached {
                            client,
                            root: key.root,
                            lang: key.lang,
                            rx,
                            ctl,
                            data,
                            active_clients,
                        });
                    }
                    entries.remove(&key);
                }
            }

            let mut set = self.inflight.0.lock().unwrap_or_else(|e| e.into_inner());
            if set.contains(&key) {
                while set.contains(&key) {
                    set = self.inflight.1.wait(set).unwrap_or_else(|e| e.into_inner());
                }
                continue;
            }
            set.insert(key.clone());
            break InflightGuard {
                inflight: &self.inflight,
                key: key.clone(),
            };
        };

        self.enforce_rss_budget(Some(&key));
        let evicted_pre = {
            let mut entries = self.lock();
            Self::evict_count_excess(&mut entries, &key.owner, &key, 1)
        };
        Self::finish_evicted_entries(evicted_pre);

        let owner_is_root = self.owner_is_root(&key.owner);

        let io = (self.spawner)(&key, launch)?;
        let child_pid = io.child.as_ref().and_then(tokio::process::Child::id);
        let gen = self.next_gen.fetch_add(1, Ordering::Relaxed);
        let init_tick = self.activity_seq.fetch_add(1, Ordering::Relaxed);
        let open_docs = Arc::new(AtomicUsize::new(0));
        let active_clients = Arc::new(AtomicUsize::new(1));
        let last_active = Arc::new(AtomicU64::new(init_tick));
        let (ctl, ctl_rx) = mpsc::unbounded_channel();
        let (data, data_rx) = mpsc::channel(DATA_CAPACITY);
        let _ = ctl.send(Ctl::Attach { client, tx: out_tx });

        let evicted_post = {
            let mut entries = self.lock();
            let ev = Self::evict_count_excess(&mut entries, &key.owner, &key, 1);
            entries.insert(
                key.clone(),
                Entry {
                    gen,
                    child_pid,
                    ctl: ctl.clone(),
                    data: data.clone(),
                    open_docs: Arc::clone(&open_docs),
                    active_clients: Arc::clone(&active_clients),
                    last_active: Arc::clone(&last_active),
                    go_build_tags: go_tags.clone(),
                },
            );
            ev
        };
        Self::finish_evicted_entries(evicted_post);
        drop(_inflight_guard);
        self.enforce_rss_budget(Some(&key));

        let resolved_root = key.root.clone();
        let resolved_lang = key.lang.clone();
        tokio::spawn(run_actor(
            Arc::clone(self),
            key,
            gen,
            io,
            ctl_rx,
            data_rx,
            open_docs,
            Arc::clone(&active_clients),
            last_active,
            owner_is_root,
            go_tags,
        ));
        Ok(Attached {
            client,
            root: resolved_root,
            lang: resolved_lang,
            rx,
            ctl,
            data,
            active_clients,
        })
    }

    pub(super) fn remove_if(&self, key: &PoolKey, gen: u64) {
        let mut entries = self.lock();
        if entries.get(key).is_some_and(|e| e.gen == gen) {
            entries.remove(key);
        }
    }
}
