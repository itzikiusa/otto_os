//! Product agent session cwds: a unique scratch child per session when the
//! story has no real cwd (codex usage attribution), pre-trusted once at the
//! stable root, and swept of day-old leftovers no session still uses.

use tracing::warn;

/// Resolve the cwd a product agent session should run in, making it unique when
/// it would otherwise be the shared system temp-dir fallback.
///
/// WHY: the usage tailer attributes codex sessions by their cwd (1:1 `by_cwd`).
/// Product sessions (analysis fan-out, rewrite, test-gen, plan-gen) fall back to
/// `std::env::temp_dir()` when a story has no real cwd. Multiple codex sessions
/// then share `/tmp` (or `$TMPDIR`) and collide → all attributed to "external"
/// instead of the workspace. Giving each session its own temp subdir restores a
/// 1:1 cwd→session mapping. (Claude attributes by its own session id, so it's
/// unaffected — but a unique dir is harmless for it.)
///
/// A REAL story cwd (the user's repo) passes through UNCHANGED — the architecture
/// lens needs the real repo. Only the shared-temp fallback is rewritten, to a
/// freshly-created unique child of the temp dir.
pub fn session_cwd(requested: &str) -> String {
    let temp = std::env::temp_dir();

    // Compare against the shared temp-dir fallback. Canonicalize both so e.g.
    // /var vs /private/var (macOS) or a trailing slash don't defeat the match;
    // fall back to a raw path compare if canonicalization fails.
    let requested_path = std::path::Path::new(requested);
    let is_shared_temp = match (requested_path.canonicalize(), temp.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => requested_path == temp.as_path(),
    };

    if !is_shared_temp {
        // Real story cwd — leave it untouched.
        return requested.to_string();
    }

    // Shared-temp fallback → unique per-session subdir so codex usage attributes
    // 1:1 by cwd instead of colliding on the shared temp dir. The children live
    // under ONE stable root (S4-14) that [`spawn_scratch_sweep`] clears of
    // day-old leftovers no session still uses, so scratch dirs no longer
    // accumulate in `$TMPDIR` forever.
    let root = temp.join(SCRATCH_ROOT);
    let unique = root.join(uuid::Uuid::new_v4().to_string());
    if let Err(e) = std::fs::create_dir_all(&unique) {
        tracing::debug!("product_run: create session cwd {}: {e}", unique.display());
    }
    unique.to_string_lossy().to_string()
}

/// Pre-trust a session cwd from [`session_cwd`]: a scratch child trusts the
/// stable [`SCRATCH_ROOT`] once for claude instead of adding one dead
/// `~/.claude.json` project entry per run (S4-14); a real story cwd is
/// trusted as-is.
pub fn trust_session_cwd(provider: &str, cwd: &str) {
    otto_sessions::trust::ensure_trusted_scratch(
        provider,
        &std::env::temp_dir().join(SCRATCH_ROOT),
        cwd,
    );
}

/// Stable parent of every product scratch cwd (under the temp dir).
pub const SCRATCH_ROOT: &str = "otto-product-scratch";
/// Scratch dirs older than this — and used by no unarchived session — are
/// removed by [`spawn_scratch_sweep`].
pub const SCRATCH_MAX_AGE: std::time::Duration = std::time::Duration::from_secs(24 * 3600);
/// At most one sweep per this interval (each one reads the sessions table).
const SCRATCH_SWEEP_EVERY_SECS: u64 = 3600;

/// Sweep the product scratch root in the background (called when a product
/// run starts; throttled to one sweep an hour, one at a time). A product
/// analysis / test / plan session is a visible, RESUMABLE session that can
/// stay open for days, and a dir's mtime does not move while an agent only
/// reads in it — so age alone swept live sessions' cwds and broke their
/// resume (S4-304). Every cwd an unarchived session still points at is kept.
pub fn spawn_scratch_sweep(pool: otto_state::DbPool) {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    static SWEEPING: AtomicBool = AtomicBool::new(false);
    static LAST: AtomicU64 = AtomicU64::new(0);
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let last = LAST.load(Ordering::Acquire);
    if (last != 0 && now.saturating_sub(last) < SCRATCH_SWEEP_EVERY_SECS)
        || SWEEPING.swap(true, Ordering::AcqRel)
    {
        return;
    }
    LAST.store(now, Ordering::Release);
    let Ok(handle) = tokio::runtime::Handle::try_current() else {
        SWEEPING.store(false, Ordering::Release);
        return;
    };
    handle.spawn(async move {
        let root = std::env::temp_dir().join(SCRATCH_ROOT);
        let keep = otto_state::SessionsRepo::new(pool)
            .unarchived_cwds_under(&root.to_string_lossy())
            .await;
        match keep {
            Ok(keep) => {
                let _ = tokio::task::spawn_blocking(move || {
                    sweep_stale_scratch(&root, SCRATCH_MAX_AGE, &keep)
                })
                .await;
            }
            // Without the in-use set nothing is provably safe to remove.
            Err(e) => warn!("product scratch sweep skipped: {e}"),
        }
        SWEEPING.store(false, Ordering::Release);
    });
}

/// Remove children of `root` last modified more than `max_age` ago, except
/// the paths in `keep` (best-effort; a missing root is fine). Blocking: call
/// it from the blocking pool (see [`spawn_scratch_sweep`]) or a test.
#[allow(clippy::disallowed_methods)] // sync helper: only run via spawn_scratch_sweep's spawn_blocking (or a test)
pub fn sweep_stale_scratch(
    root: &std::path::Path,
    max_age: std::time::Duration,
    keep: &std::collections::HashSet<String>,
) {
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let now = std::time::SystemTime::now();
    for e in entries.flatten() {
        if keep.contains(e.path().to_string_lossy().as_ref()) {
            continue;
        }
        let stale = e
            .metadata()
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .is_some_and(|age| age > max_age);
        if stale {
            let _ = std::fs::remove_dir_all(e.path());
        }
    }
}
