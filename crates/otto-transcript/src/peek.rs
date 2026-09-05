//! Cheap transcript peek for the History index (design §4.6): read the head
//! 64 KB + tail 16 KB only, fold each end, and lift the fields the index
//! stores. Never reads a whole file — the index walks ~3 GB of transcripts.

use std::path::Path;

use crate::fold::FoldOpts;
use crate::model::Provider;
use crate::records::read_head_tail;

pub const PEEK_HEAD: u64 = 64 * 1024;
pub const PEEK_TAIL: u64 = 16 * 1024;

#[derive(Debug, Clone, Default)]
pub struct Peek {
    pub provider_session_id: Option<String>,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub first_prompt: Option<String>,
    pub started_at: Option<String>,
    pub last_active_at: Option<String>,
    /// Turn count — exact only when the whole file fit in the head (`Some`),
    /// `None` when the middle was skipped.
    pub turns: Option<u64>,
}

pub fn peek(provider: Provider, path: &Path) -> std::io::Result<Peek> {
    let (head, tail) = read_head_tail(path, PEEK_HEAD, PEEK_TAIL)?;
    let whole = tail.is_empty();
    let h = crate::fold(provider, &head, FoldOpts::default());
    let t = if whole {
        None
    } else {
        Some(crate::fold(provider, &tail, FoldOpts::default()))
    };
    let title = t.as_ref().and_then(|t| t.title.clone()).or_else(|| h.title.clone());
    let last_active_at = t
        .as_ref()
        .and_then(|t| t.last_ts.clone())
        .or_else(|| h.last_ts.clone());
    Ok(Peek {
        provider_session_id: h.session_id.clone().or_else(|| session_id_from_name(provider, path)),
        cwd: h.cwd.clone().or_else(|| t.as_ref().and_then(|t| t.cwd.clone())),
        title,
        first_prompt: h.first_prompt.clone(),
        started_at: h.first_ts.clone(),
        last_active_at,
        turns: whole.then_some(h.stats.turns),
    })
}

/// Claude files are `<sid>.jsonl`; Codex rollouts end in `-<uuid>.jsonl`
/// (the uuid is the tail after the `rollout-<ISO-ts>-` prefix).
pub fn session_id_from_name(provider: Provider, path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?;
    match provider {
        Provider::Claude => Some(stem.to_string()),
        Provider::Codex => {
            // rollout-2026-06-18T08-53-25-<uuid>: the uuid is the last 36 chars.
            let s = stem.strip_prefix("rollout-").unwrap_or(stem);
            if s.len() > 36 {
                Some(s[s.len() - 36..].to_string())
            } else {
                Some(s.to_string())
            }
        }
        Provider::Agy => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn codex_uuid_is_the_filename_tail() {
        let p = Path::new("/x/rollout-2026-06-18T08-53-25-019ed94a-994a-7010-b01f-9b840c5b7068.jsonl");
        assert_eq!(
            session_id_from_name(Provider::Codex, p).as_deref(),
            Some("019ed94a-994a-7010-b01f-9b840c5b7068")
        );
        assert_eq!(session_id_from_name(Provider::Claude, Path::new("/y/abc.jsonl")).as_deref(), Some("abc"));
    }

    #[test]
    fn peek_reads_a_small_file_whole() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("s1.jsonl");
        std::fs::write(&p, "{\"type\":\"user\",\"uuid\":\"u\",\"timestamp\":\"2026-01-01T00:00:00Z\",\"sessionId\":\"s1\",\"cwd\":\"/repo\",\"message\":{\"role\":\"user\",\"content\":\"hello there\"}}\n{\"type\":\"ai-title\",\"aiTitle\":\"Hello\"}\n").unwrap();
        let pk = peek(Provider::Claude, &p).unwrap();
        assert_eq!(pk.provider_session_id.as_deref(), Some("s1"));
        assert_eq!(pk.cwd.as_deref(), Some("/repo"));
        assert_eq!(pk.title.as_deref(), Some("Hello"));
        assert_eq!(pk.first_prompt.as_deref(), Some("hello there"));
        assert_eq!(pk.turns, Some(1));
    }
}
