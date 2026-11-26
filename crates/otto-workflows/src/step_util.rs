//! Pure helpers of the workflow engine's steps, moved out of otto-server's
//! `workflow_engine.rs`: node labels and the chat brief, skill and list
//! params, the seeded run input, code-block extraction, and how a step's
//! thread events land on its node state (phases, wait, needs-you, the
//! thread record — engine.md §2.2, §4.2).

use otto_core::agent_runtime::{
    Attention, SubThreadUse, TranscriptDigest, TurnTokenUsage, WaitReason,
};
use otto_core::workflows::{
    ChildThread, HumanEntry, NodeRunState, PhaseMark, StepPhase, StepThread, WorkflowNode,
};
use otto_core::Id;
use serde_json::Value;

/// A label for a node in progress messages: its name, else its kind.
pub fn node_label(node: &WorkflowNode) -> String {
    if node.name.trim().is_empty() {
        node.kind.clone()
    } else {
        node.name.clone()
    }
}

/// Whether a node kind is worth a "started/finished" line in the chat thread.
/// Structural/plumbing kinds (log/transform/delay/condition/manual_trigger) are
/// skipped so a long pipeline doesn't drown the thread — the user asked for the
/// meaningful steps, "without it being too overwhelming". `review_run` is excluded
/// here because it streams its OWN richer block (started → score → findings).
pub fn is_reportable(kind: &str) -> bool {
    !matches!(
        kind,
        "manual_trigger" | "log" | "transform" | "delay" | "condition" | "review_run"
    )
}

/// Collapse runs of whitespace/newlines into single spaces and trim.
pub fn collapse_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Truncate to at most `max` chars, preferring to cut at the last sentence
/// terminator so the snippet reads as whole sentences (the user wants brief,
/// well-formatted summaries — ≤ a short paragraph). Appends `…` if cut.
pub fn truncate_sentences(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let head: String = s.chars().take(max).collect();
    // Prefer the last sentence boundary in the kept window (only if it keeps a
    // reasonable amount, so we don't truncate to almost nothing).
    if let Some(cut) = head.rfind(['.', '!', '?', '\n']) {
        if cut >= max / 2 {
            return format!("{}…", head[..=cut].trim());
        }
    }
    format!("{}…", head.trim())
}

/// A short, chat-friendly summary of a node's product (agent reply / analysis /
/// plan / summary text), whitespace-collapsed and truncated to a brief block.
/// Returns `None` for purely structural outputs (nothing worth posting).
pub fn brief_summary(output: &Value) -> Option<String> {
    let raw = ["reply", "analysis", "plan_md", "body_md", "summary", "note"]
        .iter()
        .find_map(|k| output.get(*k).and_then(Value::as_str))
        .map(str::to_string)?;
    let s = truncate_sentences(&collapse_ws(&raw), 700);
    if s.is_empty() || s == "…" {
        None
    } else {
        Some(s)
    }
}

/// Collect skill names a node wants injected: `skill` (string) and/or `skills`
/// (array of strings), de-duplicated, in declared order.
pub fn node_skill_names(params: &Value) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut push = |s: &str| {
        let s = s.trim();
        if !s.is_empty() && !out.iter().any(|x| x == s) {
            out.push(s.to_string());
        }
    };
    if let Some(s) = params.get("skill").and_then(Value::as_str) {
        push(s);
    }
    for key in ["skills", "lenses"] {
        if let Some(arr) = params.get(key).and_then(Value::as_array) {
            for v in arr {
                if let Some(s) = v.as_str() {
                    push(s);
                }
            }
        }
    }
    out
}

/// Parse a node param into a `Vec<String>` (accepts a JSON array of strings or a
/// comma-separated string), trimmed + non-empty. Used for `providers`/`lenses`.
pub fn param_str_list(params: &Value, key: &str) -> Vec<String> {
    match params.get(key) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| v.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .collect(),
        Some(Value::String(s)) => s
            .split(',')
            .map(|x| x.trim().to_string())
            .filter(|x| !x.is_empty())
            .collect(),
        _ => vec![],
    }
}

/// Seed the run input from the first VALID declared repo entry — explicit
/// input keys always win — and expose the normalized entries as
/// `repos: [{repo_id, worktree, base}]`, the shape `collect_pr_targets` and
/// the loop's ref harvest already consume. Pure; unit-tested below.
pub fn seed_input_from_entries(input: Value, entries: &[crate::context::RepoEntry]) -> Value {
    let valid: Vec<&crate::context::RepoEntry> =
        entries.iter().filter(|e| e.error.is_none()).collect();
    if valid.is_empty() {
        return input;
    }
    let mut m = match input {
        Value::Object(m) => m,
        Value::Null => serde_json::Map::new(),
        other => {
            let mut m = serde_json::Map::new();
            m.insert("input".into(), other);
            m
        }
    };
    let missing = |m: &serde_json::Map<String, Value>, k: &str| {
        m.get(k)
            .and_then(Value::as_str)
            .map(|s| s.trim().is_empty())
            .unwrap_or(true)
    };
    if missing(&m, "working_directory") {
        if let Some(w) = valid.iter().find_map(|e| e.worktree.clone()) {
            m.insert("working_directory".into(), Value::String(w));
        }
    }
    if missing(&m, "base") {
        if let Some(b) = valid.iter().find_map(|e| e.base.clone()) {
            m.insert("base".into(), Value::String(b));
        }
    }
    if missing(&m, "repo_id") {
        if let Some(r) = valid.iter().find_map(|e| e.repo_id.clone()) {
            m.insert("repo_id".into(), Value::String(r));
        }
    }
    // Replace the raw declarations with the normalized targets; the original
    // user shape stays visible in repos.json.
    m.insert(
        "repos".into(),
        Value::Array(
            valid
                .iter()
                .map(|e| crate::context::entry_to_target(e))
                .collect(),
        ),
    );
    Value::Object(m)
}

/// Some triggers (chat) send `msg`, agent-facing steps and `prompt.md` want
/// `prompt` — fill `prompt` from `msg` when `prompt` is absent or blank, and
/// never the reverse (an explicit `prompt` always wins; `msg` still renders
/// separately in the brief unless it's identical, see `render_brief`). A
/// non-object input (or one already without either field) passes through
/// unchanged.
pub fn normalize_prompt(input: Value) -> Value {
    let Value::Object(mut m) = input else {
        return input;
    };
    let has_prompt = m
        .get("prompt")
        .and_then(Value::as_str)
        .is_some_and(|s| !s.trim().is_empty());
    if !has_prompt {
        if let Some(msg) = m
            .get("msg")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            m.insert("prompt".into(), Value::String(msg.to_string()));
        }
    }
    Value::Object(m)
}

/// Extract the body of the first fenced code block (preferring the `lang` fence).
pub fn extract_code_block(text: &str, lang: &str) -> Option<String> {
    let fence = format!("```{lang}");
    let start = text
        .find(&fence)
        .map(|i| i + fence.len())
        .or_else(|| text.find("```").map(|i| i + 3))?;
    let rest = &text[start..];
    let end = rest.find("```")?;
    Some(rest[..end].trim().to_string())
}

/// File extension for a `canvas` workflow node's written diagram artifact,
/// keyed on its `mode` param. `excalidraw` writes JSON; `d2` writes `.d2`
/// source; everything else (the default `mermaid`, and any unrecognized mode)
/// writes `.mmd`.
pub fn canvas_node_ext(mode: &str) -> &'static str {
    match mode {
        "excalidraw" => "json",
        "d2" => "d2",
        _ => "mmd",
    }
}

/// Collect openable session id(s) from a node's output (`session_id` string
/// and/or `sessions` array) into `into`, de-duplicated. Used so a node whose
/// session id rode in its output — including a *cached* re-run — still surfaces
/// the session on the run, complementing the live channel report.
pub fn harvest_session_ids(output: &Value, into: &mut Vec<String>) {
    if let Some(s) = output.get("session_id").and_then(Value::as_str) {
        if !s.is_empty() && !into.iter().any(|x| x == s) {
            into.push(s.to_string());
        }
    }
    if let Some(arr) = output.get("sessions").and_then(Value::as_array) {
        for v in arr {
            if let Some(s) = v.as_str() {
                if !s.is_empty() && !into.iter().any(|x| x == s) {
                    into.push(s.to_string());
                }
            }
        }
    }
}

/// The node's human name (falls back to its kind) — drives step-file names.
pub fn node_display_name(node: &WorkflowNode) -> &str {
    if node.name.trim().is_empty() {
        &node.kind
    } else {
        &node.name
    }
}

/// Last 6 chars of a run id — a compact, human-scannable tag that disambiguates
/// concurrent runs of the same workflow in session titles / lists.
pub fn short_id(id: &Id) -> String {
    let n = id.chars().count();
    id.chars().skip(n.saturating_sub(6)).collect()
}

/// Merge a node output's published repo reference(s) into the run's repos
/// registry (`repos.json`) — the "each step adds to the repos file" contract:
/// the top-level `repo_id`+`base`/`worktree`, plus every entry of a published
/// `repos[]` array (a multi-repo review/loop). Only the explicit `worktree`
/// key counts (a bare `working_directory` is too ambient to be authoritative).
pub fn merge_published_refs(files: &crate::context::RunContextFiles, out: &Value) {
    let merge_one = |v: &Value| {
        if let Some(rid) = v
            .get("repo_id")
            .and_then(Value::as_str)
            .filter(|s| !s.trim().is_empty())
        {
            files.merge_published(
                rid,
                v.get("base").and_then(Value::as_str),
                v.get("worktree").and_then(Value::as_str),
            );
        }
    };
    merge_one(out);
    if let Some(arr) = out.get("repos").and_then(Value::as_array) {
        for v in arr {
            merge_one(v);
        }
    }
}

/// What a step's thread reports to its node state (`otto-server` sends it
/// over the node's association channel, in order with its sessions).
pub enum StepNote {
    Thread {
        thread_id: Id,
        start_ordinal: u32,
        provider: String,
        continued_from: Option<String>,
    },
    Phase(StepPhase, Option<WaitReason>),
    NeedsYou(Attention),
    Resumed {
        answer: Option<String>,
        by: Option<Id>,
    },
    /// A person's verification or a consumed steering note (goal.md §2.4).
    Human(HumanEntry),
    Settled(Box<Settled>),
}

/// What a settled step's thread leaves on the node.
#[derive(Debug, Clone, Default)]
pub struct Settled {
    pub turns: u32,
    pub usage: TurnTokenUsage,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub digest: Option<TranscriptDigest>,
    pub error_class: Option<String>,
    pub sub_threads: Vec<SubThreadUse>,
}

/// Apply a [`StepNote`] to the node; `true` = publish now.
pub fn record(st: &mut NodeRunState, n: StepNote) -> bool {
    match n {
        StepNote::Thread {
            thread_id,
            start_ordinal,
            provider,
            continued_from,
        } => {
            // An earlier thread of this node (a retry, a loop's earlier
            // iteration) stays on record as a child, so it is closed and kept too.
            if let Some(old) = st.thread.take().filter(|o| o.thread_id != thread_id) {
                st.children.push(ChildThread {
                    id: old.thread_id.clone(),
                    thread_id: Some(old.thread_id),
                    label: format!("{} · earlier turn", old.provider),
                    provider: old.provider,
                    model: old.model,
                    effort: old.effort,
                    status: "success".into(),
                    started_at: None,
                    ended_at: None,
                    note: None,
                    usage: old.usage,
                });
            }
            st.thread = Some(StepThread {
                thread_id,
                runner: "threads".into(),
                provider,
                model: None,
                effort: None,
                continued_from,
                start_ordinal: Some(start_ordinal.into()),
                turns: 0,
                usage: Default::default(),
                digest: None,
                closed_at: None,
                available: None,
            });
            mark(st, StepPhase::Starting);
            true
        }
        StepNote::Phase(phase, wait) => {
            st.wait = wait;
            mark(st, phase)
        }
        StepNote::NeedsYou(a) => {
            st.attention = Some(a);
            mark(st, StepPhase::NeedsYou);
            true
        }
        StepNote::Resumed { answer, by } => {
            let asked = st.attention.take();
            if let (Some(text), Some(a)) = (answer, asked) {
                push_human(
                    &mut st.human_log,
                    HumanEntry {
                        kind: "answer".into(),
                        ask: Some(a.text),
                        text: otto_core::text::clip_bytes(&text, 2048).to_string(),
                        met: None,
                        revision: None,
                        iteration: None,
                        by: by.unwrap_or_default(),
                        at: chrono::Utc::now(),
                    },
                );
            }
            mark(st, StepPhase::Working);
            true
        }
        StepNote::Human(e) => {
            push_human(&mut st.human_log, e);
            true
        }
        StepNote::Settled(o) => {
            if let Some(t) = st.thread.as_mut() {
                t.turns = o.turns;
                t.usage = o.usage.clone();
                t.model = o.model.clone();
                t.effort = o.effort.clone();
                t.digest = o.digest.clone();
            }
            st.error_class = o.error_class.clone();
            st.children
                .extend(o.sub_threads.iter().map(|s| ChildThread {
                    id: s.id.clone(),
                    thread_id: Some(s.id.clone()),
                    label: s.title.clone().unwrap_or_else(|| s.provider.clone()),
                    provider: s.provider.clone(),
                    model: s.model.clone(),
                    effort: None,
                    status: s.status.clone(),
                    started_at: None,
                    ended_at: None,
                    note: None,
                    usage: s.usage.clone(),
                }));
            st.wait = None;
            st.attention = None;
            true
        }
    }
}

/// Append to a node's `human_log` (≤ 20). Over the cap the oldest entry goes
/// — never a `verify` record of the newest iteration, which the current check
/// depends on (goal.md §2.4).
pub fn push_human(log: &mut Vec<HumanEntry>, e: HumanEntry) {
    log.push(e);
    let current = log
        .iter()
        .filter(|e| e.kind == "verify")
        .filter_map(|e| e.iteration)
        .max();
    while log.len() > 20 {
        let i = log
            .iter()
            .position(|e| !(e.kind == "verify" && e.iteration == current))
            .unwrap_or(0);
        log.remove(i);
    }
}

/// Enter `phase` (a new mark on the timeline, capped at 40: the first and
/// the last 39); `false` when the node already is in it.
fn mark(st: &mut NodeRunState, phase: StepPhase) -> bool {
    if st.phase == Some(phase) {
        return false;
    }
    st.phase = Some(phase);
    st.phases.push(PhaseMark {
        phase,
        at: chrono::Utc::now(),
        note: None,
    });
    if st.phases.len() > 40 {
        st.phases.remove(1);
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(kind: &str, text: &str, iteration: Option<u32>) -> HumanEntry {
        HumanEntry {
            kind: kind.into(),
            ask: Some("c1".into()),
            text: text.into(),
            met: Some(true),
            revision: None,
            iteration,
            by: "Ann".into(),
            at: chrono::Utc::now(),
        }
    }

    /// 25 notes and the 3 current verifications: the cap drops old notes,
    /// never the records the current check depends on (GL24.6).
    #[test]
    fn the_human_log_cap_keeps_the_current_iterations_verify_records() {
        let mut log = vec![];
        push_human(&mut log, entry("verify", "old", Some(1)));
        for i in 0..3 {
            push_human(&mut log, entry("verify", &format!("v{i}"), Some(2)));
        }
        for i in 0..25 {
            push_human(&mut log, entry("steer", &format!("n{i}"), None));
        }
        assert_eq!(log.len(), 20);
        let kept: Vec<&str> = log
            .iter()
            .filter(|e| e.kind == "verify")
            .map(|e| e.text.as_str())
            .collect();
        assert_eq!(kept, ["v0", "v1", "v2"]);
        assert_eq!(log.last().unwrap().text, "n24");
    }
}
