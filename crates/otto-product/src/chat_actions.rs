//! The story Chat's action cards: questions, notes and a design diagram the
//! agent proposes and a person applies one card at a time.
//!
//! One implementation, two routes: the merged Chat
//! (`POST /product/refinement-threads/{tid}/apply`, see `refine`) and the
//! legacy Discovery Chat route (`POST /product/discovery-chats/{cid}/apply`,
//! see `chat`), kept so an old client still lands on the same rules.
//!
//! A proposed story body is NOT a card. The Chat writes it as a `suggested`
//! version (`ProductService::propose_story_body`), reviewed and published
//! from Rewrite; an `apply_draft` that still reaches this module (an old
//! client, an agent that ignored the contract) follows the same rule and
//! never edits a draft in place.

use otto_core::{Error, Id, Result};
use otto_state::{CanvasRepo, NewEvent, NewNote, NewQuestion, NewScene};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use crate::service::ProductService;

/// The action kinds a Chat turn may carry as cards.
pub const CARD_ACTIONS: [&str; 3] = ["add_questions", "add_notes", "create_canvas"];

/// Body of an apply request: the action object the agent emitted (tolerant,
/// dispatched by its `type`).
#[derive(Debug, Deserialize)]
pub struct ApplyReq {
    pub action: Value,
}

/// What applying one action changed.
#[derive(Debug, Default, Serialize)]
pub struct ApplyResult {
    /// A `suggested` version was written (an `apply_draft` only).
    pub story_updated: bool,
    /// The suggested version's number when `story_updated`.
    pub version_no: Option<i64>,
    pub created_question_ids: Vec<Id>,
    pub created_note_ids: Vec<Id>,
    pub canvas_id: Option<Id>,
}

/// The story an action lands on and the person applying it.
pub struct ActionTarget<'a> {
    pub story_id: &'a Id,
    pub workspace_id: &'a Id,
    pub user_id: &'a Id,
}

/// Keep only the card actions from an agent's `actions` array (unknown kinds
/// and non-objects are dropped, so the UI never renders a card it cannot
/// apply).
pub fn card_actions(actions: &[Value]) -> Vec<Value> {
    actions
        .iter()
        .filter(|a| {
            a.get("type")
                .and_then(Value::as_str)
                .is_some_and(|t| CARD_ACTIONS.contains(&t))
        })
        .cloned()
        .collect()
}

/// Apply ONE action (dispatched by its `type`). The caller has checked the
/// person may edit the story.
pub async fn apply_story_action(
    svc: &ProductService,
    canvas: &CanvasRepo,
    target: &ActionTarget<'_>,
    action: &Value,
) -> Result<ApplyResult> {
    let kind = action.get("type").and_then(Value::as_str).unwrap_or("");
    let mut result = ApplyResult::default();
    match kind {
        "apply_draft" => {
            let body_md = action.get("body_md").and_then(Value::as_str).unwrap_or("");
            let title = action.get("title").and_then(Value::as_str);
            let version = svc
                .propose_story_body(
                    target.story_id,
                    title,
                    body_md,
                    Some("Proposed in Chat".into()),
                    target.user_id,
                )
                .await?;
            svc.repo
                .add_event(NewEvent {
                    story_id: target.story_id.clone(),
                    section: "refine".into(),
                    kind: "refined".into(),
                    summary: format!("Suggestion added from Chat: {}", version.title),
                    actor_id: Some(target.user_id.clone()),
                    meta_json: Some(json!({ "version_no": version.version_no }).to_string()),
                })
                .await?;
            result.story_updated = true;
            result.version_no = Some(version.version_no);
        }
        "add_questions" => {
            for q in action
                .get("questions")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                let text = q.get("text").and_then(Value::as_str).unwrap_or("");
                if text.trim().is_empty() {
                    continue;
                }
                let field = |k: &str, default: &str| {
                    q.get(k)
                        .and_then(Value::as_str)
                        .unwrap_or(default)
                        .to_string()
                };
                let created = svc
                    .repo
                    .create_question(NewQuestion {
                        story_id: target.story_id.clone(),
                        analysis_id: None,
                        text: text.to_string(),
                        rationale: field("rationale", ""),
                        category: field("category", "other"),
                        created_by: target.user_id.clone(),
                    })
                    .await?;
                result.created_question_ids.push(created.id);
            }
        }
        "add_notes" => {
            for n in action
                .get("notes")
                .and_then(Value::as_array)
                .map(Vec::as_slice)
                .unwrap_or_default()
            {
                let body = n.get("body").and_then(Value::as_str).unwrap_or("");
                if body.trim().is_empty() {
                    continue;
                }
                let created = svc
                    .repo
                    .create_note(NewNote {
                        story_id: target.story_id.clone(),
                        section: Some("discovery".into()),
                        body: body.to_string(),
                        author_id: target.user_id.clone(),
                    })
                    .await?;
                result.created_note_ids.push(created.id);
            }
        }
        "create_canvas" => {
            result.canvas_id = Some(create_design_from_action(canvas, target, action).await?);
        }
        other => {
            return Err(Error::Invalid(format!("unknown chat action type: {other}")));
        }
    }
    Ok(result)
}

/// THE one place a Chat diagram proposal becomes a design. Today it is a
/// Canvas scene linked to the story; when Canvas folds into Design Hall this
/// is the single function to re-point (the result stays the new item's id).
///
/// The route is gated on Product (the `/product/` policy prefix), so a
/// Product Editor creates the scene as a byproduct of the chat; the Canvas
/// capability decides whether they can then open it.
pub async fn create_design_from_action(
    canvas: &CanvasRepo,
    target: &ActionTarget<'_>,
    action: &Value,
) -> Result<Id> {
    let title = action
        .get("title")
        .and_then(Value::as_str)
        .filter(|t| !t.trim().is_empty())
        .unwrap_or("Discovery diagram")
        .to_string();
    let doc = canvas_doc_from_action(action, &title);
    let scene = canvas
        .create(NewScene {
            workspace_id: target.workspace_id.clone(),
            story_id: Some(target.story_id.clone()),
            title,
            doc_json: doc.to_string(),
            provider: "claude".into(),
            section: None,
            created_by: target.user_id.clone(),
        })
        .await?;
    Ok(scene.id)
}

/// Build a Scene doc from a `create_canvas` action (mermaid or nodes) in the
/// FILE-BACKED canvas shape (`{type:"otto-canvas", version:1, format, source}`)
/// — the only shape Canvas Studio renders. The legacy Svelte-Flow doc
/// (`{schema:1, nodes:[{kind:"mermaid",…}]}`) this used to emit has no
/// top-level `source`, so every "Open in Canvas" from the chat landed on a
/// permanently blank board.
fn canvas_doc_from_action(action: &Value, title: &str) -> Value {
    let source = match action.get("mermaid").and_then(|m| m.as_str()) {
        Some(src) => src.to_string(),
        // Structured nodes/edges → synthesize a mermaid flowchart so the
        // proposed diagram still lands editable instead of being dropped.
        None => {
            let list = |k: &str| {
                action
                    .get(k)
                    .and_then(Value::as_array)
                    .cloned()
                    .unwrap_or_default()
            };
            mermaid_from_nodes(&list("nodes"), &list("edges"))
        }
    };
    json!({
        "type": "otto-canvas",
        "version": 1,
        "format": "mermaid",
        "title": title,
        "source": source,
    })
}

/// Best-effort mermaid flowchart from a `create_canvas` action's node/edge
/// lists. Node ids are sanitized to mermaid-safe tokens; labels are quoted.
fn mermaid_from_nodes(nodes: &[Value], edges: &[Value]) -> String {
    let sid = |raw: &str| -> String {
        let s: String = raw
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
            .collect();
        if s.is_empty() {
            "n".to_string()
        } else {
            s
        }
    };
    let mut out = String::from("flowchart TD\n");
    for (i, n) in nodes.iter().enumerate() {
        let id = n
            .get("id")
            .and_then(Value::as_str)
            .map(sid)
            .unwrap_or_else(|| format!("n{i}"));
        let label = n
            .get("label")
            .or_else(|| n.get("title"))
            .or_else(|| n.get("text"))
            .and_then(Value::as_str)
            .unwrap_or(&id);
        out.push_str(&format!("  {id}[\"{}\"]\n", label.replace('"', "'")));
    }
    for e in edges {
        let from = e
            .get("from")
            .or_else(|| e.get("source"))
            .and_then(Value::as_str);
        let to = e
            .get("to")
            .or_else(|| e.get("target"))
            .and_then(Value::as_str);
        if let (Some(f), Some(t)) = (from, to) {
            out.push_str(&format!("  {} --> {}\n", sid(f), sid(t)));
        }
    }
    out
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::Arc;

    use chrono::Utc;
    use otto_core::new_id;
    use otto_core::secrets::SecretStore;
    use otto_state::{DbPool, IssuesRepo, ProductRepo};

    use super::*;

    #[test]
    fn card_actions_keep_only_appliable_kinds() {
        let raw = vec![
            json!({"type":"add_questions","questions":[]}),
            json!({"type":"apply_draft","title":"T","body_md":"B"}),
            json!({"type":"launch_rockets"}),
            json!("not an object"),
            json!({"type":"add_notes","notes":[]}),
            json!({"type":"create_canvas","title":"D"}),
        ];
        let kept: Vec<_> = card_actions(&raw)
            .iter()
            .map(|a| a["type"].as_str().unwrap().to_string())
            .collect();
        assert_eq!(kept, ["add_questions", "add_notes", "create_canvas"]);
    }

    #[test]
    fn canvas_doc_from_mermaid_action() {
        // Regression: must be the FILE-BACKED shape (format+source) — the
        // legacy nodes[] shape rendered a permanently blank canvas.
        let action =
            json!({"type":"create_canvas","title":"Seq","mermaid":"sequenceDiagram\n A->>B: x"});
        let doc = canvas_doc_from_action(&action, "Seq");
        assert_eq!(doc["type"], "otto-canvas");
        assert_eq!(doc["format"], "mermaid");
        assert!(doc["source"].as_str().unwrap().contains("A->>B"));
    }

    #[test]
    fn canvas_doc_from_nodes_action_synthesizes_mermaid() {
        let action = json!({
            "type": "create_canvas",
            "nodes": [
                {"id": "a", "label": "Start"},
                {"id": "b-2", "label": "End \"quoted\""},
            ],
            "edges": [{"from": "a", "to": "b-2"}],
        });
        let doc = canvas_doc_from_action(&action, "Flow");
        assert_eq!(doc["format"], "mermaid");
        let src = doc["source"].as_str().unwrap();
        assert!(src.starts_with("flowchart TD"));
        assert!(src.contains("a[\"Start\"]"));
        assert!(
            src.contains("b_2[\"End 'quoted'\"]"),
            "sanitized id + quotes: {src}"
        );
        assert!(src.contains("a --> b_2"));
    }

    // -----------------------------------------------------------------------
    // Applying against a real schema
    // -----------------------------------------------------------------------

    struct NoSecrets;
    impl SecretStore for NoSecrets {
        fn put(&self, _k: &str, _v: &str) -> otto_core::Result<()> {
            Ok(())
        }
        fn get(&self, _k: &str) -> otto_core::Result<Option<String>> {
            Ok(None)
        }
        fn delete(&self, _k: &str) -> otto_core::Result<()> {
            Ok(())
        }
    }

    async fn mem_pool() -> DbPool {
        let opts = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(true);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(opts)
            .await
            .unwrap();
        sqlx::migrate!("../otto-state/migrations")
            .run(&pool)
            .await
            .unwrap();
        pool.into()
    }

    /// A user, a workspace and a local draft story: `(svc, canvas, user, ws, story)`.
    async fn fixture() -> (ProductService, CanvasRepo, Id, Id, Id) {
        let (svc, pool, user, ws, sid) = fixture_with_pool().await;
        (svc, CanvasRepo::new(pool), user, ws, sid)
    }

    /// [`fixture`] with the pool itself, for tests that need another repo:
    /// `(svc, pool, user, ws, story)`.
    pub(crate) async fn fixture_with_pool() -> (ProductService, DbPool, Id, Id, Id) {
        let pool = mem_pool().await;
        let now = Utc::now().to_rfc3339();
        let user = new_id();
        sqlx::query(
            "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
             VALUES (?, 'chatter', 'hash', 'Chatter', 0, ?)",
        )
        .bind(&user)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        let ws = new_id();
        sqlx::query(
            "INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, 'ws', '/tmp', ?)",
        )
        .bind(&ws)
        .bind(&now)
        .execute(&pool)
        .await
        .unwrap();
        let secrets: Arc<dyn SecretStore> = Arc::new(NoSecrets);
        let svc = ProductService::new(
            ProductRepo::new(pool.clone()),
            IssuesRepo::new(pool.clone()),
            secrets,
        );
        let story = svc.create_draft(&ws, &user, Some("D")).await.unwrap();
        (svc, pool, user, ws, story.story.id)
    }

    #[tokio::test]
    async fn questions_and_notes_land_on_the_story_and_blank_items_are_skipped() {
        let (svc, canvas, user, ws, sid) = fixture().await;
        let target = ActionTarget {
            story_id: &sid,
            workspace_id: &ws,
            user_id: &user,
        };
        let q = json!({"type":"add_questions","questions":[
            {"text":"What is the lockout policy?","category":"security"},
            {"text":"   "}
        ]});
        let r = apply_story_action(&svc, &canvas, &target, &q)
            .await
            .unwrap();
        assert_eq!(r.created_question_ids.len(), 1);
        let n = json!({"type":"add_notes","notes":[{"body":"billing has a similar flow"}]});
        let r = apply_story_action(&svc, &canvas, &target, &n)
            .await
            .unwrap();
        assert_eq!(r.created_note_ids.len(), 1);
        assert!(!r.story_updated);

        let questions = svc.repo.list_questions(&sid).await.unwrap();
        assert_eq!(questions.len(), 1);
        assert_eq!(questions[0].category, "security");
        assert_eq!(svc.repo.list_notes(&sid).await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn a_diagram_becomes_a_design_linked_to_the_story() {
        let (svc, canvas, user, ws, sid) = fixture().await;
        let target = ActionTarget {
            story_id: &sid,
            workspace_id: &ws,
            user_id: &user,
        };
        let a = json!({"type":"create_canvas","title":"Login","mermaid":"flowchart TD\n A-->B"});
        let r = apply_story_action(&svc, &canvas, &target, &a)
            .await
            .unwrap();
        assert!(r.canvas_id.is_some());
    }

    #[tokio::test]
    async fn an_apply_draft_becomes_a_suggestion_and_an_unknown_kind_is_refused() {
        let (svc, canvas, user, ws, sid) = fixture().await;
        svc.update_draft_body(&sid, "D", "My own text", &user)
            .await
            .unwrap();
        let target = ActionTarget {
            story_id: &sid,
            workspace_id: &ws,
            user_id: &user,
        };
        let a = json!({"type":"apply_draft","title":"Agent","body_md":"Agent body"});
        let r = apply_story_action(&svc, &canvas, &target, &a)
            .await
            .unwrap();
        assert!(r.story_updated);
        assert!(r.version_no.is_some());
        // The person's draft is untouched; the agent's body is a suggestion.
        let detail = svc.story_detail(&sid).await.unwrap();
        assert_eq!(detail.source.unwrap().body_md, "My own text");
        let suggested = svc
            .repo
            .latest_version_of_kind(&sid, "suggested")
            .await
            .unwrap()
            .expect("suggested version");
        assert_eq!(suggested.body_md, "Agent body");

        let err = apply_story_action(&svc, &canvas, &target, &json!({"type":"nope"}))
            .await
            .expect_err("unknown kind refused");
        assert!(matches!(err, Error::Invalid(_)), "got {err:?}");
    }
}
