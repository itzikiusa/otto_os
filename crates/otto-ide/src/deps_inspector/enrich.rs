//! Registry enrichment for `deps_inspector` (`npm`, `crates.io`, `PyPI`, `Maven`).

#[path = "enrich_maven.rs"]
mod enrich_maven;
#[path = "enrich_osv.rs"]
mod enrich_osv;

use std::collections::{HashMap, HashSet};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use futures_util::stream::{self, StreamExt};
use reqwest::StatusCode;
use serde_json::Value;

use crate::types::IdeDepItem;

pub(super) use self::enrich_maven::{enrich_maven_updates, enrich_pypi_updates};
pub(super) use self::enrich_osv::enrich_osv_vulnerabilities;
pub(crate) use self::enrich_osv::{
    apply_best_newer_versions, compare_semver, parse_semver_tuple, same_major_minor,
};

pub(super) const MAX_REMOTE_LOOKUPS: usize = 30;
pub(super) const REGISTRY_CONCURRENCY: usize = 8;
const ENRICH_BATCH_DEADLINE: Duration = Duration::from_secs(3);
const PER_REQUEST_TIMEOUT: Duration = Duration::from_millis(2000);
pub(super) const REGISTRY_CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const REGISTRY_NOT_FOUND_TTL: Duration = Duration::from_secs(60);
const MAX_REGISTRY_CACHE_ENTRIES: usize = 2_000;

#[derive(Clone, Debug)]
pub(super) struct CachedRegistryMeta {
    pub(super) versions: Vec<String>,
    pub(super) deprecated: Option<String>,
    pub(super) single_latest_only: bool,
    pub(super) inserted_at: Instant,
    pub(super) expires_at: Instant,
}

static REGISTRY_CACHE: OnceLock<RwLock<HashMap<(String, String), CachedRegistryMeta>>> =
    OnceLock::new();

fn registry_cache() -> &'static RwLock<HashMap<(String, String), CachedRegistryMeta>> {
    REGISTRY_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

pub(super) fn shared_http_client() -> Option<&'static reqwest::Client> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            otto_netguard::guarded_client_builder()
                .https_only(true)
                .timeout(Duration::from_millis(2200))
                .user_agent("otto-ide/0.2")
                .build()
                .ok()
        })
        .as_ref()
}

pub(crate) async fn guarded_get_status_and_text(
    url: &str,
    per_req_timeout: Duration,
) -> Option<(StatusCode, String)> {
    let client = shared_http_client()?;
    let current = reqwest::Url::parse(url).ok()?;
    if current.scheme() != "https" {
        return None;
    }
    otto_netguard::check_url(current.as_str()).await.ok()?;
    let resp = client
        .get(current)
        .timeout(per_req_timeout)
        .send()
        .await
        .ok()?;
    let status = resp.status();
    if status.is_redirection() || resp.url().scheme() != "https" {
        return None;
    }
    let body = resp.text().await.ok()?;
    Some((status, body))
}

pub(super) async fn guarded_post_json(
    url: &str,
    payload: &Value,
    per_req_timeout: Duration,
) -> Option<Value> {
    let client = shared_http_client()?;
    let parsed = reqwest::Url::parse(url).ok()?;
    if parsed.scheme() != "https" {
        return None;
    }
    otto_netguard::check_url(parsed.as_str()).await.ok()?;
    let resp = client
        .post(parsed)
        .timeout(per_req_timeout)
        .json(payload)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    resp.json::<Value>().await.ok()
}

pub(super) fn cache_get(ecosystem: &str, name: &str, now: Instant) -> Option<CachedRegistryMeta> {
    let key = (ecosystem.to_string(), name.to_string());
    let guard = registry_cache().read().unwrap_or_else(|e| e.into_inner());
    let entry = guard.get(&key)?;
    (now < entry.expires_at).then(|| entry.clone())
}

pub(super) fn cache_put(ecosystem: &str, name: &str, entry: CachedRegistryMeta, now: Instant) {
    let mut map = registry_cache().write().unwrap_or_else(|e| e.into_inner());
    let key = (ecosystem.to_string(), name.to_string());
    if !map.contains_key(&key) && map.len() >= MAX_REGISTRY_CACHE_ENTRIES {
        map.retain(|_, v| now < v.expires_at);
        if map.len() >= MAX_REGISTRY_CACHE_ENTRIES {
            if let Some(oldest_key) = map
                .iter()
                .min_by_key(|(_, v)| v.inserted_at)
                .map(|(k, _)| k.clone())
            {
                map.remove(&oldest_key);
            }
        }
    }
    map.insert(key, entry);
}

fn apply_cached_meta(item: &mut IdeDepItem, meta: &CachedRegistryMeta) {
    if meta.single_latest_only {
        if let Some(lv) = meta.versions.first() {
            if compare_semver(lv, &item.current_version).is_gt() {
                item.latest_version = Some(lv.clone());
                if same_major_minor(&item.current_version, lv) {
                    item.latest_patch = Some(lv.clone());
                }
            }
        }
    } else {
        apply_best_newer_versions(item, &meta.versions);
    }
    if let Some(dep) = &meta.deprecated {
        if !dep.is_empty() {
            item.deprecated = Some(dep.clone());
        }
    }
}

fn valid_npm_name(name: &str) -> bool {
    if name.is_empty()
        || name.len() > 214
        || name.contains("..")
        || name.contains('\\')
        || name
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '?' | '#'))
    {
        return false;
    }
    if let Some(rest) = name.strip_prefix('@') {
        let Some((scope, pkg)) = rest.split_once('/') else {
            return false;
        };
        !scope.is_empty() && !pkg.is_empty() && !pkg.contains('/')
    } else {
        !name.contains('/')
    }
}

fn valid_simple_pkg_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 128
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn valid_maven_coord_part(part: &str) -> bool {
    !part.is_empty()
        && part.len() <= 160
        && !part.starts_with('.')
        && !part.contains("..")
        && part
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
}

fn collect_uncached_names(ecosystem: &str, items: &mut [IdeDepItem]) -> Vec<String> {
    let now = Instant::now();
    let mut uncached = Vec::new();
    let mut seen = HashSet::new();
    for item in items.iter_mut() {
        if let Some(meta) = cache_get(ecosystem, &item.name, now) {
            apply_cached_meta(item, &meta);
        } else if uncached.len() < MAX_REMOTE_LOOKUPS && seen.insert(item.name.clone()) {
            uncached.push(item.name.clone());
        }
    }
    uncached
}

fn apply_all_cached(ecosystem: &str, items: &mut [IdeDepItem]) {
    let now = Instant::now();
    for item in items.iter_mut() {
        if let Some(meta) = cache_get(ecosystem, &item.name, now) {
            apply_cached_meta(item, &meta);
        }
    }
}

pub(super) async fn enrich_npm_updates(items: &mut [IdeDepItem]) {
    if items.is_empty() || otto_core::e2e::isolated() {
        return;
    }
    let uncached = collect_uncached_names("npm", items);
    if !uncached.is_empty() {
        let _ = tokio::time::timeout(
            ENRICH_BATCH_DEADLINE,
            stream::iter(uncached)
                .map(|name| async move {
                    if !valid_npm_name(&name) {
                        return;
                    }
                    let url = format!("https://registry.npmjs.org/{name}/latest");
                    let Some((status, body)) =
                        guarded_get_status_and_text(&url, PER_REQUEST_TIMEOUT).await
                    else {
                        return;
                    };
                    let now = Instant::now();
                    if status == StatusCode::NOT_FOUND {
                        cache_put(
                            "npm",
                            &name,
                            CachedRegistryMeta {
                                versions: Vec::new(),
                                deprecated: None,
                                single_latest_only: true,
                                inserted_at: now,
                                expires_at: now + REGISTRY_NOT_FOUND_TTL,
                            },
                            now,
                        );
                        return;
                    }
                    if !status.is_success() {
                        return;
                    }
                    let Ok(val) = serde_json::from_str::<Value>(&body) else {
                        return;
                    };
                    let versions = val
                        .get("version")
                        .and_then(Value::as_str)
                        .map(|s| vec![s.to_string()])
                        .unwrap_or_default();
                    let deprecated = val
                        .get("deprecated")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        .map(str::to_string);
                    cache_put(
                        "npm",
                        &name,
                        CachedRegistryMeta {
                            versions,
                            deprecated,
                            single_latest_only: true,
                            inserted_at: now,
                            expires_at: now + REGISTRY_CACHE_TTL,
                        },
                        now,
                    );
                })
                .buffer_unordered(REGISTRY_CONCURRENCY)
                .collect::<Vec<()>>(),
        )
        .await;
        apply_all_cached("npm", items);
    }
}

pub(super) async fn enrich_crates_updates(items: &mut [IdeDepItem]) {
    if items.is_empty() || otto_core::e2e::isolated() {
        return;
    }
    let uncached = collect_uncached_names("cargo", items);
    if !uncached.is_empty() {
        let _ = tokio::time::timeout(
            ENRICH_BATCH_DEADLINE,
            stream::iter(uncached)
                .map(|name| async move {
                    if !name.is_ascii() || !valid_simple_pkg_name(&name) || name.contains('.') {
                        return;
                    }
                    let lower = name.to_ascii_lowercase();
                    let prefix = match lower.len() {
                        1 => "1".to_string(),
                        2 => "2".to_string(),
                        3 => format!("3/{}", &lower[..1]),
                        _ => format!("{}/{}", &lower[..2], &lower[2..4]),
                    };
                    let url = format!("https://index.crates.io/{prefix}/{lower}");
                    let Some((status, body)) =
                        guarded_get_status_and_text(&url, PER_REQUEST_TIMEOUT).await
                    else {
                        return;
                    };
                    let now = Instant::now();
                    if status == StatusCode::NOT_FOUND {
                        cache_put(
                            "cargo",
                            &name,
                            CachedRegistryMeta {
                                versions: Vec::new(),
                                deprecated: None,
                                single_latest_only: false,
                                inserted_at: now,
                                expires_at: now + REGISTRY_NOT_FOUND_TTL,
                            },
                            now,
                        );
                        return;
                    }
                    if !status.is_success() {
                        return;
                    }
                    let mut versions = Vec::new();
                    for line in body.lines() {
                        if let Ok(v) = serde_json::from_str::<Value>(line) {
                            if !v.get("yanked").and_then(Value::as_bool).unwrap_or(false) {
                                if let Some(vers) = v.get("vers").and_then(Value::as_str) {
                                    versions.push(vers.to_string());
                                }
                            }
                        }
                    }
                    cache_put(
                        "cargo",
                        &name,
                        CachedRegistryMeta {
                            versions,
                            deprecated: None,
                            single_latest_only: false,
                            inserted_at: now,
                            expires_at: now + REGISTRY_CACHE_TTL,
                        },
                        now,
                    );
                })
                .buffer_unordered(REGISTRY_CONCURRENCY)
                .collect::<Vec<()>>(),
        )
        .await;
        apply_all_cached("cargo", items);
    }
}
