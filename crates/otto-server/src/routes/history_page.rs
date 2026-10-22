//! Bounded, paged History listing. The legacy array route
//! (`transcript::history`) delegates here.
use super::transcript;
use crate::transcript::resolve;
use crate::{
    auth::{require_ws_role, CurrentUser},
    error::{ApiError, ApiResult},
    state::ServerCtx,
};
use axum::{
    extract::{Path as AxPath, Query, State},
    Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use otto_core::{domain::WorkspaceRole, Error, Id};
use otto_state::history_page::{self, Candidate, PageRequest};
use otto_transcript::{HistoryEntry, Provider};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;

#[derive(Default, Deserialize)]
pub struct HistoryPageQuery {
    q: Option<String>,
    provider: Option<String>,
    status: Option<String>,
    cwd: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
}
#[derive(Serialize)]
pub struct HistoryPage {
    entries: Vec<HistoryEntry>,
    next_cursor: Option<String>,
}
#[derive(Serialize, Deserialize)]
struct Cursor {
    version: u8,
    scope: String,
    at: String,
    key: String,
}
fn clean(s: Option<&str>) -> Option<&str> {
    s.map(str::trim).filter(|s| !s.is_empty())
}
impl HistoryPageQuery {
    fn scope(&self, workspace: &str, owner: &str, admin: bool) -> String {
        let encoded = serde_json::to_vec(&(
            workspace,
            owner,
            admin,
            clean(self.q.as_deref()),
            clean(self.provider.as_deref()),
            clean(self.status.as_deref()),
            clean(self.cwd.as_deref()),
        ))
        .unwrap_or_default();
        hex::encode(Sha256::digest(encoded))
    }
    fn cursor(&self, scope: &str) -> ApiResult<Option<Cursor>> {
        let Some(encoded) = self.cursor.as_deref() else {
            return Ok(None);
        };
        let invalid = || {
            ApiError(Error::Invalid(
                "invalid history cursor; reload the list".into(),
            ))
        };
        if encoded.len() > 4096 {
            return Err(invalid());
        }
        let bytes = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| invalid())?;
        let cursor: Cursor = serde_json::from_slice(&bytes).map_err(|_| invalid())?;
        if cursor.version != 1 || cursor.scope != scope {
            return Err(invalid());
        }
        Ok(Some(cursor))
    }
    fn matches(&self, entry: &HistoryEntry) -> bool {
        if clean(self.provider.as_deref())
            .and_then(Provider::parse)
            .is_some_and(|p| p != entry.provider)
        {
            return false;
        }
        let Some(needle) = clean(self.q.as_deref()).map(str::to_lowercase) else {
            return true;
        };
        [
            entry.title.as_deref(),
            entry.first_prompt.as_deref(),
            Some(entry.cwd.as_str()),
        ]
        .into_iter()
        .flatten()
        .any(|s| s.to_lowercase().contains(&needle))
    }
}

pub async fn list(
    AxPath(wid): AxPath<Id>,
    Query(q): Query<HistoryPageQuery>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
) -> ApiResult<Json<HistoryPage>> {
    page(ctx, &user, wid, &q, None).await.map(Json)
}

/// Keyset key that sorts after every session id / `path:` key, so a legacy
/// `before` timestamp is exclusive (`at = before AND id > KEY` never holds).
const LEGACY_BEFORE_KEY: &str = "\u{10FFFF}";

/// The legacy `GET /workspaces/{wid}/history` array: the same bounded page,
/// with its `before` timestamp (exclusive) in place of the opaque cursor.
pub(crate) async fn legacy_entries(
    ctx: &ServerCtx,
    user: &otto_core::domain::User,
    wid: &Id,
    legacy: transcript::HistoryQuery,
    limit: usize,
) -> ApiResult<Vec<HistoryEntry>> {
    let q = HistoryPageQuery {
        q: legacy.q,
        provider: legacy.provider,
        status: legacy.status,
        cwd: legacy.cwd,
        cursor: None,
        limit: Some(limit),
    };
    let before = clean(legacy.before.as_deref()).map(str::to_string);
    Ok(page(ctx.clone(), user, wid.clone(), &q, before)
        .await?
        .entries)
}

async fn page(
    ctx: ServerCtx,
    user: &otto_core::domain::User,
    wid: Id,
    q: &HistoryPageQuery,
    legacy_before: Option<String>,
) -> ApiResult<HistoryPage> {
    require_ws_role(&ctx, user, &wid, WorkspaceRole::Viewer).await?;
    let admin = transcript::is_ws_admin(&ctx, user, &wid).await;
    let scope = q.scope(&wid, &user.id, admin);
    let cursor = match legacy_before {
        Some(at) => Some(Cursor {
            version: 1,
            scope: scope.clone(),
            at,
            key: LEGACY_BEFORE_KEY.into(),
        }),
        None => q.cursor(&scope)?,
    };
    let limit = q.limit.unwrap_or(100).clamp(1, 1000);
    let budget = 4 * limit;
    let candidates = crate::telemetry::measure(
        "history.candidates",
        history_page::candidates(
            &ctx.pool,
            PageRequest {
                workspace: &wid,
                owner: &user.id,
                admin,
                status: clean(q.status.as_deref()),
                cwd: clean(q.cwd.as_deref()),
                before: cursor.as_ref().map(|c| (c.at.as_str(), c.key.as_str())),
                budget,
            },
        ),
    )
    .await
    .map_err(ApiError)?;
    let total = candidates.len();
    let data_dir = ctx.data_dir.clone();
    // Rows with no persisted path whose last root scan found nothing (an
    // ended session whose transcript the provider pruned) skip the scan while
    // the mark is fresh (backend-performance-4-06).
    let unpersisted: Vec<String> = candidates
        .iter()
        .take(budget)
        .filter(|c| c.persisted_path.is_none())
        .filter_map(|c| c.session.as_ref().map(|s| s.id.clone()))
        .collect();
    let skip = resolve::fresh_missing_marks(&ctx.pool, &unpersisted).await;
    // Only the selected bounded window enters filesystem resolution. Preserve
    // actual resolver provider/path rather than trusting stale metadata hints.
    // A persisted path costs one stat (a pruned file is `TranscriptMissing`,
    // never a root scan); only a row with no persisted path scans the roots,
    // and a path found that way is persisted below so it scans at most once.
    let resolved = tokio::task::spawn_blocking(move || {
        candidates
            .into_iter()
            .take(budget)
            .map(|candidate| {
                let mut missing = false;
                let resolved = candidate.session.as_ref().and_then(|session| {
                    if skip.contains(&session.id) {
                        return None;
                    }
                    let persisted = candidate.persisted_path.as_deref();
                    let r = transcript::resolve_transcript_sync(&data_dir, persisted, session);
                    missing = resolve::should_mark_missing(session, persisted, &r);
                    r.ok()
                });
                (candidate, resolved, missing)
            })
            .collect::<Vec<_>>()
    })
    .await
    .map_err(|e| ApiError(Error::Internal(format!("history resolve task: {e}"))))?;
    let mut newly_missing = Vec::new();
    for (candidate, resolved, missing) in &resolved {
        let Some(session) = candidate.session.as_ref() else {
            continue;
        };
        if let Some((r, true)) = resolved {
            resolve::record_found(&ctx.pool, &session.id, &r.path).await;
        } else if *missing {
            newly_missing.push(session.id.clone());
        }
    }
    resolve::mark_missing(&ctx.pool, &newly_missing).await;
    let resolved: Vec<_> = resolved
        .into_iter()
        .map(|(candidate, r, _)| (candidate, r.map(|(r, _)| r)))
        .collect();
    let paths: Vec<_> = resolved
        .iter()
        .filter_map(|(_, r)| r.as_ref().map(|r| r.path.to_string_lossy().into_owned()))
        .collect();
    let indexed = crate::telemetry::measure(
        "history.indexed_paths",
        history_page::indexed_paths(&ctx.pool, &paths),
    )
    .await
    .map_err(ApiError)?;
    let indexed: HashMap<_, _> = indexed
        .into_iter()
        .map(|row| (row.path.clone(), row))
        .collect();
    let entries = resolved.into_iter().map(|(candidate, resolved)| {
        let entry = if let (Some(session), Some(resolved)) = (candidate.session.as_ref(), resolved)
        {
            let path = resolved.path.to_string_lossy().into_owned();
            let row = indexed.get(&path);
            Some(HistoryEntry {
                session_id: Some(session.id.clone()),
                provider: resolved.provider,
                title: Some(session.title.clone())
                    .filter(|s| !s.trim().is_empty())
                    .or_else(|| row.and_then(|r| r.title.clone())),
                first_prompt: row.and_then(|r| r.first_prompt.clone()),
                cwd: session.cwd.clone(),
                repo_name: transcript::repo_name(&session.cwd),
                started_at: session.created_at.to_rfc3339(),
                last_active_at: candidate.at.clone(),
                turns: row.and_then(|r| r.turns).map(|n| n.max(0) as u64),
                status: session.status.as_str().into(),
                transcript_path: path,
                resumable: transcript::history_resumable(session),
                archived: session.archived,
            })
        } else {
            candidate.indexed.as_ref().and_then(|row| {
                let provider = Provider::parse(&row.provider)?;
                let cwd = row.cwd.clone().unwrap_or_default();
                Some(HistoryEntry {
                    session_id: None,
                    provider,
                    title: row.title.clone(),
                    first_prompt: row.first_prompt.clone(),
                    repo_name: transcript::repo_name(&cwd),
                    cwd,
                    started_at: row
                        .started_at
                        .clone()
                        .unwrap_or_else(|| candidate.at.clone()),
                    last_active_at: candidate.at.clone(),
                    turns: row.turns.map(|n| n.max(0) as u64),
                    status: "on_disk".into(),
                    transcript_path: row.path.clone(),
                    resumable: row.provider_session_id.is_some(),
                    archived: false,
                })
            })
        };
        (candidate, entry)
    });
    Ok(finish_page(q, &scope, limit, total, entries))
}

fn finish_page(
    q: &HistoryPageQuery,
    scope: &str,
    limit: usize,
    total: usize,
    candidates: impl Iterator<Item = (Candidate, Option<HistoryEntry>)>,
) -> HistoryPage {
    let mut entries = Vec::new();
    let mut last = None;
    let mut scanned = 0;
    for (candidate, entry) in candidates {
        scanned += 1;
        last = Some(Cursor {
            version: 1,
            scope: scope.into(),
            at: candidate.at,
            key: candidate.key,
        });
        if let Some(entry) = entry.filter(|entry| q.matches(entry)) {
            entries.push(entry);
        }
        if entries.len() == limit {
            break;
        }
    }
    let next_cursor = if scanned < total {
        last.and_then(|c| serde_json::to_vec(&c).ok())
            .map(|bytes| URL_SAFE_NO_PAD.encode(bytes))
    } else {
        None
    };
    HistoryPage {
        entries,
        next_cursor,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn entry(title: &str) -> HistoryEntry {
        serde_json::from_value(serde_json::json!({"session_id":null,"provider":"claude","title":title,"first_prompt":null,"cwd":"/repo","repo_name":"repo","started_at":"date","last_active_at":"date","turns":1,"status":"on_disk","transcript_path":"/fixture","resumable":false})).unwrap()
    }
    #[test]
    fn unicode_and_literal_substrings_preserve_existing_matching() {
        let q = HistoryPageQuery {
            q: Some("k_%".into()),
            ..Default::default()
        };
        assert!(q.matches(&entry("K_%")));
        assert!(!q.matches(&entry("Kabc")));
    }
    #[test]
    fn empty_budget_page_keeps_scanned_cursor_and_scope() {
        let q = HistoryPageQuery {
            q: Some("later".into()),
            ..Default::default()
        };
        let scope = q.scope("ws", "owner", false);
        let rows = (0..400).map(|n| {
            (
                Candidate {
                    key: format!("s{n}"),
                    at: "date".into(),
                    session: None,
                    indexed: None,
                    persisted_path: None,
                },
                Some(entry("unmatched")),
            )
        });
        let page = finish_page(&q, &scope, 100, 401, rows);
        assert!(page.entries.is_empty());
        let follow = HistoryPageQuery {
            cursor: page.next_cursor,
            ..Default::default()
        };
        let cursor = follow.cursor(&scope).unwrap().unwrap();
        assert_eq!(cursor.key, "s399");
        assert!(follow.cursor("another-owner").is_err());
    }
    #[test]
    fn matching_beyond_first_hundred_is_returned_without_skipping_cursor_lookahead() {
        let q = HistoryPageQuery {
            q: Some("match".into()),
            ..Default::default()
        };
        let rows = (0..400).map(|n| {
            (
                Candidate {
                    key: format!("s{n}"),
                    at: "date".into(),
                    session: None,
                    indexed: None,
                    persisted_path: None,
                },
                Some(entry(if n == 250 { "match" } else { "other" })),
            )
        });
        let page = finish_page(&q, "scope", 1, 401, rows);
        assert_eq!(page.entries.len(), 1);
        let follow = HistoryPageQuery {
            cursor: page.next_cursor,
            ..Default::default()
        };
        assert_eq!(follow.cursor("scope").unwrap().unwrap().key, "s250");
    }
}
