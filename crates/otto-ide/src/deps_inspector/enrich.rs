//! Registry enrichment, OSV batch vulnerability lookup, and semver helpers for `deps_inspector`.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use futures_util::stream::{self, StreamExt};
use regex::Regex;
use reqwest::StatusCode;
use serde_json::{json, Value};

use crate::java_workspace::resolve_m2_repo;
use crate::types::{IdeDepItem, IdeDepVulnerability};

const MAX_REMOTE_LOOKUPS: usize = 30;
const REGISTRY_CONCURRENCY: usize = 8;
const ENRICH_BATCH_DEADLINE: Duration = Duration::from_secs(3);
const PER_REQUEST_TIMEOUT: Duration = Duration::from_millis(2000);
const REGISTRY_CACHE_TTL: Duration = Duration::from_secs(15 * 60);
const REGISTRY_NOT_FOUND_TTL: Duration = Duration::from_secs(60);
const MAX_REGISTRY_CACHE_ENTRIES: usize = 2_000;
const MAX_OSV_VULN_CACHE_ENTRIES: usize = 512;
const MAX_REDIRECT_HOPS: usize = 4;

#[derive(Clone, Debug)]
struct CachedRegistryMeta {
    versions: Vec<String>,
    deprecated: Option<String>,
    single_latest_only: bool,
    inserted_at: Instant,
    expires_at: Instant,
}

static REGISTRY_CACHE: OnceLock<RwLock<HashMap<(String, String), CachedRegistryMeta>>> =
    OnceLock::new();
static OSV_VULN_CACHE: OnceLock<RwLock<HashMap<String, (Instant, Value)>>> = OnceLock::new();

fn registry_cache() -> &'static RwLock<HashMap<(String, String), CachedRegistryMeta>> {
    REGISTRY_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn osv_vuln_cache() -> &'static RwLock<HashMap<String, (Instant, Value)>> {
    OSV_VULN_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
}

fn shared_http_client() -> Option<&'static reqwest::Client> {
    static CLIENT: OnceLock<Option<reqwest::Client>> = OnceLock::new();
    CLIENT
        .get_or_init(|| {
            otto_netguard::guarded_client_builder()
                .https_only(true)
                .redirect(reqwest::redirect::Policy::none())
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
    let mut current = reqwest::Url::parse(url).ok()?;
    for _ in 0..=MAX_REDIRECT_HOPS {
        if current.scheme() != "https" {
            return None;
        }
        otto_netguard::check_url(current.as_str()).await.ok()?;
        let resp = client
            .get(current.clone())
            .timeout(per_req_timeout)
            .send()
            .await
            .ok()?;
        let status = resp.status();
        if matches!(
            status,
            StatusCode::MOVED_PERMANENTLY
                | StatusCode::FOUND
                | StatusCode::SEE_OTHER
                | StatusCode::TEMPORARY_REDIRECT
                | StatusCode::PERMANENT_REDIRECT
        ) {
            let loc = resp
                .headers()
                .get(reqwest::header::LOCATION)?
                .to_str()
                .ok()?;
            current = current.join(loc).ok()?;
            continue;
        }
        let body = resp.text().await.ok()?;
        return Some((status, body));
    }
    None
}

async fn guarded_post_json(url: &str, payload: &Value, per_req_timeout: Duration) -> Option<Value> {
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

fn cache_get(ecosystem: &str, name: &str, now: Instant) -> Option<CachedRegistryMeta> {
    let key = (ecosystem.to_string(), name.to_string());
    let guard = registry_cache().read().unwrap_or_else(|e| e.into_inner());
    let entry = guard.get(&key)?;
    (now < entry.expires_at).then(|| entry.clone())
}

fn cache_put(ecosystem: &str, name: &str, entry: CachedRegistryMeta, now: Instant) {
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
                    if !valid_simple_pkg_name(&name) || name.contains('.') {
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

pub(super) async fn enrich_pypi_updates(items: &mut [IdeDepItem]) {
    if items.is_empty() || otto_core::e2e::isolated() {
        return;
    }
    let uncached = collect_uncached_names("pypi", items);
    if !uncached.is_empty() {
        let _ = tokio::time::timeout(
            ENRICH_BATCH_DEADLINE,
            stream::iter(uncached)
                .map(|name| async move {
                    if !valid_simple_pkg_name(&name) {
                        return;
                    }
                    let url = format!("https://pypi.org/pypi/{name}/json");
                    let Some((status, body)) =
                        guarded_get_status_and_text(&url, PER_REQUEST_TIMEOUT).await
                    else {
                        return;
                    };
                    let now = Instant::now();
                    if status == StatusCode::NOT_FOUND {
                        cache_put(
                            "pypi",
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
                        .get("info")
                        .and_then(|i| i.get("version"))
                        .and_then(Value::as_str)
                        .map(|s| vec![s.to_string()])
                        .unwrap_or_default();
                    cache_put(
                        "pypi",
                        &name,
                        CachedRegistryMeta {
                            versions,
                            deprecated: None,
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
        apply_all_cached("pypi", items);
    }
}

pub(super) async fn enrich_maven_updates(
    items: &mut [IdeDepItem],
    env_map: &BTreeMap<String, String>,
) {
    if items.is_empty() {
        return;
    }
    let m2 = resolve_m2_repo(env_map);
    let ver_re = Regex::new(r"<version>\s*([^<\s]+)\s*</version>").unwrap();
    let now = Instant::now();
    let mut remote_needed: Vec<String> = Vec::new();
    let mut seen_remote = HashSet::new();

    for item in items.iter_mut() {
        let Some((group, artifact)) = item.name.split_once(':') else {
            continue;
        };
        if !valid_maven_coord_part(group) || !valid_maven_coord_part(artifact) {
            continue;
        }
        let mut versions: Vec<String> = Vec::new();
        if let Some(m2_dir) = &m2 {
            let art_dir = m2_dir.join(group.replace('.', "/")).join(artifact);
            if let Ok(mut rd) = tokio::fs::read_dir(&art_dir).await {
                while let Ok(Some(entry)) = rd.next_entry().await {
                    let epath = entry.path();
                    if epath.is_dir() {
                        if let Some(v) = entry.file_name().to_str() {
                            versions.push(v.to_string());
                        }
                    } else if entry
                        .file_name()
                        .to_string_lossy()
                        .starts_with("maven-metadata")
                    {
                        if let Ok(xml) = tokio::fs::read_to_string(&epath).await {
                            for cap in ver_re.captures_iter(&xml) {
                                versions.push(cap[1].to_string());
                            }
                        }
                    }
                }
            }
        }
        if !versions.is_empty() {
            apply_best_newer_versions(item, &versions);
            continue;
        }
        if let Some(meta) = cache_get("maven", &item.name, now) {
            apply_cached_meta(item, &meta);
        } else if !otto_core::e2e::isolated()
            && remote_needed.len() < MAX_REMOTE_LOOKUPS
            && seen_remote.insert(item.name.clone())
        {
            remote_needed.push(item.name.clone());
        }
    }

    if !remote_needed.is_empty() {
        let ver_re_ref = &ver_re;
        let _ = tokio::time::timeout(
            ENRICH_BATCH_DEADLINE,
            stream::iter(remote_needed)
                .map(|coord| async move {
                    let Some((group, artifact)) = coord.split_once(':') else {
                        return;
                    };
                    let url = format!(
                        "https://repo1.maven.org/maven2/{}/{artifact}/maven-metadata.xml",
                        group.replace('.', "/")
                    );
                    let Some((status, xml)) =
                        guarded_get_status_and_text(&url, Duration::from_millis(2200)).await
                    else {
                        return;
                    };
                    let fetched_now = Instant::now();
                    if status == StatusCode::NOT_FOUND {
                        cache_put(
                            "maven",
                            &coord,
                            CachedRegistryMeta {
                                versions: Vec::new(),
                                deprecated: None,
                                single_latest_only: false,
                                inserted_at: fetched_now,
                                expires_at: fetched_now + REGISTRY_NOT_FOUND_TTL,
                            },
                            fetched_now,
                        );
                        return;
                    }
                    if !status.is_success() {
                        return;
                    }
                    let versions: Vec<String> = ver_re_ref
                        .captures_iter(&xml)
                        .map(|cap| cap[1].to_string())
                        .collect();
                    cache_put(
                        "maven",
                        &coord,
                        CachedRegistryMeta {
                            versions,
                            deprecated: None,
                            single_latest_only: false,
                            inserted_at: fetched_now,
                            expires_at: fetched_now + REGISTRY_CACHE_TTL,
                        },
                        fetched_now,
                    );
                })
                .buffer_unordered(REGISTRY_CONCURRENCY)
                .collect::<Vec<()>>(),
        )
        .await;

        let post_now = Instant::now();
        for item in items.iter_mut() {
            if item.latest_version.is_none() {
                if let Some(meta) = cache_get("maven", &item.name, post_now) {
                    apply_cached_meta(item, &meta);
                }
            }
        }
    }
}

fn valid_osv_id(vid: &str) -> bool {
    !vid.is_empty()
        && vid.len() <= 128
        && !vid.contains("..")
        && vid
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':'))
}

async fn resolve_osv_vuln_detail(v: &Value) -> Option<(String, Value)> {
    let vid = v.get("id").and_then(Value::as_str)?;
    if !valid_osv_id(vid) {
        return None;
    }
    if v.get("summary").is_some() {
        return Some((vid.to_string(), v.clone()));
    }
    let now = Instant::now();
    if let Some(cached) = osv_vuln_cache()
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .get(vid)
        .and_then(|(exp, val)| (now < *exp).then(|| val.clone()))
    {
        return Some((vid.to_string(), cached));
    }
    let url = format!("https://api.osv.dev/v1/vulns/{vid}");
    let detail = match guarded_get_status_and_text(&url, Duration::from_millis(1500)).await {
        Some((status, body)) if status.is_success() => {
            serde_json::from_str::<Value>(&body).unwrap_or_else(|_| v.clone())
        }
        _ => v.clone(),
    };
    {
        let mut cache = osv_vuln_cache().write().unwrap_or_else(|e| e.into_inner());
        let ins_now = Instant::now();
        if cache.len() >= MAX_OSV_VULN_CACHE_ENTRIES {
            cache.retain(|_, (exp, _)| ins_now < *exp);
            if cache.len() >= MAX_OSV_VULN_CACHE_ENTRIES {
                if let Some(k) = cache.keys().next().cloned() {
                    cache.remove(&k);
                }
            }
        }
        cache.insert(
            vid.to_string(),
            (ins_now + REGISTRY_CACHE_TTL, detail.clone()),
        );
    }
    Some((vid.to_string(), detail))
}

pub(super) async fn enrich_osv_vulnerabilities(osv_ecosystem: &str, items: &mut [IdeDepItem]) {
    let mut queries = Vec::new();
    let mut mapped_indices = Vec::new();
    for (idx, item) in items.iter().enumerate() {
        if item.current_version.is_empty() || item.current_version.starts_with("${") {
            continue;
        }
        let ver = if osv_ecosystem == "Go" {
            item.current_version.trim_start_matches('v').to_string()
        } else {
            item.current_version.clone()
        };
        queries.push(json!({
            "package": { "name": item.name, "ecosystem": osv_ecosystem },
            "version": ver,
        }));
        mapped_indices.push(idx);
    }
    if queries.is_empty() {
        return;
    }
    let Some(batch_val) = guarded_post_json(
        "https://api.osv.dev/v1/querybatch",
        &json!({ "queries": queries }),
        Duration::from_secs(3),
    )
    .await
    else {
        return;
    };
    let Some(results) = batch_val.get("results").and_then(Value::as_array) else {
        return;
    };

    let mut pending: Vec<(usize, Value)> = Vec::new();
    for (q_idx, res) in results.iter().enumerate() {
        let Some(&item_idx) = mapped_indices.get(q_idx) else {
            continue;
        };
        let Some(vulns) = res.get("vulns").and_then(Value::as_array) else {
            continue;
        };
        for v in vulns.iter().take(4) {
            pending.push((item_idx, v.clone()));
        }
    }
    if pending.is_empty() {
        return;
    }

    let resolved: Vec<(usize, Option<(String, Value)>)> = tokio::time::timeout(
        Duration::from_secs(3),
        stream::iter(pending)
            .map(|(item_idx, v)| async move { (item_idx, resolve_osv_vuln_detail(&v).await) })
            .buffer_unordered(REGISTRY_CONCURRENCY)
            .collect(),
    )
    .await
    .unwrap_or_default();

    for (item_idx, detail_opt) in resolved {
        let Some((vid, detail)) = detail_opt else {
            continue;
        };
        let cve = detail
            .get("aliases")
            .and_then(Value::as_array)
            .and_then(|arr| {
                arr.iter()
                    .filter_map(Value::as_str)
                    .find(|a| a.starts_with("CVE-"))
            })
            .map(str::to_string)
            .or_else(|| vid.starts_with("CVE-").then(|| vid.to_string()));
        let severity = detail
            .get("database_specific")
            .and_then(|d| d.get("severity"))
            .and_then(Value::as_str)
            .unwrap_or("HIGH")
            .to_string();
        let summary = detail
            .get("summary")
            .and_then(Value::as_str)
            .unwrap_or("Known security vulnerability reported in OSV database")
            .to_string();
        let fixed_version = extract_osv_fixed_version(&detail, osv_ecosystem == "Go");
        items[item_idx].vulnerabilities.push(IdeDepVulnerability {
            id: vid,
            cve,
            severity,
            summary,
            fixed_version,
        });
    }
}

fn extract_osv_fixed_version(detail: &Value, prefix_v: bool) -> Option<String> {
    let affected = detail.get("affected")?.as_array()?;
    for aff in affected {
        if let Some(ranges) = aff.get("ranges").and_then(Value::as_array) {
            for r in ranges {
                if let Some(events) = r.get("events").and_then(Value::as_array) {
                    for ev in events {
                        if let Some(f) = ev.get("fixed").and_then(Value::as_str) {
                            return Some(if prefix_v && !f.starts_with('v') {
                                format!("v{f}")
                            } else {
                                f.to_string()
                            });
                        }
                    }
                }
            }
        }
    }
    None
}

pub(crate) fn apply_best_newer_versions(item: &mut IdeDepItem, candidates: &[String]) {
    let cur = &item.current_version;
    if cur.is_empty() {
        return;
    }
    let mut best_all: Option<&String> = None;
    let mut best_patch: Option<&String> = None;
    for cand in candidates {
        if cand.contains('-') && !cur.contains('-') {
            continue;
        }
        if compare_semver(cand, cur).is_gt() {
            if best_all.is_none_or(|b| compare_semver(cand, b).is_gt()) {
                best_all = Some(cand);
            }
            if same_major_minor(cur, cand)
                && best_patch.is_none_or(|b| compare_semver(cand, b).is_gt())
            {
                best_patch = Some(cand);
            }
        }
    }
    if let Some(b) = best_all {
        item.latest_version = Some(b.clone());
    }
    if let Some(p) = best_patch {
        item.latest_patch = Some(p.clone());
    }
}

pub(crate) fn parse_semver_tuple(raw: &str) -> (u64, u64, u64) {
    let clean = raw.trim().trim_start_matches('v');
    let core = clean.split(['-', '+']).next().unwrap_or(clean);
    let mut nums = core.split('.').map(|s| s.parse::<u64>().unwrap_or(0));
    (
        nums.next().unwrap_or(0),
        nums.next().unwrap_or(0),
        nums.next().unwrap_or(0),
    )
}

pub(crate) fn compare_semver(a: &str, b: &str) -> std::cmp::Ordering {
    parse_semver_tuple(a).cmp(&parse_semver_tuple(b))
}

pub(crate) fn same_major_minor(a: &str, b: &str) -> bool {
    let (maj_a, min_a, _) = parse_semver_tuple(a);
    let (maj_b, min_b, _) = parse_semver_tuple(b);
    maj_a == maj_b && min_a == min_b
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn netguard_blocks_loopback_private_metadata_and_plain_http_fetches() {
        for blocked in [
            "https://127.0.0.1:7700/maven-metadata.xml",
            "https://localhost:8443/latest",
            "https://169.254.169.254/latest/meta-data",
            "https://10.0.0.5/internal/pkg",
            "http://registry.npmjs.org/vite/latest",
        ] {
            assert!(
                guarded_get_status_and_text(blocked, Duration::from_millis(200))
                    .await
                    .is_none(),
                "expected netguard to block {blocked}"
            );
        }
    }

    #[test]
    fn registry_cache_hits_and_evicts_expired_entries() {
        let now = Instant::now();
        cache_put(
            "npm",
            "__test_cached_pkg__",
            CachedRegistryMeta {
                versions: vec!["2.4.1".into()],
                deprecated: Some("use replacement".into()),
                single_latest_only: true,
                inserted_at: now,
                expires_at: now + Duration::from_secs(60),
            },
            now,
        );
        let hit = cache_get("npm", "__test_cached_pkg__", now + Duration::from_secs(5))
            .expect("fresh cache entry");
        assert_eq!(hit.versions, vec!["2.4.1"]);
        assert!(
            cache_get("npm", "__test_cached_pkg__", now + Duration::from_secs(61)).is_none(),
            "expired cache entry must not be returned"
        );
    }
}
