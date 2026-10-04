//! The subagent tree. Claude writes each spawned agent's transcript to
//! `<sid>/subagents/agent-<agent-id>.jsonl` next to `<sid>.jsonl`, plus an
//! `agent-<agent-id>.meta.json` sidecar
//! (`{agentType, description, toolUseId, parentAgentId?, spawnDepth, model?}`).
//! The directory is FLAT and includes depth-2/3 agents spawned by sibling
//! subagents (45% of them are unreachable from the parent's `Agent` results),
//! so the tree comes from the sidecars, never from tool results. Inside a
//! subagent file `sessionId` is the PARENT's id — `agentId` is the key.

use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

use crate::model::SubagentMeta;
use crate::util::{string_of, u64_of};

/// `<dir>/<stem>/subagents` for the transcript at `transcript_path`.
pub fn subagents_dir(transcript_path: &Path) -> Option<PathBuf> {
    let stem = transcript_path.file_stem()?.to_str()?;
    Some(transcript_path.parent()?.join(stem).join("subagents"))
}

/// The JSONL of one subagent (`?sub=<agent_id>`). `agent_id` is validated to a
/// plain `[A-Za-z0-9_-]` token so a client value can never leave the dir.
pub fn subagent_path(transcript_path: &Path, agent_id: &str) -> Option<PathBuf> {
    if agent_id.is_empty()
        || !agent_id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    {
        return None;
    }
    Some(subagents_dir(transcript_path)?.join(format!("agent-{agent_id}.jsonl")))
}

/// Sidecar metadata is independently bounded: many tiny files still consume
/// map/tree entries, and one unbounded description must not bypass fold limits.
pub const SUBAGENT_CHARGE_CAP: usize = 8 * 1024 * 1024;
const META_BYTES_CAP: usize = 1024 * 1024;
fn metadata_budget_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::OutOfMemory,
        "transcript resource limit: subagent metadata exceeds its budget",
    )
}

/// Check the source bytes/count before any sidecars are parsed. The factor
/// covers scanner cache + sorted tree + folder options and JSON field overhead.
pub fn subagent_charge(transcript_path: &Path) -> std::io::Result<usize> {
    let Some(dir) = subagents_dir(transcript_path) else {
        return Ok(0);
    };
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(e),
    };
    let mut charge = 0usize;
    for entry in entries {
        let entry = entry?;
        if entry.file_name().to_str().and_then(meta_id).is_none() {
            continue;
        }
        let bytes = entry.metadata()?.len();
        if bytes > META_BYTES_CAP as u64 {
            return Err(metadata_budget_error());
        }
        charge = charge
            .saturating_add(bytes as usize * 8)
            .saturating_add(2048);
        if charge > SUBAGENT_CHARGE_CAP {
            return Err(metadata_budget_error());
        }
    }
    Ok(charge)
}

/// The interactive reader checks sidecar limits before building its tree.
pub fn read_subagents_limited(transcript_path: &Path) -> std::io::Result<Vec<SubagentMeta>> {
    subagent_charge(transcript_path)?;
    Ok(read_subagents(transcript_path))
}

/// Read every `*.meta.json` sidecar. Missing dir → empty. Sorted by depth then
/// id so the tree renders deterministically.
pub fn read_subagents(transcript_path: &Path) -> Vec<SubagentMeta> {
    let Some(dir) = subagents_dir(transcript_path) else {
        return Vec::new();
    };
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(name) = name.to_str() else { continue };
        let Some(id) = meta_id(name) else {
            continue;
        };
        if let Some(m) = read_meta(&entry.path(), id) {
            out.push(m);
        }
    }
    sort_tree(&mut out);
    out
}

/// `agent-<id>.meta.json` → `<id>` (a bare `<id>.meta.json` keeps its stem).
fn meta_id(name: &str) -> Option<&str> {
    name.strip_suffix(".meta.json")
        .map(|s| s.strip_prefix("agent-").unwrap_or(s))
}

/// Read + parse one sidecar; `None` when unreadable or not (yet) valid JSON.
fn read_meta(path: &Path, id: &str) -> Option<SubagentMeta> {
    let mut raw = String::new();
    std::fs::File::open(path)
        .ok()?
        .take(META_BYTES_CAP as u64 + 1)
        .read_to_string(&mut raw)
        .ok()?;
    if raw.len() > META_BYTES_CAP {
        return None;
    }
    let v = serde_json::from_str::<serde_json::Value>(&raw).ok()?;
    Some(SubagentMeta {
        agent_id: string_of(&v, "agentId").unwrap_or_else(|| id.to_string()),
        parent_agent_id: string_of(&v, "parentAgentId"),
        depth: u64_of(&v, "spawnDepth").unwrap_or(1) as u32,
        agent_type: string_of(&v, "agentType").unwrap_or_else(|| "agent".into()),
        description: string_of(&v, "description").unwrap_or_default(),
        model: string_of(&v, "model"),
        tool_use_id: string_of(&v, "toolUseId"),
    })
}

fn sort_tree(out: &mut [SubagentMeta]) {
    out.sort_by(|a, b| {
        a.depth
            .cmp(&b.depth)
            .then_with(|| a.agent_id.cmp(&b.agent_id))
    });
}

/// Incremental [`read_subagents`] for pollers (the live tail, the workflow
/// oracle). A session with hundreds of sub-agents made every poll re-open and
/// re-parse hundreds of sidecars (47–71 ms measured). This keeps each parsed
/// sidecar keyed by `(len, mtime)` and skips the directory walk entirely while
/// the `subagents/` dir's own mtime is unchanged (a new sidecar is a new dir
/// entry). A sidecar that did not parse yet (caught mid-write) forces a walk
/// on the next refresh, and a stat-only walk runs at least every
/// [`SubagentScanner::RESTAT_EVERY`] so an in-place rewrite is still seen.
/// The tree it yields is exactly what `read_subagents` returns.
#[derive(Debug, Default, Clone)]
pub struct SubagentScanner {
    dir_mtime: Option<SystemTime>,
    /// file name → (len, mtime, parsed sidecar).
    files: HashMap<String, (u64, Option<SystemTime>, Option<SubagentMeta>)>,
    /// Some sidecar did not parse — walk again next time.
    incomplete: bool,
    last_walk: Option<Instant>,
    tree: Vec<SubagentMeta>,
}

impl SubagentScanner {
    /// Upper bound between two directory walks (stat-only for unchanged files).
    pub const RESTAT_EVERY: Duration = Duration::from_secs(10);

    pub fn new() -> Self {
        Self::default()
    }

    /// The tree as of the last [`refresh`](Self::refresh).
    pub fn tree(&self) -> &[SubagentMeta] {
        &self.tree
    }

    /// Whether a refresh would inspect sidecars. Lets bounded live callers
    /// reserve their metadata charge before the scanner allocates it.
    pub fn needs_refresh(&self, transcript_path: &Path) -> bool {
        let mtime = subagents_dir(transcript_path)
            .and_then(|dir| std::fs::metadata(dir).ok())
            .and_then(|m| m.modified().ok());
        self.last_walk
            .is_none_or(|t| t.elapsed() >= Self::RESTAT_EVERY)
            || self.incomplete
            || mtime.is_none()
            || mtime != self.dir_mtime
    }

    /// Bring the tree up to date for the transcript at `transcript_path`.
    /// Returns `true` when it changed. Blocking IO — call off the runtime.
    pub fn refresh(&mut self, transcript_path: &Path) -> bool {
        let Some(dir) = subagents_dir(transcript_path) else {
            return self.clear();
        };
        let dir_mtime = match std::fs::metadata(&dir) {
            Ok(m) => m.modified().ok(),
            Err(_) => return self.clear(),
        };
        let due = self
            .last_walk
            .is_none_or(|t| t.elapsed() >= Self::RESTAT_EVERY);
        if !due && !self.incomplete && dir_mtime.is_some() && dir_mtime == self.dir_mtime {
            return false;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            return self.clear();
        };
        self.dir_mtime = dir_mtime;
        self.last_walk = Some(Instant::now());
        self.incomplete = false;
        let mut files = HashMap::with_capacity(self.files.len());
        for entry in entries.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(id) = meta_id(name) else {
                continue;
            };
            let (len, mtime) = match entry.metadata() {
                Ok(m) => (m.len(), m.modified().ok()),
                Err(_) => (u64::MAX, None),
            };
            let parsed = match self.files.remove(name) {
                Some((l, t, Some(m))) if l == len && t == mtime && mtime.is_some() => Some(m),
                _ => read_meta(&entry.path(), id),
            };
            if parsed.is_none() {
                self.incomplete = true;
            }
            files.insert(name.to_string(), (len, mtime, parsed));
        }
        self.files = files;
        let mut tree: Vec<SubagentMeta> = self
            .files
            .values()
            .filter_map(|(_, _, m)| m.clone())
            .collect();
        sort_tree(&mut tree);
        let changed = tree != self.tree;
        self.tree = tree;
        changed
    }

    fn clear(&mut self) -> bool {
        let changed = !self.tree.is_empty();
        *self = Self {
            last_walk: Some(Instant::now()),
            ..Self::default()
        };
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn huge_sidecar_is_rejected_before_parsing() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("session.jsonl");
        let dir = subagents_dir(&path).unwrap();
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::File::create(dir.join("agent-large.meta.json"))
            .unwrap()
            .set_len(META_BYTES_CAP as u64 + 1)
            .unwrap();
        assert_eq!(
            read_subagents_limited(&path).unwrap_err().kind(),
            std::io::ErrorKind::OutOfMemory
        );
    }

    #[test]
    fn reads_sidecars_into_a_flat_tree() {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path().join("sid.jsonl");
        std::fs::write(&t, "").unwrap();
        let sub = subagents_dir(&t).unwrap();
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(
            sub.join("agent-a1.meta.json"),
            r#"{"agentType":"Explore","description":"Map UI","toolUseId":"toolu_1","spawnDepth":1}"#,
        )
        .unwrap();
        std::fs::write(
            sub.join("agent-b2.meta.json"),
            r#"{"agentType":"general-purpose","description":"child","toolUseId":"toolu_9","parentAgentId":"a1","spawnDepth":2,"model":"opus"}"#,
        )
        .unwrap();
        std::fs::write(sub.join("agent-a1.jsonl"), "").unwrap();
        let tree = read_subagents(&t);
        assert_eq!(tree.len(), 2);
        assert_eq!(tree[0].agent_id, "a1");
        assert_eq!(tree[0].tool_use_id.as_deref(), Some("toolu_1"));
        assert_eq!(tree[1].parent_agent_id.as_deref(), Some("a1"));
        assert_eq!(tree[1].depth, 2);
        assert_eq!(subagent_path(&t, "a1").unwrap(), sub.join("agent-a1.jsonl"));
        assert!(subagent_path(&t, "../x").is_none());
        assert!(read_subagents(&dir.path().join("none.jsonl")).is_empty());
    }

    #[test]
    fn scanner_matches_read_subagents_and_only_rereads_what_changed() {
        let dir = tempfile::tempdir().unwrap();
        let t = dir.path().join("sid.jsonl");
        std::fs::write(&t, "").unwrap();
        let mut sc = SubagentScanner::new();
        // No dir yet → empty, unchanged.
        assert!(!sc.refresh(&t));
        assert!(sc.tree().is_empty());
        let sub = subagents_dir(&t).unwrap();
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(
            sub.join("agent-a1.meta.json"),
            r#"{"agentType":"Explore","description":"one","toolUseId":"toolu_1","spawnDepth":1}"#,
        )
        .unwrap();
        assert!(sc.refresh(&t));
        assert_eq!(sc.tree(), read_subagents(&t).as_slice());
        // Unchanged dir → no walk, no change.
        assert!(!sc.refresh(&t));
        // A sidecar caught mid-write (empty) is skipped like read_subagents
        // does, and retried on the next refresh even with an unchanged dir.
        std::fs::write(sub.join("agent-b2.meta.json"), "").unwrap();
        sc.refresh(&t);
        assert_eq!(sc.tree(), read_subagents(&t).as_slice());
        assert_eq!(sc.tree().len(), 1);
        std::fs::write(
            sub.join("agent-b2.meta.json"),
            r#"{"agentType":"general-purpose","description":"two","toolUseId":"toolu_2","parentAgentId":"a1","spawnDepth":2}"#,
        )
        .unwrap();
        assert!(sc.refresh(&t), "the incomplete sidecar is re-read");
        assert_eq!(sc.tree().len(), 2);
        assert_eq!(sc.tree(), read_subagents(&t).as_slice());
        // Cached entries are reused: poison the cache for a1 and make sure an
        // unchanged (len, mtime) keeps the cached value on a forced walk.
        if let Some((_, _, Some(m))) = sc.files.get_mut("agent-a1.meta.json") {
            m.description = "cached".into();
        }
        sc.last_walk = None; // force a walk
        sc.refresh(&t);
        assert_eq!(
            sc.tree()[0].description,
            "cached",
            "unchanged file not re-read"
        );
        // Removing the dir empties the tree.
        std::fs::remove_dir_all(&sub).unwrap();
        assert!(sc.refresh(&t));
        assert!(sc.tree().is_empty());
    }
}
