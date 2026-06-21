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
//!   - vault memories (`MemoriesRepo` keyword search)
//!   - git repos (`ctx.git_store`)
//!   - broker clusters (`BrokerClustersRepo`)
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
use otto_core::domain::WorkspaceRole;
use otto_core::Id;
use otto_state::{ApiClientRepo, BrokerClustersRepo, MemoriesRepo, WorkflowsRepo};
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
    /// `"swarm_project"`, `"memory"`, `"repo"`, `"broker_cluster"`.
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

    // --- 1. Product stories -----------------------------------------------
    if let Ok(stories) = ctx.product_repo.list_stories().await {
        for s in stories
            .into_iter()
            .filter(|s| matches_title_or_sub(&s.title, &s.source_key, &q))
            .take(CAP)
        {
            all.push(Scored {
                score: score_title(&s.title, &q),
                updated_at: s.updated_at.to_rfc3339(),
                hit: SearchHit {
                    kind: "story".into(),
                    id: s.id.to_string(),
                    title: s.title.clone(),
                    subtitle: Some(s.source_key.clone()),
                    actions: vec!["open".into(), "send-to-agent".into(), "copy-context".into()],
                },
            });
        }
    }

    // --- 2. Workflows -----------------------------------------------------
    {
        let repo = WorkflowsRepo::new(ctx.pool.clone());
        if let Ok(wfs) = repo.list(&ws_id).await {
            for wf in wfs
                .into_iter()
                .filter(|w| matches_title_or_sub(&w.name, &w.description, &q))
                .take(CAP)
            {
                all.push(Scored {
                    score: score_title(&wf.name, &q),
                    // Workflow has no updated_at; use id as a stable tiebreaker stub.
                    updated_at: wf.id.to_string(),
                    hit: SearchHit {
                        kind: "workflow".into(),
                        id: wf.id.to_string(),
                        title: wf.name.clone(),
                        subtitle: if wf.description.is_empty() {
                            None
                        } else {
                            Some(wf.description.clone())
                        },
                        actions: vec!["open".into(), "rerun".into()],
                    },
                });
            }
        }
    }

    // --- 3. API-client saved requests -------------------------------------
    {
        let repo = ApiClientRepo::new(ctx.pool.clone());
        if let Ok(reqs) = repo.list_requests(&ws_id, None).await {
            for r in reqs
                .into_iter()
                .filter(|r| {
                    matches_title_or_sub(&r.name, &format!("{} {}", r.method, r.url), &q)
                })
                .take(CAP)
            {
                all.push(Scored {
                    score: score_title(&r.name, &q),
                    updated_at: r.updated_at.to_rfc3339(),
                    hit: SearchHit {
                        kind: "api_request".into(),
                        id: r.id.to_string(),
                        title: r.name.clone(),
                        subtitle: Some(format!("{} {}", r.method, r.url)),
                        actions: vec!["open".into(), "rerun".into(), "send-to-agent".into()],
                    },
                });
            }
        }
    }

    // --- 4. Swarm projects + tasks ----------------------------------------
    {
        let repo = &ctx.swarm_repo;
        if let Ok(swarms) = repo.list_swarms(&ws_id).await {
            let mut proj_count = 0usize;
            let mut task_count = 0usize;
            'outer: for swarm in &swarms {
                // Projects first.
                if proj_count < CAP {
                    if let Ok(projects) = repo.list_projects(&swarm.id).await {
                        for p in projects
                            .into_iter()
                            .filter(|p| matches_title_or_sub(&p.name, &p.description, &q))
                        {
                            if proj_count >= CAP {
                                break;
                            }
                            all.push(Scored {
                                score: score_title(&p.name, &q),
                                updated_at: p.updated_at.to_rfc3339(),
                                hit: SearchHit {
                                    kind: "swarm_project".into(),
                                    id: p.id.to_string(),
                                    title: p.name.clone(),
                                    subtitle: Some(swarm.name.clone()),
                                    actions: vec!["open".into(), "send-to-agent".into()],
                                },
                            });
                            proj_count += 1;
                        }
                    }
                }
                // Tasks.
                if task_count < CAP {
                    if let Ok(tasks) = repo.list_tasks_for_swarm(&swarm.id).await {
                        for t in tasks
                            .into_iter()
                            .filter(|t| matches_title_or_sub(&t.title, &t.description, &q))
                        {
                            if task_count >= CAP {
                                break;
                            }
                            all.push(Scored {
                                score: score_title(&t.title, &q),
                                updated_at: t.updated_at.to_rfc3339(),
                                hit: SearchHit {
                                    kind: "swarm_task".into(),
                                    id: t.id.to_string(),
                                    title: t.title.clone(),
                                    subtitle: Some(format!("{} · {}", swarm.name, t.status)),
                                    actions: vec![
                                        "open".into(),
                                        "send-to-agent".into(),
                                        "copy-context".into(),
                                    ],
                                },
                            });
                            task_count += 1;
                        }
                    }
                }
                if proj_count >= CAP && task_count >= CAP {
                    break 'outer;
                }
            }
        }
    }

    // --- 5. Vault memories ------------------------------------------------
    {
        let repo = MemoriesRepo::new(ctx.pool.clone());
        let filter = otto_state::memory::SearchFilter {
            collection: None,
            story_id: None,
            include_inactive: false,
            limit: CAP as i64,
        };
        if let Ok(hits) = repo.search_keyword(ws_id.as_str(), &q, &filter).await {
            for (mem, _score) in hits.into_iter().take(CAP) {
                all.push(Scored {
                    // Memory hits are always subtitle-level: their title is the key
                    // but score 2 would let them stomp over more-specific story hits.
                    score: if mem.title.to_lowercase().contains(&q) { 2 } else { 1 },
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

    // --- 6. Git repos (name + remote URL only; no live fetch) -------------
    if let Ok(repos) = ctx.git_store.list_repos(&ws_id).await {
        for r in repos
            .into_iter()
            .filter(|r| {
                matches_title_or_sub(&r.name, r.remote_url.as_deref().unwrap_or(""), &q)
            })
            .take(CAP)
        {
            all.push(Scored {
                score: score_title(&r.name, &q),
                updated_at: r.created_at.to_rfc3339(),
                hit: SearchHit {
                    kind: "repo".into(),
                    id: r.id.to_string(),
                    title: r.name.clone(),
                    subtitle: r.remote_url.clone(),
                    actions: vec!["open".into(), "review".into()],
                },
            });
        }
    }

    // --- 7. Broker clusters -----------------------------------------------
    {
        let repo = BrokerClustersRepo::new(ctx.pool.clone());
        if let Ok(clusters) = repo.list_visible(&ws_id).await {
            for c in clusters
                .into_iter()
                .filter(|c| matches_title_or_sub(&c.name, &c.bootstrap_servers, &q))
                .take(CAP)
            {
                all.push(Scored {
                    score: score_title(&c.name, &q),
                    updated_at: c.created_at.to_rfc3339(),
                    hit: SearchHit {
                        kind: "broker_cluster".into(),
                        id: c.id.to_string(),
                        title: c.name.clone(),
                        subtitle: Some(c.bootstrap_servers.clone()),
                        actions: vec!["open".into(), "send-to-agent".into()],
                    },
                });
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
// Ranking helpers
// ---------------------------------------------------------------------------

/// 2 = title contains the query, 1 = only the secondary string does.
fn score_title(title: &str, q: &str) -> i32 {
    if title.to_lowercase().contains(q) { 2 } else { 1 }
}

/// True when either the title or the secondary string contains the query.
fn matches_title_or_sub(title: &str, secondary: &str, q: &str) -> bool {
    title.to_lowercase().contains(q) || secondary.to_lowercase().contains(q)
}

// ---------------------------------------------------------------------------
// Router — one line wired into `module_routers()` in modules.rs
// ---------------------------------------------------------------------------

/// Registers `GET /workspaces/{id}/search`; merged into `module_routers()`.
pub fn search_routes() -> axum::Router<ServerCtx> {
    axum::Router::new().route(
        "/workspaces/{id}/search",
        axum::routing::get(search),
    )
}
