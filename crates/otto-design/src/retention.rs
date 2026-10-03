//! Retention planning + the scheduled pass.
//!
//! Two ways the policy runs: an admin's `POST /design/admin/prune` (a dry run
//! unless `apply: true`, any version age), and the daily scheduled pass
//! ([`spawn_scheduler`], on by default, `OTTO_DESIGN_AUTO_PRUNE=0` turns it
//! off) which only squashes autosaves OLDER than a day
//! (`OTTO_DESIGN_PRUNE_MIN_AGE_SECS`, default 86 400) and then GCs the blobs
//! nothing references. Every content PUT keeps a full content-addressed copy,
//! so without it the blob store only ever grew.
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
/// Scheduled pass: only versions older than this are candidates.
pub const DEFAULT_MIN_AGE_SECS: i64 = 86_400;
/// Scheduled pass cadence.
pub const SCHEDULE_EVERY_SECS: u64 = 86_400;
/// First scheduled pass after boot (keeps it off the startup path).
pub const FIRST_RUN_AFTER_SECS: u64 = 600;

/// Scheduled-pass settings, read from the environment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SchedulerConfig {
    pub enabled: bool,
    pub window_secs: i64,
    pub min_age_secs: i64,
}

impl SchedulerConfig {
    pub fn from_env() -> Self {
        Self::from_lookup(|k| std::env::var(k).ok())
    }

    /// Pure parse (unit-tested): `OTTO_DESIGN_AUTO_PRUNE` = 0/false/off
    /// disables; the window and minimum age fall back to the defaults when
    /// unset or not a positive integer.
    pub fn from_lookup(get: impl Fn(&str) -> Option<String>) -> Self {
        let enabled = !matches!(
            get("OTTO_DESIGN_AUTO_PRUNE")
                .map(|v| v.trim().to_ascii_lowercase())
                .as_deref(),
            Some("0" | "false" | "off" | "no")
        );
        let num = |k: &str, d: i64| {
            get(k)
                .and_then(|v| v.trim().parse::<i64>().ok())
                .filter(|n| *n > 0)
                .unwrap_or(d)
        };
        Self {
            enabled,
            window_secs: num("OTTO_DESIGN_PRUNE_WINDOW_SECS", DEFAULT_WINDOW_SECS),
            min_age_secs: num("OTTO_DESIGN_PRUNE_MIN_AGE_SECS", DEFAULT_MIN_AGE_SECS),
        }
    }
}

/// Outcome of the most recent scheduled pass (for the storage gauge).
static LAST_RUN: std::sync::Mutex<Option<crate::types::ScheduledPruneRun>> =
    std::sync::Mutex::new(None);

pub fn last_run() -> Option<crate::types::ScheduledPruneRun> {
    LAST_RUN.lock().ok().and_then(|g| g.clone())
}

/// Run one scheduled pass now and record it.
pub async fn run_scheduled_once(
    svc: &crate::DesignService,
    cfg: &SchedulerConfig,
) -> otto_core::Result<crate::types::PruneReport> {
    let report = svc
        .prune_scheduled(cfg.window_secs, cfg.min_age_secs)
        .await?;
    if let Ok(mut g) = LAST_RUN.lock() {
        *g = Some(crate::types::ScheduledPruneRun {
            at: crate::store::stamp(Utc::now()),
            versions_removed: report.versions.len(),
            blobs_removed: report.blobs.len(),
        });
    }
    Ok(report)
}

/// Daily background retention (boot hook). A no-op when disabled.
pub fn spawn_scheduler(svc: crate::DesignService) {
    let cfg = SchedulerConfig::from_env();
    if !cfg.enabled {
        tracing::info!("design: scheduled retention disabled (OTTO_DESIGN_AUTO_PRUNE)");
        return;
    }
    tokio::spawn(async move {
        tokio::time::sleep(std::time::Duration::from_secs(FIRST_RUN_AFTER_SECS)).await;
        let mut every = tokio::time::interval(std::time::Duration::from_secs(SCHEDULE_EVERY_SECS));
        every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        loop {
            every.tick().await;
            match run_scheduled_once(&svc, &cfg).await {
                Ok(r) if !r.versions.is_empty() => tracing::info!(
                    versions = r.versions.len(),
                    blobs = r.blobs.len(),
                    "design: scheduled retention squashed old autosaves"
                ),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e, "design: scheduled retention failed"),
            }
        }
    });
}

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
    fn scheduler_config_defaults_and_off_switch() {
        let none = SchedulerConfig::from_lookup(|_| None);
        assert_eq!(
            none,
            SchedulerConfig {
                enabled: true,
                window_secs: DEFAULT_WINDOW_SECS,
                min_age_secs: DEFAULT_MIN_AGE_SECS
            }
        );
        for off in ["0", "false", "OFF", " no "] {
            let c = SchedulerConfig::from_lookup(|k| {
                (k == "OTTO_DESIGN_AUTO_PRUNE").then(|| off.to_string())
            });
            assert!(!c.enabled, "{off:?} disables");
        }
        let c = SchedulerConfig::from_lookup(|k| match k {
            "OTTO_DESIGN_PRUNE_WINDOW_SECS" => Some("120".into()),
            "OTTO_DESIGN_PRUNE_MIN_AGE_SECS" => Some("-5".into()),
            _ => None,
        });
        assert_eq!((c.window_secs, c.min_age_secs), (120, DEFAULT_MIN_AGE_SECS));
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
