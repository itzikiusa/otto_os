//! Steering notes for a running or held workflow run (goal.md §2.4): queued
//! by `POST /workflow-runs/{id}/steer` in `<context>/steering.json` and taken
//! by the next agent step that starts. Moved from Goal Loops' queue; the
//! path is the run's context folder.

use std::path::{Path, PathBuf};

use otto_core::{Error, Result};

/// Characters accepted in one note.
pub const MAX_NOTE_CHARS: usize = 4_000;
/// Notes waiting for the next agent step.
pub const MAX_QUEUED: usize = 20;

fn path(context_dir: &Path) -> PathBuf {
    context_dir.join("steering.json")
}

/// The trimmed note, or why it is refused.
pub fn validate_note(raw: &str) -> Result<String> {
    let note = raw.trim();
    if note.is_empty() {
        return Err(Error::Invalid("a steering note cannot be empty".into()));
    }
    if note.chars().count() > MAX_NOTE_CHARS {
        return Err(Error::Invalid(format!(
            "a steering note is at most {MAX_NOTE_CHARS} characters"
        )));
    }
    Ok(note.to_string())
}

/// The queued notes, oldest first (none when the file is missing).
pub async fn peek(context_dir: &Path) -> Vec<String> {
    let bytes = tokio::fs::read(path(context_dir)).await.unwrap_or_default();
    serde_json::from_slice(&bytes).unwrap_or_default()
}

/// Queue a note; the queue after it. 409 when [`MAX_QUEUED`] are waiting.
pub async fn queue(context_dir: &Path, raw: &str) -> Result<Vec<String>> {
    let note = validate_note(raw)?;
    let mut notes = peek(context_dir).await;
    if notes.len() >= MAX_QUEUED {
        return Err(Error::Conflict(format!(
            "{MAX_QUEUED} notes are already waiting for the next step"
        )));
    }
    notes.push(note);
    tokio::fs::create_dir_all(context_dir)
        .await
        .map_err(|e| Error::Internal(format!("create steering dir: {e}")))?;
    let tmp = context_dir.join(format!("steering.json.tmp.{}", otto_core::new_id()));
    let bytes = serde_json::to_vec(&notes).map_err(|e| Error::Internal(e.to_string()))?;
    tokio::fs::write(&tmp, bytes)
        .await
        .map_err(|e| Error::Internal(format!("write steering notes: {e}")))?;
    tokio::fs::rename(&tmp, path(context_dir))
        .await
        .map_err(|e| Error::Internal(format!("commit steering notes: {e}")))?;
    Ok(notes)
}

/// Take and clear every queued note (the next agent step's start).
pub async fn take(context_dir: &Path) -> Vec<String> {
    let notes = peek(context_dir).await;
    if !notes.is_empty() {
        let _ = tokio::fs::remove_file(path(context_dir)).await;
    }
    notes
}

/// The `[steering]` prompt section for taken notes (empty when none).
pub fn section(by: &str, notes: &[String]) -> String {
    if notes.is_empty() {
        return String::new();
    }
    let mut out =
        format!("\n\n[steering]\nNotes from {by} while this run was going (newest last):\n");
    for (i, n) in notes.iter().enumerate() {
        out.push_str(&format!("{}. {n}\n", i + 1));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn notes_queue_in_the_run_folder_and_are_taken_once() {
        let dir = tempfile::tempdir().unwrap();
        let ctx = dir.path().join("run-1");
        assert!(peek(&ctx).await.is_empty());
        assert!(queue(&ctx, "  ").await.is_err());
        assert!(queue(&ctx, &"x".repeat(MAX_NOTE_CHARS + 1)).await.is_err());
        assert_eq!(
            queue(&ctx, " focus on auth ").await.unwrap(),
            ["focus on auth"]
        );
        assert_eq!(
            queue(&ctx, "skip UI").await.unwrap(),
            ["focus on auth", "skip UI"]
        );
        assert!(ctx.join("steering.json").exists());
        let taken = take(&ctx).await;
        assert_eq!(taken, ["focus on auth", "skip UI"]);
        assert!(take(&ctx).await.is_empty());
        let s = section("Dana", &taken);
        assert!(
            s.contains("[steering]\nNotes from Dana") && s.contains("2. skip UI\n"),
            "{s}"
        );
        assert_eq!(section("Dana", &[]), "");
    }

    #[tokio::test]
    async fn the_queue_is_capped() {
        let dir = tempfile::tempdir().unwrap();
        for i in 0..MAX_QUEUED {
            queue(dir.path(), &format!("n{i}")).await.unwrap();
        }
        let err = queue(dir.path(), "one more").await.unwrap_err();
        assert!(matches!(err, Error::Conflict(_)), "{err:?}");
    }
}
