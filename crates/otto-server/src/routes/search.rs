//! Cross-module search: `GET /workspaces/{id}/search?q=` → `Vec<SearchHit>`.
//!
//! Queries each available source with a small per-source cap (~5), merges the
//! hits, and ranks them: title match (score 2) > subtitle match (score 1),
//! breaking ties by recency (most-recently-updated timestamp, lexicographic).
//!
//! Sources queried (read-only):
//!   - product stories (`ctx.product_repo`)
//!   - workflows (`WorkflowsRepo`)
//!   - API-client saved requests (`ApiClientRepo`)
//!   - swarm projects + tasks (`ctx.swarm_repo`)
//!   - memories (`MemoriesRepo` keyword search)
//!   - git repos (`ctx.git_store`)
//!   - broker clusters (`BrokerClustersRepo`)
//!   - canvas scenes (`ctx.canvas_repo`)
//!
//! Skipped sources:
//!   - live git commits/PRs/branches — require remote round-trip; excluded to
//!     keep latency bounded. Users navigate to the repo view from the hit.
//!   - DB saved queries — no structured query table at this time; DB Explorer
//!     queries are ephemeral.
//!
//! Gate: `WorkspaceRole::Viewer`.

use axum::extract::{Path, Query, State};
use axum::Json;
use otto_core::domain::{Capability, Feature, WorkspaceRole};
use otto_core::Id;
use otto_state::MemoriesRepo;
use serde::{Deserialize, Serialize};

use crate::auth::CurrentUser;
use crate::error::ApiResult;
use crate::state::ServerCtx;

// ---------------------------------------------------------------------------
// DTOs — module-local, not re-exported to other crates
// ---------------------------------------------------------------------------

/// A single ranked search hit across all object types.
#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    /// Discriminant: `"story"`, `"workflow"`, `"api_request"`, `"swarm_task"`,
    /// `"swarm_project"`, `"memory"`, `"vault_note"`, `"repo"`,
    /// `"broker_cluster"`, `"canvas"`.
    pub kind: String,
    /// Object id (workspace-scoped row id).
    pub id: String,
    /// Primary display text (object name / title).
    pub title: String,
    /// Secondary display text — method + url for requests, status for tasks, etc.
    pub subtitle: Option<String>,
    /// Contextual action labels. First is the primary "open" navigation target.
    pub actions: Vec<String>,
}

/// Internal pair: ranked hit awaiting the merge sort.
struct Scored {
    /// 2 = title match, 1 = subtitle/secondary match.
    score: i32,
    /// ISO-8601 string used as a recency tiebreaker (lex ≈ chrono).
    updated_at: String,
    hit: SearchHit,
}

/// Query parameters: `q` is the search string.
#[derive(Debug, Deserialize)]
pub struct SearchParams {
    #[serde(default)]
    pub q: String,
}

/// Maximum number of hits returned per source before the cross-source merge.
const CAP: usize = 5;

// ---------------------------------------------------------------------------
// Route handler
// ---------------------------------------------------------------------------

/// `GET /workspaces/{id}/search?q=` — cross-module, ranked search.
pub async fn search(
    Path(ws_id): Path<Id>,
    State(ctx): State<ServerCtx>,
    CurrentUser(user): CurrentUser,
    Query(params): Query<SearchParams>,
) -> ApiResult<Json<Vec<SearchHit>>> {
    crate::auth::require_ws_role(&ctx, &user, &ws_id, WorkspaceRole::Viewer).await?;

    let q = params.q.trim().to_lowercase();
    // Require at least 2 characters to avoid full-table fan-out on single keystrokes.
    if q.len() < 2 {
        return Ok(Json(vec![]));
    }

    let mut all: Vec<Scored> = Vec::new();

    // Authorize each source BEFORE touching its data. Agents View permits the
    // palette itself, not every module it aggregates. One grant read serves
    // the whole request; a failed authorization read fails closed.
    let grants = if user.is_root {
        vec![]
    } else {
        otto_state::GrantsRepo::new(ctx.pool.clone())
            .grants_for(&user.id)
            .await
            .map_err(crate::error::ApiError)?
    };
    let can = |feature: Feature| {
        user.is_root
            || grants
                .iter()
                .any(|(f, level)| *f == feature && *level >= Capability::View)
    };
    for source in otto_state::search::Source::ALL {
        if !can(source.feature()) {
            continue;
        }
        if let Ok(rows) = otto_state::search::search(&ctx.pool, source, &ws_id, &q, CAP).await {
            for row in rows {
                let actions: &[&str] = match source {
                    otto_state::search::Source::Story | otto_state::search::Source::SwarmTask => {
                        &["open", "send-to-agent", "copy-context"]
                    }
                    otto_state::search::Source::Workflow => &["open", "rerun"],
                    otto_state::search::Source::ApiRequest => &["open", "rerun", "send-to-agent"],
                    otto_state::search::Source::Repo => &["open", "review"],
                    otto_state::search::Source::Canvas => &["Open in Canvas"],
                    _ => &["open", "send-to-agent"],
                };
                all.push(Scored {
                    score: row.score,
                    updated_at: row.updated_at,
                    hit: SearchHit {
                        kind: source.kind().into(),
                        id: row.id,
                        title: row.title,
                        subtitle: row.subtitle,
                        actions: actions.iter().map(|s| (*s).into()).collect(),
                    },
                });
            }
        }
    }

    // --- 5. Vault memories ------------------------------------------------
    if can(Feature::Product) {
        let repo = MemoriesRepo::new(ctx.pool.clone());
        let filter = otto_state::memory::SearchFilter {
            collection: None,
            story_id: None,
            include_inactive: false,
            limit: CAP as i64,
            viewer: (!user.is_root).then(|| user.id.clone()),
            ..Default::default()
        };
        if let Ok(hits) = repo.search_keyword(ws_id.as_str(), &q, &filter).await {
            for (mem, _score) in hits.into_iter().take(CAP) {
                all.push(Scored {
                    // Memory hits are always subtitle-level: their title is the key
                    // but score 2 would let them stomp over more-specific story hits.
                    score: if mem.title.to_lowercase().contains(&q) {
                        2
                    } else {
                        1
                    },
                    updated_at: mem.created_at.clone(),
                    hit: SearchHit {
                        kind: "memory".into(),
                        id: mem.id.clone(),
                        title: mem.title.clone(),
                        subtitle: Some(format!("{} / {}", mem.collection, mem.kind)),
                        actions: vec!["open".into(), "copy-context".into()],
                    },
                });
            }
        }
    }

    // --- 5b. Vault notes (docs home) ---------------------------------------
    // FTS over every registered vault; the hit id is `<vault_id>:<path>` so the
    // UI can route to `#/vault` with the right vault + note selected.
    if let Ok(vaults) = if can(Feature::Product) {
        ctx.vault.list(ws_id.as_str()).await
    } else {
        Ok(vec![])
    } {
        let mut remaining = CAP;
        for v in vaults {
            if remaining == 0 {
                break;
            }
            let req = otto_vault::types::SearchReq {
                query: q.clone(),
                limit: remaining,
                ..Default::default()
            };
            if let Ok(hits) = ctx.vault.search(ws_id.as_str(), v.id, &req).await {
                for h in hits.into_iter().take(remaining) {
                    remaining -= 1;
                    all.push(Scored {
                        score: if h.title.to_lowercase().contains(&q) {
                            2
                        } else {
                            1
                        },
                        updated_at: String::new(),
                        hit: SearchHit {
                            kind: "vault_note".into(),
                            id: format!("{}:{}", v.id, h.path),
                            title: h.title,
                            subtitle: Some(h.path),
                            actions: vec!["open".into()],
                        },
                    });
                }
            }
        }
    }

    // --- Rank: title-match first, then recency (desc) ---------------------
    all.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| b.updated_at.cmp(&a.updated_at))
    });

    Ok(Json(all.into_iter().map(|s| s.hit).collect()))
}

// ---------------------------------------------------------------------------
// Router — one line wired into `module_routers()` in modules.rs
// ---------------------------------------------------------------------------

/// Registers `GET /workspaces/{id}/search`; merged into `module_routers()`.
pub fn search_routes() -> axum::Router<ServerCtx> {
    axum::Router::new().route("/workspaces/{id}/search", axum::routing::get(search))
}

#[cfg(test)]
#[path = "../../tests/unit/search.rs"]
mod tests;
