//! Pre-trust workspace folders for agent CLIs so new sessions don't stall on
//! interactive "do you trust this folder?" prompts.
//!
//! Otto workspaces are explicitly chosen by the user, so the daemon marks
//! them trusted in each CLI's own config before spawning:
//! - claude: `~/.claude.json` → `projects.<path>.hasTrustDialogAccepted`
//! - codex:  `~/.codex/config.toml` → `[projects."<path>"] trust_level`
//!
//! Unknown providers are left alone (best effort, never fatal).

use std::path::PathBuf;

/// Mark `cwd` as trusted for `provider`. Failures are logged, never fatal.
pub fn ensure_trusted(provider: &str, cwd: &str) {
    let result = match provider {
        "claude" => trust_claude(cwd),
        "codex" => trust_codex(cwd),
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
    let mut root: serde_json::Value = match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).map_err(|e| format!("parse {}: {e}", path.display()))?,
        Err(_) => serde_json::json!({}),
    };

    let obj = root
        .as_object_mut()
        .ok_or_else(|| format!("{} is not a JSON object", path.display()))?;
    let projects = obj
        .entry("projects")
        .or_insert_with(|| serde_json::json!({}));
    let projects = projects
        .as_object_mut()
        .ok_or_else(|| "projects key is not an object".to_string())?;

    let mut changed = false;
    for variant in path_variants(cwd) {
        let entry = projects
            .entry(variant)
            .or_insert_with(|| serde_json::json!({}));
        let entry_obj = entry
            .as_object_mut()
            .ok_or_else(|| "project entry is not an object".to_string())?;
        if entry_obj.get("hasTrustDialogAccepted") != Some(&serde_json::Value::Bool(true)) {
            entry_obj.insert(
                "hasTrustDialogAccepted".to_string(),
                serde_json::Value::Bool(true),
            );
            changed = true;
        }
        if entry_obj.get("hasCompletedProjectOnboarding") != Some(&serde_json::Value::Bool(true)) {
            entry_obj.insert(
                "hasCompletedProjectOnboarding".to_string(),
                serde_json::Value::Bool(true),
            );
            changed = true;
        }
    }
    if !changed {
        return Ok(());
    }

    // Atomic-ish write: temp file in the same dir, then rename.
    let tmp = path.with_extension("json.otto-tmp");
    let serialized =
        serde_json::to_string(&root).map_err(|e| format!("serialize claude config: {e}"))?;
    std::fs::write(&tmp, serialized).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("rename onto {}: {e}", path.display()))?;
    tracing::info!(cwd, "pre-trusted folder for claude");
    Ok(())
}

fn trust_codex(cwd: &str) -> Result<(), String> {
    let dir = home()?.join(".codex");
    let path = dir.join("config.toml");
    let current = std::fs::read_to_string(&path).unwrap_or_default();

    let header = format!("[projects.\"{cwd}\"]");
    if current.contains(&header) {
        return Ok(());
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("create {}: {e}", dir.display()))?;
    let mut next = current;
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    next.push_str(&format!("\n{header}\ntrust_level = \"trusted\"\n"));
    std::fs::write(&path, next).map_err(|e| format!("write {}: {e}", path.display()))?;
    tracing::info!(cwd, "pre-trusted folder for codex");
    Ok(())
}
