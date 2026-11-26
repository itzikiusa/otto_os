//! What a workflow run says in its chat thread (engine.md §9, ux.md §11,
//! addendum-chat-optimize.md §1.1): the live status message, the final post,
//! the needs-you post and the step-file captions — pure renderers over the
//! run's nodes and [`RunOverview`] — and [`ChatThread`] / [`LiveStatus`],
//! what the engine's progress pump posts through in either style.
//!
//! Text is GFM; each adapter converts it (`send_formatted` /
//! `edit_formatted` / `upload_with_caption`).

use std::collections::{HashMap, VecDeque};
use std::time::Duration;

use chrono::{DateTime, FixedOffset, Utc};
use otto_channels::adapter::{edit_needs_new_post, Adapter};
use otto_core::agent_runtime::{Attention, AttentionKind};
use otto_core::workflows::{
    DeliverableState, HoldKind, NodeRunState, NodeStatus, RunOverview, RunState, StepPhase,
    WorkflowGraph, WorkflowNode,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::time::Instant;

/// How a chat-started run reports in its thread: the run input's
/// `chat_updates` (a chat binding's spec value is copied there), else the
/// Start node's param, else `live`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChatUpdates {
    /// One status message edited in place, every step file captioned, new
    /// posts for needs-you, holds and the final result.
    Live,
    /// Today's thread byte for byte: a line per step and its file.
    PerStep,
}

impl ChatUpdates {
    pub const KEY: &'static str = "chat_updates";

    pub fn resolve(graph: &WorkflowGraph, input: &Value) -> Self {
        let start = graph.nodes.iter().find(|n| n.kind == "manual_trigger");
        let value = input
            .get(Self::KEY)
            .or_else(|| start.and_then(|n| n.params.get(Self::KEY)))
            .and_then(Value::as_str);
        if value == Some("per_step") {
            Self::PerStep
        } else {
            Self::Live
        }
    }
}

/// The steps the status message lists: today's reportable kinds (utility
/// kinds stay off) plus the review, the step people most want to see.
pub fn is_status_step(kind: &str) -> bool {
    !matches!(
        kind,
        "manual_trigger" | "log" | "transform" | "delay" | "condition"
    )
}

/// A step's name as the thread shows it: the author's, else its kind.
pub fn step_name(node: &WorkflowNode) -> &str {
    if node.name.trim().is_empty() {
        &node.kind
    } else {
        &node.name
    }
}

/// Who and when a rendering is for: the workflow's name, the clock, and the
/// daemon's local offset for "since 10:42".
#[derive(Debug, Clone, Copy)]
pub struct ChatView<'a> {
    pub workflow: &'a str,
    pub now: DateTime<Utc>,
    pub tz: FixedOffset,
}

/// Over this many characters the finished steps fold into one line.
pub const STATUS_FOLD_CHARS: usize = 3_000;

/// The live status message (engine.md §9.2).
pub fn render_status(
    view: &ChatView,
    overview: &RunOverview,
    nodes: &[NodeRunState],
    graph: &WorkflowGraph,
) -> String {
    let by_id: HashMap<&str, &NodeRunState> =
        nodes.iter().map(|n| (n.node_id.as_str(), n)).collect();
    let listed: Vec<(&WorkflowNode, Option<&NodeRunState>)> = pipeline_order(graph)
        .into_iter()
        .filter(|n| is_status_step(&n.kind))
        .map(|n| (n, by_id.get(n.id.as_str()).copied()))
        .collect();
    let head = format!("*{}*", view.workflow);
    if overview.state == RunState::Queued {
        let pos = overview.queue.as_ref().map_or(1, |q| q.position.max(1));
        return format!("{head} · queued\n⏳ Queued — {} in line", ordinal(pos));
    }
    let settled = |s: Option<&NodeRunState>| {
        s.is_some_and(|s| !matches!(s.status, NodeStatus::Pending | NodeStatus::Running))
    };
    let (done, total) = if overview.steps_total > 0 {
        (overview.steps_done, overview.steps_total)
    } else {
        let done = listed.iter().filter(|(_, s)| settled(*s)).count();
        (done as u32, listed.len() as u32)
    };
    let reviewing = listed
        .iter()
        .any(|(n, s)| n.kind == "review_run" && s.is_some_and(|s| s.status == NodeStatus::Running));
    let mut first = format!(
        "{head} · {} · {done} of {total} steps · {}",
        state_word(overview.state, reviewing),
        clock(run_elapsed(view, overview, nodes))
    );
    if let Some(t) = overview.typical_ms {
        first.push_str(&format!(" (usually {})", approx(t)));
    }
    let lines: Vec<(bool, String)> = listed
        .iter()
        .map(|(n, s)| {
            let ok = s.is_some_and(|s| s.status == NodeStatus::Success);
            (ok, status_line(view, n, *s))
        })
        .collect();
    let full = std::iter::once(first.clone())
        .chain(lines.iter().map(|(_, l)| l.clone()))
        .collect::<Vec<_>>()
        .join("\n");
    if full.chars().count() <= STATUS_FOLD_CHARS {
        return full;
    }
    let folded = lines.iter().filter(|(ok, _)| *ok).count();
    let mut out = vec![first];
    let mut said = false;
    for (ok, line) in lines {
        match (ok, said) {
            (true, false) => {
                out.push(format!("✅ {folded} steps done"));
                said = true;
            }
            (true, true) => {}
            (false, _) => out.push(line),
        }
    }
    out.join("\n")
}

/// One step's line on the status message.
fn status_line(view: &ChatView, node: &WorkflowNode, state: Option<&NodeRunState>) -> String {
    let name = step_name(node);
    let Some(s) = state else {
        return format!("◻ {name}");
    };
    match s.status {
        NodeStatus::Pending => format!("◻ {name}"),
        NodeStatus::Success => {
            let mut line = format!("✅ {name} · {}", clock(s.duration_ms.unwrap_or(0)));
            if let Some(b) = s.brief.as_deref().filter(|b| !b.trim().is_empty()) {
                line.push_str(&format!(" — {}", clip(b, 100)));
            }
            line
        }
        NodeStatus::Error => format!("❌ {name} failed — {}", clip(error_of(s), 120)),
        NodeStatus::Skipped if s.error_class.as_deref() == Some("canceled") => {
            format!("⏹ {name} — stopped")
        }
        NodeStatus::Skipped => match s.error.as_deref().filter(|e| !e.trim().is_empty()) {
            Some(why) => format!("⏭ {name} — skipped ({})", clip(why, 120)),
            None => format!("⏭ {name} — skipped"),
        },
        NodeStatus::Running => match (s.phase, &s.wait, &s.attention) {
            (Some(StepPhase::NeedsYou), _, Some(a)) => {
                format!("⚠️ {name} — {}", clip(&a.text, 160))
            }
            (Some(StepPhase::Waiting | StepPhase::Queued), Some(w), _) => {
                format!("⏳ {name} — {} since {}", w.text, hhmm(view, w.since))
            }
            (Some(StepPhase::Queued), None, _) => format!("⏳ {name} — queued"),
            _ => {
                let elapsed = s
                    .started_at
                    .map(|t| (view.now - t).num_milliseconds().max(0) as u64)
                    .unwrap_or(0);
                let mut line = format!("▶ {name} · {}", clock(elapsed));
                if let Some(words) = crate::overview::fan_words(&node.kind, s) {
                    line.push_str(&format!(" — {words}"));
                }
                line
            }
        },
    }
}

/// The post a finished (or held) run ends its thread with (engine.md §9.2);
/// `link` is the "Open in Otto" run page URL when the Mac has a reachable one.
pub fn render_final(
    view: &ChatView,
    overview: &RunOverview,
    nodes: &[NodeRunState],
    graph: &WorkflowGraph,
    link: Option<&str>,
) -> String {
    let name = view.workflow;
    let took = clock(run_elapsed(view, overview, nodes));
    let names: HashMap<&str, &str> = graph
        .nodes
        .iter()
        .map(|n| (n.id.as_str(), step_name(n)))
        .collect();
    let mut lines = vec![];
    if let Some(hold) = &overview.hold {
        let reason = clip(&hold.reason, 300);
        lines.push(match hold.kind {
            HoldKind::Paused => format!("⏸ *{name}* paused — {reason}"),
            HoldKind::Blocked => format!("⚠️ *{name}* is blocked — {reason}"),
            HoldKind::Exhausted => format!("⚠️ *{name}* is out of budget — {reason}"),
            HoldKind::Unknown => format!("⏸ *{name}* is on hold — {reason}"),
        });
    } else {
        match overview.state {
            RunState::Failed => {
                let (step, error) = match &overview.failed_step {
                    Some(f) => (f.name.clone(), f.error.clone()),
                    None => nodes
                        .iter()
                        .find(|n| n.status == NodeStatus::Error)
                        .map(|n| {
                            let step = names.get(n.node_id.as_str()).copied().unwrap_or(&n.node_id);
                            (step.to_string(), error_of(n).to_string())
                        })
                        .unwrap_or_default(),
                };
                lines.push(if step.is_empty() {
                    format!("❌ *{name}* failed · {took}")
                } else {
                    format!("❌ *{name}* failed at *{step}* — {}", clip(&error, 300))
                });
            }
            RunState::Canceled => {
                let at = nodes
                    .iter()
                    .find(|n| n.error_class.as_deref() == Some("canceled"))
                    .and_then(|n| names.get(n.node_id.as_str()));
                lines.push(match at {
                    Some(step) => format!("⏹ *{name}* canceled at *{step}* · {took}"),
                    None => format!("⏹ *{name}* canceled · {took}"),
                });
            }
            _ => {
                let short = overview
                    .deliverables
                    .iter()
                    .any(|d| d.state != DeliverableState::Done);
                let icon = if short { "⚠️" } else { "✅" };
                lines.push(match review_verdict(nodes, graph) {
                    Some(Verdict {
                        score,
                        passed: true,
                        ..
                    }) => {
                        format!("{icon} *{name}* passed — {score}/100 · {took}")
                    }
                    // The run went on (its notify branch said so), the review did not pass.
                    Some(Verdict { score, .. }) => {
                        format!("⚠️ *{name}* done — review {score}/100, not passed · {took}")
                    }
                    None => format!("{icon} *{name}* done · {took}"),
                });
            }
        }
    }
    for d in &overview.deliverables {
        let detail = d.detail.as_deref().filter(|s| !s.trim().is_empty());
        let words = match d.state {
            DeliverableState::Done => detail.unwrap_or("posted").to_string(),
            DeliverableState::Partial => detail.unwrap_or("partly posted").to_string(),
            DeliverableState::Failed => match detail {
                Some(why) => format!("not posted ({why})"),
                None => "not posted".into(),
            },
            DeliverableState::PendingApproval => "pending approval".into(),
            DeliverableState::Unknown => "not confirmed".into(),
        };
        lines.push(match &d.url {
            Some(url) => format!("• [{}]({url}): {words}", d.label),
            None => format!("• {}: {words}", d.label),
        });
    }
    let posted = overview
        .deliverables
        .iter()
        .any(|d| matches!(d.state, DeliverableState::Done | DeliverableState::Partial));
    if overview.state == RunState::Failed && !posted && overview.hold.is_none() {
        if let Some(pr) = overview.subject.as_ref().and_then(|s| s.pr.as_ref()) {
            lines.push(format!("Nothing was posted on PR #{}.", pr.number));
        }
    }
    if let Some(url) = link {
        lines.push(format!("[Open in Otto]({url})"));
    }
    lines.join("\n")
}

/// The new post when a step starts waiting on a person (edits do not
/// notify). `mention` is the requester's Slack user id (none on Telegram).
pub fn render_needs_you(
    view: &ChatView,
    step: &str,
    attention: &Attention,
    mention: Option<&str>,
    link: Option<&str>,
) -> String {
    let who = mention.map(|u| format!("<@{u}> ")).unwrap_or_default();
    let text = attention.text.trim().trim_end_matches('.');
    let (what, todo) = match attention.kind {
        AttentionKind::Approval => (
            "is waiting for an approval",
            "Approve it in Otto (Workflows → Runs).",
        ),
        AttentionKind::Question => ("has a question", "Answer it in Otto (Workflows → Runs)."),
        AttentionKind::Verify => (
            "is waiting for you to verify",
            "Verify it in Otto (Workflows → Runs).",
        ),
        AttentionKind::UsageLimit => (
            "stopped at the usage limit",
            "Otto continues it when the limit resets.",
        ),
        AttentionKind::Unknown => ("needs you", "Open it in Otto (Workflows → Runs)."),
    };
    let detail = if text.is_empty() {
        String::new()
    } else {
        format!(" — {}", clip(text, 300))
    };
    let mut out = format!(
        "⚠️ {who}*{}* needs you — *{step}* {what}{detail}. {todo}",
        view.workflow
    );
    if let Some(url) = link {
        out.push_str(&format!("\n[Open in Otto]({url})"));
    }
    out
}

/// The one-line caption of a step's handoff file (addendum §1.1), redacted
/// like the file body. `iteration` marks a loop's inner step file.
pub fn step_caption(step: &str, state: &NodeRunState, iteration: Option<u32>) -> String {
    let failed = state.status == NodeStatus::Error;
    let error = clip(error_of(state), 120);
    let caption = match (iteration, failed) {
        (None, false) => format!("✅ {step} · {}", clock(state.duration_ms.unwrap_or(0))),
        (None, true) => format!("❌ {step} failed — {error}"),
        (Some(k), false) => format!("› ✅ {step} · iteration {k}"),
        (Some(k), true) => format!("› ❌ {step} failed · iteration {k} — {error}"),
    };
    otto_core::redact::redact_text(&caption).value
}

/// Where the live status message lives — stored as the run's
/// `chat_status_json` so a restart, a resume or a Retry step edits the
/// same message instead of posting a second one.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StatusMessage {
    pub channel: String,
    pub chat: String,
    #[serde(default)]
    pub thread: Option<String>,
    #[serde(default)]
    pub message_id: Option<String>,
}

/// At most one edit per this interval; the pump keeps only the newest
/// pending text meanwhile and always sends it once the interval is over.
pub const STATUS_EDIT_EVERY: Duration = Duration::from_secs(3);

/// The longest rate-limit wait honoured before an update is given up.
const MAX_RATE_WAIT: Duration = Duration::from_secs(60);

/// The status message of one run, as the progress pump drives it.
pub struct LiveStatus {
    pub msg: StatusMessage,
    shown: Option<String>,
    last_at: Option<Instant>,
}

impl LiveStatus {
    pub fn new(msg: StatusMessage) -> Self {
        Self {
            msg,
            shown: None,
            last_at: None,
        }
    }

    /// How long the pump waits before the next [`Self::show`].
    pub fn wait(&self) -> Duration {
        self.last_at.map_or(Duration::ZERO, |t| {
            STATUS_EDIT_EVERY.saturating_sub(t.elapsed())
        })
    }

    /// Show `text`. Posts when there is no message yet, otherwise edits it;
    /// an unchanged text sends nothing; a message that can no longer be
    /// edited (deleted, past the edit window) is replaced by a new post; a
    /// rate limit is waited out once. `Ok(true)`: `msg.message_id` changed —
    /// persist `msg`.
    pub async fn show(&mut self, adapter: &dyn Adapter, text: &str) -> anyhow::Result<bool> {
        if self.shown.as_deref() == Some(text) && self.msg.message_id.is_some() {
            return Ok(false);
        }
        let mut result = self.try_show(adapter, text).await;
        if let Some(delay) = result
            .as_ref()
            .err()
            .and_then(otto_channels::mirror::retry_after_hint)
        {
            tokio::time::sleep(delay.min(MAX_RATE_WAIT)).await;
            result = self.try_show(adapter, text).await;
        }
        self.last_at = Some(Instant::now());
        let moved = result?;
        self.shown = Some(text.to_string());
        Ok(moved)
    }

    async fn try_show(&mut self, adapter: &dyn Adapter, text: &str) -> anyhow::Result<bool> {
        if let Some(id) = self.msg.message_id.clone() {
            match adapter.edit_formatted(&self.msg.chat, &id, text).await {
                Ok(()) => return Ok(false),
                Err(e) if edit_needs_new_post(&e) => {
                    tracing::info!(
                        "workflow chat: status message {id} is gone ({e}); posting a new one"
                    );
                }
                Err(e) => return Err(e),
            }
        }
        let id = adapter
            .send_formatted(&self.msg.chat, self.msg.thread.as_deref(), text)
            .await?;
        self.msg.message_id = Some(id);
        Ok(true)
    }
}

/// One run's chat thread as the progress pump drives it, in either style
/// (owner's answer 1): `live` drops the per-step text lines and keeps one
/// status message, but every step file is still posted (captioned) and the
/// final post and its file still follow; `per_step` is today's thread.
/// Posting is serialized by the caller (one pump per run).
pub struct ChatThread {
    pub style: ChatUpdates,
    /// The status message (its `msg` is `chat_status_json`).
    pub live: LiveStatus,
    adapter: std::sync::Arc<dyn Adapter>,
    pending: Option<String>,
}

impl ChatThread {
    pub fn new(
        style: ChatUpdates,
        adapter: std::sync::Arc<dyn Adapter>,
        msg: StatusMessage,
    ) -> Self {
        Self {
            style,
            live: LiveStatus::new(msg),
            adapter,
            pending: None,
        }
    }

    fn chat(&self) -> (String, Option<String>) {
        (self.live.msg.chat.clone(), self.live.msg.thread.clone())
    }

    /// A per-step text line (today's "▶ started" / "✅ done" / review lines).
    /// Live style drops it: the status message says it.
    pub async fn line(&mut self, text: &str) {
        if self.style == ChatUpdates::PerStep {
            self.send(text).await;
        }
    }

    /// The status message's newest text: shown now when the last edit is 3 s
    /// old, else kept (newer texts replace it) until [`Self::flush`]. Per-step
    /// style has no status message. `true`: the message moved — persist
    /// `live.msg`.
    pub async fn status(&mut self, text: String) -> bool {
        if self.style == ChatUpdates::PerStep {
            return false;
        }
        self.pending = Some(text);
        if self.live.wait().is_zero() {
            self.flush().await
        } else {
            false
        }
    }

    /// When the pump should call [`Self::flush`] (a kept status text waits).
    pub fn flush_in(&self) -> Option<Duration> {
        self.pending.as_ref().map(|_| self.live.wait())
    }

    /// Show the kept status text, if any (the guaranteed trailing edit).
    pub async fn flush(&mut self) -> bool {
        let Some(text) = self.pending.take() else {
            return false;
        };
        match self.live.show(self.adapter.as_ref(), &text).await {
            Ok(moved) => moved,
            Err(e) => {
                tracing::warn!("workflow chat: status message update failed: {e}");
                false
            }
        }
    }

    /// A step's handoff file, when the step settles — in both styles; live
    /// style captions it. A rate limit is waited out and the upload retried
    /// once; a file that still fails is logged and skipped.
    pub async fn step_file(&mut self, name: &str, content: &[u8], caption: &str) {
        let (chat, thread) = self.chat();
        let caption = (self.style == ChatUpdates::Live).then_some(caption);
        let upload = || async {
            match caption {
                Some(c) => {
                    self.adapter
                        .upload_with_caption(&chat, thread.as_deref(), name, content, c)
                        .await
                }
                None => {
                    self.adapter
                        .upload(&chat, thread.as_deref(), name, content)
                        .await
                }
            }
        };
        let mut result = upload().await;
        if let Some(delay) = result
            .as_ref()
            .err()
            .and_then(otto_channels::mirror::retry_after_hint)
        {
            tokio::time::sleep(delay.min(MAX_RATE_WAIT)).await;
            result = upload().await;
        }
        if let Err(e) = result {
            tracing::warn!("workflow chat: step file {name} not posted: {e}");
        }
    }

    /// A new post (needs-you, hold, the final message), after the status
    /// message is brought up to date; `true` when that moved the status
    /// message (store it, like [`Self::flush`]'s).
    pub async fn post(&mut self, text: &str) -> bool {
        let moved = self.flush().await;
        self.send(text).await;
        moved
    }

    /// A file after a post (`final-output.md`), uncaptioned in both styles.
    pub async fn file(&mut self, name: &str, content: &[u8]) {
        let (chat, thread) = self.chat();
        if let Err(e) = self
            .adapter
            .upload(&chat, thread.as_deref(), name, content)
            .await
        {
            tracing::warn!("workflow chat: {name} not posted: {e}");
        }
    }

    async fn send(&self, text: &str) {
        let (chat, thread) = self.chat();
        if let Err(e) = self
            .adapter
            .send_formatted(&chat, thread.as_deref(), text)
            .await
        {
            tracing::warn!("workflow chat: post failed: {e}");
        }
    }
}

/// The graph's nodes in pipeline order: a stable topological order (ties
/// keep the graph's order), so the card reads top to bottom as the run goes.
fn pipeline_order(graph: &WorkflowGraph) -> Vec<&WorkflowNode> {
    let index: HashMap<&str, usize> = graph
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| (n.id.as_str(), i))
        .collect();
    let mut indegree = vec![0usize; graph.nodes.len()];
    let mut next: Vec<Vec<usize>> = vec![vec![]; graph.nodes.len()];
    for e in &graph.edges {
        if let (Some(&a), Some(&b)) = (index.get(e.source.as_str()), index.get(e.target.as_str())) {
            next[a].push(b);
            indegree[b] += 1;
        }
    }
    let mut ready: VecDeque<usize> = (0..graph.nodes.len())
        .filter(|&i| indegree[i] == 0)
        .collect();
    let mut order = Vec::with_capacity(graph.nodes.len());
    let mut seen = vec![false; graph.nodes.len()];
    while let Some(i) = ready.pop_front() {
        seen[i] = true;
        order.push(i);
        let mut opened: Vec<usize> = vec![];
        for &j in &next[i] {
            indegree[j] -= 1;
            if indegree[j] == 0 {
                opened.push(j);
            }
        }
        opened.sort_unstable();
        ready.extend(opened);
    }
    // A cycle (loops are one node, so none is expected): the rest as stored.
    order.extend((0..graph.nodes.len()).filter(|&i| !seen[i]));
    order.into_iter().map(|i| &graph.nodes[i]).collect()
}

fn state_word(state: RunState, reviewing: bool) -> &'static str {
    match state {
        RunState::Waiting => "waiting",
        RunState::NeedsYou => "needs you",
        RunState::Queued => "queued",
        RunState::Succeeded => "done",
        RunState::Failed => "failed",
        RunState::Canceled => "canceled",
        RunState::Paused => "paused",
        RunState::Blocked => "blocked",
        RunState::Exhausted => "out of budget",
        RunState::Running | RunState::Unknown if reviewing => "reviewing",
        RunState::Running | RunState::Unknown => "working",
    }
}

/// The run's wall clock: the overview's, else since its first step started.
fn run_elapsed(view: &ChatView, overview: &RunOverview, nodes: &[NodeRunState]) -> u64 {
    if overview.elapsed_ms > 0 {
        return overview.elapsed_ms;
    }
    nodes
        .iter()
        .filter_map(|n| n.started_at)
        .min()
        .map_or(0, |t| (view.now - t).num_milliseconds().max(0) as u64)
}

/// A review step's outcome (`review_run`'s output: score, passed — a
/// blocking finding fails a high score too).
struct Verdict {
    score: u64,
    passed: bool,
}

fn review_verdict(nodes: &[NodeRunState], graph: &WorkflowGraph) -> Option<Verdict> {
    let out = graph
        .nodes
        .iter()
        .filter(|n| n.kind == "review_run")
        .filter_map(|n| nodes.iter().find(|s| s.node_id == n.id))
        .find_map(|s| s.output.as_ref().filter(|o| o.get("score").is_some()))?;
    Some(Verdict {
        score: out["score"].as_f64()?.round() as u64,
        passed: out["passed"].as_bool().unwrap_or(true),
    })
}

fn error_of(state: &NodeRunState) -> &str {
    state
        .error
        .as_deref()
        .and_then(|e| e.lines().find(|l| !l.trim().is_empty()))
        .map_or("", str::trim)
}

/// "0s", "14s", "1m 08s", "11m 52s", "1h 05m".
pub fn clock(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m {:02}s", s / 60, s % 60),
        _ => format!("{}h {:02}m", s / 3600, (s % 3600) / 60),
    }
}

/// "45s", "12m", "1h 5m" — a typical duration, to the minute.
fn approx(ms: u64) -> String {
    let s = ms / 1000;
    match s {
        0..=59 => format!("{s}s"),
        60..=3599 => format!("{}m", (s + 30) / 60),
        _ => format!("{}h {}m", s / 3600, (s % 3600) / 60),
    }
}

fn hhmm(view: &ChatView, t: DateTime<Utc>) -> String {
    t.with_timezone(&view.tz).format("%H:%M").to_string()
}

fn ordinal(n: u32) -> String {
    let suffix = match (n % 10, n % 100) {
        (_, 11..=13) => "th",
        (1, _) => "st",
        (2, _) => "nd",
        (3, _) => "rd",
        _ => "th",
    };
    format!("{n}{suffix}")
}

fn clip(s: &str, max: usize) -> String {
    otto_channels::adapter::clip_chars(s.trim(), max)
}

#[cfg(test)]
#[path = "chat_status_tests.rs"]
mod tests;
