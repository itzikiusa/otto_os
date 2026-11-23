//! Mid-loop operator steering notes, live spend circuit-breaker evaluation, and
//! worktree diff DTOs for Goal Loops (`E-01`).
//!
//! Steering notes are persisted atomically under
//! `<data_dir>/goal-loops/<loop_id>/steering.json` so they survive daemon
//! restarts without requiring a schema migration, and are consumed on the next
//! iteration's planner + executor prompts (without requiring `blocked` state).

use std::path::{Path, PathBuf};

use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};

/// Maximum UTF-8 characters accepted in one steering note.
pub const MAX_STEERING_NOTE_CHARS: usize = 4_000;
/// Maximum number of queued notes awaiting the next iteration.
pub const MAX_QUEUED_NOTES: usize = 20;
/// Default cap on unified diff bytes returned by `GET /goal-loops/{id}/diff`.
pub const DEFAULT_DIFF_PATCH_CAP: usize = 256 * 1024;

/// Request body for `POST /api/v1/goal-loops/{id}/steer`.
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SteerGoalLoopReq {
    pub note: String,
}

/// Response body for `POST /api/v1/goal-loops/{id}/steer`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct SteerGoalLoopResp {
    pub loop_id: String,
    pub note: String,
    pub queued_notes: Vec<String>,
}

/// Response body for `GET /api/v1/goal-loops/{id}/diff`.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct GoalLoopDiff {
    pub loop_id: String,
    pub available: bool,
    pub branch: Option<String>,
    pub stat: String,
    pub patch: String,
    pub truncated: bool,
    pub untracked: Vec<String>,
}

fn steering_path(data_dir: &Path, loop_id: &str) -> Result<PathBuf> {
    let safe = otto_core::paths::safe_component(loop_id)
        .ok_or_else(|| Error::Invalid("invalid goal loop id".into()))?;
    Ok(data_dir.join("goal-loops").join(safe).join("steering.json"))
}

/// Validate and normalize one operator steering note.
pub fn validate_note(raw: &str) -> Result<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err(Error::Invalid("steering note cannot be empty".into()));
    }
    if trimmed.chars().count() > MAX_STEERING_NOTE_CHARS {
        return Err(Error::Invalid(format!(
            "steering note exceeds {MAX_STEERING_NOTE_CHARS} characters"
        )));
    }
    Ok(trimmed.to_string())
}

async fn write_notes(path: &Path, notes: &[String]) -> Result<()> {
    if notes.is_empty() {
        let _ = tokio::fs::remove_file(path).await;
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| Error::Internal(format!("create steering dir: {e}")))?;
    }
    let bytes = serde_json::to_vec(notes)
        .map_err(|e| Error::Internal(format!("serialize steering notes: {e}")))?;
    let tmp = path.with_extension(format!("json.tmp.{}", otto_core::new_id()));
    tokio::fs::write(&tmp, &bytes)
        .await
        .map_err(|e| Error::Internal(format!("write steering tmp: {e}")))?;
    if let Err(e) = tokio::fs::rename(&tmp, path).await {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(Error::Internal(format!("commit steering notes: {e}")));
    }
    Ok(())
}

/// Read currently queued steering notes without consuming them.
pub async fn peek_notes(data_dir: &Path, loop_id: &str) -> Vec<String> {
    let Ok(path) = steering_path(data_dir, loop_id) else {
        return Vec::new();
    };
    let Ok(bytes) = tokio::fs::read(&path).await else {
        return Vec::new();
    };
    serde_json::from_slice::<Vec<String>>(&bytes).unwrap_or_default()
}

/// Append a validated steering note for `loop_id` and return the updated queue.
pub async fn queue_note(data_dir: &Path, loop_id: &str, raw_note: &str) -> Result<Vec<String>> {
    let note = validate_note(raw_note)?;
    let path = steering_path(data_dir, loop_id)?;
    let mut notes = peek_notes(data_dir, loop_id).await;
    if notes.len() >= MAX_QUEUED_NOTES {
        return Err(Error::Conflict(format!(
            "at most {MAX_QUEUED_NOTES} steering notes may be queued before the next iteration"
        )));
    }
    notes.push(note);
    write_notes(&path, &notes).await?;
    Ok(notes)
}

/// Atomically consume and clear all queued steering notes for the next iteration.
pub async fn take_notes(data_dir: &Path, loop_id: &str) -> Vec<String> {
    let Ok(path) = steering_path(data_dir, loop_id) else {
        return Vec::new();
    };
    let notes = peek_notes(data_dir, loop_id).await;
    if !notes.is_empty() {
        let _ = tokio::fs::remove_file(&path).await;
    }
    notes
}

/// Append `steering_section` to `prompt` when non-empty.
pub fn with_steering(mut prompt: String, steering_section: &str) -> String {
    prompt.push_str(steering_section);
    prompt
}

/// Atomically consume queued steering notes and format them as a prompt section
/// (empty string when none were queued).
pub async fn take_steering_section(data_dir: &Path, loop_id: &str) -> String {
    format_steering_section(&take_notes(data_dir, loop_id).await)
}

/// Format consumed steering notes as a high-priority markdown section for the
/// planner and executor prompts. Returns an empty string when no notes exist.
pub fn format_steering_section(notes: &[String]) -> String {
    if notes.is_empty() {
        return String::new();
    }
    let mut out = String::from(
        "\n\n## Mid-Loop Operator Steering Notes\nApply these direct operator instructions on this iteration:\n",
    );
    for (i, n) in notes.iter().enumerate() {
        out.push_str(&format!("{}. {}\n", i + 1, n.trim()));
    }
    out
}

/// Evaluate whether live spend has reached or exceeded the loop's `max_cost_usd`
/// circuit breaker. Returns `Some(reason)` when tripped, `None` when within
/// budget or when no cap / spend reading is available.
pub fn check_cost_cap(spent_usd: Option<f64>, max_cost_usd: Option<f64>) -> Option<String> {
    let cap = max_cost_usd.filter(|c| c.is_finite() && *c > 0.0)?;
    let spent = spent_usd.filter(|s| s.is_finite() && *s >= 0.0)?;
    if spent >= cap {
        Some(format!("cost cap reached (${spent:.2} / ${cap:.2})"))
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn queue_peek_and_take_steering_notes_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        assert!(peek_notes(dir.path(), "loop-1").await.is_empty());
        assert!(queue_note(dir.path(), "loop-1", "   ").await.is_err());

        let q1 = queue_note(dir.path(), "loop-1", "  focus on auth tests  ")
            .await
            .unwrap();
        assert_eq!(q1, vec!["focus on auth tests"]);
        let q2 = queue_note(dir.path(), "loop-1", "skip UI refactor")
            .await
            .unwrap();
        assert_eq!(q2, vec!["focus on auth tests", "skip UI refactor"]);
        assert_eq!(peek_notes(dir.path(), "loop-1").await, q2);

        let taken = take_notes(dir.path(), "loop-1").await;
        assert_eq!(taken, q2);
        assert!(peek_notes(dir.path(), "loop-1").await.is_empty());

        let section = format_steering_section(&taken);
        assert!(section.contains("## Mid-Loop Operator Steering Notes"));
        assert!(section.contains("1. focus on auth tests"));
        assert!(section.contains("2. skip UI refactor"));
        assert!(format_steering_section(&[]).is_empty());
    }

    #[test]
    fn check_cost_cap_trips_at_or_above_budget_and_degrades_gracefully() {
        assert_eq!(check_cost_cap(None, Some(5.0)), None);
        assert_eq!(check_cost_cap(Some(10.0), None), None);
        assert_eq!(check_cost_cap(Some(4.99), Some(5.0)), None);
        let reason = check_cost_cap(Some(5.0), Some(5.0)).expect("must trip at cap");
        assert!(reason.contains("$5.00 / $5.00"), "{reason}");
        let over = check_cost_cap(Some(7.25), Some(5.0)).expect("must trip above cap");
        assert!(over.contains("$7.25 / $5.00"), "{over}");
    }
}
