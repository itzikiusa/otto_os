//! OSV batch vulnerability lookup, semver comparison helpers, and unit tests
//! for [`super`] (`deps_inspector::enrich`).

use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};
use std::time::{Duration, Instant};

use futures_util::stream::{self, StreamExt};
use serde_json::{json, Value};

use crate::types::{IdeDepItem, IdeDepVulnerability};

use super::{
    guarded_get_status_and_text, guarded_post_json, REGISTRY_CACHE_TTL, REGISTRY_CONCURRENCY,
};

const MAX_OSV_VULN_CACHE_ENTRIES: usize = 512;

static OSV_VULN_CACHE: OnceLock<RwLock<HashMap<String, (Instant, Value)>>> = OnceLock::new();

fn osv_vuln_cache() -> &'static RwLock<HashMap<String, (Instant, Value)>> {
    OSV_VULN_CACHE.get_or_init(|| RwLock::new(HashMap::new()))
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

pub(in crate::deps_inspector) async fn enrich_osv_vulnerabilities(
    osv_ecosystem: &str,
    items: &mut [IdeDepItem],
) {
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
    use super::super::{cache_get, cache_put, CachedRegistryMeta};
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
