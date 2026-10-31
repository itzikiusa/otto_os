//! Session → transcript resolution (architecture-3-03): the provider roots
//! the daemon reads, the effective provider of a session, and the lookup of
//! the JSONL behind it (E2E fixture hook → persisted path → root scan). The
//! conversation routes, History, the assistant host, the history indexer and
//! the agent-task nudger all resolve through here instead of reaching into
//! the `routes/transcript.rs` handler file.
//!
//! A root scan that finds nothing is remembered (`sessions.transcript_missing_at`,
//! 0191) so History does not re-scan an ended session whose transcript the
//! provider pruned on every page, filter change and search; see
//! [`fresh_missing_marks`].

use std::path::{Path, PathBuf};

use otto_core::domain::Session;
use otto_state::SessionsRepo;
use otto_transcript::roots::TranscriptRoots;
use otto_transcript::{Provider, UnavailableReason};

use crate::state::ServerCtx;

/// `$HOME` (the root of `~/.claude`), falling back to the OS home dir.
#[cfg(test)]
pub(crate) fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Every provider root the daemon reads transcripts from — the shared
/// [`otto_transcript::roots::TranscriptRoots::discover`] (architecture-6-01):
/// `OTTO_TRANSCRIPT_ROOTS` → the isolated e2e fixture under `data_dir` → the
/// local defaults plus every subscription account home.
#[cfg(test)]
pub(crate) fn transcript_roots(data_dir: &Path) -> TranscriptRoots {
    TranscriptRoots::discover(&home_dir(), data_dir)
}

/// The provider whose transcript this session has: its own for agent sessions,
/// the captured nested one for a plain terminal the user ran `claude`/`codex` in.
pub(crate) fn effective_provider(session: &Session) -> String {
    if matches!(session.provider.as_str(), "claude" | "codex" | "agy") {
        return session.provider.clone();
    }
    session
        .meta
        .get("nested_provider")
        .and_then(|v| v.as_str())
        .unwrap_or(&session.provider)
        .to_string()
}

/// A session's resolved transcript.
#[derive(Debug, Clone)]
pub(crate) struct Resolved {
    pub provider: Provider,
    pub path: PathBuf,
}

/// Pure (filesystem-only) resolution of the transcript behind `session`
/// (§4.2): the E2E fixture hook, then the persisted row (`persisted`), then
/// the provider roots. `fresh` is true when the roots had to be searched —
/// the caller may persist the result.
pub(crate) fn resolve_transcript_sync(
    data_dir: &Path,
    persisted: Option<&str>,
    session: &Session,
) -> Result<(Resolved, bool), UnavailableReason> {
    let provider_name = effective_provider(session);
    let provider = match provider_name.as_str() {
        "claude" => Provider::Claude,
        "codex" => Provider::Codex,
        "agy" => Provider::Agy,
        _ => return Err(UnavailableReason::ProviderUnsupported),
    };
    // E2E hook (design §8, B → A): under `OTTO_E2E=1` a seeded session may point
    // at a fixture transcript via `meta.e2e_transcript_path`; the provider comes
    // from the filename (`rollout-*` = codex, else claude). Inert in production.
    if e2e_enabled() {
        if let Some(p) = session
            .meta
            .get("e2e_transcript_path")
            .and_then(|v| v.as_str())
        {
            let p = PathBuf::from(p);
            if let Some(provider) = otto_transcript::provider_for_path(&p).filter(|_| p.is_file()) {
                return Ok((Resolved { provider, path: p }, false));
            }
            return Err(UnavailableReason::TranscriptMissing);
        }
    }
    // O(1) path: the persisted row (set by the capture scan or a prior read).
    // A persisted path whose file is gone was pruned by the provider (Claude
    // drops transcripts after ~30 days): the root scan would re-find nothing,
    // at one stat per project dir (Codex: a whole rollout-tree walk), on every
    // History page and search keystroke — so stop here.
    if let Some(p) = persisted {
        let p = PathBuf::from(p);
        if p.is_file() {
            return Ok((Resolved { provider, path: p }, false));
        }
        return Err(UnavailableReason::TranscriptMissing);
    }
    let cwd = session
        .meta
        .get("nested_cwd")
        .and_then(|v| v.as_str())
        .unwrap_or(&session.cwd);
    // The session's own account home, else the local defaults (never another
    // account's tree); an invalid account id yields no roots at all.
    let roots = TranscriptRoots::for_session(&session.meta, data_dir);
    let path = otto_sessions::transcript_path_in(
        &roots,
        &provider_name,
        cwd,
        session.provider_session_id.as_deref(),
    )?;
    if !path.is_file() {
        return Err(UnavailableReason::TranscriptMissing);
    }
    Ok((Resolved { provider, path }, true))
}

/// Resolve the transcript behind `session`, persisting a freshly found path
/// (one cheap UPDATE, once per session) so later lookups skip the root scan.
pub(crate) async fn resolve_transcript(
    ctx: &ServerCtx,
    session: &Session,
) -> Result<Resolved, UnavailableReason> {
    let repo = SessionsRepo::new(ctx.pool.clone());
    let persisted = repo.transcript_path(&session.id).await.ok().flatten();
    let data_dir = ctx.data_dir.clone();
    let owned_session = session.clone();
    let (resolved, fresh) = crate::offload::blocking(move || {
        resolve_transcript_sync(&data_dir, persisted.as_deref(), &owned_session)
    })
    .await?;
    if fresh {
        record_found(&ctx.pool, &session.id, &resolved.path).await;
    }
    Ok(resolved)
}

/// The isolated e2e daemon (`otto_core::e2e::isolated`); unlocks the
/// fixture-transcript hooks only.
pub(crate) fn e2e_enabled() -> bool {
    otto_transcript::roots::isolated()
}

/// Persist a transcript path a root scan found, clearing any "no transcript"
/// mark in the same statement. Best effort.
pub(crate) async fn record_found(pool: &otto_state::DbPool, id: &str, path: &Path) {
    let _ = sqlx::query(
        "UPDATE sessions SET transcript_path = ?, transcript_missing_at = NULL WHERE id = ?",
    )
    .bind(path.to_string_lossy().as_ref())
    .bind(id)
    .execute(pool)
    .await;
}
