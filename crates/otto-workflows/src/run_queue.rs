//! The daemon-wide run queue, seen from outside the engine: the one cap every
//! reader shares, the gate the engine parks runs on, a snapshot of where a new
//! run would land, and the "it started" message a queued chat-triggered run
//! posts when it finally gets its slot.
//!
//! The engine (`otto-server/src/workflow_engine.rs`) executes at most
//! [`max_parallel_runs`] runs at once and parks the rest FIFO on [`run_gate`].
//! The chat trigger answers "⏳ Queued workflow …" for a run that lands behind
//! the gate — and until now that thread heard nothing more until the first
//! step reported, 30–60 minutes later or never noticed at all.
//! [`announce_dequeued`] closes that gap, ONLY for a run that actually waited:
//! a run that started at once already said "🚀 Started".

use std::sync::OnceLock;
use std::time::Duration;

use otto_core::domain::Channel;
use otto_core::workflows::Workflow;
use otto_state::WorkflowsRepo;
use serde_json::Value;

use crate::chat_resolve::{slot_outlook, SlotOutlook};
use crate::WorkflowCtx;

/// Default cap on workflow runs EXECUTING at once, daemon-wide. Every run
/// beyond it queues FIFO in the engine's `spawn_run` and stays `pending`. A
/// single run can fan out dozens of agent PTYs (a `review_run` node alone
/// launches the whole reviewer fleet), so uncapped parallel runs exhausted fds
/// and starved interactive sessions; 2 keeps one long run from blocking a
/// quick one while bounding the process/fd blast radius.
pub const DEFAULT_MAX_PARALLEL_RUNS: usize = 2;

/// The effective cap: `OTTO_WF_MAX_PARALLEL_RUNS` (≥ 1) when set, else
/// [`DEFAULT_MAX_PARALLEL_RUNS`]. Read once — the gate's permit count can't
/// change while runs hold permits, so every reader (the gate, the chat
/// acknowledgement, the Workflows page) must see that same first value.
pub fn max_parallel_runs() -> usize {
    static N: OnceLock<usize> = OnceLock::new();
    *N.get_or_init(|| parse_cap(std::env::var("OTTO_WF_MAX_PARALLEL_RUNS").ok().as_deref()))
}

/// The cap an `OTTO_WF_MAX_PARALLEL_RUNS` value asks for: a positive integer,
/// else the default (unset, blank, zero and garbage all mean "default").
fn parse_cap(raw: Option<&str>) -> usize {
    raw.and_then(|s| s.trim().parse::<usize>().ok())
        .filter(|n| *n >= 1)
        .unwrap_or(DEFAULT_MAX_PARALLEL_RUNS)
}

/// Daemon-wide run gate. Tokio's semaphore queues waiters FIFO, so runs start
/// in the order they were requested.
pub fn run_gate() -> &'static tokio::sync::Semaphore {
    static GATE: OnceLock<tokio::sync::Semaphore> = OnceLock::new();
    GATE.get_or_init(|| tokio::sync::Semaphore::new(max_parallel_runs()))
}

/// Whether a run admitted now starts or queues, from the persisted run states:
/// queued = fresh `pending` rows with no step progress; executing = the other
/// live runs, minus those parked on an approval (which hand their slot back).
/// `exclude` drops a run that was already inserted (a trigger reads this after
/// its own admission). A snapshot — the engine's in-memory gate is the
/// authority — so `None` on any read error.
pub async fn slot_outlook_now(
    repo: &WorkflowsRepo,
    cap: usize,
    exclude: Option<&str>,
) -> Option<SlotOutlook> {
    let queued: std::collections::HashSet<String> = repo
        .queued_run_ids()
        .await
        .ok()?
        .into_iter()
        .filter(|id| Some(id.as_str()) != exclude)
        .collect();
    let mut executing = 0;
    for id in repo.list_active_run_ids_global().await.ok()? {
        if queued.contains(&id) || Some(id.as_str()) == exclude {
            continue;
        }
        if let Ok(Some((_, waiting_approval))) = repo.run_status(&id).await {
            executing += usize::from(!waiting_approval);
        }
    }
    Some(slot_outlook(executing, queued.len(), cap))
}

/// One of a workspace's queued runs and its place in the daemon-wide line
/// (1 = next to start when a slot frees).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct QueuedPlace {
    pub run_id: String,
    pub position: usize,
}

/// `GET /workspaces/{wid}/workflow-runs/queue`: the parallel-run cap and the
/// workspace's queued runs with their GLOBAL place in line — the queue spans
/// every workspace, so a position counted from one workspace's list alone
/// would be wrong (other workspaces' run ids are never listed).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct QueueView {
    pub max_parallel_runs: usize,
    pub queued: Vec<QueuedPlace>,
}

/// The places of `mine` in the FIFO `line` (oldest first), 1-based.
pub fn places(line: &[String], mine: &std::collections::HashSet<String>) -> Vec<QueuedPlace> {
    line.iter()
        .enumerate()
        .filter(|(_, id)| mine.contains(*id))
        .map(|(i, id)| QueuedPlace {
            run_id: id.clone(),
            position: i + 1,
        })
        .collect()
}

/// Build the [`QueueView`] of `workspace_id`.
pub async fn queue_view(
    repo: &WorkflowsRepo,
    workspace_id: &otto_core::Id,
) -> otto_core::Result<QueueView> {
    let line = repo.queued_run_ids().await?;
    let mine = repo
        .list_active_runs(workspace_id)
        .await?
        .into_iter()
        .map(|r| r.run_id)
        .collect();
    Ok(QueueView {
        max_parallel_runs: max_parallel_runs(),
        queued: places(&line, &mine),
    })
}

/// Where a run reports back to: the chat integration + channel/thread the
/// trigger arrived on. Resolved once from the run input.
pub struct ChatTarget {
    /// Workspace whose integration reports the run — always the workflow's own
    /// (see [`result_workspace`]; chat triggers only start workflows of the
    /// receiving workspace, S3-03).
    pub ws: String,
    pub channel: Channel,
    pub chat: String,
    pub thread: Option<String>,
}

/// The workspace whose Slack/Telegram integration reports a run: always the
/// workflow's own. Chat triggers only start workflows of the receiving
/// workspace (S3-03), so a legitimate `origin_workspace_id` equals it; any
/// other value is a forged/legacy input and must not borrow another
/// workspace's bot token (S3-02).
pub fn result_workspace(workflow: &Workflow, origin: Option<&str>) -> String {
    if let Some(o) = origin.filter(|o| *o != workflow.workspace_id) {
        tracing::warn!(workflow_id = %workflow.id, origin = o,
              "run input names another workspace as its origin — reporting via the workflow's own");
    }
    workflow.workspace_id.clone()
}

/// Resolve the chat target for live progress + result delivery from the run
/// input. Honors an explicit `result_chat`(+`result_channel`/`result_thread`)
/// override, else the incoming-hook origin (`channel`/`chat`/`thread`).
/// Returns `None` for a manual UI run or a webhook-only trigger (nothing to
/// stream to).
pub fn resolve_chat_target(workflow: &Workflow, input: &Value) -> Option<ChatTarget> {
    let obj = input.as_object()?;
    let str_at = |k: &str| obj.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
    let ws = result_workspace(workflow, str_at("origin_workspace_id"));
    let (channel, chat, thread) = match str_at("result_chat") {
        Some(c) => (
            str_at("result_channel").or_else(|| str_at("channel")),
            Some(c),
            str_at("result_thread"),
        ),
        None => (str_at("channel"), str_at("chat"), str_at("thread")),
    };
    let channel = match channel {
        Some("slack") => Channel::Slack,
        Some("telegram") => Channel::Telegram,
        _ => return None,
    };
    Some(ChatTarget {
        ws,
        channel,
        chat: chat?.to_string(),
        thread: thread.map(str::to_string),
    })
}

/// A wait as a person says it: "40 s", "12 min", "1 h 5 min" (whole units,
/// rounded down — "queued for 59 s" never reads as a minute).
pub fn format_wait(waited: Duration) -> String {
    let secs = waited.as_secs();
    match secs {
        0..=59 => format!("{secs} s"),
        60..=3599 => format!("{} min", secs / 60),
        _ => {
            let (h, m) = (secs / 3600, (secs % 3600) / 60);
            if m == 0 {
                format!("{h} h")
            } else {
                format!("{h} h {m} min")
            }
        }
    }
}

/// The thread message when a queued run gets its slot. It starts with the
/// same "🚀 Started workflow" prefix as the immediate acknowledgement, so the
/// chat trigger's own-ack loop guard recognises it too.
pub fn dequeued_reply(name: &str, run_id: &str, waited: Duration) -> String {
    format!(
        "🚀 Started workflow **{name}** (run `{run_id}`) — it was queued for {}.",
        format_wait(waited)
    )
}

/// What to announce when a run leaves the queue, and where: `None` when the
/// run never waited (`waited` is `None` — it got a slot at once and its
/// acknowledgement already said "Started") or has no chat thread to tell
/// (a manual UI run, a webhook-only trigger).
pub fn dequeue_announcement(
    workflow: &Workflow,
    run_id: &str,
    input: &Value,
    waited: Option<Duration>,
) -> Option<(ChatTarget, String)> {
    let waited = waited?;
    let target = resolve_chat_target(workflow, input)?;
    if target.chat.trim().is_empty() {
        return None;
    }
    Some((target, dequeued_reply(&workflow.name, run_id, waited)))
}

/// Upper bound on the announcement: the run holds its slot while this posts,
/// and a hung Slack call must not keep its first step from starting.
const ANNOUNCE_TIMEOUT: Duration = Duration::from_secs(10);

/// Post [`dequeue_announcement`] to the run's chat thread — best-effort
/// (redacted, bounded by [`ANNOUNCE_TIMEOUT`], failures logged at debug). The
/// engine calls this right after a parked run gets its permit, BEFORE its
/// first step, so the thread reads "Started" ahead of any step progress.
pub async fn announce_dequeued<C: WorkflowCtx>(
    ctx: &C,
    workflow: &Workflow,
    run_id: &str,
    input: &Value,
    waited: Option<Duration>,
) {
    let Some((target, text)) = dequeue_announcement(workflow, run_id, input, waited) else {
        return;
    };
    let post = async {
        let integ = otto_state::IntegrationsRepo::new(ctx.pool().clone())
            .get(&target.ws, target.channel)
            .await
            .ok()??;
        let adapter = otto_channels::improve_notify::build_adapter(ctx.secrets(), &integ).await?;
        let text = otto_core::redact::redact_text(&text).value;
        adapter
            .send_formatted(&target.chat, target.thread.as_deref(), &text)
            .await
            .map_err(|e| tracing::debug!(%run_id, "dequeue announcement failed: {e}"))
            .ok()
    };
    if tokio::time::timeout(ANNOUNCE_TIMEOUT, post).await.is_err() {
        tracing::debug!(%run_id, "dequeue announcement timed out");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::workflows::WorkflowGraph;
    use serde_json::json;

    fn wf() -> Workflow {
        Workflow {
            id: "wf1".into(),
            workspace_id: "wf-ws".into(),
            name: "PR Reviewer".into(),
            description: String::new(),
            instructions: String::new(),
            graph: WorkflowGraph::default(),
            created_by: "u".into(),
            created_at: chrono::Utc::now(),
            updated_at: chrono::Utc::now(),
            version: 1,
            on_restart: "resume".into(),
        }
    }

    #[test]
    fn cap_parses_positive_integers_else_defaults_to_two() {
        assert_eq!(parse_cap(None), 2);
        assert_eq!(parse_cap(Some(" 4 ")), 4);
        assert_eq!(parse_cap(Some("0")), 2);
        assert_eq!(parse_cap(Some("lots")), 2);
        assert_eq!(parse_cap(Some("")), 2);
    }

    /// The gate is sized from the same cap the chat acknowledgement and the
    /// Workflows page read (nothing in this crate's tests takes a permit).
    #[test]
    fn the_gate_is_sized_from_the_shared_cap() {
        assert_eq!(run_gate().available_permits(), max_parallel_runs());
    }

    #[test]
    fn places_count_the_whole_line_but_list_only_this_workspace() {
        let line: Vec<String> = ["a", "b", "c", "d"].iter().map(|s| s.to_string()).collect();
        let mine = ["b", "d"].iter().map(|s| s.to_string()).collect();
        assert_eq!(
            places(&line, &mine),
            vec![
                QueuedPlace {
                    run_id: "b".into(),
                    position: 2
                },
                QueuedPlace {
                    run_id: "d".into(),
                    position: 4
                },
            ]
        );
        assert!(places(&line, &Default::default()).is_empty());
    }

    #[test]
    fn waits_read_in_whole_human_units() {
        assert_eq!(format_wait(Duration::from_secs(0)), "0 s");
        assert_eq!(format_wait(Duration::from_secs(59)), "59 s");
        assert_eq!(format_wait(Duration::from_secs(60)), "1 min");
        assert_eq!(format_wait(Duration::from_secs(47 * 60 + 30)), "47 min");
        assert_eq!(format_wait(Duration::from_secs(3600)), "1 h");
        assert_eq!(
            format_wait(Duration::from_secs(3600 + 5 * 60 + 9)),
            "1 h 5 min"
        );
    }

    #[test]
    fn dequeued_reply_names_the_run_and_the_wait() {
        assert_eq!(
            dequeued_reply("PR Reviewer", "r1", Duration::from_secs(25 * 60)),
            "🚀 Started workflow **PR Reviewer** (run `r1`) — it was queued for 25 min."
        );
    }

    #[test]
    fn only_a_run_that_waited_and_has_a_thread_is_announced() {
        let slack = json!({ "channel": "slack", "chat": "C1", "thread": "169.1" });
        // Started at once: its acknowledgement already said "Started".
        assert!(dequeue_announcement(&wf(), "r1", &slack, None).is_none());
        // Waited, from a Slack thread: announced in that thread.
        let (target, text) =
            dequeue_announcement(&wf(), "r1", &slack, Some(Duration::from_secs(90)))
                .expect("a queued Slack run is announced");
        assert!(matches!(target.channel, Channel::Slack));
        assert_eq!(target.chat, "C1");
        assert_eq!(target.thread.as_deref(), Some("169.1"));
        assert_eq!(target.ws, "wf-ws");
        assert!(text.starts_with("🚀 Started workflow **PR Reviewer** (run `r1`)"));
        assert!(text.ends_with("queued for 1 min."));
        // Waited, but a manual UI run: nobody to tell.
        let manual = json!({ "repo_id": "r" });
        assert!(
            dequeue_announcement(&wf(), "r1", &manual, Some(Duration::from_secs(90))).is_none()
        );
        // A schedule trigger reporting to a chat is told too.
        let scheduled =
            json!({ "trigger": "schedule", "result_channel": "slack", "result_chat": "C9" });
        let (target, _) =
            dequeue_announcement(&wf(), "r1", &scheduled, Some(Duration::from_secs(5)))
                .expect("result chat is a target");
        assert_eq!(target.chat, "C9");
    }

    #[test]
    fn chat_target_prefers_the_explicit_result_destination() {
        let t = resolve_chat_target(
            &wf(),
            &json!({ "channel": "slack", "chat": "C1", "result_chat": "C2", "result_channel": "telegram",
                     "origin_workspace_id": "other-ws" }),
        )
        .expect("target");
        assert!(matches!(t.channel, Channel::Telegram));
        assert_eq!(t.chat, "C2");
        assert_eq!(t.ws, "wf-ws", "S3-02: never another workspace's bot");
        assert!(
            resolve_chat_target(&wf(), &json!({ "channel": "webhook", "chat": "x" })).is_none()
        );
    }
}
