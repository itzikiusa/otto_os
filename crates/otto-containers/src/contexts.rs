//! Docker context and Unix socket discovery + persisted Otto selection.
//!
//! Discovery is read-only against `~/.docker/*` — Otto never rewrites the
//! user's Docker config here. Otto's own active selection is persisted under
//! `<data_dir>/containers-context.json`.

use std::collections::BTreeSet;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use futures_util::future::join_all;
use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};
use tokio::fs;
use tokio::net::UnixStream;
use tokio::time::{timeout, Duration};

use crate::types::{ContainersContextsResp, DockerContextInfo, EngineKind};

const CONTEXT_STORE_FILE: &str = "containers-context.json";
const REACHABILITY_TIMEOUT: Duration = Duration::from_millis(250);
static TMP_FILE_SEQ: AtomicU64 = AtomicU64::new(0);

/// Persisted user selection stored under `<data_dir>/containers-context.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedContextSelection {
    pub name: String,
    pub socket_path: String,
}

/// Resolved Docker endpoint used by a request.
#[derive(Debug, Clone)]
pub struct ResolvedEndpoint {
    pub context_name: String,
    pub socket_path: PathBuf,
    pub engine_kind: EngineKind,
}

/// Validate and normalize a user-supplied Unix socket path (strips `unix://`).
pub fn normalize_socket_path(raw: &str) -> Result<PathBuf> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::Invalid("socket_path cannot be empty".into()));
    }
    if trimmed.contains('\0') {
        return Err(Error::Invalid("socket_path contains NUL byte".into()));
    }
    let stripped = trimmed.strip_prefix("unix://").unwrap_or(trimmed);
    let path = PathBuf::from(stripped);
    if !path.is_absolute() {
        return Err(Error::Invalid(format!(
            "socket_path must be an absolute path: {stripped}"
        )));
    }
    for comp in path.components() {
        if matches!(comp, Component::ParentDir) {
            return Err(Error::Invalid(
                "socket_path must not contain `..` segments".into(),
            ));
        }
    }
    Ok(path)
}

/// Classify the container engine backing a context name / socket path / `/info`.
pub fn classify_engine(
    context_name: &str,
    socket_path: &Path,
    os_hint: Option<&str>,
) -> EngineKind {
    let os_lower = os_hint.unwrap_or_default().to_ascii_lowercase();
    if os_lower.contains("kiri") {
        return EngineKind::Kiri;
    }
    if os_lower.contains("otto") {
        return EngineKind::Otto;
    }
    if os_lower.contains("orbstack") {
        return EngineKind::Orbstack;
    }
    if os_lower.contains("docker desktop") {
        return EngineKind::DockerDesktop;
    }
    if os_lower.contains("colima") {
        return EngineKind::Colima;
    }
    if os_lower.contains("rancher") {
        return EngineKind::RancherDesktop;
    }
    if os_lower.contains("podman") {
        return EngineKind::Podman;
    }

    let name_lower = context_name.to_ascii_lowercase();
    let path_lower = socket_path.to_string_lossy().to_ascii_lowercase();
    let combined = format!("{name_lower} {path_lower}");
    if combined.contains("/.kiri/") || name_lower == "kiri" {
        EngineKind::Kiri
    } else if combined.contains("/.otto/") || name_lower == "otto" {
        EngineKind::Otto
    } else if combined.contains("orbstack") {
        EngineKind::Orbstack
    } else if combined.contains("colima") {
        EngineKind::Colima
    } else if combined.contains("desktop-linux")
        || combined.contains("docker.app")
        || combined.contains("/.docker/run/docker.sock")
    {
        EngineKind::DockerDesktop
    } else if combined.contains("/.rd/") || combined.contains("rancher") {
        EngineKind::RancherDesktop
    } else if combined.contains("podman") {
        EngineKind::Podman
    } else if combined.contains("lima") {
        EngineKind::Lima
    } else if name_lower == "custom" {
        EngineKind::Custom
    } else {
        EngineKind::Unknown
    }
}

/// Load Otto's saved context override from `<data_dir>/containers-context.json`.
pub async fn load_saved_selection(data_dir: &Path) -> Option<SavedContextSelection> {
    let path = data_dir.join(CONTEXT_STORE_FILE);
    let raw = fs::read_to_string(&path).await.ok()?;
    serde_json::from_str::<SavedContextSelection>(&raw).ok()
}

/// Persist Otto's selected context/socket atomically under `<data_dir>` using a
/// unique per-write temp filename so concurrent saves never collide.
pub async fn save_selection(data_dir: &Path, selection: &SavedContextSelection) -> Result<()> {
    fs::create_dir_all(data_dir)
        .await
        .map_err(|e| Error::Internal(format!("create data dir: {e}")))?;
    let target = data_dir.join(CONTEXT_STORE_FILE);
    let seq = TMP_FILE_SEQ.fetch_add(1, Ordering::Relaxed);
    let tmp = data_dir.join(format!(
        "{CONTEXT_STORE_FILE}.{}.{seq}.tmp",
        std::process::id()
    ));
    let payload = serde_json::to_vec_pretty(selection)
        .map_err(|e| Error::Internal(format!("serialize context selection: {e}")))?;
    if let Err(e) = fs::write(&tmp, payload).await {
        let _ = fs::remove_file(&tmp).await;
        return Err(Error::Internal(format!("write temp context file: {e}")));
    }
    if let Err(e) = fs::rename(&tmp, &target).await {
        let _ = fs::remove_file(&tmp).await;
        return Err(Error::Internal(format!("commit context file: {e}")));
    }
    Ok(())
}

/// Read `currentContext` from `<home>/.docker/config.json`, defaulting to `"default"`.
async fn read_docker_current_context(home: &Path) -> String {
    let cfg_path = home.join(".docker").join("config.json");
    let Ok(raw) = fs::read_to_string(&cfg_path).await else {
        return "default".into();
    };
    let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) else {
        return "default".into();
    };
    val.get("currentContext")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("default")
        .to_string()
}

/// Read Docker CLI contexts from `<home>/.docker/contexts/meta/*/meta.json`.
async fn read_docker_cli_contexts(home: &Path) -> Vec<(String, PathBuf, Option<String>)> {
    let meta_root = home.join(".docker").join("contexts").join("meta");
    let mut out = Vec::new();
    let Ok(mut entries) = fs::read_dir(&meta_root).await else {
        return out;
    };
    while let Ok(Some(entry)) = entries.next_entry().await {
        let meta_file = entry.path().join("meta.json");
        let Ok(raw) = fs::read_to_string(&meta_file).await else {
            continue;
        };
        let Ok(val) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        let Some(name) = val
            .get("Name")
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        let Some(host) = val
            .get("Endpoints")
            .and_then(|e| e.get("docker"))
            .and_then(|d| d.get("Host"))
            .and_then(|h| h.as_str())
        else {
            continue;
        };
        if let Some(sock) = host.strip_prefix("unix://") {
            if let Ok(path) = normalize_socket_path(sock) {
                let desc = val
                    .get("Metadata")
                    .and_then(|m| m.get("Description"))
                    .and_then(|d| d.as_str())
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty());
                out.push((name.to_string(), path, desc));
            }
        }
    }
    out
}

/// Quick Unix socket connectability probe (does not send an HTTP request).
async fn is_socket_reachable(path: &Path) -> bool {
    if fs::symlink_metadata(path).await.is_err() {
        return false;
    }
    matches!(
        timeout(REACHABILITY_TIMEOUT, UnixStream::connect(path)).await,
        Ok(Ok(_))
    )
}

/// Resolve symlink target for better engine classification (e.g. `/var/run/docker.sock`).
async fn classify_with_symlink(name: &str, path: &Path) -> EngineKind {
    let direct = classify_engine(name, path, None);
    if direct != EngineKind::Unknown {
        return direct;
    }
    if let Ok(target) = fs::read_link(path).await {
        return classify_engine(name, &target, None);
    }
    EngineKind::Unknown
}

/// Assemble the ordered list of candidate `(name, socket_path, description)` entries.
async fn collect_context_candidates(
    resolved_home: Option<&Path>,
    socket_override: Option<&Path>,
    docker_host_env: Option<&str>,
    saved: Option<&SavedContextSelection>,
) -> Vec<(String, PathBuf, Option<String>)> {
    let mut candidates: Vec<(String, PathBuf, Option<String>)> = Vec::new();
    let mut seen_names: BTreeSet<String> = BTreeSet::new();
    let mut seen_paths: BTreeSet<PathBuf> = BTreeSet::new();

    if let Some(ov) = socket_override {
        seen_names.insert("override".into());
        seen_paths.insert(ov.to_path_buf());
        candidates.push((
            "override".into(),
            ov.to_path_buf(),
            Some("Configured socket override".into()),
        ));
    }

    if let Some(env_host) = docker_host_env {
        if let Some(raw) = env_host.strip_prefix("unix://") {
            if let Ok(p) = normalize_socket_path(raw) {
                if seen_paths.insert(p.clone()) {
                    seen_names.insert("docker-host".into());
                    candidates.push((
                        "docker-host".into(),
                        p,
                        Some("DOCKER_HOST environment variable".into()),
                    ));
                }
            }
        }
    }

    if let Some(h) = resolved_home {
        for (name, path, desc) in read_docker_cli_contexts(h).await {
            if seen_names.insert(name.clone()) {
                seen_paths.insert(path.clone());
                candidates.push((name, path, desc));
            }
        }

        let well_known = [
            (
                "kiri",
                h.join(".kiri/run/docker.sock"),
                "Kiri embedded container runtime",
            ),
            (
                "otto",
                h.join(".otto/run/docker.sock"),
                "Otto container engine",
            ),
            (
                "desktop-linux",
                h.join(".docker/run/docker.sock"),
                "Docker Desktop",
            ),
            ("orbstack", h.join(".orbstack/run/docker.sock"), "OrbStack"),
            ("colima", h.join(".colima/default/docker.sock"), "Colima"),
            (
                "rancher-desktop",
                h.join(".rd/docker.sock"),
                "Rancher Desktop",
            ),
        ];
        for (name, path, desc) in well_known {
            let exists = fs::symlink_metadata(&path).await.is_ok();
            if exists && !seen_names.contains(name) && !seen_paths.contains(&path) {
                seen_names.insert(name.into());
                seen_paths.insert(path.clone());
                candidates.push((name.into(), path, Some(desc.into())));
            }
        }
    }

    let default_sock = PathBuf::from("/var/run/docker.sock");
    if !seen_names.contains("default") && !seen_paths.contains(&default_sock) {
        seen_names.insert("default".into());
        seen_paths.insert(default_sock.clone());
        candidates.push((
            "default".into(),
            default_sock,
            Some("Default Unix socket".into()),
        ));
    }

    // Apply persisted user selection: if it specifies a custom socket path not
    // yet in `seen_paths`, update any same-named candidate's socket path (so a
    // colliding name never silently activates the discovered socket instead) or
    // append it as a new candidate.
    if let Some(sel) = saved {
        if let Ok(p) = normalize_socket_path(&sel.socket_path) {
            if !seen_paths.contains(&p) {
                if let Some(existing) = candidates.iter_mut().find(|(n, _, _)| n == &sel.name) {
                    existing.1 = p;
                    existing.2 = Some("Saved Otto selection".into());
                } else {
                    candidates.push((sel.name.clone(), p, Some("Saved Otto selection".into())));
                }
            }
        }
    }

    candidates
}

/// Probe reachability and engine kind for all candidates concurrently.
async fn probe_candidates(
    candidates: Vec<(String, PathBuf, Option<String>)>,
    docker_default: &str,
) -> Vec<DockerContextInfo> {
    let futures = candidates.into_iter().map(|(name, path, desc)| async move {
        let (reachable, engine_kind) = tokio::join!(
            is_socket_reachable(&path),
            classify_with_symlink(&name, &path),
        );
        let socket_str = path.to_string_lossy().into_owned();
        let is_docker_default = name == docker_default;
        DockerContextInfo {
            name,
            host: format!("unix://{socket_str}"),
            socket_path: socket_str,
            engine_kind,
            description: desc,
            is_active: false,
            is_docker_default,
            reachable,
        }
    });
    join_all(futures).await
}

/// Choose active context index: explicit override > saved Otto selection (matching
/// `socket_path` before `name`) > DOCKER_HOST > Docker CLI currentContext (if
/// reachable) > first reachable > Docker CLI currentContext > first candidate.
fn select_active_index(
    infos: &[DockerContextInfo],
    socket_override: Option<&Path>,
    saved: Option<&SavedContextSelection>,
) -> Option<usize> {
    if socket_override.is_some() {
        return (!infos.is_empty()).then_some(0);
    }
    if let Some(sel) = saved {
        if let Some(idx) = infos
            .iter()
            .position(|c| c.socket_path == sel.socket_path)
            .or_else(|| infos.iter().position(|c| c.name == sel.name))
        {
            return Some(idx);
        }
    }
    infos
        .iter()
        .position(|c| c.name == "docker-host" && c.reachable)
        .or_else(|| {
            infos
                .iter()
                .position(|c| c.is_docker_default && c.reachable)
        })
        .or_else(|| infos.iter().position(|c| c.reachable))
        .or_else(|| infos.iter().position(|c| c.is_docker_default))
        .or_else(|| (!infos.is_empty()).then_some(0))
}

/// Discover all Docker contexts and well-known Unix sockets, plus the active one.
pub async fn discover_contexts(
    home: Option<&Path>,
    data_dir: &Path,
    socket_override: Option<&Path>,
    docker_host_env: Option<&str>,
) -> ContainersContextsResp {
    let resolved_home = home.map(PathBuf::from).or_else(dirs::home_dir);
    let (docker_default, saved) = tokio::join!(
        async {
            match resolved_home.as_deref() {
                Some(h) => read_docker_current_context(h).await,
                None => "default".into(),
            }
        },
        load_saved_selection(data_dir),
    );

    let candidates = collect_context_candidates(
        resolved_home.as_deref(),
        socket_override,
        docker_host_env,
        saved.as_ref(),
    )
    .await;
    let mut infos = probe_candidates(candidates, &docker_default).await;
    let active_idx = select_active_index(&infos, socket_override, saved.as_ref());

    let (active_context, active_socket) = match active_idx {
        Some(idx) => {
            infos[idx].is_active = true;
            (infos[idx].name.clone(), infos[idx].socket_path.clone())
        }
        None => ("default".into(), "/var/run/docker.sock".into()),
    };

    ContainersContextsResp {
        active_context,
        active_socket,
        contexts: infos,
    }
}

/// Extract the active `ResolvedEndpoint` from a discovery response.
pub(crate) fn active_endpoint_from_resp(resp: &ContainersContextsResp) -> ResolvedEndpoint {
    if let Some(active) = resp.contexts.iter().find(|c| c.is_active) {
        ResolvedEndpoint {
            context_name: active.name.clone(),
            socket_path: PathBuf::from(&active.socket_path),
            engine_kind: active.engine_kind,
        }
    } else {
        ResolvedEndpoint {
            context_name: resp.active_context.clone(),
            socket_path: PathBuf::from(&resp.active_socket),
            engine_kind: EngineKind::Unknown,
        }
    }
}

/// Resolve the active Docker socket endpoint for the current request.
pub async fn resolve_active_endpoint(
    home: Option<&Path>,
    data_dir: &Path,
    socket_override: Option<&Path>,
) -> ResolvedEndpoint {
    let env_host = std::env::var("DOCKER_HOST").ok();
    let resp = discover_contexts(
        home,
        data_dir,
        socket_override,
        if socket_override.is_some() {
            None
        } else {
            env_host.as_deref()
        },
    )
    .await;
    active_endpoint_from_resp(&resp)
}

/// Locate the `docker` CLI binary asynchronously without spawning `which`.
pub async fn locate_docker_cli(home: Option<&Path>) -> Option<PathBuf> {
    let mut dirs_to_check: Vec<PathBuf> = Vec::new();
    if let Some(h) = home.map(PathBuf::from).or_else(dirs::home_dir) {
        dirs_to_check.push(h.join(".otto/bin"));
    }
    if let Ok(path_var) = std::env::var("PATH") {
        dirs_to_check.extend(std::env::split_paths(&path_var));
    }
    for extra in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/Applications/Docker.app/Contents/Resources/bin",
        "/usr/bin",
    ] {
        dirs_to_check.push(PathBuf::from(extra));
    }

    for dir in dirs_to_check {
        let candidate = dir.join("docker");
        if let Ok(meta) = fs::metadata(&candidate).await {
            if meta.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
