//! Pure helpers of the workflow engine's steps, moved out of otto-server's
//! `workflow_engine.rs`: node labels and the chat brief, skill and list
//! params, the seeded run input, code-block extraction, and how a step's
//! thread events land on its node state (phases, wait, needs-you, the
//! thread record — engine.md §2.2, §4.2).

use otto_core::agent_runtime::{
    Attention, SubThreadUse, TranscriptDigest, TurnTokenUsage, WaitReason,
};
use otto_core::text::clip_bytes as truncate;
use otto_core::workflows::{
    ChildThread, HumanEntry, NodeRunState, NodeStatus, PhaseMark, RunStatus, StepPhase, StepThread,
    Workflow, WorkflowCheckpoint, WorkflowNode,
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
        /// A loop's inner step: its child row's label ("Work · iteration 2").
        label: Option<String>,
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

/// A loop's inner step (`<loop>#<i>.<k>`): its row label, "Work · iteration 2".
pub fn iteration_label(node_id: &str, name: &str) -> Option<String> {
    let i = node_id.split_once('#')?.1.split('.').next()?;
    Some(format!("{name} · iteration {i}"))
}

/// Apply a [`StepNote`] to the node; `true` = publish now.
pub fn record(st: &mut NodeRunState, n: StepNote) -> bool {
    match n {
        StepNote::Thread {
            thread_id,
            start_ordinal,
            provider,
            continued_from,
            label,
        } => {
            // An earlier thread of this node (a retry, a loop's earlier
            // iteration) stays on record as a child, so it is closed and kept too.
            if let Some(old) = st.thread.take().filter(|o| o.thread_id != thread_id) {
                let row = st
                    .children
                    .iter_mut()
                    .find(|c| c.thread_id.as_ref() == Some(&old.thread_id));
                if let Some(c) = row {
                    if c.status == "running" {
                        c.status = "success".into();
                    }
                    c.usage = old.usage.clone();
                    c.ended_at = Some(chrono::Utc::now());
                } else {
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
            }
            // A loop's inner step shows as its own row, newest iteration last.
            if let Some(label) = label {
                match st
                    .children
                    .iter_mut()
                    .find(|c| c.thread_id.as_ref() == Some(&thread_id))
                {
                    Some(c) => (c.label, c.status) = (label, "running".into()),
                    None => st.children.push(ChildThread {
                        id: thread_id.clone(),
                        thread_id: Some(thread_id.clone()),
                        label,
                        provider: provider.clone(),
                        model: None,
                        effort: None,
                        status: "running".into(),
                        started_at: Some(chrono::Utc::now()),
                        ended_at: None,
                        note: None,
                        usage: Default::default(),
                    }),
                }
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
            if let Some(t) = &st.thread {
                let row = st
                    .children
                    .iter_mut()
                    .find(|c| c.thread_id.as_ref() == Some(&t.thread_id));
                if let Some(c) = row.filter(|c| c.status == "running") {
                    c.status = if o.error_class.is_some() {
                        "error"
                    } else {
                        "success"
                    }
                    .into();
                    (c.usage, c.ended_at) = (o.usage.clone(), Some(chrono::Utc::now()));
                }
            }
            if let Some(t) = st.thread.as_mut() {
                t.turns = o.turns;
                t.usage = o.usage.clone();
                t.model = o.model.clone();
                t.effort = o.effort.clone();
                t.digest = o.digest.clone();
            }
            st.error_class = o.error_class.clone();
            // What the thread put outside Otto, from its outward tool calls (§6.3).
            if let Some(d) = &o.digest {
                st.deliverables = crate::deliverables::from_actions(&d.actions);
            }
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

/// The deliverables a built-in node returned in its output (§6.3).
pub fn output_deliverables(out: &Value) -> Vec<otto_core::workflows::Deliverable> {
    out.get("deliverables")
        .and_then(|d| serde_json::from_value(d.clone()).ok())
        .unwrap_or_default()
}

/// Enter `phase` with a note on its mark ("attempt 2 of 3 — …", "canceled"),
/// even when the node already is in it.
pub fn note_mark(st: &mut NodeRunState, phase: StepPhase, note: &str) {
    st.phase = Some(phase);
    st.phases.push(PhaseMark {
        phase,
        at: chrono::Utc::now(),
        note: Some(otto_core::text::clip_chars(note, 200)),
    });
    if st.phases.len() > 40 {
        st.phases.remove(1);
    }
}

/// A running step the run's cancel stopped: it settles `skipped` at
/// finalize, and this says "Stopped" rather than "Skipped" (§2.1).
pub fn mark_canceled(st: &mut NodeRunState) {
    st.error_class = Some("canceled".into());
    note_mark(st, st.phase.unwrap_or(StepPhase::Working), "canceled");
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

/// Build a `(brief, full_markdown)` summary of a finished run. `brief` is the
/// short chat message; `full_markdown` is the attached `summary.md`.
pub fn build_run_summary(
    workflow: &Workflow,
    states: &[NodeRunState],
    status: RunStatus,
) -> (String, String) {
    let emoji = match status {
        RunStatus::Success => "✅",
        RunStatus::Error => "❌",
        RunStatus::Canceled => "⏹",
        _ => "•",
    };
    let total = states.len();
    let ok = states
        .iter()
        .filter(|s| s.status == NodeStatus::Success)
        .count();
    let failed = states
        .iter()
        .filter(|s| s.status == NodeStatus::Error)
        .count();
    let skipped = states
        .iter()
        .filter(|s| s.status == NodeStatus::Skipped)
        .count();

    // Pull a review score out of any node output that carries one.
    let score = states.iter().find_map(|s| {
        s.output
            .as_ref()
            .and_then(|o| o.get("score"))
            .and_then(Value::as_i64)
    });

    let counts = format!("{ok}/{total} steps ok · {failed} failed · {skipped} skipped");
    let score_line = score
        .map(|sc| format!("\n*Review score:* {sc}/100"))
        .unwrap_or_default();

    let brief = format!(
        "{emoji} *{}* — {}\n{counts}{score_line}\n_Full summary attached (summary.md)._",
        workflow.name,
        status.as_str(),
    );

    // Full markdown: per-step detail.
    let mut md = String::new();
    md.push_str(&format!("# Workflow run — {}\n\n", workflow.name));
    md.push_str(&format!("- **Status:** {} {}\n", emoji, status.as_str()));
    md.push_str(&format!("- **Steps:** {counts}\n"));
    if let Some(sc) = score {
        md.push_str(&format!("- **Review score:** {sc}/100\n"));
    }
    md.push_str("\n## Steps\n\n");
    for (i, s) in states.iter().enumerate() {
        let kind = workflow
            .graph
            .nodes
            .iter()
            .find(|n| n.id == s.node_id)
            .map(|n| {
                if n.name.is_empty() {
                    n.kind.clone()
                } else {
                    format!("{} ({})", n.name, n.kind)
                }
            })
            .unwrap_or_else(|| s.node_id.clone());
        let dur = s
            .duration_ms
            .map(|d| format!(" · {d}ms"))
            .unwrap_or_default();
        let attempts = match s.attempts {
            Some(a) if a > 1 => format!(" · {a} attempts"),
            _ => String::new(),
        };
        md.push_str(&format!(
            "{}. **{kind}** — {}{dur}{attempts}\n",
            i + 1,
            s.status.as_str()
        ));
        if let Some(err) = &s.error {
            md.push_str(&format!("   - error: {}\n", truncate(err, 300)));
        }
        // A short peek at the work product (agent reply or compact JSON).
        if let Some(out) = &s.output {
            let peek = out
                .get("reply")
                .and_then(Value::as_str)
                .map(|r| r.to_string())
                .unwrap_or_else(|| out.to_string());
            let peek = truncate(peek.trim(), 240);
            if !peek.is_empty() && peek != "null" {
                md.push_str(&format!("   - {}\n", peek.replace('\n', " ")));
            }
        }
    }
    (brief, md)
}

/// The run note for a repo whose in-worktree hooks Otto's own git skipped.
pub fn skipped_hooks_note(repo: &str) -> String {
    format!(
        "⚠ git_pr: {repo}'s hooks live inside its work tree (core.hooksPath, e.g. husky) \
         and did not run for Otto's own commit / push — an agent there can edit them, so \
         unattended runs skip them; CI or your next commit still runs them"
    )
}

/// Gather the set of PR targets a `git_pr` node should open — one per changed
/// repo. Sources: explicit `params.repos[]`, a loop's published `input.repos[]`,
/// the input itself as a single reference (a review/loop output carrying
/// `repo_id`/`worktree`), and fan-in (multiple predecessor outputs keyed by node
/// id, each a review with its own `repo_id`). Each entry is `{repo_id?,
/// worktree?, worktree_path?, base?}`. Empty ⇒ the caller uses one implicit
/// target from the run context. Deduplication by resolved repo_id is the
/// caller's job.
pub fn collect_pr_targets(p: &Value, input: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let has_ref = |v: &Value| {
        ["repo_id", "worktree", "worktree_path"].iter().any(|k| {
            v.get(*k)
                .and_then(Value::as_str)
                .map(|s| !s.trim().is_empty())
                .unwrap_or(false)
        })
    };
    if let Some(arr) = p.get("repos").and_then(Value::as_array) {
        out.extend(arr.iter().filter(|v| has_ref(v)).cloned());
    }
    if let Some(arr) = input.get("repos").and_then(Value::as_array) {
        out.extend(arr.iter().filter(|v| has_ref(v)).cloned());
    }
    if has_ref(input) {
        out.push(input.clone());
    }
    if let Value::Object(m) = input {
        for v in m.values() {
            if v.is_object() && has_ref(v) {
                out.push(v.clone());
            }
        }
    }
    out
}

/// Parse a `checks` value into (name, command) pairs. Accepts an array of plain
/// command strings, or of objects `{name?, cmd}`. Drives the reviewer's
/// user-defined verification commands (e.g. `go test -tags=component ./...`).
pub fn parse_checks(v: Option<&Value>) -> Vec<(String, String)> {
    let Some(arr) = v.and_then(Value::as_array) else {
        return vec![];
    };
    let mut out = Vec::new();
    for (i, item) in arr.iter().enumerate() {
        match item {
            Value::String(s) if !s.trim().is_empty() => {
                out.push((format!("check {}", i + 1), s.trim().to_string()));
            }
            Value::Object(_) => {
                if let Some(cmd) = item
                    .get("cmd")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                {
                    let name = item
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .filter(|s| !s.is_empty())
                        .unwrap_or(cmd)
                        .to_string();
                    out.push((name, cmd.to_string()));
                }
            }
            _ => {}
        }
    }
    out
}

/// Locate the interrupted step's handoff file: `step{N}-{slug(name)}.md` in
/// `<data_dir>/workflow-context/<run_id>/`, where N (the execution-order step
/// number) is unknown after a restart — so match on the slug, require the
/// mtime to postdate the step's recorded start (a leftover from an earlier
/// attempt is cleared before each submit, but stay defensive), and require
/// non-trivial content. Returns the file's content.
#[allow(clippy::disallowed_methods)] // sync helper: the engine runs it via offload::blocking
pub fn find_step_handoff(
    dir: &std::path::Path,
    slug: &str,
    started_at: Option<chrono::DateTime<chrono::Utc>>,
) -> Option<String> {
    let entries = std::fs::read_dir(dir).ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // step{N}-{slug}.md exactly (no loop-iteration/inner suffixes — those
        // belong to inner steps the loop node itself re-drives).
        let Some(rest) = name.strip_prefix("step") else {
            continue;
        };
        let Some(tail) = rest
            .split_once('-')
            .and_then(|(n, tail)| n.chars().all(|c| c.is_ascii_digit()).then_some(tail))
        else {
            continue;
        };
        if tail != format!("{slug}.md") {
            continue;
        }
        let meta = entry.metadata().ok()?;
        if let (Some(start), Ok(modified)) = (started_at, meta.modified()) {
            let modified: chrono::DateTime<chrono::Utc> = modified.into();
            if modified < start {
                continue;
            }
        }
        let content = std::fs::read_to_string(entry.path()).ok()?;
        if content.trim().len() > 40 {
            return Some(content);
        }
    }
    None
}

pub fn loop_checkpoint(
    node: &WorkflowNode,
    loop_id: &str,
    iteration: u64,
    step_index: usize,
    input: Value,
) -> WorkflowCheckpoint {
    WorkflowCheckpoint {
        node_id: node.id.clone(),
        loop_id: loop_id.into(),
        iteration,
        step_index,
        kind: node.kind.clone(),
        name: node_display_name(node).into(),
        status: NodeStatus::Pending,
        input,
        updated_at: chrono::Utc::now(),
        ..Default::default()
    }
}

/// The directory an agent node runs in: a per-node `cwd`/`working_directory`
/// param, else the run-level working dir, else the workspace root. Tilde-expanded.
pub fn node_cwd(node: &WorkflowNode, input: &Value, run_cwd: &str) -> String {
    let p = &node.params;
    let pick = p
        .get("cwd")
        .or_else(|| p.get("working_directory"))
        .and_then(Value::as_str)
        .or_else(|| input.get("working_directory").and_then(Value::as_str))
        .filter(|s| !s.trim().is_empty());
    match pick {
        Some(s) => otto_core::paths::expand_tilde(s),
        None => run_cwd.to_string(),
    }
}

/// Data-dir-relative file of a `canvas` step's diagram. Step ids are only
/// checked for being non-empty and unique, so an id like `../../../tmp/x`
/// wrote (and created directories) outside the data dir; such an id falls
/// back to its slug, while an ordinary id keeps its exact file name.
pub fn canvas_rel(run_id: &str, node_id: &str, ext: &str) -> String {
    let name = otto_core::paths::safe_component(node_id)
        .filter(|n| !n.starts_with('.'))
        .map(str::to_string)
        .unwrap_or_else(|| crate::context::slug(node_id));
    format!("workflow-canvas/{run_id}/{name}.{ext}")
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::workflows::DeliverableState;

    fn st() -> NodeRunState {
        serde_json::from_value(serde_json::json!({"node_id": "n1", "status": "running"})).unwrap()
    }

    #[test]
    fn a_retry_and_a_cancel_leave_a_noted_mark() {
        let mut s = st();
        note_mark(&mut s, StepPhase::Queued, "attempt 2 of 3 — overloaded");
        mark_canceled(&mut s);
        let notes: Vec<_> = s
            .phases
            .iter()
            .map(|m| (m.phase, m.note.as_deref()))
            .collect();
        assert_eq!(
            notes,
            [
                (StepPhase::Queued, Some("attempt 2 of 3 — overloaded")),
                (StepPhase::Queued, Some("canceled")),
            ]
        );
        assert_eq!(s.error_class.as_deref(), Some("canceled"));
    }

    #[test]
    fn a_built_in_node_returns_its_deliverables_in_its_output() {
        let out = serde_json::json!({"deliverables": [{"label": "PR comments", "state": "done", "detail": "2 posted"}]});
        let d = output_deliverables(&out);
        assert_eq!(
            (d[0].label.as_str(), d[0].state),
            ("PR comments", DeliverableState::Done)
        );
        assert!(output_deliverables(&serde_json::json!({"reply": "x"})).is_empty());
    }

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

#[cfg(test)]
mod iteration_row_tests {
    use super::*;

    fn started(id: &str, label: Option<&str>) -> StepNote {
        StepNote::Thread {
            thread_id: id.into(),
            start_ordinal: 0,
            provider: "claude".into(),
            continued_from: None,
            label: label.map(str::to_string),
        }
    }

    #[test]
    fn a_loops_inner_threads_are_rows_named_by_step_and_iteration() {
        assert_eq!(
            iteration_label("goal#2.0", "Work").as_deref(),
            Some("Work · iteration 2")
        );
        assert_eq!(iteration_label("work", "Work"), None);
        let mut st = NodeRunState::default();
        record(&mut st, started("a", Some("Work · iteration 1")));
        record(&mut st, StepNote::Settled(Box::default()));
        record(&mut st, started("b", Some("Check criteria · iteration 1")));
        record(&mut st, started("a", Some("Work · iteration 2")));
        let rows: Vec<_> = st
            .children
            .iter()
            .map(|c| (c.label.as_str(), c.status.as_str()))
            .collect();
        assert_eq!(
            rows,
            [
                ("Work · iteration 2", "running"),
                ("Check criteria · iteration 1", "success")
            ]
        );
        // An unlabelled step keeps today's "earlier turn" archive.
        let mut st = NodeRunState::default();
        record(&mut st, started("x", None));
        record(&mut st, started("y", None));
        assert_eq!(st.children[0].label, "claude · earlier turn");
    }
}
