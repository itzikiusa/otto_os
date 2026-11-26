//! The run's one computed summary (`RunOverview`, engine.md §2.3): the state
//! a person reads, one headline sentence, the counts, what runs now, who must
//! act, where the time went. The UI, the chat status message, MCP and the
//! phone all read this object, so the words are the same everywhere. Pure.

use chrono::{DateTime, Utc};
use otto_core::agent_runtime::{AttentionKind, TurnTokenUsage};
use otto_core::workflows::{
    AttentionRef, DeliverableState, FailedStep, FanProgress, HoldKind, NodeRunState, NodeStatus,
    Now, Origin, QueueInfo, RunHold, RunOverview, RunState, RunStatus, StepPhase, TimeBuckets,
    WorkflowGraph, WorkflowRun,
};
use serde_json::Value;

/// Bounds that keep the overview small (≤ 4 KiB on the WS event).
const MAX_NOW_NODES: usize = 8;
const MAX_ATTENTION: usize = 8;
const MAX_DELIVERABLES: usize = 12;
const MAX_HEADLINE: usize = 160;
const MAX_TEXT: usize = 300;

/// The hold that counts for a run with this status (DR-105): the one rule,
/// [`RunHold::is_effective`] (`otto_state`'s `HOLD_SQL` is its SQL twin).
pub fn effective_hold(status: RunStatus, hold: Option<&RunHold>) -> Option<&RunHold> {
    hold.filter(|h| h.is_effective(status))
}

/// Compute the run's overview.
///
/// `queue` is the run's place in line while it is pending, `typical_ms` the
/// median of the workflow's last 20 successful runs (`None` under 3), `now`
/// the clock (the end of a run that has not finished). `origin` and
/// `subject` are computed once at admission (names from the directory
/// cache, PR URLs) and carried over from `run.overview`; without one, the
/// origin falls back to the trigger in the run input. `rev` is the row's.
pub fn run_overview(
    run: &WorkflowRun,
    graph: &WorkflowGraph,
    queue: Option<QueueInfo>,
    typical_ms: Option<u64>,
    now: DateTime<Utc>,
) -> RunOverview {
    let name = |id: &str| step_name(graph, id);
    let hold = effective_hold(run.status, run.hold.as_ref()).cloned();
    let running: Vec<&NodeRunState> = run
        .nodes
        .iter()
        .filter(|n| n.status == NodeStatus::Running)
        .collect();

    let mut attention: Vec<AttentionRef> = running
        .iter()
        .filter_map(|n| {
            n.attention.clone().map(|mut a| {
                a.text = clip(&a.text, MAX_TEXT);
                AttentionRef {
                    node_id: n.node_id.clone(),
                    name: name(&n.node_id),
                    attention: a,
                }
            })
        })
        .collect();
    attention.sort_by_key(|a| a.attention.since);
    attention.truncate(MAX_ATTENTION);

    let state = match (&hold, run.status) {
        (Some(h), _) => match h.kind {
            HoldKind::Paused => RunState::Paused,
            HoldKind::Blocked => RunState::Blocked,
            HoldKind::Exhausted => RunState::Exhausted,
            HoldKind::Unknown => RunState::Unknown,
        },
        (None, RunStatus::Pending) => RunState::Queued,
        (None, RunStatus::Success) => RunState::Succeeded,
        (None, RunStatus::Error) => RunState::Failed,
        (None, RunStatus::Canceled) => RunState::Canceled,
        (None, RunStatus::Running) if !attention.is_empty() || run.waiting_approval => {
            RunState::NeedsYou
        }
        (None, RunStatus::Running)
            if !running.is_empty()
                && running
                    .iter()
                    .all(|n| matches!(n.phase, Some(StepPhase::Waiting | StepPhase::Queued))) =>
        {
            RunState::Waiting
        }
        (None, RunStatus::Running) => RunState::Running,
    };

    let in_step_order = {
        let mut nodes: Vec<(usize, &NodeRunState)> = run.nodes.iter().enumerate().collect();
        nodes.sort_by_key(|(i, n)| (n.step_no.unwrap_or(u32::MAX), *i));
        nodes.into_iter().map(|(_, n)| n).collect::<Vec<_>>()
    };
    let executed = |n: &&NodeRunState| n.status != NodeStatus::Skipped || n.started_at.is_some();
    let steps_total = run.nodes.iter().filter(executed).count() as u32;
    let steps_done = run
        .nodes
        .iter()
        .filter(executed)
        .filter(|n| !matches!(n.status, NodeStatus::Pending | NodeStatus::Running))
        .count() as u32;

    let now_info = (!running.is_empty()).then(|| {
        let phase = running
            .iter()
            .filter_map(|n| n.phase)
            .max_by_key(|p| phase_rank(*p))
            .unwrap_or(StepPhase::Working);
        Now {
            node_ids: running
                .iter()
                .take(MAX_NOW_NODES)
                .map(|n| n.node_id.clone())
                .collect(),
            node_names: running
                .iter()
                .take(MAX_NOW_NODES)
                .map(|n| name(&n.node_id))
                .collect(),
            phase,
            wait_text: running
                .iter()
                .find_map(|n| n.wait.as_ref())
                .map(|w| clip(&w.text, 200)),
            fan: running
                .iter()
                .find(|n| !n.children.is_empty())
                .map(|n| fan(n)),
        }
    });

    let failed_step = in_step_order
        .iter()
        .find(|n| n.status == NodeStatus::Error)
        .map(|n| FailedStep {
            node_id: n.node_id.clone(),
            name: name(&n.node_id),
            error: clip(n.error.as_deref().unwrap_or("failed"), MAX_TEXT),
            error_class: n.error_class.clone(),
        });

    let mut deliverables: Vec<_> = in_step_order
        .iter()
        .flat_map(|n| n.deliverables.iter().cloned())
        .collect();
    deliverables.truncate(MAX_DELIVERABLES);

    let mut usage = TurnTokenUsage::default();
    for n in &run.nodes {
        if let Some(t) = &n.thread {
            add_usage(&mut usage, &t.usage);
        }
        for c in &n.children {
            add_usage(&mut usage, &c.usage);
        }
    }

    let end = run.finished_at.unwrap_or(now);
    let prev = run.overview.as_ref();
    let mut overview = RunOverview {
        rev: run.rev,
        state,
        headline: String::new(),
        steps_done,
        steps_total,
        now: now_info,
        attention,
        queue: queue.filter(|_| run.status == RunStatus::Pending),
        failed_step,
        deliverables,
        usage,
        time: time_buckets(run, end),
        origin: prev
            .and_then(|o| o.origin.clone())
            .or_else(|| origin_from_input(&run.input)),
        subject: prev.and_then(|o| o.subject.clone()),
        elapsed_ms: millis(run.started_at, end),
        typical_ms,
        goal: prev.and_then(|o| o.goal.clone()),
        hold,
    };
    overview.headline = clip(&headline(run, graph, &overview), MAX_HEADLINE);
    overview
}

/// The one sentence (engine.md §2.3, ux.md §2.3, goal.md §2.3 — first match).
fn headline(run: &WorkflowRun, graph: &WorkflowGraph, o: &RunOverview) -> String {
    let name = |id: &str| step_name(graph, id);
    let running: Vec<&NodeRunState> = run
        .nodes
        .iter()
        .filter(|n| n.status == NodeStatus::Running)
        .collect();
    let first_line = |s: &str| s.lines().next().unwrap_or_default().trim().to_string();
    match o.state {
        RunState::Paused => {
            let hold = o.hold.as_ref();
            let at = hold.and_then(|h| h.node_ids.first()).map(|id| name(id));
            match (hold.and_then(|h| h.by.as_ref()), at) {
                (Some(by), Some(at)) => format!("Paused by {by} at {at}"),
                (None, Some(at)) => format!("Paused at {at}"),
                (Some(by), None) => format!("Paused by {by}"),
                (None, None) => "Paused".into(),
            }
        }
        RunState::Blocked => format!(
            "Blocked: {}",
            o.hold
                .as_ref()
                .map(|h| h.reason.as_str())
                .unwrap_or_default()
        ),
        RunState::Exhausted => format!(
            "Out of budget: {} — extend to continue",
            o.hold
                .as_ref()
                .map(|h| h.reason.as_str())
                .unwrap_or_default()
        ),
        RunState::NeedsYou => match o.attention.first() {
            Some(a) if a.attention.kind == AttentionKind::UsageLimit => a.attention.text.clone(),
            Some(a) => format!("Needs you — {}", a.attention.text),
            None => match run.approval_node_id.as_deref() {
                Some(gate) => format!("Needs you — approve {} to continue", name(gate)),
                None => "Needs you".into(),
            },
        },
        RunState::Waiting => {
            let n = running
                .iter()
                .find(|n| n.wait.is_some())
                .or(running.first())
                .copied();
            match n {
                Some(n) => format!(
                    "{}: {}",
                    name(&n.node_id),
                    n.wait
                        .as_ref()
                        .map(|w| w.text.as_str())
                        .unwrap_or("waiting")
                ),
                None => "Waiting".into(),
            }
        }
        RunState::Queued => match &o.queue {
            Some(q) => format!("Queued — {} in line", ordinal(q.position)),
            None => "Queued".into(),
        },
        RunState::Running => match running.as_slice() {
            [] => "Working".into(),
            [n] => format!("{} — {}", name(&n.node_id), activity(graph, n)),
            several => {
                let names: Vec<String> = several.iter().take(3).map(|n| name(&n.node_id)).collect();
                let more = if several.len() > 3 { " …" } else { "" };
                format!(
                    "{} steps running: {}{more}",
                    several.len(),
                    names.join(", ")
                )
            }
        },
        RunState::Failed => match &o.failed_step {
            Some(f) => format!("Failed at {}: {}", f.name, first_line(&f.error)),
            None => match run.error.as_deref() {
                Some(e) => format!("Failed: {}", first_line(e)),
                None => "Failed".into(),
            },
        },
        RunState::Succeeded => succeeded(run, graph, o),
        RunState::Canceled => {
            let at = run
                .nodes
                .iter()
                .find(|n| n.error_class.as_deref() == Some("canceled"))
                .map(|n| name(&n.node_id));
            match at {
                Some(at) => format!("Canceled at {at}"),
                None => "Canceled".into(),
            }
        }
        RunState::Unknown => String::new(),
    }
}

/// "Passed — 86/100, 5 of 5 PR comments posted", "Done — 11 steps", or a
/// deliverable that did not land first ("Done, but the Jira comment was not
/// posted: …").
fn succeeded(run: &WorkflowRun, graph: &WorkflowGraph, o: &RunOverview) -> String {
    if let Some(d) = o
        .deliverables
        .iter()
        .find(|d| d.state != DeliverableState::Done && d.state != DeliverableState::Unknown)
    {
        let plural = d.label.ends_with('s');
        let detail = d
            .detail
            .as_deref()
            .map(|s| format!(": {s}"))
            .unwrap_or_default();
        return match d.state {
            DeliverableState::Failed => {
                format!(
                    "Done, but the {} {} not posted{detail}",
                    d.label,
                    if plural { "were" } else { "was" }
                )
            }
            DeliverableState::Partial => {
                format!(
                    "Done, but the {} {} only partly posted{detail}",
                    d.label,
                    if plural { "were" } else { "was" }
                )
            }
            _ => format!(
                "Done, but the {} {} waiting for approval",
                d.label,
                if plural { "are" } else { "is" }
            ),
        };
    }
    let delivered: Vec<String> = o
        .deliverables
        .iter()
        .map(|d| match d.detail.as_deref() {
            Some(detail) => match detail
                .strip_suffix(" posted")
                .filter(|n| n.starts_with(|c: char| c.is_ascii_digit()))
            {
                Some(count) => format!("{count} {} posted", d.label),
                None => format!("{}: {detail}", d.label),
            },
            None => format!("{} posted", d.label),
        })
        .collect();
    let review = run.nodes.iter().find_map(|n| {
        let kind = graph
            .nodes
            .iter()
            .find(|g| g.id == n.node_id)?
            .kind
            .as_str();
        let out = n
            .output
            .as_ref()
            .filter(|_| kind == "review_run" && n.status == NodeStatus::Success)?;
        Some((
            out.get("score")?.as_f64()?,
            out.get("passed").and_then(Value::as_bool) == Some(true),
        ))
    });
    let tail = if delivered.is_empty() {
        String::new()
    } else {
        format!(", {}", delivered.join(", "))
    };
    match review {
        Some((score, passed)) => format!(
            "{} — {}/100{tail}",
            if passed { "Passed" } else { "Not passed" },
            score.round() as i64
        ),
        None if !delivered.is_empty() => format!("Done — {}", delivered.join(", ")),
        None => format!("Done — {} steps", o.steps_done),
    }
}

/// What one running step is doing: a review's "9 of 12 reviewers done", the
/// runtime's current activity line, or "working".
fn activity(graph: &WorkflowGraph, n: &NodeRunState) -> String {
    if !n.children.is_empty() {
        let f = fan(n);
        let kind = graph
            .nodes
            .iter()
            .find(|g| g.id == n.node_id)
            .map(|g| g.kind.as_str());
        let what = if kind == Some("review_run") {
            " reviewers"
        } else {
            ""
        };
        return format!("{} of {}{what} done", f.done, f.total);
    }
    n.activity
        .as_ref()
        .map(|a| a.phase.trim())
        .filter(|p| !p.is_empty())
        .map(|p| clip(p, 120))
        .unwrap_or_else(|| "working".into())
}

fn fan(n: &NodeRunState) -> FanProgress {
    let count = |states: &[&str]| {
        n.children
            .iter()
            .filter(|c| states.contains(&c.status.as_str()))
            .count() as u32
    };
    FanProgress {
        done: count(&["success", "error", "skipped"]),
        total: n.children.len() as u32,
        running: count(&["running", "needs_you"]),
    }
}

/// Where the run's wall clock went: every instant from `run.started_at` to
/// `end` is counted once, in the best bucket any step is in at that instant
/// (working ≥ starting > waiting > with you > queued). Before the first step
/// started the run was queued; a gap between steps is the engine working.
pub fn time_buckets(run: &WorkflowRun, end: DateTime<Utc>) -> TimeBuckets {
    // (from, to, rank): rank 4 working, 3 waiting, 2 with you, 1 queued.
    let mut spans: Vec<(i64, i64, u8)> = Vec::new();
    let end_ms = end.timestamp_millis();
    for n in &run.nodes {
        let Some(started) = n.started_at else {
            continue;
        };
        let node_end = match (n.status, n.duration_ms) {
            (NodeStatus::Running, _) | (_, None) => end_ms,
            (_, Some(d)) => started.timestamp_millis() + d as i64,
        }
        .min(end_ms);
        if n.phases.is_empty() {
            spans.push((started.timestamp_millis(), node_end, 4));
            continue;
        }
        for (i, mark) in n.phases.iter().enumerate() {
            let to = n
                .phases
                .get(i + 1)
                .map(|m| m.at.timestamp_millis())
                .unwrap_or(node_end)
                .min(node_end);
            let rank = match mark.phase {
                StepPhase::Queued => 1,
                StepPhase::NeedsYou => 2,
                StepPhase::Waiting => 3,
                _ => 4,
            };
            spans.push((mark.at.timestamp_millis(), to, rank));
        }
    }
    let start_ms = run.started_at.timestamp_millis();
    let first_step = spans.iter().map(|s| s.0).min().unwrap_or(end_ms);
    let mut cuts: Vec<i64> = spans
        .iter()
        .flat_map(|s| [s.0, s.1])
        .chain([start_ms, end_ms])
        .collect();
    cuts.retain(|c| (start_ms..=end_ms).contains(c));
    cuts.sort_unstable();
    cuts.dedup();
    let mut t = TimeBuckets::default();
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        let rank = spans
            .iter()
            .filter(|s| s.0 <= a && b <= s.1)
            .map(|s| s.2)
            .max();
        let rank = rank.unwrap_or(if a < first_step { 1 } else { 4 });
        let bucket = match rank {
            1 => &mut t.queued_ms,
            2 => &mut t.with_you_ms,
            3 => &mut t.waiting_ms,
            _ => &mut t.working_ms,
        };
        *bucket += (b - a) as u64;
    }
    t
}

/// The origin when admission recorded none: the trigger kind and the ids the
/// input carries (the engine replaces them with names from the directory).
pub fn origin_from_input(input: &Value) -> Option<Origin> {
    let get = |k: &str| {
        input
            .get(k)
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
    };
    Some(Origin {
        kind: get("trigger")?,
        channel_name: get("channel_name").or_else(|| get("chat")),
        user_name: get("user_name").or_else(|| get("user")),
        thread_url: None,
    })
}

fn add_usage(sum: &mut TurnTokenUsage, u: &TurnTokenUsage) {
    sum.input_tokens += u.input_tokens;
    sum.output_tokens += u.output_tokens;
    let add = |a: Option<u64>, b: Option<u64>| {
        if a.is_none() && b.is_none() {
            None
        } else {
            Some(a.unwrap_or(0) + b.unwrap_or(0))
        }
    };
    sum.cached_input_tokens = add(sum.cached_input_tokens, u.cached_input_tokens);
    sum.reasoning_tokens = add(sum.reasoning_tokens, u.reasoning_tokens);
    sum.cost_usd = match (sum.cost_usd, u.cost_usd) {
        (None, None) => None,
        (a, b) => Some(a.unwrap_or(0.0) + b.unwrap_or(0.0)),
    };
}

fn phase_rank(p: StepPhase) -> u8 {
    match p {
        StepPhase::Working => 6,
        StepPhase::Finishing => 5,
        StepPhase::Starting => 4,
        StepPhase::NeedsYou => 3,
        StepPhase::Waiting => 2,
        StepPhase::Queued => 1,
        StepPhase::Unknown => 0,
    }
}

/// A node's display name: its name, else its kind, else its id.
fn step_name(graph: &WorkflowGraph, id: &str) -> String {
    graph
        .nodes
        .iter()
        .find(|n| n.id == id)
        .map(|n| {
            if n.name.trim().is_empty() {
                n.kind.clone()
            } else {
                n.name.clone()
            }
        })
        .unwrap_or_else(|| id.to_string())
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

fn millis(from: DateTime<Utc>, to: DateTime<Utc>) -> u64 {
    (to - from).num_milliseconds().max(0) as u64
}

/// At most `max` characters, with "…" when cut.
fn clip(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max - 1).collect();
    out.push('…');
    out
}

#[cfg(test)]
mod tests;
