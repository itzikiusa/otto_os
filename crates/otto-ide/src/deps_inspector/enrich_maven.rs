//! PyPI and Maven Central / `~/.m2` version-update enrichment for [`super`].

use std::collections::{BTreeMap, HashSet};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use futures_util::stream::{self, StreamExt};
use regex::Regex;
use reqwest::StatusCode;
use serde_json::Value;

use crate::java_workspace::resolve_m2_repo;
use crate::types::IdeDepItem;

use super::{
    apply_all_cached, apply_best_newer_versions, apply_cached_meta, cache_get, cache_put,
    collect_uncached_names, guarded_get_status_and_text, valid_maven_coord_part,
    valid_simple_pkg_name, CachedRegistryMeta, ENRICH_BATCH_DEADLINE, MAX_REMOTE_LOOKUPS,
    PER_REQUEST_TIMEOUT, REGISTRY_CACHE_TTL, REGISTRY_CONCURRENCY, REGISTRY_NOT_FOUND_TTL,
};

pub(in crate::deps_inspector) async fn enrich_pypi_updates(items: &mut [IdeDepItem]) {
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

pub(in crate::deps_inspector) async fn enrich_maven_updates(
    items: &mut [IdeDepItem],
    env_map: &BTreeMap<String, String>,
) {
    if items.is_empty() {
        return;
    }
    let m2 = resolve_m2_repo(env_map);
    static RE_MAVEN_VER: OnceLock<Regex> = OnceLock::new();
    let ver_re =
        RE_MAVEN_VER.get_or_init(|| Regex::new(r"<version>\s*([^<\s]+)\s*</version>").unwrap());
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
                    let versions: Vec<String> = ver_re
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
