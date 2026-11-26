//! The scheduler driver on real nodes (engine.md §3.6, test plan §14.2):
//! nodes run side by side, a failing sibling does not stop the other, and a
//! cancel stops every running node at once.

use super::super::super::*;
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

/// Create the workflow and a run of it; returns what `run_workflow` takes.
async fn setup(
    ws_id: &str,
    graph: WorkflowGraph,
) -> (
    ServerCtx,
    Workspace,
    Workflow,
    Id,
    WorkflowsRepo,
    tempfile::TempDir,
) {
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
    let repo = WorkflowsRepo::new(pool);
    let wf = repo
        .create(&ws.id, "Parallel", "", "", &graph, &"root".into())
        .await
        .unwrap();
    let run = repo
        .create_run(&wf.id, &ws.id, &json!({}), Some(&"root".into()))
        .await
        .unwrap();
    (ctx, ws, wf, run.id, repo, tmp)
}

fn state<'a>(run: &'a WorkflowRun, id: &str) -> &'a NodeRunState {
    run.nodes
        .iter()
        .find(|n| n.node_id == id)
        .unwrap_or_else(|| panic!("no {id}: {:?} {:?}", run.status, run.error))
}

/// A sibling failing while another runs: the other runs to its end, the
/// failed one's dependents error-skip, the run ends `error` only then, and
/// the step numbers are the serial engine's.
#[tokio::test]
async fn a_failing_sibling_lets_the_other_finish_and_poisons_only_its_own_branch() {
    let graph = WorkflowGraph {
        nodes: vec![
            node("start", "log", json!({})),
            node("slow", "delay", json!({"ms": 400})),
            node("bad", "budget_gate", json!({})),
            node("after_slow", "log", json!({})),
            node("after_bad", "log", json!({})),
        ],
        edges: vec![
            edge("start", "slow"),
            edge("start", "bad"),
            edge("slow", "after_slow"),
            edge("bad", "after_bad"),
        ],
    };
    let (ctx, ws, wf, run_id, repo, _tmp) = setup("par-ws", graph).await;
    // An unreadable budget config fails the gate closed.
    otto_state::SettingsRepo::new(ctx.pool.clone())
        .put(
            crate::budget::BUDGETS_KEY,
            &json!({"enforce": "yes please"}),
        )
        .await
        .unwrap();
    tokio::time::timeout(
        Duration::from_secs(20),
        run_workflow(
            ctx,
            ws,
            wf,
            run_id.clone(),
            json!({}),
            RunScope::default(),
            None,
        ),
    )
    .await
    .expect("the run settles");
    let run = repo.get_run(&run_id).await.unwrap();
    assert_eq!(run.status, RunStatus::Error);
    let (slow, bad) = (state(&run, "slow"), state(&run, "bad"));
    assert_eq!(
        slow.status,
        NodeStatus::Success,
        "the sibling ran to its end"
    );
    assert_eq!(bad.status, NodeStatus::Error);
    assert_eq!(state(&run, "after_slow").status, NodeStatus::Success);
    assert_eq!(state(&run, "after_bad").status, NodeStatus::Skipped);
    // Run order start, slow, bad, after_slow, after_bad; the skipped one has no number.
    let numbers: Vec<_> = ["start", "slow", "bad", "after_slow", "after_bad"]
        .iter()
        .map(|id| state(&run, id).step_no)
        .collect();
    assert_eq!(numbers, [Some(1), Some(2), Some(3), Some(4), None]);
    // The failure was decided while the slow sibling still ran.
    assert!(
        bad.started_at.unwrap() < slow.started_at.unwrap() + chrono::Duration::milliseconds(400),
        "bad started only after slow finished"
    );
}

/// Three nodes running at once all stop at a cancel; nothing settles as a
/// success afterwards and the run is `canceled`.
#[tokio::test]
async fn a_cancel_stops_every_running_node_at_once() {
    let delay = || json!({"ms": 8000});
    let graph = WorkflowGraph {
        nodes: vec![
            node("start", "log", json!({})),
            node("a", "delay", delay()),
            node("b", "delay", delay()),
            node("c", "delay", delay()),
        ],
        edges: vec![edge("start", "a"), edge("start", "b"), edge("start", "c")],
    };
    let (ctx, ws, wf, run_id, repo, _tmp) = setup("cancel-ws", graph).await;
    let driver = tokio::spawn(run_workflow(
        ctx,
        ws,
        wf,
        run_id.clone(),
        json!({}),
        RunScope::default(),
        None,
    ));
    // Wait until all three run, then cancel.
    let started = Instant::now();
    loop {
        let run = repo.get_run(&run_id).await.unwrap();
        let running = run
            .nodes
            .iter()
            .filter(|n| n.node_id != "start" && n.status == NodeStatus::Running)
            .count();
        if running == 3 {
            break;
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "three nodes run at once"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    repo.request_cancel(&run_id).await.unwrap();
    tokio::time::timeout(Duration::from_secs(4), driver)
        .await
        .expect("the cancel stops the run long before the delays end")
        .unwrap();
    let run = repo.get_run(&run_id).await.unwrap();
    assert_eq!(run.status, RunStatus::Canceled);
    for id in ["a", "b", "c"] {
        assert_ne!(state(&run, id).status, NodeStatus::Success, "{id}");
    }
}

/// A stand-in for the thread runner: reports the agreed events (a machine
/// wait, the thread, a question answered by a person), writes the handoff
/// and returns its usage; records what the engine closes and keeps.
#[derive(Default)]
struct FakeRunner {
    turns: std::sync::Mutex<Vec<otto_agent_run::step::StepTurn>>,
    closed: std::sync::Mutex<Vec<Id>>,
    retained: std::sync::Mutex<Vec<Id>>,
}

#[async_trait::async_trait]
impl otto_agent_run::step::StepRunner for FakeRunner {
    async fn run(
        &self,
        t: otto_agent_run::step::StepTurn,
        ev: tokio::sync::mpsc::UnboundedSender<otto_agent_run::step::StepEvent>,
        _cancel: otto_core::cancel_signal::CancelSignal,
    ) -> Result<otto_agent_run::step::StepOutcome> {
        use otto_agent_run::step::StepEvent as E;
        use otto_core::agent_runtime::{Attention, AttentionKind, WaitReason};
        self.turns.lock().unwrap().push(t.clone());
        let now = chrono::Utc::now();
        let wait = WaitReason {
            code: "slots".into(),
            text: "Waiting for a free agent slot — 5 of 5 running".into(),
            since: now,
            detail: None,
        };
        let ask = Attention {
            kind: AttentionKind::Question,
            via: None,
            text: "Which branch?".into(),
            since: now,
            thread_id: Some("th-1".into()),
            request_id: Some("rq-1".into()),
            resets_at: None,
            criterion_id: None,
        };
        let _ = ev.send(E::Waiting(wait));
        let _ = ev.send(E::Started {
            thread_id: "th-1".into(),
            start_ordinal: 0,
        });
        let _ = ev.send(E::Working);
        let _ = ev.send(E::NeedsYou(ask));
        tokio::time::sleep(Duration::from_millis(50)).await;
        let _ = ev.send(E::Resumed {
            request_id: Some("rq-1".into()),
            answer: Some("main".into()),
            by: Some("root".into()),
        });
        std::fs::write(t.handoff.as_ref().unwrap(), "# done\n").unwrap();
        let _ = ev.send(E::Finishing);
        Ok(otto_agent_run::step::StepOutcome {
            thread_id: "th-1".into(),
            reply: "all done".into(),
            handoff_written: true,
            turn_status: "completed".into(),
            error: None,
            context_usage: None,
            turns: 2,
            usage: otto_core::agent_runtime::TurnTokenUsage {
                input_tokens: 7,
                output_tokens: 3,
                ..Default::default()
            },
            model: Some("fake-model".into()),
            effort: t.effort.clone(),
            sub_threads: vec![],
            digest: None,
        })
    }
    async fn resume(
        &self,
        _: &Id,
        _: u32,
        _: &otto_agent_run::step::StepTurn,
        _: tokio::sync::mpsc::UnboundedSender<otto_agent_run::step::StepEvent>,
        _: otto_core::cancel_signal::CancelSignal,
    ) -> Result<otto_agent_run::step::StepOutcome> {
        unreachable!("no restart in this test")
    }
    async fn close(&self, id: &Id, _: chrono::DateTime<chrono::Utc>) {
        self.closed.lock().unwrap().push(id.clone());
    }
    async fn set_retain_until(&self, ids: &[Id], _: chrono::DateTime<chrono::Utc>) {
        self.retained.lock().unwrap().extend(ids.iter().cloned());
    }
    async fn release_process(&self, _: &Id) {}
    async fn delete(&self, _: &Id) {}
}

/// Test plan §2 agent steps: an agent step runs on a runtime thread by
/// default; its phases, machine wait, question and answer land on the node,
/// the thread (not a PTY session) is recorded with its usage, effort and
/// timeout reach the runner, and the thread is closed at settle and kept
/// at the run's end.
#[tokio::test]
async fn an_agent_step_runs_on_a_thread_and_leaves_its_record_on_the_run() {
    let fake = std::sync::Arc::new(FakeRunner::default());
    super::super::super::step_thread::FAKE_RUNNER.with(|f| *f.borrow_mut() = Some(fake.clone()));
    let graph = WorkflowGraph {
        nodes: vec![
            node("start", "log", json!({})),
            node(
                "agent",
                "agent_prompt",
                json!({"prompt": "do it", "provider": "claude", "effort": "high", "timeout_s": 120}),
            ),
        ],
        edges: vec![edge("start", "agent")],
    };
    let (ctx, ws, wf, run_id, repo, _tmp) = setup("thread-ws", graph).await;
    tokio::time::timeout(
        Duration::from_secs(20),
        run_workflow(
            ctx,
            ws,
            wf,
            run_id.clone(),
            json!({}),
            RunScope::default(),
            None,
        ),
    )
    .await
    .expect("the run settles");
    let run = repo.get_run(&run_id).await.unwrap();
    assert_eq!(run.status, RunStatus::Success, "{:?}", run.error);
    let st = state(&run, "agent");
    let t = st.thread.as_ref().expect("the thread is on the node");
    assert_eq!(
        (t.thread_id.as_str(), t.runner.as_str()),
        ("th-1", "threads")
    );
    assert_eq!(
        (t.turns, t.usage.input_tokens, t.usage.output_tokens),
        (2, 7, 3)
    );
    assert_eq!(t.effort.as_deref(), Some("high"));
    assert!(t.closed_at.is_some(), "closed at settle");
    assert!(
        st.sessions.is_empty(),
        "a thread is never listed as a PTY session"
    );
    let out = st.output.as_ref().unwrap();
    assert_eq!(out["thread_id"], "th-1");
    assert!(out.get("session_id").is_none());
    let phases: Vec<_> = st.phases.iter().map(|m| m.phase).collect();
    use otto_core::workflows::StepPhase as P;
    assert_eq!(
        phases,
        [
            P::Waiting,
            P::Starting,
            P::Working,
            P::NeedsYou,
            P::Working,
            P::Finishing
        ]
    );
    assert!(st.phase.is_none(), "settled is not a phase");
    assert!(st.attention.is_none() && st.wait.is_none());
    assert_eq!(st.human_log.len(), 1);
    assert_eq!(
        (
            st.human_log[0].ask.as_deref(),
            st.human_log[0].text.as_str()
        ),
        (Some("Which branch?"), "main")
    );
    let turn = fake.turns.lock().unwrap()[0].clone();
    assert_eq!(turn.effort.as_deref(), Some("high"));
    assert_eq!(turn.timeout, Some(Duration::from_secs(120)));
    assert!(turn
        .prompt
        .contains("[workflow step — completion protocol]"));
    assert_eq!(turn.meta["work"]["kind"], "workflow_run");
    assert!(fake.closed.lock().unwrap().contains(&"th-1".to_string()));
    assert!(fake.retained.lock().unwrap().contains(&"th-1".to_string()));
}
