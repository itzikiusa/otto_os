//! `POST /ingest/swarm/board` — a swarm agent posts to its shared board using the
//! per-session ingest token (same gate as `/ingest/claude`). The agent runs the
//! materialized `otto-post` helper, which sends `X-Otto-Session` + `X-Otto-Token`.
//! The session's `meta` (set when the swarm spawned it) carries `swarm_id` and
//! `agent_id`. Always returns 204 (fire-and-forget for the agent).

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::Json;
use otto_core::event::Event;
use otto_core::Id;
use otto_state::{NewAttachment, NewMessage};
use serde::Deserialize;
use serde_json::{json, Value};

use crate::design_format::DesignFormat;
use crate::state::ServerCtx;

#[derive(Deserialize)]
pub struct BoardIngestReq {
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub to_agent_id: Option<Id>,
    pub body: String,
}

pub async fn board_ingest(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    Json(req): Json<BoardIngestReq>,
) -> StatusCode {
    let sid: Id = match headers.get("x-otto-session").and_then(|v| v.to_str().ok()) {
        Some(s) => s.to_string(),
        None => return StatusCode::NO_CONTENT,
    };
    let token = headers
        .get("x-otto-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !ctx.manager.verify_ingest_token(&sid, token) {
        return StatusCode::NO_CONTENT;
    }
    let session = match ctx.manager.get(&sid).await {
        Ok(s) => s,
        Err(_) => return StatusCode::NO_CONTENT,
    };
    let meta = &session.meta;
    let swarm_id = meta.get("swarm_id").and_then(Value::as_str);
    let agent_id = meta.get("agent_id").and_then(Value::as_str);
    let (Some(swarm_id), Some(agent_id)) = (swarm_id, agent_id) else {
        return StatusCode::NO_CONTENT; // not a swarm session
    };
    let project_id = meta.get("project_id").and_then(Value::as_str).map(str::to_string);
    let task_id = meta.get("task_id").and_then(Value::as_str).map(str::to_string);
    let run_id = meta.get("run_id").and_then(Value::as_str).map(str::to_string);

    let body = req.body.trim();
    if body.is_empty() {
        return StatusCode::NO_CONTENT;
    }
    let kind = req.kind.unwrap_or_else(|| "message".into());

    match ctx
        .swarm_repo
        .create_message(NewMessage {
            swarm_id: swarm_id.to_string(),
            workspace_id: session.workspace_id.clone(),
            project_id,
            task_id,
            run_id,
            author_agent_id: Some(agent_id.to_string()),
            author_user_id: None,
            to_agent_id: req.to_agent_id,
            kind,
            body: body.to_string(),
            meta: json!({ "session_id": sid }),
        })
        .await
    {
        Ok(msg) => {
            let _ = ctx.events.send(Event::SwarmMessagePosted {
                workspace_id: session.workspace_id.clone(),
                swarm_id: swarm_id.to_string(),
                message: serde_json::to_value(&msg).unwrap_or_default(),
            });
        }
        Err(e) => tracing::warn!("swarm board ingest: {e}"),
    }
    StatusCode::NO_CONTENT
}

/// `POST /ingest/swarm/product` body. The shell flag `--kind` maps to
/// `tree_kind` — the same key `POST …/children` and `PATCH /product/stories/{sid}`
/// use.
#[derive(Deserialize)]
pub struct ProductIngestReq {
    #[serde(default)]
    pub title: Option<String>,
    pub body_md: String,
    /// `doc` (default) | `story`. Ignored for the legacy top-level path.
    #[serde(default)]
    pub tree_kind: Option<String>,
    /// Folder inside the epic; defaults to the publishing agent's role title.
    #[serde(default)]
    pub folder: Option<String>,
}

/// `POST /ingest/swarm/product` — a swarm (PO/feature-design) agent publishes a
/// feature DRAFT to the Product page via the materialized `otto-product` helper.
/// Same per-session auth as the board ingest (auth failures stay a silent 204 so
/// nothing leaks). With a swarm PROJECT in the session meta the draft is filed
/// as a child of the project's epic (§3.1): the project's linked story if set,
/// else ONE `tree_kind:'epic'` draft minted per project. A child whose
/// normalized title already exists under the epic is UPDATED (a new `suggested`
/// version) instead of duplicated. Without a project (a swarm without one) the
/// legacy behaviour — a fresh top-level draft — is unchanged. Bad input
/// (`tree_kind` outside `doc|story`) is a 400 so the helper can report it.
pub async fn product_ingest(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    Json(req): Json<ProductIngestReq>,
) -> StatusCode {
    let sid: Id = match headers.get("x-otto-session").and_then(|v| v.to_str().ok()) {
        Some(s) => s.to_string(),
        None => return StatusCode::NO_CONTENT,
    };
    let token = headers
        .get("x-otto-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !ctx.manager.verify_ingest_token(&sid, token) {
        return StatusCode::NO_CONTENT;
    }
    let session = match ctx.manager.get(&sid).await {
        Ok(s) => s,
        Err(_) => return StatusCode::NO_CONTENT,
    };
    // Only swarm sessions may write drafts.
    if session.meta.get("swarm_id").and_then(Value::as_str).is_none() {
        return StatusCode::NO_CONTENT;
    }
    let body = req.body_md.trim();
    if body.is_empty() {
        return StatusCode::BAD_REQUEST;
    }
    let title = req
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .unwrap_or("Feature draft");
    let tree_kind = match req.tree_kind.as_deref() {
        None => "doc",
        Some(k) => match otto_product::validate_tree_kind(k) {
            Ok("epic") => {
                tracing::warn!("swarm product ingest: --kind epic rejected (children nest one level)");
                return StatusCode::BAD_REQUEST;
            }
            Ok(k) => k,
            Err(e) => {
                tracing::warn!("swarm product ingest: {e}");
                return StatusCode::BAD_REQUEST;
            }
        },
    };
    let project_id = session.meta.get("project_id").and_then(Value::as_str).map(str::to_string);
    let agent_id = session.meta.get("agent_id").and_then(Value::as_str).map(str::to_string);

    // Epic resolution — `None` falls back to the legacy top-level draft.
    let epic_id = match project_id.as_deref() {
        Some(pid) => resolve_epic(&ctx, pid, &session.created_by).await,
        None => None,
    };
    let Some(epic_id) = epic_id else {
        match ctx
            .product
            .create_draft(&session.workspace_id, &session.created_by, Some(title))
            .await
        {
            Ok(detail) => {
                let _ = ctx
                    .product
                    .update_draft_body(&detail.story.id, title, body, &session.created_by)
                    .await;
                let _ = ctx.events.send(Event::ProductChanged {
                    workspace_id: session.workspace_id.clone(),
                    story_id: detail.story.id,
                    section: "source".into(),
                    status: "draft".into(),
                });
            }
            Err(e) => tracing::warn!("swarm product ingest: {e}"),
        }
        return StatusCode::NO_CONTENT;
    };

    // Folder: explicit `--folder`, else the agent's role title (e.g. "Designer").
    let mut folder = req
        .folder
        .as_deref()
        .map(str::trim)
        .filter(|f| !f.is_empty())
        .map(str::to_string);
    if folder.is_none() {
        if let Some(aid) = agent_id.as_deref() {
            folder = ctx
                .swarm_repo
                .get_agent(&aid.to_string())
                .await
                .ok()
                .map(|a| a.title.trim().to_string())
                .filter(|t| !t.is_empty());
        }
    }
    let folder = folder.unwrap_or_default();

    // Title-dedupe under the epic: same normalized title → new `suggested`
    // version on the existing child (the story view picks it up), no new row.
    let children = ctx.product_repo.get_children(&epic_id).await.unwrap_or_default();
    let story_id = if let Some(existing) = find_child_by_title(&children, title) {
        let res = ctx
            .product_repo
            .add_version(otto_state::NewVersion {
                story_id: existing.id.clone(),
                kind: "suggested".into(),
                title: title.to_string(),
                body_md: body.to_string(),
                raw_json: None,
                change_notes: Some("Updated by a swarm agent via otto-product".into()),
                created_by: session.created_by.clone(),
            })
            .await;
        if let Err(e) = res {
            tracing::warn!("swarm product ingest: update child: {e}");
            return StatusCode::NO_CONTENT;
        }
        existing.id.clone()
    } else {
        let epic_ws = match ctx.product_repo.get_story(&epic_id).await {
            Ok(s) => s.workspace_id,
            Err(_) => session.workspace_id.clone(),
        };
        match ctx
            .product
            .create_draft_in_tree(
                &epic_ws,
                &session.created_by,
                Some(title),
                tree_kind,
                Some(&epic_id),
                &folder,
            )
            .await
        {
            Ok(detail) => {
                let _ = ctx
                    .product
                    .update_draft_body(&detail.story.id, title, body, &session.created_by)
                    .await;
                detail.story.id
            }
            Err(e) => {
                tracing::warn!("swarm product ingest: create child: {e}");
                return StatusCode::NO_CONTENT;
            }
        }
    };
    let _ = ctx.events.send(Event::ProductChanged {
        workspace_id: session.workspace_id.clone(),
        story_id,
        section: "tree".into(),
        status: "changed".into(),
    });
    StatusCode::NO_CONTENT
}

/// The epic a swarm project's agents publish into (§3.1 step 2):
/// - `project.story_id` set → that story is the root, UNTOUCHED (the UI shows it
///   as an epic because it has children);
/// - else mint ONE `tree_kind:'epic'` draft named after the project goal and
///   link it with `link_story_if_unlinked` — first writer wins; the loser (or a
///   `Conflict` from the unique `story_id` index) deletes its orphan epic and
///   re-reads the project to file under the winner's epic.
///
/// `None` when the project can't be read (the caller falls back to a top-level
/// draft rather than dropping the agent's work).
async fn resolve_epic(ctx: &ServerCtx, project_id: &str, by: &Id) -> Option<Id> {
    let pid = project_id.to_string();
    let project = match ctx.swarm_repo.get_project(&pid).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("swarm product ingest: project {pid}: {e}");
            return None;
        }
    };
    if let Some(sid) = project.story_id.as_ref() {
        if ctx.product_repo.get_story(sid).await.is_ok() {
            return Some(sid.clone());
        }
        // Linked story was deleted: the link is stale — fall through to mint.
    }
    let title = epic_title(project.name.as_str(), project.goal_md.as_deref());
    let detail = match ctx
        .product
        .create_draft_in_tree(&project.workspace_id, by, Some(&title), "epic", None, "")
        .await
    {
        Ok(d) => d,
        Err(e) => {
            tracing::warn!("swarm product ingest: mint epic: {e}");
            return None;
        }
    };
    let linked = if project.story_id.is_some() {
        // Stale link → replace it outright (no race: the row already had a value).
        ctx.swarm_repo
            .update_project(
                &pid,
                otto_state::ProjectPatch {
                    story_id: Some(Some(detail.story.id.clone())),
                    ..Default::default()
                },
            )
            .await
            .map(|_| true)
    } else {
        ctx.swarm_repo.link_story_if_unlinked(&pid, &detail.story.id).await
    };
    match linked {
        Ok(true) => {
            let _ = ctx.events.send(Event::ProductChanged {
                workspace_id: project.workspace_id.clone(),
                story_id: detail.story.id.clone(),
                section: "tree".into(),
                status: "changed".into(),
            });
            Some(detail.story.id)
        }
        Ok(false) | Err(otto_core::Error::Conflict(_)) => {
            // Lost the race: drop our orphan and use the winner's epic.
            let _ = ctx.product_repo.delete_story(&detail.story.id).await;
            ctx.swarm_repo
                .get_project(&pid)
                .await
                .ok()
                .and_then(|p| p.story_id)
        }
        Err(e) => {
            tracing::warn!("swarm product ingest: link epic: {e}");
            let _ = ctx.product_repo.delete_story(&detail.story.id).await;
            None
        }
    }
}

/// Epic title for a freshly minted project epic: the first non-empty line of
/// the goal (markdown heading marks stripped), else the project name; capped.
fn epic_title(project_name: &str, goal_md: Option<&str>) -> String {
    let from_goal = goal_md
        .and_then(|g| g.lines().map(|l| l.trim().trim_start_matches('#').trim()).find(|l| !l.is_empty()))
        .map(str::to_string)
        .filter(|l| !l.is_empty());
    let raw = from_goal.unwrap_or_else(|| project_name.trim().to_string());
    let raw = if raw.is_empty() { "Swarm project".to_string() } else { raw };
    let mut out: String = raw.chars().take(120).collect();
    if out.len() < raw.len() {
        out.push('…');
    }
    out
}

/// Title normalization for the dedupe: case-insensitive, whitespace-collapsed,
/// trailing punctuation dropped — "Tier ladder  screens." == "tier ladder screens".
fn normalize_title(t: &str) -> String {
    let collapsed = t.split_whitespace().collect::<Vec<_>>().join(" ");
    collapsed
        .trim_end_matches(['.', ':', '!', '…'])
        .trim()
        .to_lowercase()
}

fn find_child_by_title<'a>(
    children: &'a [otto_state::ProductStory],
    title: &str,
) -> Option<&'a otto_state::ProductStory> {
    let want = normalize_title(title);
    if want.is_empty() {
        return None;
    }
    children.iter().find(|c| normalize_title(&c.title) == want)
}

/// Storage sub-path under `data_dir` for story attachments (mirrors
/// `product_media::ATTACH_ROOT`).
const ATTACH_ROOT: &str = "product/attachments";

#[derive(Deserialize)]
pub struct MockupIngestReq {
    pub title: String,
    /// `html` (default) | `mermaid` | `excalidraw` | `scene3d`; anything else → 400.
    #[serde(default)]
    pub format: Option<String>,
    pub content: String,
    /// Optional arena asset group override (`meta_json.group`); defaults per
    /// format (Screens / Diagrams / Boards / 3D).
    #[serde(default)]
    pub folder: Option<String>,
}

/// `POST /ingest/swarm/mockup` — a swarm discovery/design agent publishes a
/// design artifact (HTML screen, Mermaid diagram, Excalidraw board or `scene3d`
/// document). Same per-session auth as the board ingest. The target story is
/// derived server-side — the agent never supplies a story id — as: the
/// project's Discovery run's story **or** `project.story_id` **or** the epic
/// resolved for the project (minted if needed). Unknown formats and invalid
/// `scene3d` documents are a 400; no `project_id` in the session is a 204.
pub async fn ingest_mockup(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    Json(req): Json<MockupIngestReq>,
) -> StatusCode {
    let sid: Id = match headers.get("x-otto-session").and_then(|v| v.to_str().ok()) {
        Some(s) => s.to_string(),
        None => return StatusCode::NO_CONTENT,
    };
    let token = headers
        .get("x-otto-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !ctx.manager.verify_ingest_token(&sid, token) {
        return StatusCode::NO_CONTENT;
    }
    let session = match ctx.manager.get(&sid).await {
        Ok(s) => s,
        Err(_) => return StatusCode::NO_CONTENT,
    };
    let project_id = match session.meta.get("project_id").and_then(Value::as_str) {
        Some(p) => p.to_string(),
        None => return StatusCode::NO_CONTENT, // a swarm without a project
    };

    let title = req.title.trim();
    if title.is_empty() || req.content.is_empty() {
        return StatusCode::BAD_REQUEST;
    }
    let format = match crate::design_format::parse_or_default(req.format.as_deref(), DesignFormat::Html) {
        Ok(f) => f,
        Err(e) => {
            tracing::warn!("swarm mockup ingest: {e}");
            return StatusCode::BAD_REQUEST;
        }
    };
    if format == DesignFormat::Scene3d {
        if let Err(e) = crate::design_scene3d::validate_bytes(req.content.as_bytes()) {
            tracing::warn!("swarm mockup ingest: {e}");
            return StatusCode::BAD_REQUEST;
        }
    }

    // Target: discovery run's story → project.story_id → the project's epic.
    let story_id = match ctx.discovery_repo.get_by_project(&project_id).await {
        Ok(Some(run)) => Some(run.story_id),
        _ => resolve_epic(&ctx, &project_id, &session.created_by).await,
    };
    let Some(story_id) = story_id else {
        tracing::warn!("swarm mockup ingest: no story resolves for project {project_id}");
        return StatusCode::NO_CONTENT;
    };
    let story = match ctx.product_repo.get_story(&story_id).await {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!("swarm mockup ingest: story {story_id}: {e}");
            return StatusCode::NO_CONTENT;
        }
    };

    // Mirror `product_media::upload_attachment`'s storage-path convention:
    // `data_dir/product/attachments/<story_id>/<id><ext>`, with `storage_path`
    // stored RELATIVE to `data_dir`. Story ids are daemon-generated, but they
    // become a path component under the attachments root — confine the joins.
    let id = otto_core::new_id();
    let rel = format!("{ATTACH_ROOT}/{}/{}{}", story.id, id, format.ext());
    let Some(dir) = otto_core::paths::confine_join(&ctx.data_dir.join(ATTACH_ROOT), &story.id) else {
        tracing::warn!("swarm mockup ingest: unsafe story id {:?}", story.id);
        return StatusCode::NO_CONTENT;
    };
    if let Err(e) = tokio::fs::create_dir_all(&dir).await {
        tracing::warn!("swarm mockup ingest: create dir: {e}");
        return StatusCode::NO_CONTENT;
    }
    let Some(full) = otto_core::paths::confine_join(&ctx.data_dir, &rel) else {
        return StatusCode::NO_CONTENT;
    };
    if let Err(e) = tokio::fs::write(&full, req.content.as_bytes()).await {
        tracing::warn!("swarm mockup ingest: write: {e}");
        return StatusCode::NO_CONTENT;
    }
    let size_bytes = req.content.len() as i64;
    let group = req
        .folder
        .as_deref()
        .map(str::trim)
        .filter(|g| !g.is_empty())
        .unwrap_or(format.default_group());

    match ctx
        .attachment_repo
        .create(NewAttachment {
            story_id: story.id.clone(),
            workspace_id: story.workspace_id.clone(),
            filename: crate::product_media::sanitize_filename(&format!("{title}{}", format.ext())),
            mime: format.mime().into(),
            size_bytes,
            sha256: None,
            storage_path: rel,
            kind: format.attachment_kind().into(),
            source: "agent".into(),
            meta_json: Some(json!({ "format": format, "group": group }).to_string()),
            created_by: session.created_by.clone(),
        })
        .await
    {
        Ok(att) => {
            let _ = ctx.events.send(Event::MockupUpdated {
                workspace_id: story.workspace_id.clone(),
                story_id: story.id.clone(),
                attachment_id: att.id,
                format: format.to_string(),
                content: Some(req.content),
            });
        }
        Err(e) => tracing::warn!("swarm mockup ingest: {e}"),
    }
    StatusCode::NO_CONTENT
}

#[derive(Deserialize)]
pub struct DiscoveryReportIngestReq {
    pub report_md: String,
}

/// `POST /ingest/swarm/discovery-report` — a swarm discovery agent publishes the
/// consolidated discovery report for the story under discovery. Same per-session
/// auth as the board ingest; the target run is derived from the session's
/// `meta.project_id`. Always 204 (fire-and-forget).
pub async fn ingest_discovery_report(
    State(ctx): State<ServerCtx>,
    headers: HeaderMap,
    Json(req): Json<DiscoveryReportIngestReq>,
) -> StatusCode {
    let sid: Id = match headers.get("x-otto-session").and_then(|v| v.to_str().ok()) {
        Some(s) => s.to_string(),
        None => return StatusCode::NO_CONTENT,
    };
    let token = headers
        .get("x-otto-token")
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default();
    if !ctx.manager.verify_ingest_token(&sid, token) {
        return StatusCode::NO_CONTENT;
    }
    let session = match ctx.manager.get(&sid).await {
        Ok(s) => s,
        Err(_) => return StatusCode::NO_CONTENT,
    };
    let project_id = match session.meta.get("project_id").and_then(Value::as_str) {
        Some(p) => p.to_string(),
        None => return StatusCode::NO_CONTENT,
    };
    let run = match ctx.discovery_repo.get_by_project(&project_id).await {
        Ok(Some(r)) => r,
        _ => return StatusCode::NO_CONTENT,
    };

    let report = req.report_md.trim();
    if report.is_empty() {
        return StatusCode::NO_CONTENT;
    }
    if let Err(e) = ctx.discovery_repo.set_report(&run.id, report).await {
        tracing::warn!("swarm discovery report ingest: {e}");
    }
    StatusCode::NO_CONTENT
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_mockup_format_is_rejected_not_html() {
        // Previously `svg` / `other` silently became HTML; now the ingest answers 400.
        for bad in ["svg", "other", "python"] {
            assert!(bad.parse::<DesignFormat>().is_err(), "{bad}");
        }
        assert_eq!("mermaid".parse::<DesignFormat>().unwrap().ext(), ".mmd");
        assert_eq!("html".parse::<DesignFormat>().unwrap().mime(), "text/html");
        assert_eq!(
            crate::design_format::parse_or_default(None, DesignFormat::Html).unwrap(),
            DesignFormat::Html
        );
    }

    #[test]
    fn epic_title_prefers_goal_heading_then_name() {
        assert_eq!(epic_title("proj", Some("# Loyalty programme\n\nDetails…")), "Loyalty programme");
        assert_eq!(epic_title("proj", Some("\n\n  ")), "proj");
        assert_eq!(epic_title("proj", None), "proj");
        assert_eq!(epic_title("  ", None), "Swarm project");
        let long = "x".repeat(300);
        let t = epic_title("p", Some(&long));
        assert!(t.chars().count() <= 121 && t.ends_with('…'));
    }

    #[test]
    fn title_dedupe_is_normalized() {
        assert_eq!(normalize_title("  Tier   ladder screens. "), "tier ladder screens");
        assert_eq!(normalize_title("TIER LADDER SCREENS"), "tier ladder screens");
        assert_eq!(normalize_title("..."), "");
        let mk = |title: &str| otto_state::ProductStory {
            id: otto_core::new_id(),
            workspace_id: "w".into(),
            source_kind: "draft".into(),
            account_id: String::new(),
            source_key: String::new(),
            title: title.into(),
            url: String::new(),
            issue_type: None,
            stage: "draft".into(),
            cwd: None,
            watch_enabled: false,
            watch_cadence_min: 15,
            watch_cursor: None,
            confluence_tests_page_id: None,
            confluence_tests_url: None,
            tags: String::new(),
            parent_id: Some("epic".into()),
            tree_kind: "doc".into(),
            folder: "Design".into(),
            created_by: "u".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
        };
        let kids = vec![mk("Tier ladder screens"), mk("Rewards kiosk")];
        assert_eq!(find_child_by_title(&kids, "tier ladder  screens.").map(|c| c.id.as_str()), Some(kids[0].id.as_str()));
        assert!(find_child_by_title(&kids, "Something new").is_none());
        assert!(find_child_by_title(&kids, "").is_none());
    }
}
