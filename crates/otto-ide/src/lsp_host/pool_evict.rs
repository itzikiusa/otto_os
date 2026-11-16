//! RSS accounting, status snapshots, manifest file-watch notifications,
//! workspace-switch eviction, and inactive-first LRU victim selection for [`LspPool`].

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use tokio::sync::mpsc;

use super::pool::{
    Ctl, Entry, EvictReason, LspPool, PoolKey, LSP_RSS_BUDGET_BYTES, MAX_GLOBAL_SERVERS,
    MAX_SERVERS_PER_OWNER,
};
#[cfg(unix)]
use super::pool_spawn::kill_process_group_sync;
use super::pool_spawn::sample_pids_rss_bytes;
use super::servers;
use crate::plugins::IdePluginRegistry;
use crate::types::{IdeLspAvailableServer, IdeLspServerStat, IdeLspStatusResp};

fn is_build_manifest_name(name: &str) -> bool {
    matches!(
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
            | "Cargo.lock"
            | "package.json"
            | "package-lock.json"
            | "pnpm-lock.yaml"
            | "yarn.lock"
            | "tsconfig.json"
            | "jsconfig.json"
            | "pyproject.toml"
            | "uv.lock"
            | "poetry.lock"
            | "requirements.txt"
    )
}

impl LspPool {
    /// Terminate and evict all pooled language server processes immediately.
    pub fn shutdown(&self) {
        let evicted: Vec<Entry> = {
            let mut entries = self.lock();
            entries.drain().map(|(_, e)| e).collect()
        };
        Self::finish_evicted_entries(evicted, EvictReason::Shutdown);
    }

    pub(super) fn finish_evicted_entries(evicted: Vec<Entry>, reason: EvictReason) {
        for entry in evicted {
            #[cfg(unix)]
            if let Some(pid) = entry.child_pid {
                kill_process_group_sync(pid);
            }
            let _ = entry.ctl.send(Ctl::Evict(reason));
        }
    }

    pub(super) fn evict_stale_go_servers(&self) {
        let local_go_keys: Vec<PoolKey> = self
            .lock()
            .keys()
            .filter(|k| k.lang == "go" && k.ssh.is_none())
            .cloned()
            .collect();
        if local_go_keys.is_empty() {
            return;
        }
        let desired: HashMap<PoolKey, (Vec<String>, Vec<String>)> = local_go_keys
            .into_iter()
            .map(|k| {
                let tags = self.go_build_tags(&k.root);
                let canon_tags = crate::go_build_tags::canonicalize_go_build_tags(&tags);
                (k, (tags, canon_tags))
            })
            .collect();

        let mut evicted = Vec::new();
        {
            let mut entries = self.lock();
            for (k, (want_tags, want_canon)) in desired {
                let changed = k.go_build_tags != want_canon
                    || entries
                        .get(&k)
                        .is_some_and(|e| e.go_build_tags != want_tags);
                if changed {
                    if let Some(removed) = entries.remove(&k) {
                        evicted.push(removed);
                    }
                }
            }
        }
        Self::finish_evicted_entries(evicted, EvictReason::BuildTagsChanged);
    }

    fn cached_attached_roots(&self, root: &str) -> Vec<String> {
        match &self.environments {
            Some(envs) => envs.cached_additional_roots(root),
            None => Vec::new(),
        }
    }

    pub(super) fn compute_entry_rss_map(
        &self,
        snapshot: &[(PoolKey, Option<u32>)],
    ) -> HashMap<PoolKey, u64> {
        if let Some(probe) = &self.rss_probe_override {
            return snapshot
                .iter()
                .map(|(k, pid)| (k.clone(), probe(k, *pid)))
                .collect();
        }
        let pids: Vec<u32> = snapshot.iter().filter_map(|(_, pid)| *pid).collect();
        let by_pid = sample_pids_rss_bytes(&pids);
        snapshot
            .iter()
            .map(|(k, pid)| {
                let rss = pid.and_then(|p| by_pid.get(&p).copied()).unwrap_or(0);
                (k.clone(), rss)
            })
            .collect()
    }

    /// Snapshot live pooled language servers and combined child RSS.
    pub fn status_snapshot(
        &self,
        viewer: Option<&otto_core::domain::User>,
        data_dir: Option<&Path>,
        plugins: Option<&IdePluginRegistry>,
    ) -> IdeLspStatusResp {
        let raw: Vec<(PoolKey, Option<u32>, usize, usize)> = self
            .lock()
            .iter()
            .map(|(k, e)| {
                (
                    k.clone(),
                    e.child_pid,
                    e.active_clients.load(Ordering::Acquire),
                    e.open_docs.load(Ordering::Acquire),
                )
            })
            .collect();
        let pid_pairs: Vec<(PoolKey, Option<u32>)> =
            raw.iter().map(|(k, pid, _, _)| (k.clone(), *pid)).collect();
        let rss_map = self.compute_entry_rss_map(&pid_pairs);
        let total_rss_bytes: u64 = rss_map.values().copied().sum();
        let active_server_count = raw.len();

        let is_root = viewer.is_none_or(otto_core::auth::root_authority);
        let viewer_id = viewer.map(|u| u.id.as_str()).unwrap_or("");
        let mut servers_out: Vec<IdeLspServerStat> = raw
            .into_iter()
            .filter(|(k, _, _, _)| is_root || k.owner == viewer_id)
            .map(
                |(k, pid, active_clients, open_documents)| IdeLspServerStat {
                    rss_bytes: rss_map.get(&k).copied().unwrap_or(0),
                    language: k.lang,
                    root: k.root,
                    owner: k.owner,
                    pid: is_root.then_some(pid).flatten(),
                    active_clients,
                    open_documents,
                    remote_ssh: k.ssh.is_some(),
                },
            )
            .collect();
        servers_out.sort_by(|a, b| a.language.cmp(&b.language).then(a.root.cmp(&b.root)));

        let available_servers = match (data_dir, plugins) {
            (Some(dd), Some(pl)) => servers::detect_all_with_ide(dd, pl),
            _ => servers::detect_all(),
        }
        .into_iter()
        .map(|r| IdeLspAvailableServer {
            language: r.lang,
            available: r.available,
            command: r.command,
            install_command: r.install_command,
        })
        .collect();

        IdeLspStatusResp {
            active_server_count,
            max_servers_per_owner: MAX_SERVERS_PER_OWNER,
            max_global_servers: MAX_GLOBAL_SERVERS,
            total_rss_bytes,
            rss_budget_bytes: LSP_RSS_BUDGET_BYTES,
            servers: servers_out,
            available_servers,
        }
    }

    pub fn notify_watched_file_if_build_manifest(&self, path: &Path, change_type: u8) {
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            return;
        };
        if !is_build_manifest_name(name) {
            return;
        }
        let uri = otto_core::paths::path_to_file_uri(path);
        let candidates: Vec<(PoolKey, mpsc::UnboundedSender<Ctl>)> = self
            .lock()
            .iter()
            .filter(|(k, _)| k.ssh.is_none())
            .map(|(k, e)| (k.clone(), e.ctl.clone()))
            .collect();
        for (k, ctl) in candidates {
            if path.starts_with(Path::new(&k.root)) {
                let _ = ctl.send(Ctl::WatchedFile {
                    uri: uri.clone(),
                    change_type,
                });
            }
        }
    }

    pub fn evict_detached_other_roots_for_user(
        &self,
        owner: &str,
        matched_root: &Path,
        primary_root: &Path,
        additional_roots: &[PathBuf],
    ) {
        let mut keep_roots = HashSet::with_capacity(2 + additional_roots.len());
        let insert_clean = |set: &mut HashSet<String>, s: &str| {
            let trimmed = s.trim().trim_end_matches('/');
            if !trimmed.is_empty() {
                set.insert(trimmed.to_string());
            }
        };
        insert_clean(&mut keep_roots, &matched_root.to_string_lossy());
        insert_clean(&mut keep_roots, &primary_root.to_string_lossy());
        for ar in additional_roots {
            insert_clean(&mut keep_roots, &ar.to_string_lossy());
        }
        for ar in self.cached_attached_roots(&primary_root.to_string_lossy()) {
            insert_clean(&mut keep_roots, &ar);
        }
        for ar in self.cached_attached_roots(&matched_root.to_string_lossy()) {
            insert_clean(&mut keep_roots, &ar);
        }
        if keep_roots.is_empty() {
            return;
        }
        let snapshot_keys: Vec<PoolKey> = self
            .lock()
            .iter()
            .filter(|(k, e)| {
                k.owner == owner && k.ssh.is_none() && e.active_clients.load(Ordering::Acquire) == 0
            })
            .map(|(k, _)| k.clone())
            .collect();
        let victims: Vec<PoolKey> = snapshot_keys
            .into_iter()
            .filter(|k| {
                let kr = k.root.trim().trim_end_matches('/');
                !keep_roots.contains(kr)
                    && !self
                        .cached_attached_roots(kr)
                        .iter()
                        .any(|ar| keep_roots.contains(ar.trim().trim_end_matches('/')))
            })
            .collect();
        if victims.is_empty() {
            return;
        }
        let mut evicted = Vec::new();
        {
            let mut entries = self.lock();
            for victim in victims {
                if entries
                    .get(&victim)
                    .is_some_and(|e| e.active_clients.load(Ordering::Acquire) == 0)
                {
                    if let Some(removed) = entries.remove(&victim) {
                        evicted.push(removed);
                    }
                }
            }
        }
        Self::finish_evicted_entries(evicted, EvictReason::WorkspaceSwitched);
    }

    pub(super) fn pick_victim(
        entries: &HashMap<PoolKey, Entry>,
        protected_key: Option<&PoolKey>,
        owner_filter: Option<&str>,
        inactive_only: bool,
    ) -> Option<PoolKey> {
        let candidates = || {
            entries.iter().filter(|(k, _)| {
                protected_key != Some(*k) && owner_filter.is_none_or(|o| k.owner == o)
            })
        };
        let tier1 = candidates()
            .filter(|(_, e)| {
                e.active_clients.load(Ordering::Acquire) == 0
                    && e.open_docs.load(Ordering::Acquire) == 0
            })
            .min_by_key(|(_, e)| e.last_active.load(Ordering::Acquire))
            .map(|(k, _)| k.clone());
        if tier1.is_some() {
            return tier1;
        }
        let tier2 = candidates()
            .filter(|(_, e)| e.active_clients.load(Ordering::Acquire) == 0)
            .min_by_key(|(_, e)| e.last_active.load(Ordering::Acquire))
            .map(|(k, _)| k.clone());
        if tier2.is_some() || inactive_only {
            return tier2;
        }
        candidates()
            .min_by_key(|(_, e)| e.last_active.load(Ordering::Acquire))
            .map(|(k, _)| k.clone())
    }

    pub(super) fn evict_count_excess(
        entries: &mut HashMap<PoolKey, Entry>,
        owner: &str,
        protected_key: &PoolKey,
        reserve_slot: usize,
    ) -> Vec<Entry> {
        let mut out = Vec::new();
        let owner_cap = MAX_SERVERS_PER_OWNER.saturating_sub(reserve_slot);
        while entries
            .keys()
            .filter(|k| k.owner == owner && *k != protected_key)
            .count()
            > owner_cap
        {
            let Some(victim) = Self::pick_victim(entries, Some(protected_key), Some(owner), false)
            else {
                break;
            };
            if let Some(evicted) = entries.remove(&victim) {
                out.push(evicted);
            }
        }
        let global_cap = MAX_GLOBAL_SERVERS.saturating_sub(reserve_slot);
        while entries.keys().filter(|k| *k != protected_key).count() > global_cap {
            let Some(victim) = Self::pick_victim(entries, Some(protected_key), None, false) else {
                break;
            };
            if let Some(evicted) = entries.remove(&victim) {
                out.push(evicted);
            }
        }
        out
    }

    /// Enforce [`LSP_RSS_BUDGET_BYTES`] (`1.2 GiB`) by evicting servers that no
    /// socket is attached to, never `protected_key`. A server an editor is
    /// using is never killed for memory: staying over budget is the lesser harm
    /// (the count caps still bound how many servers exist).
    pub fn enforce_rss_budget(&self, protected_key: Option<&PoolKey>) {
        let pid_pairs: Vec<(PoolKey, Option<u32>)> = self
            .lock()
            .iter()
            .map(|(k, e)| (k.clone(), e.child_pid))
            .collect();
        if pid_pairs.len() <= 1 {
            return;
        }
        let rss_map = self.compute_entry_rss_map(&pid_pairs);
        let mut total: u64 = rss_map.values().copied().sum();
        if total <= LSP_RSS_BUDGET_BYTES {
            return;
        }
        let mut evicted_out = Vec::new();
        {
            let mut entries = self.lock();
            while total > LSP_RSS_BUDGET_BYTES && entries.len() > 1 {
                let victim = Self::pick_victim(&entries, protected_key, None, true);
                let Some(vk) = victim else {
                    break;
                };
                if let Some(evicted) = entries.remove(&vk) {
                    total = total.saturating_sub(rss_map.get(&vk).copied().unwrap_or(0));
                    evicted_out.push(evicted);
                } else {
                    break;
                }
            }
        }
        Self::finish_evicted_entries(evicted_out, EvictReason::MemoryBudget);
    }
}
