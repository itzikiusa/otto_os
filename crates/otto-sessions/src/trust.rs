//! Pre-trust workspace folders for agent CLIs so new sessions don't stall on
//! interactive "do you trust this folder?" prompts.
//!
//! Otto workspaces are explicitly chosen by the user, so the daemon marks
//! them trusted in each CLI's own config before spawning:
//! - claude: `~/.claude.json` → `projects.<path>.hasTrustDialogAccepted`
//! - codex:  `~/.codex/config.toml` → `[projects."<path>"] trust_level`
//! - grok:   `~/.grok/trusted_folders.toml` → folder-trust grant (MCP/hooks/LSP)
//!
//! Unknown providers are left alone (best effort, never fatal). The runtime
//! [`crate::prompt_guard`] still auto-accepts residual prompts for custom CLIs.
//!
//! These files belong to the CLIs, and their running processes rewrite them
//! all the time (`~/.claude.json` carries OAuth/account state, MCP settings
//! and per-project history). So a write here must be rare and careful:
//! - only when the trust grant is actually missing (claude drops
//!   `hasCompletedProjectOnboarding` on save, and requiring it made EVERY
//!   claude spawn rewrite the 200 KB file — ~130 times a day);
//! - serialized across Otto's own parallel spawns ([`config_lock`]);
//! - atomically (unique temp name in the same dir + rename, original
//!   permissions kept), and only if the file did not change since it was read
//!   — otherwise re-read, re-merge and retry, so a CLI's concurrent save is
//!   never silently reverted ([`atomic_replace`]).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::SystemTime;

/// Read-merge-write attempts before giving up on a file that keeps changing
/// under us (a later spawn simply tries again).
const WRITE_ATTEMPTS: usize = 3;

/// Serializes Otto's read-modify-writes of the CLIs' config files. Parallel
/// spawns (a review/workflow fan-out) used to interleave on one fixed temp
/// name and could rename a torn file over the user's config.
static CONFIG_LOCK: Mutex<()> = Mutex::new(());

fn config_lock() -> MutexGuard<'static, ()> {
    CONFIG_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// Cheap change detector for a config file: its size and mtime.
type Fingerprint = Option<(u64, SystemTime)>;

fn fingerprint(path: &Path) -> Fingerprint {
    let meta = std::fs::metadata(path).ok()?;
    Some((meta.len(), meta.modified().ok()?))
}

/// Replace `path` with `contents` atomically: write a uniquely named temp
/// file beside it (same dir ⇒ same volume ⇒ atomic rename), copy the
/// original's permissions (`~/.codex/config.toml` is 0600), and rename it over
/// `path` — but only while `path` still matches `read_fp`, the fingerprint
/// taken before the caller read it. `Ok(false)` = the file changed meanwhile
/// (a CLI saved it); nothing was written and the caller should re-read.
fn atomic_replace(path: &Path, contents: &[u8], read_fp: Fingerprint) -> Result<bool, String> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let tmp = path.with_file_name(format!(
        ".{name}.otto-tmp.{}.{}",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&tmp, contents).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    if let Ok(meta) = std::fs::metadata(path) {
        let _ = std::fs::set_permissions(&tmp, meta.permissions());
    }
    if fingerprint(path) != read_fp {
        let _ = std::fs::remove_file(&tmp);
        return Ok(false);
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(format!("rename onto {}: {e}", path.display()));
    }
    Ok(true)
}

/// Read `path` (missing ⇒ empty), let `merge` produce the new contents (`None`
/// ⇒ already trusted, nothing to write) and [`atomic_replace`] it, re-reading
/// and re-merging when a CLI saved the file in between. Holds
/// [`config_lock`] throughout. Returns whether the file was written.
fn update_config(
    path: &Path,
    merge: impl Fn(&str) -> Result<Option<String>, String>,
) -> Result<bool, String> {
    let _guard = config_lock();
    for _ in 0..WRITE_ATTEMPTS {
        let read_fp = fingerprint(path);
        let current = match std::fs::read_to_string(path) {
            Ok(s) => s,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(e) => return Err(format!("read {}: {e}", path.display())),
        };
        let Some(next) = merge(current.as_str())? else {
            return Ok(false);
        };
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
        }
        if atomic_replace(path, next.as_bytes(), read_fp)? {
            return Ok(true);
        }
    }
    Err(format!(
        "{} kept changing while Otto tried to update it; left alone (the next spawn retries)",
        path.display()
    ))
}

/// Mark `cwd` as trusted for `provider`. Failures are logged, never fatal.
pub fn ensure_trusted(provider: &str, cwd: &str) {
    let result = match provider {
        "claude" => trust_claude(cwd),
        "codex" => trust_codex(cwd),
        "grok" => trust_grok(cwd),
        _ => Ok(()),
    };
    if let Err(e) = result {
        tracing::warn!(provider, cwd, "could not pre-trust folder: {e}");
    }
}

/// Every path spelling an agent CLI might compare `$PWD` against: the path
/// itself, its symlink-resolved form, and the `/private` prefix macOS adds
/// for `/var` and `/tmp`. Trusting only the literal path can still leave a
/// session blocked when the CLI sees a resolved variant.
fn path_variants(cwd: &str) -> Vec<String> {
    let mut out = vec![cwd.to_string()];
    let mut add = |p: String| {
        if !p.is_empty() && !out.contains(&p) {
            out.push(p);
        }
    };
    if let Ok(real) = std::fs::canonicalize(cwd) {
        add(real.to_string_lossy().into_owned());
    }
    if let Some(rest) = cwd.strip_prefix("/var/") {
        add(format!("/private/var/{rest}"));
    }
    if let Some(rest) = cwd.strip_prefix("/tmp/") {
        add(format!("/private/tmp/{rest}"));
    }
    out
}

fn home() -> Result<PathBuf, String> {
    std::env::var("HOME")
        .map(PathBuf::from)
        .map_err(|_| "HOME not set".to_string())
}

fn trust_claude(cwd: &str) -> Result<(), String> {
    let path = home()?.join(".claude.json");
    let variants = path_variants(cwd);
    let written = update_config(&path, |current| {
        claude_with_trust(current, &variants).map_err(|e| format!("{}: {e}", path.display()))
    })?;
    if written {
        tracing::info!(cwd, "pre-trusted folder for claude");
    }
    Ok(())
}

/// `~/.claude.json` contents with every variant's trust grant set, or `None`
/// when all are already trusted. Only a missing `hasTrustDialogAccepted`
/// triggers a write: claude drops `hasCompletedProjectOnboarding` whenever it
/// saves, so treating that key as required rewrote the file on every spawn.
/// (It is still set alongside a NEW grant, which is when it matters.)
fn claude_with_trust(current: &str, variants: &[String]) -> Result<Option<String>, String> {
    let mut root: serde_json::Value = if current.trim().is_empty() {
        serde_json::json!({})
    } else {
        serde_json::from_str(current).map_err(|e| format!("parse: {e}"))?
    };
    let obj = root
        .as_object_mut()
        .ok_or_else(|| "not a JSON object".to_string())?;
    let projects = obj
        .entry("projects")
        .or_insert_with(|| serde_json::json!({}))
        .as_object_mut()
        .ok_or_else(|| "projects key is not an object".to_string())?;

    let mut changed = false;
    for variant in variants {
        let entry = projects
            .entry(variant.clone())
            .or_insert_with(|| serde_json::json!({}));
        let entry_obj = entry
            .as_object_mut()
            .ok_or_else(|| "project entry is not an object".to_string())?;
        if entry_obj.get("hasTrustDialogAccepted") != Some(&serde_json::Value::Bool(true)) {
            entry_obj.insert(
                "hasTrustDialogAccepted".to_string(),
                serde_json::Value::Bool(true),
            );
            entry_obj.insert(
                "hasCompletedProjectOnboarding".to_string(),
                serde_json::Value::Bool(true),
            );
            changed = true;
        }
    }
    if !changed {
        return Ok(None);
    }
    serde_json::to_string(&root)
        .map(Some)
        .map_err(|e| format!("serialize claude config: {e}"))
}

fn trust_codex(cwd: &str) -> Result<(), String> {
    let path = home()?.join(".codex").join("config.toml");
    let variants = path_variants(cwd);
    if update_config(&path, |current| Ok(codex_with_trust(current, &variants)))? {
        tracing::info!(cwd, "pre-trusted folder for codex");
    }
    Ok(())
}

/// `~/.codex/config.toml` with a trusted `[projects."<path>"]` table appended
/// for every variant that lacks one, or `None` when all are present. Trusts
/// every path spelling codex might compare (literal + resolved + /private
/// variants) — same rationale as claude.
fn codex_with_trust(current: &str, variants: &[String]) -> Option<String> {
    let mut next = current.to_string();
    let mut changed = false;
    for variant in variants {
        let header = format!("[projects.\"{variant}\"]");
        if next.contains(&header) {
            continue;
        }
        if !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        next.push_str(&format!("\n{header}\ntrust_level = \"trusted\"\n"));
        changed = true;
    }
    changed.then_some(next)
}

/// Pre-trust `cwd` in Grok's unified folder-trust store so sessions don't stall
/// on "Trust the authors of this folder…?" when the workspace has repo-local
/// `.mcp.json` / hooks / LSP config (Otto often writes `.mcp.json` itself).
///
/// Format mirrors what `/hooks-trust` / `--trust` persist: a table-per-path
/// under `folders."<path>"` with `trusted = true` and an integer `decided_at`.
/// Grok refuses over-broad roots (home / `/`); we skip those too.
fn trust_grok(cwd: &str) -> Result<(), String> {
    // Never record home or filesystem root — Grok rejects them and they would
    // also be dangerously broad.
    let home_str = home()?.to_string_lossy().into_owned();
    let path = home()?.join(".grok").join("trusted_folders.toml");
    let variants = path_variants(cwd);
    let now = chrono::Utc::now().timestamp();
    if update_config(&path, |current| {
        Ok(grok_with_trust(current, &variants, &home_str, now))
    })? {
        tracing::info!(cwd, "pre-trusted folder for grok");
    }
    Ok(())
}

/// Grok's trust store with a grant for every eligible variant, or `None` when
/// nothing changes.
fn grok_with_trust(current: &str, variants: &[String], home_str: &str, now: i64) -> Option<String> {
    // Grok's own store writes `decided_at` as an INTEGER unix timestamp (see a
    // manually-approved entry). We used to write an ISO-8601 STRING, which grok
    // does not accept — the "pre-trusted" folder still stalled on the trust
    // prompt. Write the integer form, and repair any string entries an earlier
    // Otto left behind (they poison grok's parse of the file).
    let mut changed = false;
    let mut next = if current.contains("decided_at = \"") {
        changed = true;
        let repaired: Vec<String> = current
            .lines()
            .map(|l| {
                let t = l.trim();
                match t
                    .strip_prefix("decided_at = \"")
                    .and_then(|r| r.strip_suffix('"'))
                {
                    Some(v) => {
                        let ts = chrono::DateTime::parse_from_rfc3339(v)
                            .map(|d| d.timestamp())
                            .unwrap_or(now);
                        format!("decided_at = {ts}")
                    }
                    None => l.to_string(),
                }
            })
            .collect();
        let mut s = repaired.join("\n");
        if !s.is_empty() && !s.ends_with('\n') {
            s.push('\n');
        }
        s
    } else {
        current.to_string()
    };
    for variant in variants {
        if variant.is_empty() || variant == "/" || variant == home_str {
            continue;
        }
        let header = format!("[folders.\"{variant}\"]");
        if next.contains(&header) {
            continue;
        }
        if !next.is_empty() && !next.ends_with('\n') {
            next.push('\n');
        }
        next.push_str(&format!("\n{header}\ntrusted = true\ndecided_at = {now}\n"));
        changed = true;
    }
    changed.then_some(next)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn v(paths: &[&str]) -> Vec<String> {
        paths.iter().map(|p| p.to_string()).collect()
    }

    /// The ~130-rewrites-a-day bug: claude drops `hasCompletedProjectOnboarding`
    /// on save; a trusted folder without it must NOT trigger another write.
    #[test]
    fn claude_trusted_folder_without_onboarding_key_is_left_alone() {
        let current = r#"{"projects":{"/w":{"hasTrustDialogAccepted":true,"lastCost":1}}}"#;
        assert_eq!(claude_with_trust(current, &v(&["/w"])).unwrap(), None);
    }

    #[test]
    fn claude_new_folder_gets_trust_and_keeps_other_state() {
        let current = r#"{"oauthAccount":{"x":1},"projects":{"/other":{"a":2}}}"#;
        let next = claude_with_trust(current, &v(&["/w"])).unwrap().expect("write");
        let root: serde_json::Value = serde_json::from_str(&next).unwrap();
        assert_eq!(root["projects"]["/w"]["hasTrustDialogAccepted"], true);
        assert_eq!(root["projects"]["/w"]["hasCompletedProjectOnboarding"], true);
        assert_eq!(root["projects"]["/other"]["a"], 2);
        assert_eq!(root["oauthAccount"]["x"], 1);
        // …and the merged result is itself a fixed point.
        assert_eq!(claude_with_trust(&next, &v(&["/w"])).unwrap(), None);
    }

    #[test]
    fn claude_corrupt_config_is_an_error_not_an_overwrite() {
        assert!(claude_with_trust("{\"projects\":", &v(&["/w"])).is_err());
    }

    #[test]
    fn codex_trust_is_idempotent() {
        let next = codex_with_trust("model = \"x\"", &v(&["/w"])).expect("write");
        assert!(next.starts_with("model = \"x\"\n"));
        assert!(next.contains("[projects.\"/w\"]\ntrust_level = \"trusted\"\n"));
        assert_eq!(codex_with_trust(&next, &v(&["/w"])), None);
    }

    #[test]
    fn atomic_replace_keeps_permissions_and_refuses_a_changed_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "old").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();

        let fp = fingerprint(&path);
        assert!(atomic_replace(&path, b"new", fp).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "new");
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "atomic replace widened the file's permissions");

        // A concurrent save after our read (size changes) must win.
        let stale = fingerprint(&path);
        std::fs::write(&path, "saved by the CLI meanwhile").unwrap();
        assert!(!atomic_replace(&path, b"ours", stale).unwrap());
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "saved by the CLI meanwhile"
        );
        // No temp files left behind either way.
        let leftovers = std::fs::read_dir(dir.path())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .map(|e| e.file_name().to_string_lossy().contains("otto-tmp"))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(leftovers, 0);
    }

    #[test]
    fn update_config_writes_once_then_is_a_no_op() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("config.toml");
        let variants = v(&["/w"]);
        assert!(update_config(&path, |c| Ok(codex_with_trust(c, &variants))).unwrap());
        let first = std::fs::read_to_string(&path).unwrap();
        assert!(!update_config(&path, |c| Ok(codex_with_trust(c, &variants))).unwrap());
        assert_eq!(std::fs::read_to_string(&path).unwrap(), first);
    }
}
