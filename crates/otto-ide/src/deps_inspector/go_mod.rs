//! Go `go.mod` parser and fast, cached `go list -mod=readonly -m -u` enrichment.

use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, UNIX_EPOCH};

use serde_json::Value;

use super::enrich::same_major_minor;
use crate::types::IdeDepItem;

const GO_MOD_UPDATES_TTL: Duration = Duration::from_secs(300);
const GO_MOD_UPDATES_MAX_ENTRIES: usize = 64;
const GO_LIST_TIMEOUT: Duration = Duration::from_millis(2500);

type FileStamp = (u64, u128);
type GoUpdateMap = HashMap<String, (Option<String>, Option<String>)>;

#[derive(Clone)]
struct CachedGoUpdates {
    stamp: (FileStamp, FileStamp),
    stored_at: Instant,
    updates: GoUpdateMap,
}

fn go_updates_cache() -> &'static Mutex<HashMap<PathBuf, CachedGoUpdates>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, CachedGoUpdates>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

#[allow(clippy::disallowed_methods)]
fn file_stamp(path: &Path) -> FileStamp {
    let Ok(meta) = fs::metadata(path) else {
        return (0, 0);
    };
    let mtime = meta
        .modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    (meta.len(), mtime)
}

fn module_matches_goprivate(module: &str, patterns_csv: &str) -> bool {
    for raw_pat in patterns_csv
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
    {
        if let Some(prefix) = raw_pat.strip_suffix("/*") {
            if module == prefix || module.starts_with(&format!("{prefix}/")) {
                return true;
            }
        } else if let Some(prefix) = raw_pat.strip_suffix('*') {
            if module.starts_with(prefix) {
                return true;
            }
        } else if module == raw_pat || module.starts_with(&format!("{raw_pat}/")) {
            return true;
        }
    }
    false
}

pub fn parse_go_mod(content: &str) -> Vec<IdeDepItem> {
    let mut items = Vec::new();
    let mut in_require = false;
    for (idx, raw_line) in content.lines().enumerate() {
        let line_no = (idx + 1) as u32;
        let code_trimmed = raw_line
            .split_once("//")
            .map_or(raw_line, |(c, _)| c)
            .trim();
        if code_trimmed.is_empty() {
            continue;
        }
        if let Some(rest) = code_trimmed.strip_prefix("require") {
            if rest.is_empty() || rest.starts_with(char::is_whitespace) || rest.starts_with('(') {
                let after = rest.trim();
                if after.starts_with('(') {
                    if after.contains(')') {
                        if let Some(item) = parse_go_require_spec(raw_line, line_no, true) {
                            items.push(item);
                        }
                    } else {
                        in_require = true;
                    }
                    continue;
                }
                if let Some(item) = parse_go_require_spec(raw_line, line_no, true) {
                    items.push(item);
                }
                continue;
            }
        }
        if in_require {
            if code_trimmed.starts_with(')') {
                in_require = false;
                continue;
            }
            if let Some(item) = parse_go_require_spec(raw_line, line_no, false) {
                items.push(item);
            }
            if code_trimmed.ends_with(')') {
                in_require = false;
            }
        }
    }
    items
}

fn parse_go_require_spec(raw_line: &str, line_no: u32, strip_kw: bool) -> Option<IdeDepItem> {
    let (code_part, comment_part) = match raw_line.split_once("//") {
        Some((c, cm)) => (c, Some(cm.trim())),
        None => (raw_line, None),
    };
    let indirect = comment_part.is_some_and(|c| c.contains("indirect"));
    let inline_dep = comment_part.and_then(|c| {
        c.strip_prefix("Deprecated:")
            .or_else(|| c.strip_prefix("deprecated:"))
            .map(|s| s.trim().to_string())
    });

    let (work, search_from) = if strip_kw {
        let pos = code_part.find("require")? + "require".len();
        (&code_part[pos..], pos)
    } else {
        (code_part, 0)
    };
    let trimmed_work = work.trim();
    let inner_work = trimmed_work
        .strip_prefix('(')
        .map(|s| s.strip_suffix(')').unwrap_or(s))
        .unwrap_or_else(|| trimmed_work.strip_suffix(')').unwrap_or(trimmed_work));
    let mut parts = inner_work.split_whitespace();
    let mod_name = parts.next()?;
    let ver = parts.next()?;
    if mod_name.starts_with('(') || mod_name.starts_with(')') {
        return None;
    }
    let name_byte = search_from + code_part[search_from..].find(mod_name)?;
    let ver_search_start = name_byte + mod_name.len();
    let ver_byte = ver_search_start + code_part[ver_search_start..].find(ver)?;
    let name_col = (code_part[..name_byte].chars().count() + 1) as u32;
    let name_end_col = name_col + mod_name.chars().count() as u32;
    let version_col = (code_part[..ver_byte].chars().count() + 1) as u32;
    let version_end_col = version_col + ver.chars().count() as u32;

    Some(IdeDepItem {
        name: mod_name.to_string(),
        current_version: ver.to_string(),
        latest_version: None,
        latest_patch: None,
        indirect,
        deprecated: inline_dep,
        line: line_no,
        name_col,
        name_end_col,
        version_line: line_no,
        version_col,
        version_end_col,
        block_start_line: line_no,
        block_end_line: line_no,
        property_name: None,
        managed_in: None,
        vulnerabilities: Vec::new(),
    })
}

fn apply_go_updates(items: &mut [IdeDepItem], updates: &GoUpdateMap) {
    for item in items.iter_mut() {
        if let Some((latest, dep)) = updates.get(&item.name) {
            if let Some(lv) = latest {
                if lv != &item.current_version {
                    item.latest_version = Some(lv.clone());
                    if same_major_minor(&item.current_version, lv) {
                        item.latest_patch = Some(lv.clone());
                    }
                }
            }
            if dep.is_some() {
                item.deprecated = dep.clone();
            }
        }
    }
}

#[allow(clippy::disallowed_methods)]
pub(super) async fn enrich_go_mod_updates(
    manifest_path: &Path,
    items: &mut [IdeDepItem],
    env_map: &BTreeMap<String, String>,
) {
    if items.is_empty() || otto_core::e2e::isolated() {
        return;
    }
    let Some(dir) = manifest_path.parent().filter(|d| d.is_dir()) else {
        return;
    };
    let canon_manifest =
        fs::canonicalize(manifest_path).unwrap_or_else(|_| manifest_path.to_path_buf());
    let stamp = (file_stamp(&canon_manifest), file_stamp(&dir.join("go.sum")));
    let mut stale_fallback: Option<GoUpdateMap> = None;
    {
        let cache = go_updates_cache().lock().unwrap_or_else(|e| e.into_inner());
        if let Some(entry) = cache.get(&canon_manifest) {
            if entry.stamp == stamp && entry.stored_at.elapsed() < GO_MOD_UPDATES_TTL {
                apply_go_updates(items, &entry.updates);
                return;
            }
            stale_fallback = Some(entry.updates.clone());
        }
    }

    let goprivate = env_map
        .get("GOPRIVATE")
        .cloned()
        .or_else(|| std::env::var("GOPRIVATE").ok())
        .unwrap_or_default();
    let non_private_mods: Vec<String> = if goprivate.trim().is_empty() {
        Vec::new()
    } else {
        items
            .iter()
            .filter(|i| !module_matches_goprivate(&i.name, &goprivate))
            .map(|i| i.name.clone())
            .collect()
    };
    if !goprivate.trim().is_empty() && non_private_mods.is_empty() {
        return;
    }

    let mut cmd = tokio::process::Command::new("go");
    cmd.args(["list", "-mod=readonly", "-m", "-u", "-json"]);
    if !goprivate.trim().is_empty() && non_private_mods.len() < items.len() {
        cmd.args(&non_private_mods);
    } else {
        cmd.arg("all");
    }
    cmd.current_dir(dir)
        .env_clear()
        .envs(crate::environments::shell_env::collect_safe_non_root_base_env())
        .envs(env_map)
        .env("GIT_TERMINAL_PROMPT", "0")
        .kill_on_drop(true);
    for go_key in [
        "GOPRIVATE",
        "GONOSUMDB",
        "GONOPROXY",
        "GOPATH",
        "GOMODCACHE",
    ] {
        if !env_map.contains_key(go_key) {
            if let Ok(v) = std::env::var(go_key) {
                if !v.is_empty() {
                    cmd.env(go_key, v);
                }
            }
        }
    }

    let Ok(Ok(out)) = tokio::time::timeout(GO_LIST_TIMEOUT, cmd.output()).await else {
        if let Some(stale) = stale_fallback {
            apply_go_updates(items, &stale);
        }
        return;
    };
    if out.stdout.is_empty() {
        if let Some(stale) = stale_fallback {
            apply_go_updates(items, &stale);
        }
        return;
    }
    let stream = serde_json::Deserializer::from_slice(&out.stdout).into_iter::<Value>();
    let mut updates: GoUpdateMap = HashMap::new();
    for val in stream.flatten() {
        let Some(path) = val.get("Path").and_then(Value::as_str) else {
            continue;
        };
        let latest = val
            .get("Update")
            .and_then(|u| u.get("Version"))
            .and_then(Value::as_str)
            .map(str::to_string);
        let dep = val
            .get("Deprecated")
            .and_then(|s| s.as_str())
            .filter(|s| !s.is_empty())
            .map(str::to_string);
        updates.insert(path.to_string(), (latest, dep));
    }
    apply_go_updates(items, &updates);

    let mut cache = go_updates_cache().lock().unwrap_or_else(|e| e.into_inner());
    cache.retain(|_, v| v.stored_at.elapsed() < GO_MOD_UPDATES_TTL);
    if cache.len() >= GO_MOD_UPDATES_MAX_ENTRIES && !cache.contains_key(&canon_manifest) {
        if let Some(oldest_key) = cache
            .iter()
            .min_by_key(|(_, v)| v.stored_at)
            .map(|(k, _)| k.clone())
        {
            cache.remove(&oldest_key);
        }
    }
    cache.insert(
        canon_manifest,
        CachedGoUpdates {
            stamp,
            stored_at: Instant::now(),
            updates,
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goprivate_pattern_matching_handles_wildcards_and_subpaths() {
        let pat = "bitbucket.org/gamescale-rnd/*,github.com/acme/private";
        assert!(module_matches_goprivate(
            "bitbucket.org/gamescale-rnd/go_utilities",
            pat
        ));
        assert!(module_matches_goprivate(
            "github.com/acme/private/subpkg",
            pat
        ));
        assert!(!module_matches_goprivate(
            "github.com/gofiber/fiber/v2",
            pat
        ));
    }
}
