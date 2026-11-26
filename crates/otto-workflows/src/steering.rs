//! Steering notes for a running or held run (goal.md §2.4), ported from
//! otto-automation's `goal_loop_steering`: queued in
//! `<context dir>/steering.json` (atomic write + rename) by
//! `POST /workflow-runs/{id}/steer` or the chat `note` op, and taken and
//! cleared by the next agent step that starts, which gets a `[steering]`
//! prompt section.

use std::path::Path;
use std::sync::Mutex;

use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};

/// Maximum characters of one note.
pub const MAX_NOTE_CHARS: usize = 4_000;
/// Maximum notes waiting for the next agent step.
pub const MAX_QUEUED: usize = 20;
const FILE: &str = "steering.json";

/// One queued note and who wrote it (display name).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    pub text: String,
    pub by: String,
}

/// Serializes queue and take, so a note queued while a step takes the queue
/// is never lost.
static LOCK: Mutex<()> = Mutex::new(());

/// A trimmed note, or why it is refused.
pub fn validate(raw: &str) -> Result<String> {
    let note = raw.trim();
    if note.is_empty() {
        return Err(Error::Invalid("steering note cannot be empty".into()));
    }
    if note.chars().count() > MAX_NOTE_CHARS {
        return Err(Error::Invalid(format!(
            "steering note exceeds {MAX_NOTE_CHARS} characters"
        )));
    }
    Ok(note.to_string())
}

/// The queued notes, oldest first, without taking them.
pub fn peek(dir: &Path) -> Vec<Note> {
    std::fs::read(dir.join(FILE))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}

/// Queue a note; returns the queue. Conflict when [`MAX_QUEUED`] are waiting.
pub fn queue(dir: &Path, raw: &str, by: &str) -> Result<Vec<Note>> {
    let text = validate(raw)?;
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut notes = peek(dir);
    if notes.len() >= MAX_QUEUED {
        return Err(Error::Conflict(format!(
            "at most {MAX_QUEUED} steering notes may wait for the next step"
        )));
    }
    notes.push(Note {
        text,
        by: by.to_string(),
    });
    let bytes = serde_json::to_vec(&notes).map_err(|e| Error::Internal(e.to_string()))?;
    let tmp = dir.join(format!("{FILE}.tmp.{}", otto_core::new_id()));
    std::fs::write(&tmp, bytes)
        .and_then(|()| std::fs::rename(&tmp, dir.join(FILE)))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            Error::Internal(format!("write steering notes: {e}"))
        })?;
    Ok(notes)
}

/// Take and clear the queue.
pub fn take(dir: &Path) -> Vec<Note> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let notes = peek(dir);
    if !notes.is_empty() {
        let _ = std::fs::remove_file(dir.join(FILE));
    }
    notes
}

/// The `[steering]` prompt section ("" for no notes).
pub fn section(notes: &[Note]) -> String {
    if notes.is_empty() {
        return String::new();
    }
    let mut names: Vec<&str> = Vec::new();
    for n in notes {
        if !names.contains(&n.by.as_str()) {
            names.push(&n.by);
        }
    }
    let mut out = format!(
        "\n\n[steering]\nNotes from {} while this run was going (newest last):\n",
        names.join(", ")
    );
    for (i, n) in notes.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", i + 1, n.text));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notes_queue_in_order_and_the_next_step_takes_them_once() {
        let dir = tempfile::tempdir().unwrap();
        assert!(queue(dir.path(), "   ", "Ann").is_err());
        queue(dir.path(), "  focus on the auth tests ", "Ann").unwrap();
        let q = queue(dir.path(), "skip the UI", "Ben").unwrap();
        assert_eq!(q.len(), 2);
        assert_eq!(q[0].text, "focus on the auth tests");
        let taken = take(dir.path());
        assert_eq!(taken, q);
        assert!(peek(dir.path()).is_empty());
        assert!(take(dir.path()).is_empty());
        let s = section(&taken);
        assert!(s.contains("[steering]\nNotes from Ann, Ben while this run was going"));
        assert!(s.contains("1. focus on the auth tests\n2. skip the UI"));
        assert!(section(&[]).is_empty());
    }

    #[test]
    fn the_queue_and_a_note_are_capped() {
        let dir = tempfile::tempdir().unwrap();
        assert!(queue(dir.path(), &"x".repeat(MAX_NOTE_CHARS + 1), "Ann").is_err());
        for i in 0..MAX_QUEUED {
            queue(dir.path(), &format!("n{i}"), "Ann").unwrap();
        }
        let err = queue(dir.path(), "one more", "Ann").unwrap_err();
        assert!(matches!(err, Error::Conflict(_)), "{err}");
    }
}
