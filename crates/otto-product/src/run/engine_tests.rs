//! The analysis and plan fan-outs end to end over a migrated in-memory DB and
//! a scripted host: each agent's reply is chosen by its provider name, so a
//! test says which lenses / planners / summarizers succeed, fail or cancel.

use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use otto_core::auth::{BoxFuture, RoleChecker};
use otto_core::domain::{User, Workspace, WorkspaceRole};
use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_core::{Id, Result};
use otto_state::{DbPool, IssuesRepo, NewAnalysis, NewStory, ProductRepo, WorkspacesRepo};
use tokio::sync::broadcast;

use super::{
    new_cancel_registry, run_analysis, run_generate_plan, signal_story_cancels, AgentSpec,
    CancelRegistry, LensRunResult, SessionAppearance, SUMMARIZER_NAME_PREFIX,
};
use crate::host::ProductRunHost;
use crate::http::ProductCtx;
use crate::service::ProductService;

struct AllowAll;

impl RoleChecker for AllowAll {
    fn check<'a>(
        &'a self,
        _user: &'a User,
        _workspace_id: &'a Id,
        _min: WorkspaceRole,
    ) -> BoxFuture<'a, Result<()>> {
        Box::pin(async { Ok(()) })
    }
}

struct NoSecrets;

impl SecretStore for NoSecrets {
    fn put(&self, _: &str, _: &str) -> Result<()> {
        Ok(())
    }
    fn get(&self, _: &str) -> Result<Option<String>> {
        Ok(None)
    }
    fn delete(&self, _: &str) -> Result<()> {
        Ok(())
    }
}

/// One agent session the host was asked to run.
#[derive(Clone, Debug)]
struct Call {
    provider: String,
    title: String,
    prompt: String,
}

#[derive(Clone)]
struct Host {
    repo: ProductRepo,
    svc: Arc<ProductService>,
    roles: Arc<dyn RoleChecker>,
    events: broadcast::Sender<Event>,
    workspaces: WorkspacesRepo,
    cancels: CancelRegistry,
    library: Arc<otto_context::Library>,
    calls: Arc<Mutex<Vec<Call>>>,
}

impl ProductCtx for Host {
    fn product(&self) -> &Arc<ProductService> {
        &self.svc
    }
    fn product_repo(&self) -> &ProductRepo {
        &self.repo
    }
    fn roles(&self) -> &Arc<dyn RoleChecker> {
        &self.roles
    }
}

fn ok(raw: serde_json::Value) -> LensRunResult {
    LensRunResult {
        raw: Some(raw.to_string()),
        session_id: None,
        errored: false,
        reason: None,
    }
}

fn exited() -> LensRunResult {
    LensRunResult {
        raw: None,
        session_id: None,
        errored: true,
        reason: Some("exited"),
    }
}

fn lens_findings(provider: &str) -> serde_json::Value {
    serde_json::json!({
        "summary": format!("S {provider}"),
        "related_repos": [], "functionalities": [], "integration_points": [], "risks": [],
        "open_questions": [
            {"text": "Shared question?", "rationale": "", "category": "scope"},
            {"text": format!("Q from {provider}?"), "rationale": "", "category": "data"}
        ],
        "suggested_learnings": []
    })
}

impl Host {
    /// The scripted reply for `provider` (see the module docs).
    fn reply(&self, provider: &str, work: Option<&serde_json::Value>) -> LensRunResult {
        match provider {
            "lens-fail" | "sum-fail" | "plan-fail" | "plan-sum-fail" => exited(),
            // Deleting the story while this lens runs (`stop_story_agents`).
            "lens-cancel" => {
                let story = work.and_then(|w| w["story_id"].as_str()).unwrap();
                signal_story_cancels(&self.cancels, story);
                ok(lens_findings(provider))
            }
            "sum-ok" => ok(serde_json::json!({
                "summary": "merged",
                "questions": [{"text": "Merged question?", "rationale": "", "category": "scope"}]
            })),
            "plan-sum-ok" => ok(serde_json::json!({"plan_markdown": "### Task 1: merged"})),
            p if p.starts_with("plan-") => {
                ok(serde_json::json!({ "plan_markdown": format!("### Task 1: {p}") }))
            }
            p => ok(lens_findings(p)),
        }
    }

    fn calls(&self) -> Vec<Call> {
        self.calls.lock().unwrap().clone()
    }

    fn ran(&self, provider: &str) -> bool {
        self.calls().iter().any(|c| c.provider == provider)
    }
}

impl ProductRunHost for Host {
    fn events(&self) -> &broadcast::Sender<Event> {
        &self.events
    }
    fn workspaces(&self) -> &WorkspacesRepo {
        &self.workspaces
    }
    fn agent_cancels(&self) -> &CancelRegistry {
        &self.cancels
    }
    fn context_library(&self) -> &otto_context::Library {
        &self.library
    }
    #[allow(clippy::too_many_arguments)]
    fn run_agent_with_recovery(
        &self,
        _ws: &Workspace,
        _user_id: &Id,
        provider: &str,
        _model: Option<&str>,
        work: Option<serde_json::Value>,
        _cwd: &str,
        prompt: &str,
        _out_path: &Path,
        _timeout: Duration,
        _agent_id: Option<&Id>,
        appearance: &SessionAppearance,
        on_session: Option<&(dyn Fn(&Id) + Send + Sync)>,
    ) -> impl std::future::Future<Output = LensRunResult> + Send {
        self.calls.lock().unwrap().push(Call {
            provider: provider.into(),
            title: appearance.title.clone(),
            prompt: prompt.into(),
        });
        if let Some(on_session) = on_session {
            on_session(&format!("sid-{provider}"));
        }
        std::future::ready(self.reply(provider, work.as_ref()))
    }
}

struct Fixture {
    host: Host,
    ws: Workspace,
    user: Id,
    story: Id,
    cwd: tempfile::TempDir,
    _library: tempfile::TempDir,
}

async fn fixture() -> Fixture {
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
    let pool: DbPool = pool.into();
    let now = chrono::Utc::now();
    let user = otto_core::new_id();
    sqlx::query(
        "INSERT INTO users (id, username, password_hash, display_name, is_root, created_at)
         VALUES (?, 'po', 'hash', 'PO', 0, ?)",
    )
    .bind(&user)
    .bind(now.to_rfc3339())
    .execute(&pool)
    .await
    .unwrap();
    let ws = Workspace {
        id: otto_core::new_id(),
        name: "ws".into(),
        root_path: "/tmp".into(),
        settings: serde_json::Value::Null,
        archived: false,
        created_at: now,
    };
    sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)")
        .bind(&ws.id)
        .bind(&ws.name)
        .bind(&ws.root_path)
        .bind(now.to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();
    let repo = ProductRepo::new(pool.clone());
    let story = repo
        .create_story(NewStory {
            workspace_id: ws.id.clone(),
            source_kind: "draft".into(),
            account_id: String::new(),
            source_key: String::new(),
            title: "Checkout story".into(),
            url: String::new(),
            issue_type: None,
            stage: "draft".into(),
            cwd: None,
            parent_id: None,
            tree_kind: "story".into(),
            folder: String::new(),
            created_by: user.clone(),
        })
        .await
        .unwrap()
        .id;
    let library = tempfile::tempdir().unwrap();
    let svc = ProductService::new(
        repo.clone(),
        IssuesRepo::new(pool.clone()),
        Arc::new(NoSecrets),
    );
    let host = Host {
        repo,
        svc: Arc::new(svc),
        roles: Arc::new(AllowAll),
        events: broadcast::channel(64).0,
        workspaces: WorkspacesRepo::new(pool),
        cancels: new_cancel_registry(),
        library: Arc::new(otto_context::Library::new(library.path())),
        calls: Arc::default(),
    };
    Fixture {
        host,
        ws,
        user,
        story,
        cwd: tempfile::tempdir().unwrap(),
        _library: library,
    }
}

fn spec(provider: &str) -> AgentSpec {
    AgentSpec {
        provider: provider.into(),
        model: None,
        skill: "story-architecture-overview".into(),
        name: format!("L-{provider}"),
    }
}

impl Fixture {
    /// Run a whole analysis with these lens providers and summarizer.
    async fn analyze(&self, lenses: &[&str], summarizer: &str) -> Id {
        let analysis = self
            .host
            .repo
            .create_analysis(NewAnalysis {
                story_id: self.story.clone(),
                source_version_id: None,
                status: "running".into(),
                created_by: self.user.clone(),
            })
            .await
            .unwrap()
            .id;
        run_analysis(
            self.host.clone(),
            self.ws.clone(),
            self.user.clone(),
            self.story.clone(),
            analysis.clone(),
            lenses.iter().copied().map(spec).collect(),
            summarizer.into(),
            self.cwd.path().to_string_lossy().into(),
            None,
        )
        .await;
        analysis
    }

    async fn questions(&self) -> Vec<String> {
        let mut texts: Vec<String> = self
            .host
            .repo
            .list_questions(&self.story)
            .await
            .unwrap()
            .into_iter()
            .map(|q| q.text)
            .collect();
        texts.sort();
        texts
    }

    /// `(status, error)` of the analysis's summarizer row, if one was opened.
    async fn summarizer_row(&self, analysis: &Id) -> Option<(String, Option<String>)> {
        let agents = self.host.repo.list_analysis_agents(analysis).await.unwrap();
        agents
            .into_iter()
            .find(|a| a.name.starts_with(SUMMARIZER_NAME_PREFIX))
            .map(|a| (a.status, a.error))
    }

    async fn plan(&self, planners: &[&str], summarizer: &str) -> Option<String> {
        run_generate_plan(
            self.host.clone(),
            self.ws.clone(),
            self.user.clone(),
            self.story.clone(),
            planners.iter().map(|p| p.to_string()).collect(),
            summarizer.into(),
            false,
            None,
            self.cwd.path().to_string_lossy().into(),
            None,
        )
        .await;
        self.host
            .repo
            .latest_version_of_kind(&self.story, "plan")
            .await
            .unwrap()
            .map(|v| v.body_md)
    }
}

#[tokio::test]
async fn a_failed_lens_is_isolated_and_the_summarizer_consolidates_the_others() {
    let f = fixture().await;
    let analysis = f.analyze(&["lens-a", "lens-fail"], "sum-ok").await;

    let summarizer = f
        .host
        .calls()
        .into_iter()
        .find(|c| c.provider == "sum-ok")
        .expect("the summarizer ran over the surviving lens");
    assert!(summarizer.prompt.contains("### Lens L-lens-a (lens-a)"));
    assert!(!summarizer.prompt.contains("L-lens-fail"));
    assert_eq!(f.questions().await, ["Merged question?"]);
    let row = f.host.repo.get_analysis(&analysis).await.unwrap();
    assert_eq!(row.status, "partial", "one lens failed");
    assert_eq!(row.summary, "merged");
    assert_eq!(
        f.summarizer_row(&analysis).await,
        Some(("done".into(), None))
    );
}

#[tokio::test]
async fn a_failed_summarizer_falls_back_to_the_merged_lens_questions() {
    let f = fixture().await;
    let analysis = f.analyze(&["lens-a", "lens-b"], "sum-fail").await;

    assert_eq!(
        f.questions().await,
        ["Q from lens-a?", "Q from lens-b?", "Shared question?"],
        "every lens question, deduped"
    );
    let row = f.host.repo.get_analysis(&analysis).await.unwrap();
    assert_eq!(row.status, "done", "both lenses succeeded");
    assert!(row.summary.contains("S lens-a") && row.summary.contains("S lens-b"));
    assert_eq!(
        f.summarizer_row(&analysis).await,
        Some((
            "error".into(),
            Some("summarizer produced no parseable JSON".into())
        ))
    );
}

/// The runner's failure edge closes the summarizer row (it used to be a
/// hand-written "stuck in running" guard).
#[tokio::test]
async fn no_lens_output_closes_the_summarizer_row_without_a_session() {
    let f = fixture().await;
    let analysis = f.analyze(&["lens-fail"], "sum-ok").await;

    assert!(!f.host.ran("sum-ok"));
    assert_eq!(
        f.summarizer_row(&analysis).await,
        Some((
            "error".into(),
            Some("no successful lens outputs to consolidate".into())
        ))
    );
    assert!(f.questions().await.is_empty());
    let row = f.host.repo.get_analysis(&analysis).await.unwrap();
    assert_eq!(row.status, "partial");
}

#[tokio::test]
async fn deleting_the_story_during_the_lenses_skips_the_summarizer_and_writes_nothing() {
    let f = fixture().await;
    let analysis = f.analyze(&["lens-cancel"], "sum-ok").await;

    assert!(!f.host.ran("sum-ok"), "no summarizer after a cancel");
    assert_eq!(f.summarizer_row(&analysis).await, None);
    assert!(f.questions().await.is_empty());
    let row = f.host.repo.get_analysis(&analysis).await.unwrap();
    assert_eq!(row.status, "running", "a cancelled run writes no result");
}

#[tokio::test]
async fn one_plan_candidate_is_the_plan_without_a_summarizer() {
    let f = fixture().await;
    let mut events = f.host.events.subscribe();
    let plan = f.plan(&["plan-a", "plan-fail"], "plan-sum-ok").await;

    assert_eq!(plan.as_deref(), Some("### Task 1: plan-a"));
    assert!(
        !f.host.ran("plan-sum-ok"),
        "one candidate needs no summarizer"
    );
    // Every planner session was surfaced for live tiling.
    let mut tiled = Vec::new();
    while let Ok(ev) = events.try_recv() {
        if let Event::PlanRun { session_ids, .. } = ev {
            tiled = session_ids;
        }
    }
    tiled.sort();
    assert_eq!(tiled, ["sid-plan-a", "sid-plan-fail"]);
}

#[tokio::test]
async fn two_plan_candidates_are_consolidated_by_the_summarizer() {
    let f = fixture().await;
    let plan = f.plan(&["plan-a", "plan-b"], "plan-sum-ok").await;

    assert_eq!(plan.as_deref(), Some("### Task 1: merged"));
    let summarizer = f
        .host
        .calls()
        .into_iter()
        .find(|c| c.provider == "plan-sum-ok")
        .expect("two candidates run the summarizer");
    assert_eq!(summarizer.title, "Plan summarizer: plan-sum-ok");
    assert!(summarizer.prompt.contains("### Task 1: plan-a"));
    assert!(summarizer.prompt.contains("### Task 1: plan-b"));
}

#[tokio::test]
async fn a_failed_plan_summarizer_keeps_a_candidate_plan() {
    let f = fixture().await;
    let plan = f.plan(&["plan-a", "plan-b"], "plan-sum-fail").await;

    assert!(f.host.ran("plan-sum-fail"));
    assert!(
        matches!(
            plan.as_deref(),
            Some("### Task 1: plan-a") | Some("### Task 1: plan-b")
        ),
        "{plan:?}"
    );
}

#[tokio::test]
async fn no_plan_candidate_records_the_error_and_no_version() {
    let f = fixture().await;
    let plan = f.plan(&["plan-fail"], "plan-sum-ok").await;

    assert_eq!(plan, None);
    assert!(!f.host.ran("plan-sum-ok"));
    let events = f
        .host
        .repo
        .list_events(&f.story, Some("plan"))
        .await
        .unwrap();
    assert!(events.iter().any(|e| e.kind == "plan_error"));
}
