//! What the swarm ingest handlers write once the token gate has passed
//! (`routes/swarm_ingest.rs`: `/ingest/swarm/product`, `/mockup`,
//! `/discovery-report`). `public_entry_gates` pins the gate itself; this suite
//! pins the bodies behind it, each with the session's OWN token so the gate is
//! never the reason a post is refused:
//!
//! - a valid post writes exactly the expected rows (and the mockup's file);
//! - an oversized or malformed body writes nothing and returns today's status
//!   (413 from the default 2 MiB body limit, 400 for bad JSON or a value the
//!   handler rejects, 422 for a missing field, 204 for an empty report);
//! - a session whose meta names another workspace's project (and through it
//!   that workspace's story and discovery run) writes nothing (S4-03).
//!
//! Pinned as the behaviour is today; a change here is a wire change. Requests
//! go through the full production router in-process (`oneshot`), so an
//! oversized body's early 413 can't race a socket reset.
use axum::body::Body;
use axum::extract::{ConnectInfo, Request};
use axum::Router;
use otto_core::domain::{SessionKind, User, Workspace};
use otto_core::Id;
use otto_server::ServerCtx;
use otto_state::{NewAgent, NewDiscoveryRun, NewProject, NewSwarm, UsersRepo};
use serde_json::{json, Value};
use tower::ServiceExt;

/// One workspace's swarm: an agent, a project, and a story under discovery
/// (a discovery run on the project).
struct Team {
    ws: Workspace,
    swarm: Id,
    agent: Id,
    project: Id,
    story: Id,
    run: Id,
}

struct World {
    ctx: ServerCtx,
    app: Router,
    root: User,
    a: Team,
    b: Team,
    tmp: tempfile::TempDir,
}

async fn team(ctx: &ServerCtx, root: &User, dir: &str, name: &str) -> Team {
    let ws = ctx.workspaces.create(name, dir, &root.id).await.unwrap();
    let swarm = ctx
        .swarm_repo
        .create_swarm(NewSwarm {
            workspace_id: ws.id.clone(),
            name: format!("{name} team"),
            description: String::new(),
            preset_slug: None,
            config: json!({}),
            max_total_runs: None,
            max_cost_usd: None,
            max_runtime_secs: None,
            max_attempts: None,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();
    let agent = ctx
        .swarm_repo
        .create_agent(NewAgent {
            swarm_id: swarm.id.clone(),
            workspace_id: ws.id.clone(),
            name: "po".into(),
            title: "Product owner".into(),
            reports_to: None,
            provider: "claude".into(),
            model: None,
            soul_name: None,
            soul_md: None,
            specialization: String::new(),
            scope_md: String::new(),
            skills: json!([]),
            schedule: None,
            cwd_mode: None,
            avatar: String::new(),
            order_idx: 0,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();
    let project = ctx
        .swarm_repo
        .create_project(NewProject {
            swarm_id: swarm.id.clone(),
            workspace_id: ws.id.clone(),
            name: "Checkout".into(),
            description: String::new(),
            repo_path: None,
            goal_md: Some("# Faster checkout\nmore".into()),
            story_id: None,
            order_idx: 0,
            created_by: root.id.clone(),
        })
        .await
        .unwrap();
    let story = ctx
        .product
        .create_draft(&ws.id, &root.id, Some("Under discovery"))
        .await
        .unwrap()
        .story;
    let run = ctx
        .discovery_repo
        .create(NewDiscoveryRun {
            story_id: story.id.clone(),
            workspace_id: ws.id.clone(),
            swarm_id: swarm.id.clone(),
            project_id: project.id.clone(),
            brief_md: String::new(),
            created_by: root.id.clone(),
        })
        .await
        .unwrap();
    Team {
        ws,
        swarm: swarm.id,
        agent: agent.id,
        project: project.id,
        story: story.id,
        run: run.id,
    }
}

async fn world() -> World {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = otto_server::test_support::mem_pool().await;
    let ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let root = UsersRepo::new(pool.clone())
        .create("root", "unused", "Root", true)
        .await
        .unwrap();
    let dir = tmp.path().to_string_lossy().to_string();
    let a = team(&ctx, &root, &dir, "A").await;
    let b = team(&ctx, &root, &dir, "B").await;
    let (api_extras, root_extras) = otto_server::modules::module_routers(&ctx);
    let app = otto_server::build_router(ctx.clone(), api_extras, root_extras);
    World {
        ctx,
        app,
        root,
        a,
        b,
        tmp,
    }
}

/// Every row the three handlers can write, read straight from the stores.
#[derive(Debug, PartialEq)]
struct Rows {
    stories: i64,
    versions: i64,
    attachments: i64,
    reports: (Option<String>, Option<String>),
    projects_linked: (Option<Id>, Option<Id>),
}

impl World {
    /// A swarm agent session in workspace A with `meta`, and its own token.
    async fn session(&self, meta: Value) -> (Id, String) {
        let s = otto_state::SessionsRepo::new(self.ctx.pool.clone())
            .create(otto_state::NewSession {
                workspace_id: self.a.ws.id.clone(),
                kind: SessionKind::Agent,
                provider: "claude".into(),
                title: "po".into(),
                cwd: self.tmp.path().to_string_lossy().to_string(),
                provider_session_id: None,
                connection_id: None,
                created_by: self.root.id.clone(),
                meta,
            })
            .await
            .unwrap();
        let token = self.ctx.manager.mint_ingest_token(&s.id);
        (s.id, token)
    }

    /// A workspace-A session whose stored meta is `meta` as written straight
    /// to the row: `SessionsRepo` refuses a foreign `project_id` on create and
    /// on patch, so a stale or forged row is the case the ingest handlers'
    /// own scope check (S4-03) is the last defence for.
    async fn forged_session(&self, meta: Value) -> (Id, String) {
        let (sid, token) = self.session(json!({})).await;
        sqlx::query("UPDATE sessions SET meta_json = ? WHERE id = ?")
            .bind(meta.to_string())
            .bind(&sid)
            .execute(&self.ctx.pool)
            .await
            .unwrap();
        let stored = self.ctx.manager.get(&sid).await.unwrap().meta;
        assert_eq!(stored, meta, "the handlers read this meta");
        (sid, token)
    }

    /// The full versions of `story` (`list_versions` leaves bodies empty).
    async fn versions(&self, story: &Id) -> Vec<otto_state::ProductStoryVersion> {
        let mut out = Vec::new();
        for v in self.ctx.product_repo.list_versions(story).await.unwrap() {
            out.push(self.ctx.product_repo.get_version(&v.id).await.unwrap());
        }
        out
    }

    /// Team A's own agent session.
    async fn own_session(&self) -> (Id, String) {
        self.session(json!({
            "swarm_id": self.a.swarm,
            "agent_id": self.a.agent,
            "workspace_id": self.a.ws.id,
            "project_id": self.a.project,
        }))
        .await
    }

    async fn post(&self, route: &str, (sid, token): &(Id, String), body: Vec<u8>) -> u16 {
        let mut req = Request::builder()
            .method("POST")
            .uri(format!("http://127.0.0.1:7700/api/v1/ingest/swarm/{route}"))
            .header("host", "127.0.0.1:7700")
            .header("x-otto-session", sid.as_str())
            .header("x-otto-token", token.as_str())
            .header("content-type", "application/json")
            .body(Body::from(body))
            .unwrap();
        // What `into_make_service_with_connect_info` gives a loopback caller.
        req.extensions_mut()
            .insert(ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                50_000,
            ))));
        self.app
            .clone()
            .oneshot(req)
            .await
            .unwrap()
            .status()
            .as_u16()
    }

    async fn post_json(&self, route: &str, who: &(Id, String), body: Value) -> u16 {
        self.post(route, who, serde_json::to_vec(&body).unwrap())
            .await
    }

    async fn report(&self, run: &Id) -> Option<String> {
        let run = self.ctx.discovery_repo.get(run).await.unwrap();
        run.unwrap().report_md
    }

    async fn linked(&self, project: &Id) -> Option<Id> {
        let project = self.ctx.swarm_repo.get_project(project).await.unwrap();
        project.story_id
    }

    async fn rows(&self) -> Rows {
        // Fixed statements (sqlx 0.9 refuses an unaudited `format!` query).
        let count = |sql: &'static str| {
            let pool = self.ctx.pool.clone();
            async move {
                sqlx::query_scalar::<_, i64>(sql)
                    .fetch_one(&pool)
                    .await
                    .unwrap()
            }
        };
        Rows {
            stories: count("SELECT COUNT(*) FROM product_stories").await,
            versions: count("SELECT COUNT(*) FROM product_story_versions").await,
            attachments: count("SELECT COUNT(*) FROM product_attachments").await,
            reports: (
                self.report(&self.a.run).await,
                self.report(&self.b.run).await,
            ),
            projects_linked: (
                self.linked(&self.a.project).await,
                self.linked(&self.b.project).await,
            ),
        }
    }
}

/// 2.5 MiB of JSON: past axum's default 2 MiB body limit.
fn oversized(field: &str, other: Value) -> Vec<u8> {
    let mut body = other;
    body[field] = json!("x".repeat(5 * 512 * 1024));
    serde_json::to_vec(&body).unwrap()
}

/// A valid product post on a project with no linked story mints ONE epic
/// (titled from the goal's first line), links it to the project, and files the
/// draft under it in the agent's role folder; the same title again (case and
/// trailing punctuation aside) adds a `suggested` version, not a second child.
#[tokio::test]
async fn product_post_files_one_child_under_a_minted_epic_and_dedupes_by_title() {
    let w = world().await;
    let me = w.own_session().await;
    let before = w.rows().await;
    let status = w
        .post_json(
            "product",
            &me,
            json!({"title": "Tier ladder", "body_md": "  # Ladder\nsteps  "}),
        )
        .await;
    assert_eq!(status, 204);
    let after = w.rows().await;
    assert_eq!(after.stories, before.stories + 2, "the epic and its child");
    assert_eq!(after.attachments, before.attachments);
    assert_eq!(after.reports, before.reports);
    let epic_id = after
        .projects_linked
        .0
        .clone()
        .expect("the project links its epic");
    assert_eq!(after.projects_linked.1, None, "B's project is untouched");
    let epic = w.ctx.product_repo.get_story(&epic_id).await.unwrap();
    assert_eq!(
        (epic.title.as_str(), epic.tree_kind.as_str()),
        ("Faster checkout", "epic")
    );
    assert_eq!(epic.workspace_id, w.a.ws.id);
    let children = w.ctx.product_repo.get_children(&epic_id).await.unwrap();
    assert_eq!(children.len(), 1, "{children:?}");
    let child = &children[0];
    assert_eq!(
        (
            child.title.as_str(),
            child.tree_kind.as_str(),
            child.folder.as_str()
        ),
        ("Tier ladder", "doc", "Product owner")
    );
    let versions = w.versions(&child.id).await;
    assert!(
        versions.iter().any(|v| v.body_md == "# Ladder\nsteps"),
        "the trimmed body is the draft: {versions:?}"
    );

    let status = w
        .post_json(
            "product",
            &me,
            json!({"title": "tier  LADDER.", "body_md": "# v2", "tree_kind": "story"}),
        )
        .await;
    assert_eq!(status, 204);
    let again = w.rows().await;
    assert_eq!(again.stories, after.stories, "deduped: no new row");
    assert_eq!(
        again.versions,
        after.versions + 1,
        "exactly one new version"
    );
    let latest = w.versions(&child.id).await;
    assert!(
        latest
            .iter()
            .any(|v| v.kind == "suggested" && v.body_md == "# v2"),
        "{latest:?}"
    );
}

/// Bad product bodies write nothing: an empty body or a `tree_kind` of `epic`
/// or an unknown kind is a 400, bad JSON a 400, a missing `body_md` a 422 and
/// a body past the limit a 413.
#[tokio::test]
async fn product_post_with_a_bad_body_writes_nothing() {
    let w = world().await;
    let me = w.own_session().await;
    let before = w.rows().await;
    let cases: [(&str, Vec<u8>, u16); 6] = [
        (
            "an empty body",
            serde_json::to_vec(&json!({"title": "T", "body_md": "  \n "})).unwrap(),
            400,
        ),
        (
            "kind epic",
            serde_json::to_vec(&json!({"body_md": "b", "tree_kind": "epic"})).unwrap(),
            400,
        ),
        (
            "an unknown kind",
            serde_json::to_vec(&json!({"body_md": "b", "tree_kind": "folder"})).unwrap(),
            400,
        ),
        ("bad JSON", b"{\"body_md\": ".to_vec(), 400),
        (
            "no body_md",
            serde_json::to_vec(&json!({"title": "T"})).unwrap(),
            422,
        ),
        ("an oversized body", oversized("body_md", json!({})), 413),
    ];
    for (label, body, want) in cases {
        assert_eq!(w.post("product", &me, body).await, want, "{label}");
        assert_eq!(w.rows().await, before, "{label} wrote something");
    }
}

/// A valid mockup lands on the story under discovery (the project's run, not
/// a minted epic): one attachment row in that story's workspace, sourced
/// `agent`, in the format's default group, with the content on disk at its
/// relative storage path.
#[tokio::test]
async fn mockup_post_attaches_one_file_to_the_story_under_discovery() {
    let w = world().await;
    let me = w.own_session().await;
    let before = w.rows().await;
    let status = w
        .post_json(
            "mockup",
            &me,
            json!({"title": " Pricing screen ", "content": "<p>tiers</p>"}),
        )
        .await;
    assert_eq!(status, 204);
    let after = w.rows().await;
    assert_eq!(after.attachments, before.attachments + 1);
    assert_eq!(after.stories, before.stories, "no epic is minted");
    assert_eq!(after.projects_linked, before.projects_linked);
    let atts = w
        .ctx
        .attachment_repo
        .list_for_story(&w.a.story)
        .await
        .unwrap();
    assert_eq!(atts.len(), 1, "{atts:?}");
    let att = &atts[0];
    assert_eq!(att.workspace_id, w.a.ws.id);
    assert_eq!(
        (
            att.filename.as_str(),
            att.kind.as_str(),
            att.source.as_str()
        ),
        ("Pricing screen.html", "mockup", "agent")
    );
    assert_eq!(att.size_bytes, 12);
    let meta: Value = serde_json::from_str(att.meta_json.as_deref().unwrap()).unwrap();
    assert_eq!(meta, json!({"format": "html", "group": "Screens"}));
    assert!(
        att.storage_path
            .starts_with(&format!("product/attachments/{}/", w.a.story)),
        "{}",
        att.storage_path
    );
    let on_disk = std::fs::read_to_string(w.ctx.data_dir.join(&att.storage_path)).unwrap();
    assert_eq!(on_disk, "<p>tiers</p>");
}

/// Bad mockup bodies write nothing — no row and no file.
#[tokio::test]
async fn mockup_post_with_a_bad_body_writes_nothing() {
    let w = world().await;
    let me = w.own_session().await;
    let before = w.rows().await;
    let cases: [(&str, Vec<u8>, u16); 7] = [
        (
            "an empty title",
            serde_json::to_vec(&json!({"title": "  ", "content": "<p/>"})).unwrap(),
            400,
        ),
        (
            "empty content",
            serde_json::to_vec(&json!({"title": "T", "content": ""})).unwrap(),
            400,
        ),
        (
            "an unknown format",
            serde_json::to_vec(&json!({"title": "T", "format": "pdf", "content": "x"})).unwrap(),
            400,
        ),
        (
            "an invalid scene3d document",
            serde_json::to_vec(&json!({"title": "T", "format": "scene3d", "content": "{}"}))
                .unwrap(),
            400,
        ),
        ("bad JSON", b"{\"title\": \"T\", ".to_vec(), 400),
        (
            "no content",
            serde_json::to_vec(&json!({"title": "T"})).unwrap(),
            422,
        ),
        (
            "an oversized body",
            oversized("content", json!({"title": "T"})),
            413,
        ),
    ];
    for (label, body, want) in cases {
        assert_eq!(w.post("mockup", &me, body).await, want, "{label}");
        assert_eq!(w.rows().await, before, "{label} wrote something");
    }
    let attach_root = w.ctx.data_dir.join("product/attachments");
    let files = std::fs::read_dir(&attach_root).map_or(0, |d| d.count());
    assert_eq!(files, 0, "no attachment directory or file was written");
}

/// The report lands on the project's own run, trimmed; a blank report is a
/// 204 that writes nothing; bad JSON, a missing field and an oversized body
/// write nothing either.
#[tokio::test]
async fn discovery_report_post_sets_only_the_projects_own_run() {
    let w = world().await;
    let me = w.own_session().await;
    let before = w.rows().await;
    let cases: [(&str, Vec<u8>, u16); 4] = [
        (
            "a blank report",
            serde_json::to_vec(&json!({"report_md": " \n\t "})).unwrap(),
            204,
        ),
        ("bad JSON", b"{\"report_md\"".to_vec(), 400),
        ("no report_md", serde_json::to_vec(&json!({})).unwrap(), 422),
        ("an oversized body", oversized("report_md", json!({})), 413),
    ];
    for (label, body, want) in cases {
        assert_eq!(w.post("discovery-report", &me, body).await, want, "{label}");
        assert_eq!(w.rows().await, before, "{label} wrote something");
    }
    let status = w
        .post_json(
            "discovery-report",
            &me,
            json!({"report_md": "\n## Findings\nok\n"}),
        )
        .await;
    assert_eq!(status, 204);
    let after = w.rows().await;
    assert_eq!(
        after.reports,
        (Some("## Findings\nok".to_string()), None),
        "A's run only, trimmed"
    );
    assert_eq!(
        (after.stories, after.versions, after.attachments),
        (before.stories, before.versions, before.attachments)
    );
}

/// S4-03: a workspace-A session whose stored meta names workspace B's project
/// (with A's swarm, or alone, so the swarm is derived from B's project) or B's
/// swarm reaches none of B's rows: no draft under B's epic, no mockup on B's
/// story under discovery, no report on B's run. Each post is a silent 204.
#[tokio::test]
async fn a_post_naming_another_workspaces_project_writes_nothing() {
    let w = world().await;
    let forged = [
        w.forged_session(json!({
            "swarm_id": w.a.swarm,
            "agent_id": w.a.agent,
            "project_id": w.b.project,
        }))
        .await,
        w.forged_session(json!({"project_id": w.b.project})).await,
        w.forged_session(json!({"swarm_id": w.b.swarm, "project_id": w.b.project}))
            .await,
    ];
    let before = w.rows().await;
    for who in &forged {
        for (route, body) in [
            ("product", json!({"title": "Plan", "body_md": "# forged"})),
            ("mockup", json!({"title": "Screen", "content": "<p>x</p>"})),
            ("discovery-report", json!({"report_md": "forged report"})),
        ] {
            assert_eq!(w.post_json(route, who, body).await, 204, "{route}");
            assert_eq!(w.rows().await, before, "{route} reached workspace B");
        }
    }
    let b_attachments = w
        .ctx
        .attachment_repo
        .list_for_story(&w.b.story)
        .await
        .unwrap();
    assert!(b_attachments.is_empty());
    // The same posts from A's own session do write: the refusals above are
    // the scope check, not a broken fixture.
    let me = w.own_session().await;
    let status = w
        .post_json("discovery-report", &me, json!({"report_md": "real"}))
        .await;
    assert_eq!(status, 204);
    assert_eq!(w.rows().await.reports, (Some("real".to_string()), None));
}
