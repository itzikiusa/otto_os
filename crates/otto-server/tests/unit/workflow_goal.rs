//! Goals inside the engine (goal.md §2.1–§2.3, GL rows): a passing check
//! ends its loop at once, budgets and the repeated-failure stop hold the run,
//! Resume and Extend continue without repeating done work, Pause is durable
//! and a Cancel beats a hold, and a person's record wakes a parked check.

use super::*;
use crate::routes::browser::tests::{mem_pool, seed_workspace, test_ctx};
use otto_core::workflows::WorkflowEdge;

fn node(id: &str, kind: &str, params: Value) -> WorkflowNode {
    WorkflowNode {
        id: id.into(),
        kind: kind.into(),
        name: String::new(),
        x: 0.0,
        y: 0.0,
        params,
        retry: None,
    }
}

fn edge(s: &str, t: &str) -> WorkflowEdge {
    WorkflowEdge {
        id: format!("{s}-{t}"),
        source: s.into(),
        target: t.into(),
        condition: None,
    }
}

/// start → goal (a loop around `steps`) with `params` merged in.
fn goal_graph(steps: Value, params: Value) -> WorkflowGraph {
    let mut p = json!({"steps": steps});
    p.as_object_mut()
        .unwrap()
        .extend(params.as_object().unwrap().clone());
    WorkflowGraph {
        nodes: vec![node("start", "log", json!({})), node("goal", "loop", p)],
        edges: vec![edge("start", "goal")],
    }
}

fn check(criteria: Value) -> Value {
    json!({"kind": "check", "name": "check", "params": {"criteria": criteria}})
}

struct Fx {
    ctx: ServerCtx,
    ws: Workspace,
    wf: Workflow,
    run_id: Id,
    repo: WorkflowsRepo,
    user: User,
    _tmp: tempfile::TempDir,
}

async fn setup(ws_id: &str, graph: WorkflowGraph) -> Fx {
    let tmp = tempfile::TempDir::new().unwrap();
    let pool = mem_pool().await;
    seed_workspace(&pool, ws_id).await;
    sqlx::query("INSERT INTO users(id,username,password_hash,display_name,is_root,created_at) VALUES('root','root','x','Root',1,?)")
        .bind(chrono::Utc::now().to_rfc3339())
        .execute(&pool)
        .await
        .unwrap();
    let ctx = test_ctx(&pool, tmp.path().to_path_buf()).await;
    let ws = ctx.workspaces.get(&ws_id.into()).await.unwrap();
    let repo = WorkflowsRepo::new(pool.clone());
    let wf = repo
        .create(&ws.id, "Goal", "", "", &graph, &"root".into())
        .await
        .unwrap();
    let run = repo
        .create_run(&wf.id, &ws.id, &json!({}), Some(&"root".into()))
        .await
        .unwrap();
    let user = otto_state::UsersRepo::new(pool)
        .get(&"root".into())
        .await
        .unwrap();
    Fx {
        ctx,
        ws,
        wf,
        run_id: run.id,
        repo,
        user,
        _tmp: tmp,
    }
}

impl Fx {
    fn drive(&self) -> tokio::task::JoinHandle<()> {
        tokio::spawn(run_workflow(
            self.ctx.clone(),
            self.ws.clone(),
            self.wf.clone(),
            self.run_id.clone(),
            json!({}),
            RunScope::default(),
            None,
        ))
    }
    async fn run(&self) -> WorkflowRun {
        let settled = tokio::time::timeout(Duration::from_secs(15), self.drive()).await;
        let run = self.repo.get_run(&self.run_id).await.unwrap();
        if settled.is_err() {
            let cps = self.repo.checkpoints(&self.run_id).await.unwrap();
            let cps: Vec<_> = cps
                .iter()
                .map(|c| (&c.node_id, c.status, &c.logs))
                .collect();
            panic!("the run did not settle: {:#?}\n{cps:#?}", run.nodes);
        }
        run
    }
    /// Wait for a re-entered run (Resume spawns its driver) to settle.
    async fn settled(&self) -> WorkflowRun {
        let t0 = Instant::now();
        loop {
            let run = self.repo.get_run(&self.run_id).await.unwrap();
            if !matches!(run.status, RunStatus::Pending | RunStatus::Running)
                && !driver_alive(&self.run_id)
            {
                return run;
            }
            assert!(
                t0.elapsed() < Duration::from_secs(30),
                "the resumed run settles"
            );
            tokio::time::sleep(Duration::from_millis(30)).await;
        }
    }
    async fn checkpoint(&self, id: &str) -> Option<otto_core::workflows::WorkflowCheckpoint> {
        self.repo.checkpoint(&self.run_id, id).await.unwrap()
    }
    fn flag(&self, name: &str) {
        let dir = self
            .ctx
            .data_dir
            .join("workflow-context")
            .join(self.run_id.as_str());
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(name), "ok").unwrap();
    }
}

fn running(run: &WorkflowRun, id: &str) -> bool {
    run.nodes
        .iter()
        .any(|n| n.node_id == id && n.status == NodeStatus::Running)
}

fn state<'a>(run: &'a WorkflowRun, id: &str) -> &'a NodeRunState {
    run.nodes.iter().find(|n| n.node_id == id).unwrap()
}

/// G4 / GL1.3: the iteration's check passes → the loop ends satisfied at
/// once; the steps after the check do not run in that iteration.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_passing_check_ends_its_loop_at_once() {
    let steps = json!([
        {"kind": "log", "name": "work"},
        check(json!([{"id": "two", "text": "second try", "verify": "expr", "expr": "iteration >= 2"}])),
        {"kind": "log", "name": "after"},
    ]);
    let fx = setup("goal-pass", goal_graph(steps, json!({"max_iterations": 3}))).await;
    let run = fx.run().await;
    assert_eq!(run.status, RunStatus::Success, "{:?}", run.error);
    let out = state(&run, "goal").output.clone().unwrap();
    assert_eq!(
        (out["satisfied"].clone(), out["iterations"].clone()),
        (json!(true), json!(2))
    );
    assert!(
        fx.checkpoint("goal#1.2").await.is_some(),
        "iteration 1 went on after its unmet check"
    );
    assert!(
        fx.checkpoint("goal#2.2").await.is_none(),
        "iteration 2 stopped at its passing check"
    );
    let c = fx.checkpoint("goal#2.1").await.unwrap().output.unwrap();
    assert_eq!(c["all_met"], true);
    assert_eq!(c["criteria"][0]["evidence"], "iteration >= 2 is true");
}

/// GL4.3 / GL7.1: out of iterations holds the run as Exhausted (stored
/// `error` + hold, attention 0); Resume without room is refused with the
/// words a person reads; Extend continues with the next iteration and the
/// done ones are not run again.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn out_of_iterations_holds_and_extend_continues_without_repeating() {
    let steps = json!([check(
        json!([{"id": "ok", "text": "flag set", "verify": "command",
        "command": "test -f \"$OTTO_CONTEXT_DIR/ok\""}])
    )]);
    let fx = setup(
        "goal-ext",
        goal_graph(steps, json!({"max_iterations": 2, "hold_when_unmet": true})),
    )
    .await;
    let run = fx.run().await;
    assert_eq!(run.status, RunStatus::Error);
    let hold = run.hold.clone().expect("held");
    assert_eq!(hold.kind, otto_core::workflows::HoldKind::Exhausted);
    assert_eq!(hold.reason, "Out of iterations: 2 of 2 used");
    assert_eq!(hold.node_ids, ["goal"]);
    assert_eq!(
        state(&run, "goal").status,
        NodeStatus::Pending,
        "the loop waits to resume"
    );
    let err = hold::resume(&fx.ctx, &fx.run_id, None).await.unwrap_err();
    assert!(
        err.to_string()
            .contains("Raise the iteration limit to resume: 2 of 2 iterations are used."),
        "{err}"
    );
    fx.flag("ok");
    let ext = otto_core::api::ExtendReq {
        iterations: Some(3),
        active_s: None,
    };
    hold::resume(&fx.ctx, &fx.run_id, Some(&ext)).await.unwrap();
    let run = fx.settled().await;
    assert_eq!(run.status, RunStatus::Success, "{:?}", run.error);
    assert!(run.hold.is_none());
    assert_eq!(
        run.limits.as_ref().unwrap().loops["goal"].max_iterations,
        Some(3)
    );
    for (id, attempts) in [("goal#1.0", 1), ("goal#2.0", 1), ("goal#3.0", 1)] {
        assert_eq!(
            fx.checkpoint(id).await.unwrap().attempts,
            attempts,
            "{id} ran once"
        );
    }
    assert_eq!(
        state(&run, "goal").output.as_ref().unwrap()["satisfied"],
        true
    );
}

/// GL5.1 / GL5.2: two identical failing checks block the run; Resume goes
/// on with the next iteration (the stored blocked verdict does not stop it
/// again) and keeps the repeat count, so one more identical failure blocks.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn two_identical_failures_block_and_resume_goes_on() {
    let steps = json!([check(
        json!([{"id": "t", "text": "tests pass", "verify": "command", "command": "echo FAIL; exit 1"}])
    )]);
    let params =
        json!({"max_iterations": 6, "hold_when_unmet": true, "stop_on_repeat_failure": true});
    let fx = setup("goal-rep", goal_graph(steps, params)).await;
    let run = fx.run().await;
    let hold = run.hold.clone().expect("held");
    assert_eq!(hold.kind, otto_core::workflows::HoldKind::Blocked);
    assert_eq!(hold.reason, otto_workflows::criteria::REPEATED_FAILURE);
    assert_eq!(hold.iteration, Some(2));
    hold::resume(&fx.ctx, &fx.run_id, None).await.unwrap();
    let run = fx.settled().await;
    let hold = run.hold.clone().expect("blocked again");
    assert_eq!(
        (hold.kind, hold.iteration),
        (otto_core::workflows::HoldKind::Blocked, Some(3))
    );
    assert_eq!(
        run.limits.as_ref().unwrap().loops["goal"].unblocked_at,
        Some(2)
    );
    assert_eq!(
        fx.checkpoint("goal#3.0").await.unwrap().output.unwrap()["repeated_failures"],
        3
    );
}

/// GL4.3 (time): banked active time at the budget holds the run as
/// Exhausted before the next inner step starts.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn banked_time_at_the_budget_exhausts_the_loop() {
    let steps = json!([{"kind": "log", "name": "work"}]);
    let fx = setup(
        "goal-time",
        goal_graph(steps, json!({"max_active_s": 300, "hold_when_unmet": true})),
    )
    .await;
    let earlier = otto_core::workflows::WorkflowCheckpoint {
        node_id: "goal#0.0".into(),
        loop_id: "goal".into(),
        status: NodeStatus::Success,
        active_ms: 300_000,
        updated_at: chrono::Utc::now(),
        ..Default::default()
    };
    fx.repo.save_checkpoint(&fx.run_id, &earlier).await.unwrap();
    let run = fx.run().await;
    let hold = run.hold.expect("held");
    assert_eq!(hold.reason, "Out of time: 5m of 5m active");
    assert!(
        fx.checkpoint("goal#1.0").await.is_none(),
        "no step started past the budget"
    );
}

/// Pause (DR-106): the hold is written before anything unwinds; the running
/// step goes back to pending (not skipped) with a "paused" mark; Resume
/// runs it and what follows, and the finished step is adopted.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn pause_holds_the_run_and_resume_continues_it() {
    let graph = WorkflowGraph {
        nodes: vec![
            node("start", "log", json!({})),
            node("d", "delay", json!({"ms": 4000})),
            node("after", "log", json!({})),
        ],
        edges: vec![edge("start", "d"), edge("d", "after")],
    };
    let fx = setup("goal-pause", graph).await;
    let driver = fx.drive();
    let t0 = Instant::now();
    while !running(&fx.repo.get_run(&fx.run_id).await.unwrap(), "d") {
        assert!(t0.elapsed() < Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let paused = hold::pause(&fx.ctx, &fx.run_id, &fx.user).await.unwrap();
    assert_eq!(paused.status, RunStatus::Canceled, "durable at once");
    assert_eq!(paused.hold.as_ref().unwrap().reason, "Paused by Root");
    tokio::time::timeout(Duration::from_secs(4), driver)
        .await
        .unwrap()
        .unwrap();
    let run = fx.repo.get_run(&fx.run_id).await.unwrap();
    let hold = run.hold.clone().expect("still held after the unwind");
    assert_eq!(hold.node_ids, ["d"]);
    let d = state(&run, "d");
    assert_eq!(d.status, NodeStatus::Pending);
    assert_eq!(d.phases.last().unwrap().note.as_deref(), Some("paused"));
    assert_eq!(
        state(&run, "after").status,
        NodeStatus::Pending,
        "never skipped"
    );
    assert!(
        hold::pause(&fx.ctx, &fx.run_id, &fx.user).await.is_err(),
        "a held run is settled"
    );
    let start_at = state(&run, "start").started_at;
    hold::resume(&fx.ctx, &fx.run_id, None).await.unwrap();
    let run = fx.settled().await;
    assert_eq!(run.status, RunStatus::Success, "{:?}", run.error);
    assert_eq!(state(&run, "after").status, NodeStatus::Success);
    assert_eq!(
        state(&run, "start").started_at,
        start_at,
        "the finished step is adopted"
    );
}

/// DR-106: a Cancel beats a hold — the hold is gone, the settled status
/// stays, and Resume is refused.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_cancel_beats_a_hold() {
    let graph = WorkflowGraph {
        nodes: vec![
            node("start", "log", json!({})),
            node("d", "delay", json!({"ms": 4000})),
        ],
        edges: vec![edge("start", "d")],
    };
    let fx = setup("goal-cancel", graph).await;
    let driver = fx.drive();
    let t0 = Instant::now();
    while !running(&fx.repo.get_run(&fx.run_id).await.unwrap(), "d") {
        assert!(t0.elapsed() < Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    hold::pause(&fx.ctx, &fx.run_id, &fx.user).await.unwrap();
    assert!(hold::cancel_held(&fx.ctx, &fx.run_id).await.unwrap());
    tokio::time::timeout(Duration::from_secs(4), driver)
        .await
        .unwrap()
        .unwrap();
    let run = fx.repo.get_run(&fx.run_id).await.unwrap();
    assert_eq!(run.status, RunStatus::Canceled);
    assert!(run.hold.is_none(), "no hold came back after the cancel");
    assert!(hold::resume(&fx.ctx, &fx.run_id, None).await.is_err());
    assert!(!hold::cancel_held(&fx.ctx, &fx.run_id).await.unwrap());
}

/// GL2.3 / DR-107: the check asks a person only once the automated
/// criteria are met; the record is addressed by the loop's id and the
/// criterion id the attention carries, is bound to the criterion's revision
/// and iteration, lands on the loop's `human_log`, and the check settles.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_parked_check_wakes_on_the_persons_record() {
    let steps = json!([check(json!([
        {"id": "cmd", "text": "builds", "verify": "command", "command": "true"},
        {"id": "look", "text": "the page looks right", "verify": "human"},
    ]))]);
    let fx = setup(
        "goal-verify",
        goal_graph(steps, json!({"max_iterations": 2})),
    )
    .await;
    let driver = fx.drive();
    let req = |met| otto_core::api::VerifyCriterionReq {
        met,
        evidence: "Saw it in the browser".into(),
    };
    let t0 = Instant::now();
    loop {
        let run = fx.repo.get_run(&fx.run_id).await.unwrap();
        let a = run
            .nodes
            .iter()
            .find(|n| n.node_id == "goal")
            .and_then(|n| n.attention.clone());
        if let Some(a) = a {
            assert_eq!(a.criterion_id.as_deref(), Some("look"));
            break;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "the check parks on the person's criterion"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let wrong = hold::verify(&fx.ctx, &fx.run_id, "goal", "cmd", &fx.user, &req(true)).await;
    assert!(
        matches!(wrong, Err(otto_core::Error::Conflict(_))),
        "{wrong:?}"
    );
    let blank = otto_core::api::VerifyCriterionReq {
        met: true,
        evidence: " ".into(),
    };
    assert!(
        hold::verify(&fx.ctx, &fx.run_id, "goal", "look", &fx.user, &blank)
            .await
            .is_err()
    );
    hold::verify(&fx.ctx, &fx.run_id, "goal", "look", &fx.user, &req(true))
        .await
        .unwrap();
    tokio::time::timeout(Duration::from_secs(10), driver)
        .await
        .unwrap()
        .unwrap();
    let run = fx.repo.get_run(&fx.run_id).await.unwrap();
    assert_eq!(run.status, RunStatus::Success, "{:?}", run.error);
    let goal = state(&run, "goal");
    let rec = goal
        .human_log
        .iter()
        .find(|e| e.kind == "verify")
        .expect("recorded");
    assert_eq!(
        (rec.ask.as_deref(), rec.met, rec.iteration, rec.by.as_str()),
        (Some("look"), Some(true), Some(1), "Root")
    );
    assert!(rec.revision.is_some());
    let c = fx.checkpoint("goal#1.0").await.unwrap().output.unwrap();
    assert_eq!(c["verdict"], "achieved");
    assert_eq!(
        c["criteria"][1]["evidence"],
        "Verified by Root: Saw it in the browser"
    );
    assert!(
        hold::verify(&fx.ctx, &fx.run_id, "goal", "look", &fx.user, &req(true))
            .await
            .is_err(),
        "no longer parked"
    );
}

/// A stand-in for the thread runner: the worker writes its step file, the
/// checker claims "met" without evidence; it records every turn and resume.
#[derive(Default)]
struct Fake {
    turns: std::sync::Mutex<Vec<otto_agent_run::step::StepTurn>>,
    resumed: std::sync::Mutex<Vec<(Id, u32)>>,
    closed: std::sync::Mutex<Vec<Id>>,
    /// The first turn hangs once its thread started (a step to pause or skip).
    hang_first: std::sync::atomic::AtomicBool,
    /// A resumed turn finds its handoff already written (restart meanwhile).
    quiet_resume: std::sync::atomic::AtomicBool,
    /// The first turn asks a question, is answered once `answer` is set, then hangs.
    ask: std::sync::atomic::AtomicBool,
    answer: std::sync::atomic::AtomicBool,
}

#[async_trait::async_trait]
impl otto_agent_run::step::StepRunner for Fake {
    async fn run(
        &self,
        t: otto_agent_run::step::StepTurn,
        ev: tokio::sync::mpsc::UnboundedSender<otto_agent_run::step::StepEvent>,
        _cancel: otto_core::cancel_signal::CancelSignal,
    ) -> Result<otto_agent_run::step::StepOutcome> {
        let n = {
            let mut turns = self.turns.lock().unwrap();
            turns.push(t.clone());
            turns.len()
        };
        let thread_id: Id = format!("th-{n}");
        let _ = ev.send(otto_agent_run::step::StepEvent::Started {
            thread_id: thread_id.clone(),
            start_ordinal: 0,
        });
        if self.ask.swap(false, std::sync::atomic::Ordering::SeqCst) {
            let _ = ev.send(otto_agent_run::step::StepEvent::NeedsYou(
                otto_core::agent_runtime::Attention {
                    kind: otto_core::agent_runtime::AttentionKind::Question,
                    via: Some(otto_core::agent_runtime::AttentionVia::Provider),
                    text: "Which one?".into(),
                    since: chrono::Utc::now(),
                    thread_id: Some(thread_id.clone()),
                    request_id: None,
                    resets_at: None,
                    criterion_id: None,
                },
            ));
            while !self.answer.load(std::sync::atomic::Ordering::SeqCst) {
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let _ = ev.send(otto_agent_run::step::StepEvent::Resumed {
                request_id: None,
                answer: Some("this one".into()),
                by: None,
            });
            std::future::pending::<()>().await;
        }
        if self
            .hang_first
            .swap(false, std::sync::atomic::Ordering::SeqCst)
        {
            std::future::pending::<()>().await;
        }
        let h = t.handoff.clone().unwrap();
        let body = if h.to_string_lossy().ends_with(".output.json") {
            r#"{"criteria":[{"id":"judged","met":true,"evidence":""}],"verdict":"achieved"}"#
        } else {
            "# worked\n"
        };
        std::fs::write(&h, body).unwrap();
        Ok(otto_agent_run::step::StepOutcome {
            thread_id,
            reply: "done".into(),
            handoff_written: true,
            turn_status: "completed".into(),
            error: None,
            context_usage: None,
            turns: 1,
            usage: Default::default(),
            model: None,
            effort: t.effort.clone(),
            sub_threads: vec![],
            digest: None,
        })
    }
    async fn resume(
        &self,
        id: &Id,
        start_ordinal: u32,
        t: &otto_agent_run::step::StepTurn,
        ev: tokio::sync::mpsc::UnboundedSender<otto_agent_run::step::StepEvent>,
        cancel: otto_core::cancel_signal::CancelSignal,
    ) -> Result<otto_agent_run::step::StepOutcome> {
        self.resumed
            .lock()
            .unwrap()
            .push((id.clone(), start_ordinal));
        if self.quiet_resume.load(std::sync::atomic::Ordering::SeqCst) {
            return Ok(otto_agent_run::step::StepOutcome {
                thread_id: id.clone(),
                reply: "done".into(),
                handoff_written: true,
                turn_status: "completed".into(),
                error: None,
                context_usage: None,
                turns: 1,
                usage: Default::default(),
                model: None,
                effort: None,
                sub_threads: vec![],
                digest: None,
            });
        }
        self.run(t.clone(), ev, cancel).await
    }
    async fn close(&self, id: &Id, _: chrono::DateTime<chrono::Utc>) {
        self.closed.lock().unwrap().push(id.clone());
    }
    async fn set_retain_until(&self, _: &[Id], _: chrono::DateTime<chrono::Utc>) {}
    async fn release_process(&self, _: &Id) {}
    async fn delete(&self, _: &Id) {}
}

fn fake() -> std::sync::Arc<Fake> {
    let f = std::sync::Arc::new(Fake::default());
    super::super::super::step_thread::FAKE_RUNNER.with(|r| *r.borrow_mut() = Some(f.clone()));
    f
}

/// GL1.1 / GL3.2: a "met" without evidence counts as not met; the checker
/// runs on a fresh thread, never the worker's; a steering note reaches the
/// next agent step's prompt once and is recorded on the loop's node.
#[tokio::test]
async fn an_unevidenced_claim_is_not_met_and_a_steering_note_reaches_the_worker() {
    let f = fake();
    let steps = json!([
        {"kind": "agent_prompt", "name": "Work", "params": {"prompt": "do the work", "provider": "claude"}},
        check(json!([{"id": "judged", "text": "it works", "verify": "agent", "how": "run it"}])),
    ]);
    let fx = setup(
        "goal-agent",
        goal_graph(steps, json!({"max_iterations": 1})),
    )
    .await;
    let queued = hold::steer(&fx.ctx, &fx.run_id, &fx.user, "  focus on the tests ")
        .await
        .unwrap();
    assert_eq!(queued, ["focus on the tests"]);
    assert_eq!(
        hold::steering_queued(&fx.ctx, &fx.run_id),
        ["focus on the tests"]
    );
    let run = fx.run().await;
    assert_eq!(run.status, RunStatus::Success, "{:?}", run.error);
    let c = fx.checkpoint("goal#1.1").await.unwrap().output.unwrap();
    assert_eq!(c["criteria"][0]["met"], false);
    assert_eq!(
        c["criteria"][0]["evidence"],
        otto_workflows::criteria::NO_EVIDENCE
    );
    assert_eq!(c["verdict"], "continue");
    let turns = f.turns.lock().unwrap().clone();
    assert_eq!(turns.len(), 2);
    assert!(
        turns[0].prompt.contains("[steering]") && turns[0].prompt.contains("1. focus on the tests")
    );
    assert!(!turns[1].prompt.contains("[steering]"), "taken once");
    assert!(
        turns[1].continue_thread.is_none(),
        "the checker never continues the worker"
    );
    assert!(turns[1].title.contains("Check criteria"));
    assert!(hold::steering_queued(&fx.ctx, &fx.run_id).is_empty());
    let steer = state(&run, "goal")
        .human_log
        .iter()
        .find(|e| e.kind == "steer")
        .cloned()
        .expect("recorded");
    assert_eq!(
        (steer.text.as_str(), steer.by.as_str()),
        ("focus on the tests", "Root")
    );
}

/// DR-111 / GL24.8: an inner agent step a restart caught on its thread
/// resumes that thread instead of a fresh prompt.
#[tokio::test]
async fn an_inner_step_caught_by_a_restart_resumes_its_thread() {
    let f = fake();
    let steps = json!([{"kind": "agent_prompt", "name": "Work", "params": {"prompt": "do it", "provider": "claude"}}]);
    let fx = setup(
        "goal-restart",
        goal_graph(steps, json!({"max_iterations": 1})),
    )
    .await;
    let caught = otto_core::workflows::WorkflowCheckpoint {
        node_id: "goal#1.0".into(),
        loop_id: "goal".into(),
        iteration: 1,
        kind: "agent_prompt".into(),
        name: "Work".into(),
        status: NodeStatus::Running,
        attempts: 1,
        updated_at: chrono::Utc::now(),
        active_ms: 4_000,
        thread_id: Some("th-9".into()),
        start_ordinal: Some(4),
        ..Default::default()
    };
    fx.repo.save_checkpoint(&fx.run_id, &caught).await.unwrap();
    let run = fx.run().await;
    assert_eq!(run.status, RunStatus::Success, "{:?}", run.error);
    assert_eq!(*f.resumed.lock().unwrap(), [("th-9".to_string(), 4)]);
    let cp = fx.checkpoint("goal#1.0").await.unwrap();
    assert!(
        cp.active_ms >= 4_000,
        "banked time survives the restart: {}",
        cp.active_ms
    );
}

/// GL24.8 / DR-111: a loop step with `same_thread` continues the thread its
/// previous iteration worked on; the checker still gets a fresh one.
#[tokio::test]
async fn a_same_thread_step_continues_its_previous_iterations_thread() {
    let f = fake();
    let once =
        "test -f \"$OTTO_CONTEXT_DIR/seen\" || { touch \"$OTTO_CONTEXT_DIR/seen\"; exit 1; }";
    let steps = json!([
        {"kind": "agent_prompt", "name": "Work", "same_thread": true, "params": {"prompt": "do it", "provider": "claude"}},
        check(json!([{"id": "twice", "text": "second time lucky", "verify": "command", "command": once}])),
    ]);
    let fx = setup("goal-same", goal_graph(steps, json!({"max_iterations": 2}))).await;
    let run = fx.run().await;
    assert_eq!(run.status, RunStatus::Success, "{:?}", run.error);
    let turns = f.turns.lock().unwrap().clone();
    assert_eq!(turns.len(), 2, "one Work turn per iteration");
    assert_eq!(turns[0].continue_thread, None);
    assert_eq!(
        turns[1].continue_thread.as_deref(),
        Some("th-1"),
        "iteration 2 continues iteration 1"
    );
}

/// GL9.1: a research goal's agent works in `<context>/research`, never the
/// workspace root.
#[tokio::test]
async fn a_research_goal_works_in_its_own_folder() {
    let f = fake();
    let steps = json!([{"kind": "agent_prompt", "name": "Work", "params": {"prompt": "find out", "provider": "claude"}}]);
    let fx = setup(
        "goal-research",
        goal_graph(steps, json!({"max_iterations": 1})),
    )
    .await;
    let input = json!({"research_dir": true});
    let drive = run_workflow(
        fx.ctx.clone(),
        fx.ws.clone(),
        fx.wf.clone(),
        fx.run_id.clone(),
        input,
        RunScope::default(),
        None,
    );
    tokio::time::timeout(Duration::from_secs(15), tokio::spawn(drive))
        .await
        .unwrap()
        .unwrap();
    let turns = f.turns.lock().unwrap().clone();
    let want = format!("workflow-context/{}/research", fx.run_id);
    assert!(
        turns[0].cwd.ends_with(&want),
        "{} does not end with {want}",
        turns[0].cwd.display()
    );
}

/// CR1-002: a node's future is a few pointers wide. The node kinds' bodies
/// are boxed, so a resumed goal's loop → check → command chain does not carry
/// every kind's frame inline; together they overflowed a 2 MiB worker stack.
#[tokio::test]
async fn a_node_future_does_not_carry_every_kinds_frame() {
    let fx = setup(
        "goal-size",
        goal_graph(json!([check(json!([]))]), json!({})),
    )
    .await;
    let env = RunEnv {
        budget: budget::ActiveBudget::new(RUN_WALL_CLOCK_TIMEOUT),
        run_id: fx.run_id.clone(),
        wf_name: fx.wf.name.clone(),
        run_cwd: String::new(),
        run_base: None,
        files: std::sync::Arc::new(crate::workflow_context::RunContextFiles::disabled(
            &fx.run_id,
        )),
        default_provider: "claude".into(),
        run_input: json!({}),
    };
    let (sessions, _sr) = tokio::sync::mpsc::unbounded_channel();
    let (logs, _lr) = tokio::sync::mpsc::unbounded_channel();
    let (activity, _ar) = tokio::sync::mpsc::unbounded_channel();
    let progress = ProgressSink::disabled();
    let scope = StepScope {
        step_no: 1,
        iter: None,
        inner_idx: None,
    };
    let node = &fx.wf.graph.nodes[1];
    let fut = execute_node(
        &fx.ctx,
        &fx.ws,
        &fx.user,
        node,
        json!({}),
        &env,
        &scope,
        &sessions,
        &logs,
        &activity,
        &progress,
    );
    // 19 KiB inline before the kinds were boxed, 592 bytes after.
    let node_size = std::mem::size_of_val(&fut);
    assert!(node_size < 2048, "a node future is {node_size} bytes");
}

/// CR1-001: a held run keeps its worktrees through the sweep that runs at
/// boot and after every finished run; it resumes in them.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_held_runs_worktrees_survive_the_sweep() {
    let steps = json!([check(
        json!([{"id": "no", "text": "never", "verify": "command", "command": "exit 1"}])
    )]);
    let fx = setup(
        "goal-sweep",
        goal_graph(steps, json!({"max_iterations": 1, "hold_when_unmet": true})),
    )
    .await;
    let run = fx.run().await;
    assert!(run.hold.is_some(), "held");
    let dir = fx
        .ctx
        .data_dir
        .join("workflow-runs")
        .join(fx.run_id.as_str());
    std::fs::create_dir_all(&dir).unwrap();
    sweep_stale_run_worktrees(&fx.ctx).await;
    assert!(dir.exists(), "the held run's worktree dir was reaped");
    assert!(hold::cancel_held(&fx.ctx, &fx.run_id).await.unwrap());
    assert!(!dir.exists(), "a canceled hold reaps it");
}

fn agent(id: &str) -> WorkflowNode {
    let mut n = node(
        id,
        "agent_prompt",
        json!({"prompt": "do it", "provider": "claude"}),
    );
    n.name = "Analyze".into();
    n
}

fn open_thread(id: &str, ordinal: u64) -> otto_core::workflows::StepThread {
    otto_core::workflows::StepThread {
        thread_id: id.into(),
        runner: "threads".into(),
        provider: "claude".into(),
        model: None,
        effort: None,
        continued_from: None,
        start_ordinal: Some(ordinal),
        turns: 0,
        usage: Default::default(),
        digest: None,
        closed_at: None,
        available: None,
    }
}

/// Wait until node `id` satisfies `f` in the stored run.
async fn until(fx: &Fx, id: &str, f: impl Fn(&NodeRunState) -> bool) -> WorkflowRun {
    let t0 = Instant::now();
    loop {
        let run = fx.repo.get_run(&fx.run_id).await.unwrap();
        if run.nodes.iter().any(|n| n.node_id == id && f(n)) {
            return run;
        }
        assert!(
            t0.elapsed() < Duration::from_secs(10),
            "{id} never got there: {:#?}",
            run.nodes
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

/// CR1-005: a thread step a restart caught resumes its thread, and the
/// handoff its agent wrote while the daemon was down is the step's answer —
/// neither deleted before the resume nor overwritten by the engine's summary.
#[tokio::test]
async fn a_restarted_thread_step_keeps_the_handoff_its_agent_wrote_meanwhile() {
    let f = fake();
    f.quiet_resume
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let graph = WorkflowGraph {
        nodes: vec![node("start", "log", json!({})), agent("a")],
        edges: vec![edge("start", "a")],
    };
    let fx = setup("thread-restart", graph).await;
    let earlier = chrono::Utc::now() - chrono::Duration::seconds(60);
    let prior = vec![
        NodeRunState {
            node_id: "start".into(),
            status: NodeStatus::Success,
            step_no: Some(1),
            started_at: Some(earlier),
            output: Some(json!({})),
            ..Default::default()
        },
        NodeRunState {
            node_id: "a".into(),
            status: NodeStatus::Pending,
            step_no: Some(2),
            started_at: Some(earlier),
            thread: Some(open_thread("th-9", 3)),
            ..Default::default()
        },
    ];
    let dir = fx
        .ctx
        .data_dir
        .join("workflow-context")
        .join(fx.run_id.as_str());
    std::fs::create_dir_all(&dir).unwrap();
    let handoff = dir.join(format!(
        "{}.md",
        crate::workflow_context::step_base_name(2, "Analyze", None, None)
    ));
    std::fs::write(&handoff, "# what the agent wrote\n").unwrap();
    let scope = RunScope {
        continue_unfinished: true,
        ..Default::default()
    };
    let drive = run_workflow(
        fx.ctx.clone(),
        fx.ws.clone(),
        fx.wf.clone(),
        fx.run_id.clone(),
        json!({}),
        scope,
        Some(prior),
    );
    tokio::time::timeout(Duration::from_secs(15), tokio::spawn(drive))
        .await
        .unwrap()
        .unwrap();
    let run = fx.repo.get_run(&fx.run_id).await.unwrap();
    assert_eq!(run.status, RunStatus::Success, "{:?}", run.error);
    assert_eq!(*f.resumed.lock().unwrap(), [("th-9".to_string(), 3)]);
    assert_eq!(
        std::fs::read_to_string(&handoff).unwrap(),
        "# what the agent wrote\n"
    );
}

/// CR1-006: a step paused on its thread is closed and marked so; Resume
/// runs it on a fresh thread instead of awaiting the stopped turn.
#[tokio::test]
async fn a_paused_thread_step_starts_afresh_on_resume() {
    let f = fake();
    f.hang_first
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let graph = WorkflowGraph {
        nodes: vec![node("start", "log", json!({})), agent("a")],
        edges: vec![edge("start", "a")],
    };
    let fx = setup("thread-pause", graph).await;
    let driver = fx.drive();
    until(&fx, "a", |n| n.thread.is_some()).await;
    hold::pause(&fx.ctx, &fx.run_id, &fx.user).await.unwrap();
    tokio::time::timeout(Duration::from_secs(4), driver)
        .await
        .unwrap()
        .unwrap();
    let run = fx.repo.get_run(&fx.run_id).await.unwrap();
    let t = state(&run, "a").thread.clone().unwrap();
    assert!(t.closed_at.is_some(), "the paused step's thread is closed");
    assert!(f.closed.lock().unwrap().contains(&"th-1".to_string()));
    hold::resume(&fx.ctx, &fx.run_id, None).await.unwrap();
    let run = fx.settled().await;
    assert_eq!(run.status, RunStatus::Success, "{:?}", run.error);
    assert!(
        f.resumed.lock().unwrap().is_empty(),
        "no resume of a stopped turn"
    );
    assert_eq!(f.turns.lock().unwrap().len(), 2);
}

/// CR1-008: skipping a running thread step stops its thread before the run
/// goes on — the agent must not keep editing the worktree beside the next step.
#[tokio::test]
async fn a_skipped_thread_step_closes_its_thread() {
    let f = fake();
    f.hang_first
        .store(true, std::sync::atomic::Ordering::SeqCst);
    let graph = WorkflowGraph {
        nodes: vec![
            node("start", "log", json!({})),
            agent("a"),
            node("b", "delay", json!({"ms": 3000})),
        ],
        edges: vec![edge("start", "a"), edge("a", "b")],
    };
    let fx = setup("thread-skip", graph).await;
    let driver = fx.drive();
    until(&fx, "a", |n| n.thread.is_some()).await;
    fx.ctx
        .wf_skip_current
        .lock()
        .unwrap()
        .insert(skip_marker_key(&fx.run_id, "a"));
    until(&fx, "b", |n| n.status == NodeStatus::Running).await;
    assert_eq!(*f.closed.lock().unwrap(), ["th-1".to_string()]);
    tokio::time::timeout(Duration::from_secs(10), driver)
        .await
        .unwrap()
        .unwrap();
}

/// CR1-055 / CR1-056: a resume the run never took dies with the run (the
/// next execution starts the step fresh), and a step thread names its
/// workflow in `meta.work`.
#[tokio::test]
async fn a_finished_run_forgets_a_resume_it_never_took() {
    let f = fake();
    let graph = WorkflowGraph {
        nodes: vec![node("start", "log", json!({})), agent("a")],
        edges: vec![edge("start", "a")],
    };
    let fx = setup("thread-forget", graph).await;
    super::super::super::step_thread::mark_resume(&fx.run_id, "a", &open_thread("th-9", 1));
    super::super::super::step_thread::forget(&fx.run_id);
    let run = fx.run().await;
    assert_eq!(run.status, RunStatus::Success, "{:?}", run.error);
    assert!(f.resumed.lock().unwrap().is_empty());
    let turns = f.turns.lock().unwrap().clone();
    assert_eq!(
        turns[0].meta["work"],
        json!({"kind": "workflow_run", "id": fx.run_id, "workflow_id": fx.wf.id})
    );
}

/// CR1-009: a digest file is named by the node id only when that id is a
/// plain file name; `../` in an imported graph writes nothing.
#[test]
fn a_digest_file_never_leaves_the_threads_folder() {
    use super::super::super::step_thread::digest_prefix;
    assert_eq!(digest_prefix("goal#1.0").as_deref(), Some("goal#1.0-"));
    for bad in ["../../../tmp/pwn", "a/b", "..", ".hidden", ""] {
        assert_eq!(digest_prefix(bad), None, "{bad}");
    }
}

/// CR1-007: while a gate waits for its approval, a sibling step's publish
/// keeps the run's overview at "needs you" and the run waiting for approval.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_siblings_publish_keeps_the_approval_wait() {
    let graph = WorkflowGraph {
        nodes: vec![
            node("start", "log", json!({})),
            node("gate", "human_approval", json!({})),
            node("d", "delay", json!({"ms": 1500})),
        ],
        edges: vec![edge("start", "gate"), edge("start", "d")],
    };
    let fx = setup("approval-sibling", graph).await;
    let _driver = fx.drive();
    // The gate's own park stores "needs you" while the sibling still runs.
    let run = until(&fx, "d", |n| n.status == NodeStatus::Running).await;
    let parked = |r: &WorkflowRun| {
        r.overview.as_ref().map(|o| o.state) == Some(otto_core::workflows::RunState::NeedsYou)
    };
    let t0 = Instant::now();
    let mut run = run;
    while !parked(&run) {
        assert_eq!(
            state(&run, "d").status,
            NodeStatus::Running,
            "parked before d ends"
        );
        assert!(t0.elapsed() < Duration::from_secs(5));
        tokio::time::sleep(Duration::from_millis(10)).await;
        run = fx.repo.get_run(&fx.run_id).await.unwrap();
    }
    let run = until(&fx, "d", |n| n.status == NodeStatus::Success).await;
    assert!(run.waiting_approval);
    let ov = run.overview.expect("stored");
    assert_eq!(ov.state, otto_core::workflows::RunState::NeedsYou, "{ov:?}");
}

/// CR1-015: Pause announces the run with its held overview (the Runs inbox
/// keeps a held run), and a canceled run stores its overview.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn pause_and_cancel_announce_the_overview() {
    let graph = WorkflowGraph {
        nodes: vec![
            node("start", "log", json!({})),
            node("d", "delay", json!({"ms": 4000})),
        ],
        edges: vec![edge("start", "d")],
    };
    let fx = setup("pause-overview", graph).await;
    let mut events = fx.ctx.events.subscribe();
    let driver = fx.drive();
    until(&fx, "d", |n| n.status == NodeStatus::Running).await;
    hold::pause(&fx.ctx, &fx.run_id, &fx.user).await.unwrap();
    let ov = loop {
        if let Ok(Event::WorkflowRunUpdated {
            status, overview, ..
        }) = events.recv().await
        {
            if status == "canceled" {
                break overview;
            }
        }
    };
    assert!(
        ov.and_then(|o| o.hold).is_some(),
        "the pause event carries the hold"
    );
    tokio::time::timeout(Duration::from_secs(4), driver)
        .await
        .unwrap()
        .unwrap();
    assert!(hold::cancel_held(&fx.ctx, &fx.run_id).await.unwrap());

    let fx = setup("cancel-overview", fx.wf.graph.clone()).await;
    let driver = fx.drive();
    until(&fx, "d", |n| n.status == NodeStatus::Running).await;
    fx.repo.request_cancel(&fx.run_id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(4), driver)
        .await
        .unwrap()
        .unwrap();
    let run = fx.repo.get_run(&fx.run_id).await.unwrap();
    assert_eq!(
        run.overview.map(|o| o.state),
        Some(otto_core::workflows::RunState::Canceled)
    );
}

/// CR1-021: a Pause that lands after the driver's own hold check (the run
/// finishing or timing out) is still a hold: the cancel finalize settles it
/// instead of skipping its steps and reaping its worktrees.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_pause_racing_the_end_still_holds() {
    let graph = WorkflowGraph {
        nodes: vec![
            node("start", "log", json!({})),
            node("d", "delay", json!({"ms": 10})),
        ],
        edges: vec![edge("start", "d")],
    };
    let fx = setup("pause-race", graph).await;
    fx.repo
        .update_run_if(
            &fx.run_id,
            &[RunStatus::Pending],
            RunStatus::Running,
            &[],
            None,
            false,
        )
        .await
        .unwrap();
    let hold = otto_workflows::hold::pause_hold(&fx.user.id, "Root");
    fx.repo
        .request_pause(&fx.run_id, &hold)
        .await
        .unwrap()
        .unwrap();
    let dir = fx
        .ctx
        .data_dir
        .join("workflow-runs")
        .join(fx.run_id.as_str());
    std::fs::create_dir_all(&dir).unwrap();
    let mut states = vec![
        NodeRunState {
            node_id: "start".into(),
            status: NodeStatus::Success,
            ..Default::default()
        },
        NodeRunState {
            node_id: "d".into(),
            status: NodeStatus::Running,
            ..Default::default()
        },
    ];
    finalize_canceled_run(
        &fx.ctx,
        &fx.repo,
        &fx.wf,
        &fx.run_id,
        &mut states,
        &json!({}),
    )
    .await;
    let run = fx.repo.get_run(&fx.run_id).await.unwrap();
    assert!(run.hold.is_some(), "still held");
    assert_eq!(state(&run, "d").status, NodeStatus::Pending, "not skipped");
    assert!(dir.exists(), "not reaped");
}

/// CR1-022: a step answered while every run slot is taken waits for a slot
/// beside the cancel watch, so a Cancel still stops the run at once.
#[tokio::test]
async fn a_cancel_stops_a_step_waiting_to_retake_its_slot() {
    let f = fake();
    f.ask.store(true, std::sync::atomic::Ordering::SeqCst);
    let graph = WorkflowGraph {
        nodes: vec![node("start", "log", json!({})), agent("a")],
        edges: vec![edge("start", "a")],
    };
    let fx = setup("retake-cancel", graph).await;
    spawn_run(
        fx.ctx.clone(),
        fx.ws.clone(),
        fx.wf.clone(),
        fx.run_id.clone(),
        json!({}),
        RunScope::default(),
        None,
    );
    until(&fx, "a", |n| n.attention.is_some()).await;
    let gate =
        otto_workflows::run_queue::gate_for(otto_workflows::run_queue::lane_of(&fx.wf.graph));
    let t0 = Instant::now();
    let mut held = vec![];
    while held.len() < otto_workflows::run_queue::max_parallel_runs() {
        held.extend(gate.try_acquire().ok());
        assert!(
            t0.elapsed() < Duration::from_secs(5),
            "the parked run gives its slot back"
        );
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    f.answer.store(true, std::sync::atomic::Ordering::SeqCst);
    until(&fx, "a", |n| n.attention.is_none()).await;
    fx.repo.request_cancel(&fx.run_id).await.unwrap();
    let run = fx.settled().await;
    assert_eq!(run.status, RunStatus::Canceled);
}

/// CR1-024: a step paused while it asked a question resumes without the
/// question, and its timeline says "paused", not "canceled" then "paused".
#[tokio::test]
async fn a_paused_question_is_gone_on_resume() {
    let f = fake();
    f.ask.store(true, std::sync::atomic::Ordering::SeqCst);
    let graph = WorkflowGraph {
        nodes: vec![node("start", "log", json!({})), agent("a")],
        edges: vec![edge("start", "a")],
    };
    let fx = setup("pause-question", graph).await;
    let driver = fx.drive();
    until(&fx, "a", |n| n.attention.is_some()).await;
    hold::pause(&fx.ctx, &fx.run_id, &fx.user).await.unwrap();
    tokio::time::timeout(Duration::from_secs(4), driver)
        .await
        .unwrap()
        .unwrap();
    let run = fx.repo.get_run(&fx.run_id).await.unwrap();
    let a = state(&run, "a");
    assert!(a.attention.is_none() && a.phase.is_none(), "{a:?}");
    let notes: Vec<_> = a.phases.iter().filter_map(|m| m.note.as_deref()).collect();
    assert_eq!(notes, ["paused"]);
}

/// CR1-026: a time budget that stops a goal before an iteration's first step
/// has not used that iteration, so Resume with more time continues.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_goal_out_of_time_before_an_iteration_resumes_with_more_time() {
    let steps = json!([{"kind": "log", "name": "work"}]);
    let params = json!({"max_iterations": 1, "max_active_s": 300, "hold_when_unmet": true});
    let fx = setup("goal-time-iter", goal_graph(steps, params)).await;
    let earlier = otto_core::workflows::WorkflowCheckpoint {
        node_id: "goal#0.0".into(),
        loop_id: "goal".into(),
        status: NodeStatus::Success,
        active_ms: 300_000,
        updated_at: chrono::Utc::now(),
        ..Default::default()
    };
    fx.repo.save_checkpoint(&fx.run_id, &earlier).await.unwrap();
    let run = fx.run().await;
    assert_eq!(
        run.hold.as_ref().unwrap().iteration,
        Some(0),
        "{:?}",
        run.hold
    );
    let more = otto_core::api::ExtendReq {
        iterations: None,
        active_s: Some(600),
    };
    hold::resume(&fx.ctx, &fx.run_id, Some(&more))
        .await
        .unwrap();
    fx.settled().await;
    assert!(fx.checkpoint("goal#1.0").await.is_some(), "iteration 1 ran");
}

/// CR1-071: with continue_on_error, a goal whose last iteration failed is
/// held out of iterations instead of failing.
#[tokio::test(flavor = "multi_thread", worker_threads = 1)]
async fn a_continue_on_error_goal_holds_after_a_failed_last_iteration() {
    let steps = json!([{"kind": "verifier", "name": "broken"}]);
    let params = json!({"max_iterations": 1, "hold_when_unmet": true, "continue_on_error": true});
    let fx = setup("goal-coe", goal_graph(steps, params)).await;
    let run = fx.run().await;
    let out = state(&run, "goal").output.clone();
    let hold = run
        .hold
        .unwrap_or_else(|| panic!("not held: {:?} {out:?}", run.error));
    assert_eq!(hold.kind, otto_core::workflows::HoldKind::Exhausted);
}

/// CR1-023: a prepare_context agent on a thread reports a `thread_id`, never
/// a `session_id` the previous build would open as a PTY session.
#[tokio::test]
async fn a_prepare_context_agent_reports_its_thread() {
    let _f = fake();
    let graph = WorkflowGraph {
        nodes: vec![
            node("start", "log", json!({})),
            node(
                "prep",
                "prepare_context",
                json!({"prompt": "gather", "provider": "claude"}),
            ),
        ],
        edges: vec![edge("start", "prep")],
    };
    let fx = setup("prep-thread", graph).await;
    let run = fx.run().await;
    let out = state(&run, "prep").output.clone().unwrap();
    assert_eq!(out["thread_id"], "th-1", "{out}");
    assert!(out.get("session_id").is_none());
}
