//! A multi-step workflow on the REAL thread runner (CR1-020): `run_workflow`
//! → `step_thread` → `ThreadStepRunner` → `AgentRuntime` on the in-memory
//! scripted agents (`otto_agent_protocol::scripted`), not the unit tests'
//! fake `StepRunner` — a fork whose one side asks a question, an approval
//! after the join, and a step that continues the asking step's thread.

use std::time::{Duration, Instant};

use otto_core::agent_runtime::{
    QuestionAnswer, RuntimeEventBody, RuntimeReplay, RuntimeRequest, RuntimeRequestStatus,
};
use otto_core::event::Event;
use otto_core::workflows::{NodeStatus, RunStatus, WorkflowGraph};
use otto_core::Id;
use otto_server::ServerCtx;
use otto_state::WorkflowsRepo;
use serde_json::{json, Value};

use crate::wf_restart::{create, node, settled, Rig};

fn agent(id: &str, verbs: &[&str], extra: Value) -> Value {
    let script: Vec<String> = verbs.iter().map(|v| format!("OTTO_FAKE: {v}")).collect();
    let mut params =
        json!({"provider": "claude", "prompt": format!("Step {id}.\n{}", script.join("\n"))});
    params
        .as_object_mut()
        .unwrap()
        .extend(extra.as_object().unwrap().clone());
    json!({"id": id, "kind": "agent_prompt", "name": format!("Step {id}"), "params": params})
}

/// The step's pending question, read from its thread's events.
async fn pending(ctx: &ServerCtx, thread: &Id) -> RuntimeRequest {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Ok(RuntimeReplay::Events { events, .. }) = ctx.agent_runtime.replay(thread, 0).await
        {
            let open = events.iter().rev().find_map(|e| match &e.body {
                RuntimeEventBody::RuntimeRequestUpdated { runtime_request: r }
                    if r.status == RuntimeRequestStatus::Pending =>
                {
                    Some(r.clone())
                }
                _ => None,
            });
            if let Some(r) = open {
                return r;
            }
        }
        assert!(Instant::now() < deadline, "no question on {thread}");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_fork_a_question_an_approval_and_a_continued_thread_run_on_the_thread_runner() {
    let rig = Rig::new().await;
    let ctx = rig.life(None).await;
    // Edges from variables: otto-core's background-source guard reads every
    // literal string under a "source" key as a session source.
    let edge =
        |from: &str, to: &str| json!({"id": format!("{from}-{to}"), "source": from, "target": to});
    let graph: WorkflowGraph = serde_json::from_value(json!({
        "nodes": [
            {"id": "n1", "kind": "manual_trigger", "name": "Start"},
            agent("a", &["remember tok-a", "ask Which branch?", "write $HANDOFF asked", "reply asked"], json!({})),
            agent("b", &["write $HANDOFF other side", "reply other side"], json!({})),
            {"id": "gate", "kind": "human_approval", "name": "Approve"},
            agent("c", &["recall", "write $HANDOFF went on"], json!({"continue_from": "a"})),
        ],
        "edges": [edge("n1", "a"), edge("n1", "b"), edge("a", "gate"), edge("b", "gate"), edge("gate", "c")],
    }))
    .unwrap();
    let (wf, run_id) = create(&ctx, &graph).await;
    let ws = ctx.workspaces.get(&"ws-restart".into()).await.unwrap();
    let driver = tokio::spawn(otto_server::workflow_engine::run_workflow(
        ctx.clone(),
        ws.clone(),
        wf,
        run_id.clone(),
        json!({}),
        Default::default(),
        None,
    ));
    let repo = WorkflowsRepo::new(ctx.pool.clone());

    // a asks; the fork's other side finishes meanwhile.
    let deadline = Instant::now() + Duration::from_secs(30);
    let run = loop {
        let run = repo.get_run(&run_id).await.unwrap();
        let asked = run
            .nodes
            .iter()
            .any(|n| n.node_id == "a" && n.attention.is_some());
        if asked && node(&run, "b").status == NodeStatus::Success {
            break run;
        }
        assert!(
            Instant::now() < deadline,
            "a never asked beside b: {:#?}",
            run.nodes
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    };
    assert_eq!(
        node(&run, "a").status,
        NodeStatus::Running,
        "a waits for its answer"
    );
    let thread_a = node(&run, "a")
        .thread
        .as_ref()
        .expect("a runs on a thread")
        .thread_id
        .clone();
    let q = pending(&ctx, &thread_a).await;
    let question_id = q.questions.first().expect("the question's form").id.clone();
    let answer = vec![QuestionAnswer::Text {
        question_id,
        text: "main".into(),
    }];
    ctx.agent_runtime
        .respond_question(&thread_a, &q.id, answer)
        .await
        .unwrap();

    // The join parks at the approval; approve it as the route does.
    let run = settled(&ctx, &run_id, true).await;
    assert!(run.waiting_approval, "{:?} {:#?}", run.error, run.nodes);
    let rev = repo
        .record_approval(&run_id, "gate", Some(&"root".into()), "", None)
        .await
        .unwrap();
    let _ = ctx.events.send(Event::WorkflowRunUpdated {
        workspace_id: ws.id.clone(),
        run_id: run_id.clone(),
        status: "running".into(),
        node_id: Some("gate".into()),
        rev,
        node: None,
        nodes_done: 0,
        nodes_total: 0,
        waiting_approval: false,
        overview: None,
    });
    let run = settled(&ctx, &run_id, false).await;
    driver.await.unwrap();
    assert_eq!(
        run.status,
        RunStatus::Success,
        "{:?} {:#?}",
        run.error,
        run.nodes
    );
    let (a, b, c) = (node(&run, "a"), node(&run, "b"), node(&run, "c"));
    let thread =
        |n: &otto_core::workflows::NodeRunState| n.thread.as_ref().map(|t| t.thread_id.clone());
    assert!(a.attention.is_none(), "the answered question is gone");
    assert_ne!(
        thread(a),
        thread(b),
        "each side of the fork has its own thread"
    );
    assert_eq!(thread(c), thread(a), "continue_from goes on on a's thread");
    let reply = |n: &otto_core::workflows::NodeRunState| {
        n.output
            .as_ref()
            .and_then(|o| o["reply"].as_str())
            .unwrap_or_default()
            .to_string()
    };
    assert!(
        reply(c).contains("recall: tok-a"),
        "c remembers a's conversation: {}",
        reply(c)
    );
    assert_eq!(
        c.brief.as_deref(),
        Some(reply(c).as_str()),
        "the step line's brief"
    );
    let handoff = |n: &otto_core::workflows::NodeRunState| {
        let name = format!("{}.md", n.step_file.as_deref().expect("a step file"));
        std::fs::read_to_string(rig_ctx_dir(&ctx, &run_id).join(name)).unwrap_or_default()
    };
    assert!(handoff(a).contains("asked"), "a's handoff: {}", handoff(a));
    assert!(
        handoff(c).contains("went on"),
        "c's handoff: {}",
        handoff(c)
    );
    ctx.agent_runtime.shutdown().await;
}

fn rig_ctx_dir(ctx: &ServerCtx, run_id: &Id) -> std::path::PathBuf {
    ctx.data_dir.join("workflow-context").join(run_id.as_str())
}

/// CR1-036: a loop's step files reach the run's chat thread captioned with
/// their iteration, like a top-level step's.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_loops_step_files_are_captioned_with_their_iteration() {
    let rig = Rig::new().await;
    let ctx = rig.life(None).await;
    let edge =
        |from: &str, to: &str| json!({"id": format!("{from}-{to}"), "source": from, "target": to});
    let work = json!({"kind": "agent_prompt", "name": "Work",
        "params": {"provider": "claude", "prompt": "Do.\nOTTO_FAKE: write $HANDOFF worked"}});
    let graph: WorkflowGraph = serde_json::from_value(json!({
        "nodes": [
            {"id": "n1", "kind": "manual_trigger", "name": "Start"},
            {"id": "l", "kind": "loop", "name": "Loop", "params": {"max_iterations": 2, "steps": [work]}},
        ],
        "edges": [edge("n1", "l")],
    }))
    .unwrap();
    let repo = WorkflowsRepo::new(ctx.pool.clone());
    let wf = repo
        .create(
            &"ws-restart".into(),
            "Captions",
            "",
            "",
            &graph,
            &"root".into(),
        )
        .await
        .unwrap();
    let input = json!({"channel": "slack", "chat": "chat-restart", "thread": "1.1", "chat_updates": "live"});
    let run = repo
        .create_run(&wf.id, &"ws-restart".into(), &input, Some(&"root".into()))
        .await
        .unwrap();
    let ws = ctx.workspaces.get(&"ws-restart".into()).await.unwrap();
    otto_server::workflow_engine::run_workflow(
        ctx.clone(),
        ws,
        wf,
        run.id.clone(),
        input,
        Default::default(),
        None,
    )
    .await;
    let run = settled(&ctx, &run.id, false).await;
    assert_eq!(
        run.status,
        RunStatus::Success,
        "{:?} {:#?}",
        run.error,
        run.nodes
    );
    let captions: Vec<String> = rig
        .rec
        .recorded()
        .into_iter()
        .filter_map(|r| match r {
            otto_channels::test_util::Recorded::Upload { caption, .. } => caption,
            _ => None,
        })
        .collect();
    for k in [1, 2] {
        let want = format!("› ✅ Work · iteration {k}");
        assert!(captions.contains(&want), "{want}: {captions:?}");
    }
    ctx.agent_runtime.shutdown().await;
}
