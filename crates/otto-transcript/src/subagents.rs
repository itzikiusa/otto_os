//! The subagent tree. Claude writes each spawned agent's transcript to
//! `<sid>/subagents/agent-<agent-id>.jsonl` next to `<sid>.jsonl`, plus an
//! `agent-<agent-id>.meta.json` sidecar
//! (`{agentType, description, toolUseId, parentAgentId?, spawnDepth, model?}`).
//! The directory is FLAT and includes depth-2/3 agents spawned by sibling
//! subagents (45% of them are unreachable from the parent's `Agent` results),
//! so the tree comes from the sidecars, never from tool results. Inside a
//! subagent file `sessionId` is the PARENT's id — `agentId` is the key.

use std::path::{Path, PathBuf};

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
        let Some(id) = name
            .strip_suffix(".meta.json")
            .map(|s| s.strip_prefix("agent-").unwrap_or(s))
        else {
            continue;
        };
        let Ok(raw) = std::fs::read_to_string(entry.path()) else {
            continue;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&raw) else {
            continue;
        };
        out.push(SubagentMeta {
            agent_id: string_of(&v, "agentId").unwrap_or_else(|| id.to_string()),
            parent_agent_id: string_of(&v, "parentAgentId"),
            depth: u64_of(&v, "spawnDepth").unwrap_or(1) as u32,
            agent_type: string_of(&v, "agentType").unwrap_or_else(|| "agent".into()),
            description: string_of(&v, "description").unwrap_or_default(),
            model: string_of(&v, "model"),
            tool_use_id: string_of(&v, "toolUseId"),
        });
    }
    out.sort_by(|a, b| a.depth.cmp(&b.depth).then_with(|| a.agent_id.cmp(&b.agent_id)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
