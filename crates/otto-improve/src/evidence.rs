//! Bounded input snapshots and successful-analysis cursors. No transcript body
//! is persisted here; checkpoint hashes contain no recoverable conversation.
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

use chrono::{Duration, Utc};
use otto_core::{domain::Session, Result};
use otto_state::{ImprovementsRepo, SessionsRepo};
use otto_transcript::Provider;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::digest::{
    add_skill, append_recent, collect_skill_mentions, digest_for_provider, SessionDigest,
};

const MAX_READ: u64 = 2 * 1024 * 1024;
#[derive(Default, Debug, Clone, Serialize, Deserialize)]
pub(crate) struct Checkpoint {
    #[serde(default)]
    path: String,
    #[serde(default)]
    identity: String,
    #[serde(default)]
    modified: String,
    #[serde(default)]
    file_len: u64,
    #[serde(default)]
    offset: u64,
    #[serde(default)]
    anchor: String,
    #[serde(default)]
    trail_id: Option<String>,
    #[serde(default)]
    skills: Vec<String>,
}

pub(crate) struct EvidenceBatch {
    pub digest: Option<SessionDigest>,
    pub checkpoint: serde_json::Value,
}

fn anchor(file: &mut std::fs::File, offset: u64) -> std::io::Result<String> {
    let start = offset.saturating_sub(256);
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![0; (offset - start) as usize];
    file.read_exact(&mut bytes)?;
    Ok(format!("{:x}", Sha256::digest(bytes)))
}

/// Read at most 2 MiB and only complete lines. A truncated/replaced source
/// resets the cursor, verified by the preceding-byte anchor rather than size.
fn read_delta(path: &Path, previous: &Checkpoint) -> std::io::Result<(String, Checkpoint)> {
    let mut file = std::fs::File::open(path)?;
    let metadata = file.metadata()?;
    let len = metadata.len();
    let identity = format!("{}:{}", metadata.dev(), metadata.ino());
    let modified = format!("{}:{}", metadata.mtime(), metadata.mtime_nsec());
    let path_text = path.to_string_lossy().to_string();
    let same = previous.path == path_text
        && previous.identity == identity
        && (previous.file_len != len || previous.modified == modified)
        && previous.offset <= len
        && anchor(&mut file, previous.offset)? == previous.anchor;
    let offset = if same { previous.offset } else { 0 };
    let start = offset.max(len.saturating_sub(MAX_READ));
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = Vec::new();
    file.take(MAX_READ).read_to_end(&mut bytes)?;
    let skip = if start > offset {
        bytes
            .iter()
            .position(|b| *b == b'\n')
            .map(|i| i + 1)
            .unwrap_or(bytes.len())
    } else {
        0
    };
    let complete = bytes
        .iter()
        .rposition(|b| *b == b'\n')
        .map(|i| i + 1)
        .unwrap_or(0);
    let next_offset = if complete > 0 {
        start + complete as u64
    } else {
        offset
    };
    let mut file = std::fs::File::open(path)?;
    let next = Checkpoint {
        path: path_text,
        identity,
        modified,
        file_len: len,
        offset: next_offset,
        anchor: anchor(&mut file, next_offset)?,
        trail_id: previous.trail_id.clone(),
        skills: previous.skills.clone(),
    };
    let body = if complete > skip {
        String::from_utf8_lossy(&bytes[skip..complete]).into_owned()
    } else {
        String::new()
    };
    Ok((body, next))
}

fn resolve_path(session: &Session, persisted: Option<String>, data_dir: &Path) -> Option<PathBuf> {
    if let Some(path) = persisted.map(PathBuf::from).filter(|p| p.is_file()) {
        return Some(path);
    }
    let provider = session
        .meta
        .get("nested_provider")
        .and_then(|v| v.as_str())
        .unwrap_or(&session.provider);
    let cwd = session
        .meta
        .get("nested_cwd")
        .and_then(|v| v.as_str())
        .unwrap_or(&session.cwd);
    if let Some(account) = session.meta.get("account_id").and_then(|v| v.as_str()) {
        let root = otto_sessions::accounts::account_home(
            &data_dir.join("provider-accounts"),
            &account.to_owned(),
        )
        .ok()?;
        return otto_sessions::transcript_path_in_roots(
            &root.join("projects"),
            &root.join("sessions"),
            provider,
            cwd,
            session.provider_session_id.as_deref(),
        )
        .ok();
    }
    // Match the conversation view's explicit/isolated roots. A throwaway daemon
    // must never discover a real user's default transcript tree.
    if let Some(raw) = std::env::var_os("OTTO_TRANSCRIPT_ROOTS") {
        let raw = raw.to_string_lossy();
        let parts: Vec<_> = raw
            .split(':')
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .collect();
        if parts.len() >= 2 {
            return otto_sessions::transcript_path_in_roots(
                Path::new(parts[0]),
                Path::new(parts[1]),
                provider,
                cwd,
                session.provider_session_id.as_deref(),
            )
            .ok();
        }
    }
    if std::env::var("OTTO_E2E").as_deref() == Ok("1") {
        let root = data_dir.join("e2e-transcripts");
        return otto_sessions::transcript_path_in_roots(
            &root.join("claude"),
            &root.join("codex"),
            provider,
            cwd,
            session.provider_session_id.as_deref(),
        )
        .ok();
    }
    let home = std::env::var_os("HOME").map(PathBuf::from)?;
    otto_sessions::transcript_path(&home, provider, cwd, session.provider_session_id.as_deref())
        .ok()
}

pub(crate) async fn collect(
    session: &Session,
    sessions: &SessionsRepo,
    improvements: &ImprovementsRepo,
    data_dir: &Path,
    previous: &serde_json::Value,
    lookback_hours: u32,
) -> Result<EvidenceBatch> {
    let mut checkpoint: Checkpoint = serde_json::from_value(previous.clone()).unwrap_or_default();
    let provider_name = session
        .meta
        .get("nested_provider")
        .and_then(|v| v.as_str())
        .unwrap_or(&session.provider)
        .to_owned();
    let mut digest = SessionDigest {
        session_id: session.id.clone(),
        title: format!("{} [{}]", session.title, provider_name),
        turns: 0,
        skills_used: vec![],
        tool_errors: 0,
        text: String::new(),
    };
    if let Some(provider) = Provider::parse(&provider_name).filter(|p| *p != Provider::Agy) {
        let persisted = sessions.transcript_path(&session.id).await?;
        let s = session.clone();
        let dir = data_dir.to_owned();
        let prev = checkpoint.clone();
        let source = tokio::task::spawn_blocking(move || {
            resolve_path(&s, persisted, &dir).and_then(|p| read_delta(&p, &prev).ok())
        })
        .await
        .map_err(|e| otto_core::Error::Internal(format!("read learning evidence: {e}")))?;
        if let Some((body, next)) = source {
            checkpoint = next;
            digest = digest_for_provider(&session.id, &digest.title, provider, &body);
        }
    }
    let since =
        otto_state::convert::fmt(Utc::now() - Duration::hours(i64::from(lookback_hours.max(1))));
    let trail = improvements
        .learning_trail(&session.id, checkpoint.trail_id.as_deref(), &since)
        .await?;
    for entry in trail {
        checkpoint.trail_id = Some(entry.id);
        if entry.kind == "skill" {
            add_skill(&mut digest.skills_used, &entry.summary);
        } else if !digest.text.contains(&entry.summary) {
            collect_skill_mentions(&entry.summary, &mut digest.skills_used);
            append_recent(
                &mut digest.text,
                &format!("\nUSER {}: {}", entry.kind, entry.summary),
            );
            digest.turns += 1;
        }
    }
    // Corrections in a later interaction still refer to the skill used earlier
    // in this session. Retain only safe names, never skill bodies or prompts.
    for skill in &checkpoint.skills {
        add_skill(&mut digest.skills_used, skill);
    }
    digest.skills_used.retain(|name| {
        !name.is_empty()
            && name.len() <= 200
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
    });
    digest.skills_used.truncate(64);
    checkpoint.skills = digest.skills_used.clone();
    Ok(EvidenceBatch {
        digest: (!digest.text.trim().is_empty() || digest.tool_errors > 0).then_some(digest),
        checkpoint: serde_json::to_value(checkpoint).unwrap_or_default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cursors_skip_consumed_lines_and_retry_incomplete_lines_and_replacements() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("s.jsonl");
        std::fs::write(&path, "first\npartial").unwrap();
        let (first, cursor) = read_delta(&path, &Checkpoint::default()).unwrap();
        assert_eq!(first, "first\n");
        assert_eq!(read_delta(&path, &cursor).unwrap().0, "");
        std::fs::write(&path, "first\npartial done\n").unwrap();
        let (delta, next) = read_delta(&path, &cursor).unwrap();
        assert_eq!(delta, "partial done\n");
        std::fs::write(&path, "other\nreplaced all\n").unwrap();
        assert_eq!(read_delta(&path, &next).unwrap().0, "other\nreplaced all\n");
    }
    #[test]
    fn replaced_file_with_identical_eof_anchor_is_reread() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("s.jsonl");
        let footer = "footer\n".repeat(100);
        std::fs::write(&path, format!("old correction\n{footer}")).unwrap();
        let (_, cursor) = read_delta(&path, &Checkpoint::default()).unwrap();
        let replacement = tmp.path().join("replacement");
        std::fs::write(&replacement, format!("new correction\n{footer}")).unwrap();
        std::fs::rename(&replacement, &path).unwrap();
        let (text, _) = read_delta(&path, &cursor).unwrap();
        assert!(text.starts_with("new correction"));
    }

    #[test]
    fn large_sources_read_recent_complete_tail() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("s.jsonl");
        std::fs::write(
            &path,
            format!("{}\nlatest correction\n", "old\n".repeat(MAX_READ as usize)),
        )
        .unwrap();
        let (text, checkpoint) = read_delta(&path, &Checkpoint::default()).unwrap();
        assert!(text.ends_with("latest correction\n"));
        assert!(text.len() <= MAX_READ as usize);
        assert!(read_delta(&path, &checkpoint).unwrap().0.is_empty());
    }
}
