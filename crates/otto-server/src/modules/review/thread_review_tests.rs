//! A whole review on the thread runner against the in-memory fake agents
//! (otto_agent_protocol::scripted, the review persona): scope assessment →
//! twelve reviewers → summarizer, through `run_review_core` as the Git page
//! and the workflow node start it.
#![allow(clippy::disallowed_methods)] // test fixtures: a git checkout and an in-memory secret
use super::*;
use otto_agent_protocol::{scripted, AgentRuntime, RuntimeConfig, SessionManagerHost};
use serde_json::json;

/// Six lenses, each on Claude and Codex (an explicit comparison): twelve reviewers.
const LENSES: usize = 6;
const REVIEWERS: usize = 2 * LENSES;

type Rig = (
    tempfile::TempDir,
    ServerCtx,
    Workspace,
    otto_core::domain::Repo,
);

async fn rig() -> Rig {
    rig_with(&[]).await
}

/// `extra`: lenses the scope agent adds beyond the declared ones.
async fn rig_with(extra: &[&str]) -> Rig {
    let tmp = tempfile::tempdir().unwrap();
    let pool = otto_state::db::test_pool().await;
    let mut ctx = ServerCtx::for_tests(&pool, tmp.path().join("data")).await;
    let fakes = tmp.path().join("fakes");
    std::fs::create_dir_all(&fakes).unwrap();
    let names = (1..=LENSES)
        .map(|i| format!("Lens {i}"))
        .chain(extra.iter().map(|s| s.to_string()));
    let lenses: Vec<_> = names
        .map(|name| json!({"name": name, "reason": "touched", "instructions": "look"}))
        .collect();
    let plan =
        json!({"summary": "small change", "related_modules": [], "risks": [], "lenses": lenses});
    let scope = json!([{"severity": "info", "body": plan.to_string()}]);
    std::fs::write(fakes.join("scope.json"), scope.to_string()).unwrap();
    let finding = json!([{"path": "src/lib.rs", "line": 1, "severity": "bug", "body": "add subtracts"},
        {"lens_score": 4, "score_reason": "a real bug"}]);
    std::fs::write(fakes.join("reviewer.json"), finding.to_string()).unwrap();
    let ids: Vec<String> = (0..REVIEWERS)
        .map(|b| otto_review::engine::raw_claim_id(b, 0))
        .collect();
    let drafts = json!([{"source_ids": ids, "path": "src/lib.rs", "line": 1, "severity": "bug",
        "body": "add subtracts", "category": "correctness", "disposition": "verified",
        "evidence": "src/lib.rs:1 returns a - b"}]);
    std::fs::write(fakes.join("summarizer.json"), drafts.to_string()).unwrap();
    let host = SessionManagerHost {
        manager: Arc::clone(&ctx.manager),
        workspaces: otto_state::WorkspacesRepo::new(pool.clone()),
        publish: Box::new(|_| {}),
        notices: otto_agent_protocol::attention::AttentionNotices::new(
            otto_state::NotificationsRepo::new(pool.clone()),
            Arc::new(|_| {}),
        ),
    };
    let env = vec![(
        "OTTO_FAKE_REVIEW_DIR".to_string(),
        fakes.to_string_lossy().into_owned(),
    )];
    ctx.agent_runtime = AgentRuntime::new(
        otto_state::AgentRuntimeRepo::new(pool.clone()),
        Arc::new(host),
        scripted::drivers(),
        RuntimeConfig {
            attachments_dir: tmp.path().join("attachments"),
            transport: Some(scripted::transport(env)),
            ..RuntimeConfig::default()
        },
    );
    let settings = otto_state::SettingsRepo::new(pool.clone());
    settings
        .put("agent_admission_enabled", &json!(false))
        .await
        .unwrap();
    let owner = otto_state::UsersRepo::new(pool.clone())
        .create("owner", "unused", "Owner", true)
        .await
        .unwrap();
    let checkout = tmp.path().join("repo");
    std::fs::create_dir_all(checkout.join("src")).unwrap();
    std::fs::write(
        checkout.join("src/lib.rs"),
        "pub fn add(a: i32, b: i32) -> i32 { a - b }\n",
    )
    .unwrap();
    for args in [
        &["init", "-q"][..],
        &["add", "."][..],
        &["commit", "-qm", "add"][..],
    ] {
        let out = otto_git::hardened_std_command()
            .current_dir(&checkout)
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .args(["-c", "user.name=t", "-c", "user.email=t@example.invalid"])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let ws = ctx
        .workspaces
        .create("Review", tmp.path().to_str().unwrap(), &owner.id)
        .await
        .unwrap();
    let repo = ctx
        .git_store
        .create_repo(otto_state::NewRepo {
            workspace_id: ws.id.clone(),
            name: "app".into(),
            path: checkout.to_string_lossy().into_owned(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();
    (tmp, ctx, ws, repo)
}

fn config() -> ReviewConfig {
    let mut cfg = otto_review::engine::default_review_config("claude");
    cfg.agents = (1..=LENSES)
        .map(|i| ReviewAgentCfg {
            name: format!("Lens {i}"),
            provider: "claude".into(),
            providers: vec!["claude".into(), "codex".into()],
            prompt: "Review it.".into(),
            ..Default::default()
        })
        .collect();
    cfg.cross_provider_check = true;
    cfg.agents[0].picks.insert(
        "codex".into(),
        otto_core::domain::ModelPick {
            model: String::new(),
            effort: "high".into(),
        },
    );
    cfg
}

const DIFF: &str = "diff --git a/src/lib.rs b/src/lib.rs\n--- a/src/lib.rs\n+++ b/src/lib.rs\n@@ -0,0 +1 @@\n+pub fn add(a: i32, b: i32) -> i32 { a - b }\n";

#[tokio::test]
async fn an_exhausted_review_deadline_reaches_the_verifier() {
    let (_tmp, ctx, ws, repo) = rig().await;
    let review = ctx
        .reviews_store
        .create_review(&repo.id, LOCAL_REVIEW_PR_NUMBER)
        .await
        .unwrap();
    let mut cfg = config();
    cfg.agents.truncate(1);
    cfg.runner = Some("threads".into());
    let result = run_review_core(
        &ctx,
        &review.id,
        &repo.path,
        DIFF.into(),
        None,
        None,
        &ws,
        &repo.id,
        0,
        None,
        Some(cfg),
        None,
        Some(std::time::Instant::now()),
    )
    .await;
    let review = ctx.reviews_store.get_review(&review.id).await.unwrap();
    assert_eq!(review.agents[0].status.as_str(), "done");
    assert!(review.agents.last().unwrap().session_id.is_none());
    let err = result.unwrap_err().to_string();
    assert!(
        err.contains(otto_review::summarizer::DEADLINE_EXCEEDED),
        "{err}"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_whole_review_runs_on_threads_scope_twelve_reviewers_and_the_summarizer() {
    let (_tmp, ctx, ws, repo) = rig().await;
    let review = ctx
        .reviews_store
        .create_review(&repo.id, LOCAL_REVIEW_PR_NUMBER)
        .await
        .unwrap();
    let res = run_review_core(
        &ctx,
        &review.id,
        &repo.path,
        DIFF.into(),
        None,
        None,
        &ws,
        &repo.id,
        0,
        None,
        Some(config()),
        None,
        None,
    )
    .await;
    let done = ctx.reviews_store.get_review(&review.id).await.unwrap();
    let seen: Vec<_> = done
        .agents
        .iter()
        .map(|a| (&a.name, &a.provider, &a.status, &a.note))
        .collect();
    assert!(res.is_ok(), "{res:?}: {seen:?}");
    let plan = done.scope_plan.as_ref().unwrap();
    assert_eq!(plan.lenses.len(), LENSES, "the scope agent's plan");
    let (rows, summarizer) = done.agents.split_at(REVIEWERS);
    for (i, row) in rows.iter().enumerate() {
        assert_eq!(
            (row.status.as_str(), row.note.as_str()),
            ("done", "1 finding"),
            "row {i}: {row:?}"
        );
        assert!(
            row.thread
                && row.session_id.is_some()
                && row.started_at.is_some()
                && row.finished_at.is_some()
        );
        assert_eq!(row.score, Some(4));
        assert_eq!(
            row.provider,
            if i % 2 == 0 { "claude" } else { "codex" },
            "each lens on both providers"
        );
    }
    assert_eq!(
        (rows[1].model.as_str(), rows[1].effort.as_str()),
        ("", "high"),
        "a lens pick reaches its row"
    );
    // Not the PTY's 1.5 s cold-start stagger on threads (16.5 s for twelve).
    let starts: Vec<_> = rows.iter().filter_map(|r| r.started_at).collect();
    let spread = *starts.iter().max().unwrap() - *starts.iter().min().unwrap();
    assert!(
        spread < chrono::Duration::from_std(2 * otto_review::engine::REVIEW_SPAWN_STAGGER).unwrap(),
        "the twelve reviewers started over {spread}"
    );
    assert!(
        summarizer[0].thread && summarizer[0].session_id.is_some(),
        "{:?}",
        summarizer[0]
    );
    let bodies: Vec<_> = done.comments.iter().map(|c| c.body.as_str()).collect();
    assert_eq!(
        bodies,
        ["add subtracts"],
        "the verifier's one comment: {:?}",
        plan.verification_notes
    );
    assert_eq!(done.comments[0].body, "add subtracts");
    // Every review agent is a closed, retained runtime thread with the review's meta.
    let mut threads = Vec::new();
    for source in ["review", "review_summarizer"] {
        threads.extend(
            ctx.agent_runtime
                .list_filtered(&ws.id, None, Some(source), None)
                .await
                .unwrap(),
        );
    }
    let review_threads: Vec<_> = threads
        .iter()
        .filter(|t| {
            t.meta
                .as_ref()
                .and_then(|m| m.get("review_id"))
                .and_then(|v| v.as_str())
                == Some(review.id.as_str())
        })
        .collect();
    assert_eq!(
        review_threads.len(),
        REVIEWERS + 2,
        "scope + reviewers + summarizer"
    );
    assert!(review_threads
        .iter()
        .all(|t| t.status == otto_core::agent_runtime::RuntimeSessionStatus::Closed));
    // Codex reviewers start first (DR-023.5); the rows keep their order.
    let mut started: Vec<_> = review_threads
        .iter()
        .filter(|t| t.title.starts_with("Lens"))
        .collect();
    started.sort_by_key(|t| t.created_at);
    let first: Vec<_> = started
        .iter()
        .take(LENSES)
        .map(|t| t.provider_slug())
        .collect();
    assert_eq!(
        first,
        vec!["codex"; LENSES],
        "every Codex reviewer before any Claude one"
    );
    let codex = review_threads
        .iter()
        .find(|t| t.title == rows[1].name)
        .unwrap();
    assert_eq!(
        (codex.effort.as_deref(), codex.provider_slug()),
        (Some("high"), "codex"),
        "the pick reached the spawned thread"
    );
}

#[tokio::test]
async fn the_terminal_runner_stays_behind_the_switch() {
    let (_tmp, ctx, _ws, _repo) = rig().await;
    assert!(
        review_threads(&ctx, None).await.is_some(),
        "threads by default"
    );
    let settings = otto_state::SettingsRepo::new(ctx.pool.clone());
    settings.put("wf_step_runner", &json!("pty")).await.unwrap();
    assert!(
        review_threads(&ctx, None).await.is_none(),
        "the setting switches every review"
    );
    let mut latched = config();
    latched.runner = Some("threads".into());
    assert!(
        review_threads(&ctx, Some(&latched)).await.is_some(),
        "a run's latch wins over the setting"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_configured_scope_drops_the_lens_the_scope_agent_adds() {
    let (tmp, ctx, ws, repo) = rig_with(&["Invented by scope"]).await;
    // The scope agent drops "Lens 6" (kept dropped) and adds one (dropped).
    let lenses: Vec<_> = (1..LENSES)
        .map(|i| format!("Lens {i}"))
        .chain(["Invented by scope".to_string()])
        .map(|name| json!({"name": name, "reason": "touched", "instructions": "look"}))
        .collect();
    let plan = json!({"summary": "s", "related_modules": [], "risks": [], "lenses": lenses});
    let scope = json!([{"severity": "info", "body": plan.to_string()}]);
    std::fs::write(tmp.path().join("fakes/scope.json"), scope.to_string()).unwrap();
    let review = ctx
        .reviews_store
        .create_review(&repo.id, LOCAL_REVIEW_PR_NUMBER)
        .await
        .unwrap();
    let mut cfg = config();
    cfg.cross_provider_check = false;
    cfg.scope_assessment = otto_core::api::ScopeAssessment::Configured;
    let res = run_review_core(
        &ctx,
        &review.id,
        &repo.path,
        DIFF.into(),
        None,
        None,
        &ws,
        &repo.id,
        0,
        None,
        Some(cfg),
        None,
        None,
    )
    .await;
    let done = ctx.reviews_store.get_review(&review.id).await.unwrap();
    assert!(res.is_ok(), "{res:?}: {:?}", done.agents);
    let lenses: Vec<_> = done
        .scope_plan
        .as_ref()
        .unwrap()
        .lenses
        .iter()
        .map(|l| l.name.clone())
        .collect();
    assert_eq!(
        lenses,
        (1..LENSES).map(|i| format!("Lens {i}")).collect::<Vec<_>>(),
        "{res:?}"
    );
    assert_eq!(
        done.agents.len(),
        LENSES,
        "one reviewer per kept lens + the summarizer"
    );
    assert!(done.agents.iter().all(|a| a.name != "Invented by scope"));
}

/// CR1-029: cancelling a review closes its agents' runtime threads (a kill
/// of a PTY session by the thread's id stopped nothing).
#[tokio::test]
async fn a_cancel_closes_the_reviews_threads() {
    let (_tmp, ctx, ws, repo) = rig().await;
    let owner = otto_state::UsersRepo::new(ctx.pool.clone())
        .list()
        .await
        .unwrap()
        .remove(0);
    let req = otto_core::agent_runtime::CreateRuntimeSessionReq {
        provider: otto_core::agent_runtime::RuntimeProvider::parse("claude"),
        cwd: Some(repo.path.clone()),
        ..Default::default()
    };
    let thread = ctx
        .agent_runtime
        .create(&owner.id, &ws.id, req)
        .await
        .unwrap();
    let review = ctx
        .reviews_store
        .create_review(&repo.id, LOCAL_REVIEW_PR_NUMBER)
        .await
        .unwrap();
    let row = otto_core::domain::ReviewAgentState {
        name: "Lens 1".into(),
        provider: "claude".into(),
        status: "running".into(),
        thread: true,
        session_id: Some(thread.id.clone()),
        ..Default::default()
    };
    ctx.reviews_store
        .set_agents(&review.id, &[row])
        .await
        .unwrap();
    let review = ctx.reviews_store.get_review(&review.id).await.unwrap();
    cancel_running_review(&ctx, &review, &ws.id).await;
    let after = ctx.agent_runtime.get(&thread.id).await.unwrap();
    assert_eq!(
        after.status,
        otto_core::agent_runtime::RuntimeSessionStatus::Closed
    );
}

/// CR1-028: a held review that comes back after the runner was switched to
/// the terminal one stops its held agents' threads and removes the PR
/// checkout it ran in, as a live review's end does.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_held_review_back_on_the_terminal_runner_stops_its_threads_and_checkout() {
    let (tmp, ctx, ws, repo) = rig().await;
    let review = ctx
        .reviews_store
        .create_review(&repo.id, LOCAL_REVIEW_PR_NUMBER)
        .await
        .unwrap();
    let owner: String = sqlx::query_scalar("SELECT id FROM users LIMIT 1")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    let req = otto_core::agent_runtime::CreateRuntimeSessionReq {
        provider: Some(otto_core::agent_runtime::RuntimeProvider::Claude),
        cwd: Some(repo.path.clone()),
        ..Default::default()
    };
    let thread = ctx.agent_runtime.create(&owner, &ws.id, req).await.unwrap();
    let agent = |name: &str, thread_id: Option<String>| otto_core::domain::ReviewAgentState {
        name: name.into(),
        provider: "claude".into(),
        status: "running".into(),
        thread: thread_id.is_some(),
        session_id: thread_id,
        ..Default::default()
    };
    let rows = [
        agent("Lens 1", Some(thread.id.to_string())),
        agent("Summarizer", None),
    ];
    ctx.reviews_store
        .set_agents(&review.id, &rows)
        .await
        .unwrap();
    let wt = tmp.path().join(format!("otto-review-wt-{}", review.id));
    let out = otto_git::hardened_std_command()
        .current_dir(&repo.path)
        .args([
            "worktree",
            "add",
            "-q",
            "-b",
            &format!("otto-review-{}", review.id),
        ])
        .arg(&wt)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let run = otto_state::ReviewRunContext {
        source_branch: None,
        cwd: Some(wt.to_string_lossy().into_owned()),
        head_sha: None,
    };
    ctx.reviews_store
        .set_run_context(&review.id, &run)
        .await
        .unwrap();
    let settings = otto_state::SettingsRepo::new(ctx.pool.clone());
    settings.put("wf_step_runner", &json!("pty")).await.unwrap();
    let live = ctx.review_cancels.register(&review.id);
    resume_held_review(ctx.clone(), review.id.clone(), live).await;
    let after = ctx.agent_runtime.get(&thread.id).await.unwrap();
    assert_eq!(
        after.status,
        otto_core::agent_runtime::RuntimeSessionStatus::Closed
    );
    assert!(!wt.exists(), "the review's checkout is removed");
    let done = ctx.reviews_store.get_review(&review.id).await.unwrap();
    assert_eq!(done.status, ReviewStatus::Error);
}

/// CR1-028: a held review whose repo is gone when the daemon comes back
/// stops its held agents' threads and ends as an error — it used to stay
/// "running" with its threads open, and a workflow step waited on it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_held_review_whose_repo_is_gone_stops_its_threads_and_ends() {
    let (_tmp, ctx, ws, repo) = rig().await;
    let review = ctx
        .reviews_store
        .create_review(&repo.id, LOCAL_REVIEW_PR_NUMBER)
        .await
        .unwrap();
    let owner: String = sqlx::query_scalar("SELECT id FROM users LIMIT 1")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    let req = otto_core::agent_runtime::CreateRuntimeSessionReq {
        provider: Some(otto_core::agent_runtime::RuntimeProvider::Claude),
        cwd: Some(repo.path.clone()),
        ..Default::default()
    };
    let thread = ctx.agent_runtime.create(&owner, &ws.id, req).await.unwrap();
    let row = otto_core::domain::ReviewAgentState {
        name: "Lens 1".into(),
        provider: "claude".into(),
        status: "running".into(),
        thread: true,
        session_id: Some(thread.id.to_string()),
        ..Default::default()
    };
    ctx.reviews_store
        .set_agents(&review.id, &[row])
        .await
        .unwrap();
    // Removing a repo in the app removes its reviews with it: this is the
    // row gone (or unreadable) underneath a review that is still stored.
    sqlx::query("DELETE FROM repos WHERE id = ?")
        .bind(&repo.id)
        .execute(&ctx.pool)
        .await
        .unwrap();
    let live = ctx.review_cancels.register(&review.id);
    resume_held_review(ctx.clone(), review.id.clone(), live).await;
    let after = ctx.agent_runtime.get(&thread.id).await.unwrap();
    assert_eq!(
        after.status,
        otto_core::agent_runtime::RuntimeSessionStatus::Closed
    );
    let done = ctx.reviews_store.get_review(&review.id).await.unwrap();
    assert_eq!(done.status, ReviewStatus::Error);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_held_summarizer_thread_is_awaited_on_restart_without_spawning_a_second() {
    let (_tmp, ctx, ws, repo) = rig().await;
    let review = ctx
        .reviews_store
        .create_review(&repo.id, LOCAL_REVIEW_PR_NUMBER)
        .await
        .unwrap();
    let owner: String = sqlx::query_scalar("SELECT id FROM users LIMIT 1")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    let mut attempt = crate::review_summarizer::Attempt::new("codex", "gpt-5").unwrap();
    attempt.persist_for_restart();
    let result_path = attempt.result_path();
    let drafts = json!([{
        "source_ids": [otto_review::engine::raw_claim_id(0, 0)],
        "path": "src/lib.rs",
        "line": 1,
        "severity": "bug",
        "body": "add subtracts",
        "category": "correctness",
        "disposition": "verified",
        "evidence": "src/lib.rs:1 returns a - b"
    }]);
    std::fs::write(&result_path, drafts.to_string()).unwrap();
    let sum_thread = ctx
        .agent_runtime
        .create(
            &owner,
            &ws.id,
            otto_core::agent_runtime::CreateRuntimeSessionReq {
                provider: Some(otto_core::agent_runtime::RuntimeProvider::Codex),
                cwd: Some(repo.path.clone()),
                title: Some("Review summarizer".into()),
                meta: Some(json!({
                    "source": "review_summarizer",
                    "review_id": review.id,
                    "result_path": result_path.to_string_lossy(),
                })),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    ctx.agent_runtime
        .prompt(&sum_thread.id, "summarize")
        .await
        .unwrap();
    // Wait briefly so the scripted agent finishes turn 1 on the held thread.
    for _ in 0..40 {
        let s = ctx.agent_runtime.get(&sum_thread.id).await.unwrap();
        if s.status == otto_core::agent_runtime::RuntimeSessionStatus::Idle {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let rows = [
        otto_core::domain::ReviewAgentState {
            name: "Lens 1".into(),
            provider: "claude".into(),
            status: "done".into(),
            findings: vec![otto_core::domain::ReviewFinding {
                path: Some("src/lib.rs".into()),
                line: Some(1),
                line_end: None,
                severity: "bug".into(),
                body: "add subtracts".into(),
                lens: None,
            }],
            ..Default::default()
        },
        otto_core::domain::ReviewAgentState {
            name: "Summarizer".into(),
            provider: "codex".into(),
            model: "gpt-5".into(),
            status: "running".into(),
            thread: true,
            session_id: Some(sum_thread.id.to_string()),
            ..Default::default()
        },
    ];
    ctx.reviews_store
        .set_agents(&review.id, &rows)
        .await
        .unwrap();
    let live = ctx.review_cancels.register(&review.id);
    resume_held_review(ctx.clone(), review.id.clone(), live).await;
    let done = ctx.reviews_store.get_review(&review.id).await.unwrap();
    assert_eq!(done.status, ReviewStatus::Done, "{:?}", done.error);
    assert_eq!(done.comments.len(), 1);
    assert_eq!(done.comments[0].body, "add subtracts");
    let sum_threads = ctx
        .agent_runtime
        .list_filtered(&ws.id, None, Some("review_summarizer"), None)
        .await
        .unwrap();
    assert_eq!(
        sum_threads.len(),
        1,
        "held summarizer must be awaited, not replaced by a second summarizer thread"
    );
    assert!(
        !result_path.parent().unwrap().exists(),
        "held summarizer temp dir cleaned up after resume"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn resume_held_review_restores_frozen_workflow_summarizer_provider_and_never_runs_claude() {
    let (_tmp, ctx, ws, repo) = rig().await;
    let owner: String = sqlx::query_scalar("SELECT id FROM users LIMIT 1")
        .fetch_one(&ctx.pool)
        .await
        .unwrap();
    // Global/repo default is claude, while the workflow review step named codex
    // (no claude anywhere) with custom scope-gate instructions.
    let wf_repo = otto_state::WorkflowsRepo::new(ctx.pool.clone());
    let wf = wf_repo
        .create(
            &ws.id,
            "NoClaude",
            "",
            "",
            &otto_core::workflows::WorkflowGraph {
                nodes: vec![],
                edges: vec![],
            },
            &owner,
        )
        .await
        .unwrap();
    let run = wf_repo
        .create_run(&wf.id, &ws.id, &json!({}), None)
        .await
        .unwrap();
    let review = ctx
        .reviews_store
        .create_review(&repo.id, LOCAL_REVIEW_PR_NUMBER)
        .await
        .unwrap();
    let mut frozen_cfg = otto_review::engine::workflow_review_config_from_json(
        "claude",
        &[json!({"lens": "correctness", "providers": ["codex"]})],
        Some(&json!({"provider": "codex", "instructions": "SCOPE GATE: server only"})),
    );
    frozen_cfg.runner = Some("threads".into());
    let cp = otto_core::workflows::WorkflowCheckpoint {
        node_id: "loop_server#review.0".into(),
        loop_id: "loop_server".into(),
        iteration: 1,
        step_index: 0,
        kind: "review_run".into(),
        name: "Review".into(),
        status: otto_core::workflows::NodeStatus::Success,
        attempts: 1,
        input: json!({
            "config": frozen_cfg,
            "run_context": "prior step brief",
        }),
        output: Some(json!({"review_id": review.id})),
        error: None,
        logs: vec![],
        updated_at: chrono::Utc::now(),
        active_ms: 0,
        thread_id: None,
        start_ordinal: None,
    };
    wf_repo.save_checkpoint(&run.id, &cp).await.unwrap();
    // Reviewers are already done; summarizer row has empty provider (or stale claude)
    // as when a prior bad resume/retry touched it — the frozen workflow checkpoint
    // config MUST win and run codex, never claude.
    let rows = [
        otto_core::domain::ReviewAgentState {
            name: "Correctness".into(),
            provider: "codex".into(),
            status: "done".into(),
            findings: vec![otto_core::domain::ReviewFinding {
                path: Some("src/lib.rs".into()),
                line: Some(1),
                line_end: None,
                severity: "bug".into(),
                body: "add subtracts".into(),
                lens: None,
            }],
            ..Default::default()
        },
        otto_core::domain::ReviewAgentState {
            name: "Summarizer".into(),
            provider: String::new(),
            status: "pending".into(),
            ..Default::default()
        },
    ];
    ctx.reviews_store
        .set_agents(&review.id, &rows)
        .await
        .unwrap();
    let live = ctx.review_cancels.register(&review.id);
    resume_held_review(ctx.clone(), review.id.clone(), live).await;
    let sum_threads = ctx
        .agent_runtime
        .list_filtered(&ws.id, None, Some("review_summarizer"), None)
        .await
        .unwrap();
    assert_eq!(sum_threads.len(), 1, "expected one summarizer thread");
    assert_eq!(
        sum_threads[0].provider_slug(),
        "codex",
        "resumed workflow review must use the workflow's frozen summarizer provider, never claude"
    );
    // Also check rerun_summarizer (used by boot summarizer_resume & workflow "Retry step"):
    // it must likewise restore the frozen workflow summarizer provider and never spawn claude.
    let done_user = otto_state::UsersRepo::new(ctx.pool.clone())
        .list()
        .await
        .unwrap()
        .remove(0);
    ctx.reviews_store
        .set_status(&review.id, ReviewStatus::Error, Some("summarizer failed"))
        .await
        .unwrap();
    rerun_summarizer(&ctx, &review.id, done_user, None)
        .await
        .unwrap();
    for _ in 0..80 {
        let r = ctx.reviews_store.get_review(&review.id).await.unwrap();
        if r.status != ReviewStatus::Running {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(25)).await;
    }
    let sum_threads = ctx
        .agent_runtime
        .list_filtered(&ws.id, None, Some("review_summarizer"), None)
        .await
        .unwrap();
    assert!(
        sum_threads.len() >= 2 && sum_threads.iter().all(|t| t.provider_slug() == "codex"),
        "rerun_summarizer must also use frozen workflow provider codex, never claude: {:?}",
        sum_threads
            .iter()
            .map(|t| t.provider_slug())
            .collect::<Vec<_>>()
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn summarizer_with_unavailable_named_provider_fails_instead_of_running_claude() {
    let (_tmp, ctx, ws, repo) = rig().await;
    let owner = otto_state::UsersRepo::new(ctx.pool.clone())
        .list()
        .await
        .unwrap()
        .remove(0);
    let review = ctx
        .reviews_store
        .create_review(&repo.id, LOCAL_REVIEW_PR_NUMBER)
        .await
        .unwrap();
    let mut cfg = otto_review::engine::workflow_review_config_from_json(
        "claude",
        &[json!({"lens": "correctness", "providers": ["codex"]})],
        Some(&json!({"provider": "missing-provider"})),
    );
    cfg.runner = Some("threads".into());
    let rows = [
        otto_core::domain::ReviewAgentState {
            name: "Correctness".into(),
            provider: "codex".into(),
            status: "done".into(),
            findings: vec![otto_core::domain::ReviewFinding {
                path: Some("src/lib.rs".into()),
                line: Some(1),
                line_end: None,
                severity: "bug".into(),
                body: "add subtracts".into(),
                lens: None,
            }],
            ..Default::default()
        },
        otto_core::domain::ReviewAgentState {
            name: "Summarizer".into(),
            provider: "missing-provider".into(),
            status: "pending".into(),
            ..Default::default()
        },
    ];
    ctx.reviews_store
        .set_agents(&review.id, &rows)
        .await
        .unwrap();
    let findings = vec![rows[0].findings.clone()];
    let err = summarize_and_persist(
        &ctx, &review.id, &repo.path, &ws, &owner, &repo.id, 0, &cfg, None, &findings, "", None,
    )
    .await
    .unwrap_err()
    .to_string();
    assert!(
        err.contains("missing-provider"),
        "error must name the unavailable provider: {err}"
    );
    let sum_threads = ctx
        .agent_runtime
        .list_filtered(&ws.id, None, Some("review_summarizer"), None)
        .await
        .unwrap();
    assert!(
        sum_threads.is_empty(),
        "unavailable named provider must never fall back to claude; spawned: {sum_threads:?}"
    );
}
