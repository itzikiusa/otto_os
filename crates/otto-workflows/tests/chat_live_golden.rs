//! Goldens for the live chat style (engine.md §9.2, addendum §1.1): each
//! Phase-0 characterization run (`otto-server/tests/fixtures/workflows/compat`)
//! replayed through [`ChatThread`] in `live` style — its nodes settling in
//! their recorded step order, each step's own context file uploaded with its
//! caption, the workflow's notify posts, then the final post and
//! `final-output.md`. Re-record with `OTTO_WF_GOLDEN_WRITE=1`.
//!
//! The per_step style's goldens are `golden/chat/*.txt` beside these,
//! produced by today's engine (`otto-server` `wf_compat`).

use std::path::{Path, PathBuf};
use std::time::Duration;

use chrono::{FixedOffset, TimeZone, Utc};
use otto_channels::test_util::{Recorded, RecordingAdapter};
use otto_channels::Adapter;
use otto_core::domain::Channel;
use otto_core::workflows::{
    FailedStep, NodeRunState, NodeStatus, RunOverview, RunState, WorkflowGraph,
};
use otto_workflows::chat_status::{
    is_status_step, render_final, render_status, step_caption, step_name, ChatThread, ChatUpdates,
    ChatView, StatusMessage,
};
use serde_json::Value;

fn compat() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../otto-server/tests/fixtures/workflows/compat")
}

fn status(s: &str) -> NodeStatus {
    serde_json::from_value(Value::from(s)).unwrap()
}

/// The live transcript, and the review steps' file names (see the test).
async fn replay(clone: &str, scenario: &str) -> (String, Vec<String>) {
    let dir = compat();
    let def: Value =
        serde_json::from_slice(&std::fs::read(dir.join(format!("clones/{clone}.json"))).unwrap())
            .unwrap();
    let graph: WorkflowGraph = serde_json::from_value(def["graph"].clone()).unwrap();
    let name = def["name"].as_str().unwrap().to_string();
    let old: Value = serde_json::from_slice(
        &std::fs::read(dir.join(format!("golden/{clone}/{scenario}.old.json"))).unwrap(),
    )
    .unwrap();
    let files = &old["context_files"];
    let step_of = |id: &str| old["step_numbers"][id].as_u64().map(|n| n as u32);
    let mut order: Vec<&Value> = old["nodes"].as_array().unwrap().iter().collect();
    order.sort_by_key(|n| step_of(n["id"].as_str().unwrap()).unwrap_or(u32::MAX));

    let start = Utc.with_ymd_and_hms(2026, 1, 1, 9, 0, 0).unwrap();
    let mut now = start;
    let view = |now| ChatView {
        workflow: &name,
        now,
        tz: FixedOffset::east_opt(0).unwrap(),
    };
    let overview = |now: chrono::DateTime<Utc>, state| RunOverview {
        state,
        elapsed_ms: (now - start).num_milliseconds() as u64,
        ..Default::default()
    };
    let rec = RecordingAdapter::new(
        Channel::Slack,
        &format!("ws-live-{clone}-{scenario}"),
        "chat-compat",
        Some("thread-compat"),
    );
    let mut thread = ChatThread::new(
        ChatUpdates::Live,
        rec.clone(),
        StatusMessage {
            channel: "slack".into(),
            chat: "chat-compat".into(),
            thread: Some("thread-compat".into()),
            message_id: None,
        },
    );
    let mut nodes: Vec<NodeRunState> = vec![];
    let mut review_files = vec![];
    thread
        .status(render_status(
            &view(now),
            &overview(now, RunState::Running),
            &nodes,
            &graph,
        ))
        .await;
    for n in order {
        let id = n["id"].as_str().unwrap();
        let node = graph.nodes.iter().find(|g| g.id == id).unwrap();
        let mut state = NodeRunState {
            node_id: id.into(),
            status: NodeStatus::Running,
            started_at: Some(now),
            step_no: step_of(id),
            ..Default::default()
        };
        // Each step waits 4 s to start and takes 4 s, so both show on the card.
        tokio::time::advance(Duration::from_secs(4)).await;
        now += chrono::Duration::seconds(4);
        state.started_at = Some(now);
        nodes.push(state.clone());
        thread
            .status(render_status(
                &view(now),
                &overview(now, RunState::Running),
                &nodes,
                &graph,
            ))
            .await;
        tokio::time::advance(Duration::from_secs(4)).await;
        now += chrono::Duration::seconds(4);
        state.status = status(n["status"].as_str().unwrap());
        state.error = n["error"].as_str().map(str::to_owned);
        state.output = Some(n["output"].clone());
        state.duration_ms = Some(4_000);
        *nodes.last_mut().unwrap() = state.clone();
        thread
            .status(render_status(
                &view(now),
                &overview(now, RunState::Running),
                &nodes,
                &graph,
            ))
            .await;
        if node.kind == "channel_notify" {
            if let Some(text) = n["output"]["message"].as_str() {
                rec.send_formatted("chat-compat", Some("thread-compat"), text)
                    .await
                    .unwrap();
            }
        }
        if !is_status_step(&node.kind) || state.status == NodeStatus::Skipped {
            continue;
        }
        let Some(no) = state.step_no else { continue };
        let file = files
            .as_object()
            .unwrap()
            .keys()
            .find(|k| k.starts_with(&format!("step{no}-")) && k.ends_with(".md"));
        if let Some(file) = file {
            if node.kind == "review_run" {
                review_files.push(file.clone());
            }
            let body = files[file.as_str()].as_str().unwrap();
            thread
                .step_file(
                    file,
                    body.as_bytes(),
                    &step_caption(step_name(node), &state, None),
                )
                .await;
        }
    }
    let failed = nodes.iter().find(|n| n.status == NodeStatus::Error);
    let state = match old["run"]["status"].as_str().unwrap() {
        "success" => RunState::Succeeded,
        "canceled" => RunState::Canceled,
        _ => RunState::Failed,
    };
    let mut fin = overview(now, state);
    fin.steps_total = 0;
    fin.failed_step = failed
        .filter(|_| state == RunState::Failed)
        .map(|n| FailedStep {
            node_id: n.node_id.clone(),
            name: step_name(graph.nodes.iter().find(|g| g.id == n.node_id).unwrap()).to_string(),
            error: n.error.clone().unwrap_or_default(),
            error_class: None,
        });
    thread
        .post(&render_final(&view(now), &fin, &nodes, &graph, None))
        .await;
    // The deliverable file is unchanged by the style: the one today's
    // thread ends with (`final-output.md`, else `summary.md`).
    let old =
        std::fs::read_to_string(dir.join(format!("golden/chat/{clone}-{scenario}.txt"))).unwrap();
    if let Some(at) = old.rfind("=== upload ") {
        let (head, body) = old[at + 11..].split_once('\n').unwrap();
        let out = head.split(' ').next().unwrap();
        thread
            .file(out, body.strip_suffix('\n').unwrap_or(body).as_bytes())
            .await;
    }
    (transcript(&rec.recorded()), review_files)
}

/// The same layout as the per_step goldens (`wf_compat`'s chat rendering).
fn transcript(log: &[Recorded]) -> String {
    let mut out = String::new();
    for r in log {
        let (head, body) = match r {
            Recorded::Send {
                id,
                chat,
                thread,
                text,
                formatted,
            } => (
                format!(
                    "send {id} to {chat} {} {}",
                    thread.as_deref().unwrap_or("(no thread)"),
                    if *formatted { "formatted" } else { "plain" }
                ),
                text.clone(),
            ),
            Recorded::Edit {
                chat,
                message_id,
                text,
                formatted,
            } => (
                format!(
                    "edit {message_id} in {chat}{}",
                    if *formatted { " formatted" } else { "" }
                ),
                text.clone(),
            ),
            Recorded::Upload {
                chat,
                thread,
                name,
                content,
                caption,
            } => (
                format!(
                    "upload {name} to {chat} {}{}",
                    thread.as_deref().unwrap_or("(no thread)"),
                    caption
                        .as_deref()
                        .map(|c| format!(" caption {c:?}"))
                        .unwrap_or_default()
                ),
                String::from_utf8_lossy(content).into_owned(),
            ),
        };
        out.push_str(&format!("=== {head}\n{body}\n"));
    }
    out
}

fn check(name: &str, got: &str) {
    let path = compat().join(format!("golden/chat/live/{name}.txt"));
    if std::env::var_os("OTTO_WF_GOLDEN_WRITE").is_some() {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, got).unwrap();
        return;
    }
    let want = std::fs::read_to_string(&path).unwrap_or_default();
    if want != got {
        let line = want
            .lines()
            .zip(got.lines())
            .position(|(a, b)| a != b)
            .unwrap_or(0);
        panic!(
            "{} differs at line {}:\n  golden: {:?}\n  now:    {:?}",
            path.display(),
            line + 1,
            want.lines().nth(line),
            got.lines().nth(line)
        );
    }
}

#[tokio::test(start_paused = true)]
async fn live_style_goldens() {
    for (clone, scenario) in [
        ("a", "passed"),
        ("a", "failed"),
        ("b", "passed"),
        ("b", "failed"),
        ("c", "passed"),
        ("c", "failed"),
        ("d", "passed"),
        ("d", "failed"),
        ("e", "no-ticket"),
        ("e", "ticket-unreadable"),
    ] {
        let (got, review_files) = replay(clone, scenario).await;
        // Owner's answer 1a: every step file today's thread posts is still
        // posted, in the same order. In the Phase-0 runs the review was a
        // stubbed transform (no file); the replay keeps the clone's
        // review_run, whose file today's engine does post (run/schedule.rs
        // settle), so that one is the only extra.
        let old =
            std::fs::read_to_string(compat().join(format!("golden/chat/{clone}-{scenario}.txt")))
                .unwrap();
        let uploads = |t: &str| -> Vec<String> {
            t.lines()
                .filter_map(|l| l.strip_prefix("=== upload "))
                .map(|l| l.split(' ').next().unwrap().to_string())
                .collect()
        };
        let live: Vec<String> = uploads(&got)
            .into_iter()
            .filter(|f| !review_files.contains(f))
            .collect();
        assert_eq!(
            live,
            uploads(&old),
            "{clone}-{scenario}: the same files, in the same order"
        );
        check(&format!("{clone}-{scenario}"), &got);
    }
}
