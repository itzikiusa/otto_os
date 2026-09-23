//! Retention planning — **opt-in only**. The daemon never prunes on its own:
//! every version is kept until an admin runs `POST /design/admin/prune` with
//! `apply: true` (a dry run by default).
//!
//! Policy (proposal §3.1): named / agent / import / sync / restore versions are
//! always kept; `autosave` versions are squashed to the LAST one per editing
//! window (default 10 minutes, a window never spans a non-autosave version);
//! and nothing in the `protected` set is ever a candidate — the head, the
//! approved version, versions pinned or cited by any link, published or
//! signal-referenced versions. Pure; unit-tested.

use std::collections::HashSet;

use chrono::{DateTime, Utc};

pub const DEFAULT_WINDOW_SECS: i64 = 600;

#[derive(Debug, Clone)]
pub struct VersionInfo {
    pub id: String,
    pub seq: i64,
    pub kind: String,
    pub created_at: DateTime<Utc>,
}

/// Version ids that MAY be pruned. `versions` may arrive in any order.
pub fn plan(
    versions: &[VersionInfo],
    protected: &HashSet<String>,
    window_secs: i64,
) -> Vec<String> {
    let window = window_secs.max(1);
    let mut sorted: Vec<&VersionInfo> = versions.iter().collect();
    sorted.sort_by_key(|v| v.seq);
    // Head is always protected even if the caller forgot it.
    let head = sorted.last().map(|v| v.id.clone());

    let mut out = Vec::new();
    let mut window_start: Option<DateTime<Utc>> = None;
    let mut window_members: Vec<&VersionInfo> = Vec::new();
    /// Keep the last autosave of the window; the rest are candidates.
    fn flush(members: &mut Vec<&VersionInfo>, out: &mut Vec<String>) {
        if members.len() > 1 {
            for v in &members[..members.len() - 1] {
                out.push(v.id.clone());
            }
        }
        members.clear();
    }
    for v in sorted {
        if v.kind != "autosave" {
            flush(&mut window_members, &mut out);
            window_start = None;
            continue;
        }
        match window_start {
            Some(start) if (v.created_at - start).num_seconds() < window => {
                window_members.push(v);
            }
            _ => {
                flush(&mut window_members, &mut out);
                window_start = Some(v.created_at);
                window_members.push(v);
            }
        }
    }
    flush(&mut window_members, &mut out);
    out.retain(|id| !protected.contains(id) && Some(id) != head.as_ref());
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn v(id: &str, seq: i64, kind: &str, secs: i64) -> VersionInfo {
        let t0 = DateTime::parse_from_rfc3339("2026-09-23T10:00:00Z")
            .unwrap()
            .with_timezone(&Utc);
        VersionInfo {
            id: id.into(),
            seq,
            kind: kind.into(),
            created_at: t0 + Duration::seconds(secs),
        }
    }

    #[test]
    fn squashes_autosaves_per_window_and_keeps_everything_else() {
        let versions = vec![
            v("a1", 1, "autosave", 0),
            v("a2", 2, "autosave", 60),
            v("a3", 3, "autosave", 120), // last of window 1 → kept
            v("n4", 4, "named", 130),    // named: kept, closes the window
            v("a5", 5, "autosave", 140),
            v("a6", 6, "autosave", 900), // new window (> 600 s after a5)
            v("a7", 7, "agent", 950),
            v("a8", 8, "autosave", 960), // head
        ];
        let pruned = plan(&versions, &HashSet::new(), DEFAULT_WINDOW_SECS);
        assert_eq!(pruned, vec!["a1".to_string(), "a2".to_string()]);
    }

    #[test]
    fn protected_and_head_are_never_candidates() {
        let versions = vec![
            v("a1", 1, "autosave", 0),
            v("a2", 2, "autosave", 10),
            v("a3", 3, "autosave", 20),
        ];
        let protected: HashSet<String> = ["a1".to_string()].into_iter().collect();
        assert_eq!(plan(&versions, &protected, 600), vec!["a2".to_string()]);
        // A single version is the head — never pruned.
        assert!(plan(&versions[..1], &HashSet::new(), 600).is_empty());
    }
}
