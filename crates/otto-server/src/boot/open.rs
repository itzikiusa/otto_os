//! Phase 1 — open the state DB: offline compaction, open + migrate, crash
//! recovery that must precede every other reader, boot maintenance.

use otto_state::{DbPool, ImprovementsRepo};

use super::{BootConfig, BootPhases};

/// Compact (offline), open and migrate the state DB, then run the recovery
/// and maintenance passes that must see it before anything else does.
/// Marks the `db_compact`, `db_open` and `maintenance` boot phases.
pub async fn open_state(cfg: &BootConfig, boot: &mut BootPhases) -> Result<DbPool, String> {
    compact_offline(cfg).await;
    boot.mark("db_compact");

    let pool = otto_state::open(&cfg.db_path)
        .await
        .map_err(|e| format!("open database: {e}"))?;
    if otto_state::maintenance::confirm_offline_compaction(&cfg.db_path) {
        tracing::info!("db maintenance: compacted database opened cleanly; old file removed");
    }
    boot.mark("db_open");
    otto_state::database_changes::DatabaseChangesRepo::new(pool.clone())
        .recover_interrupted()
        .await
        .map_err(|e| format!("recover interrupted database changes: {e}"))?;
    boot_maintenance(&pool).await;
    boot.mark("maintenance");
    fail_orphaned_improvement_runs(&pool).await;
    Ok(pool)
}

/// Offline compaction (perf2/03 N1): a large, fragmented otto.db is
/// rewritten HERE, before the pool opens — nothing can be writing, so no
/// write stalls and none made after the snapshot is lost. The old file is
/// kept until the new one has opened and migrated (confirmed in
/// [`open_state`]); an unconfirmed swap is rolled back at the next start.
async fn compact_offline(cfg: &BootConfig) {
    match otto_state::maintenance::offline_compact_at_boot(&cfg.db_path).await {
        otto_state::maintenance::OfflineOutcome::NotNeeded => {}
        otto_state::maintenance::OfflineOutcome::Compacted(r) => tracing::info!(
            "db maintenance: compacted offline {} → {} bytes in {} ms",
            r.before_bytes,
            r.after_bytes,
            r.duration_ms
        ),
        otto_state::maintenance::OfflineOutcome::RolledBack => tracing::warn!(
            "db maintenance: the last start never confirmed the compacted database — \
             restored the original file (no automatic retry)"
        ),
        otto_state::maintenance::OfflineOutcome::Skipped(why) => {
            tracing::warn!("db maintenance: offline compaction skipped: {why}")
        }
    }
}

/// Planner statistics from the first query (sqlite_stat1), plus the
/// one-time compaction when a third of the file is free pages and the live
/// data is small enough to rewrite in a second or two (perf F1) — normally
/// already done by the offline pass; this inline VACUUM only catches a file
/// that pass skipped. A bigger one waits for the next start.
async fn boot_maintenance(pool: &DbPool) {
    let t = std::time::Instant::now();
    match otto_state::maintenance::boot(pool).await {
        Ok(b) => {
            if let Some(r) = b.compacted {
                tracing::info!(
                    "db maintenance: compacted at boot {} → {} bytes in {} ms",
                    r.before_bytes,
                    r.after_bytes,
                    r.duration_ms
                );
            } else if b.deferred_compaction {
                tracing::info!(
                    "db maintenance: {} of {} bytes free — compaction deferred to the next start",
                    b.stats.free_bytes(),
                    b.stats.size_bytes()
                );
            }
            tracing::debug!("db maintenance: boot pass {} ms", t.elapsed().as_millis());
        }
        Err(e) => tracing::warn!("db boot maintenance failed: {e}"),
    }
}

/// Self-improvement runs are in-process: nothing can still be running at
/// boot, and an orphaned `running` row blocks that workspace's runs forever.
async fn fail_orphaned_improvement_runs(pool: &DbPool) {
    match ImprovementsRepo::new(pool.clone())
        .fail_orphaned_runs()
        .await
    {
        Ok(0) => {}
        Ok(n) => tracing::warn!("marked {n} interrupted self-improvement run(s) failed"),
        Err(e) => tracing::warn!("recover interrupted self-improvement runs: {e}"),
    }
}
