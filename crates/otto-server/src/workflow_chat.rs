//! Start a workflow from a chat message (Slack/Telegram/webhook), three ways:
//!
//! 1. **Legacy structured command** (field labels are case-insensitive;
//!    `Goals:` may be a bullet list or an inline comma list):
//!
//!    ```text
//!    @otto
//!    Action: Workflow
//!    Name: <workflow name>
//!    Msg: please do x y z, follow all relevant rules
//!    Jira ticket: PROJ-1111
//!    Working Directory: ~/path
//!    Branch: release/base-branch, create wt from it
//!    Relevant Info: ~/a, ~/b
//!    Goals:
//!      - 100% test coverage
//!      - under 2 minutes runtime
//!    ```
//!
//! 2. **Simplified command**: `run <name>: <prompt>` / `workflow <name>: …` /
//!    `run workflow <name>: …` ([`parse_run_command`]).
//!
//! 3. **Channel bindings**: a `chat`-kind [`otto_state::WorkflowTrigger`] pins a
//!    workflow to a channel/chat(/thread), so any message there starts it
//!    without a keyword ([`binding_matches`]).
//!
//! All three are pure-parsed + unit-tested; [`WorkflowChatTriggerImpl::try_start`]
//! resolves in that order and starts a run whose input carries the parsed
//! fields (so the first node — e.g. a "prepare relevant info" agent — can
//! gather context and pass it downstream).

use std::collections::HashMap;

use async_trait::async_trait;
use otto_channels::workflow_trigger::{WorkflowChatAck, WorkflowChatTrigger};
use otto_core::event::Event;
use otto_core::workflows::{NodeStatus, RunStatus, Workflow, WorkflowRun};
use otto_state::{TriggersRepo, WorkflowTrigger, WorkflowsRepo};
use serde_json::{json, Value};

use crate::state::ServerCtx;

/// Our own acknowledgement replies always start with this. Defensive loop
/// guard: the Slack/Telegram adapters already drop the bot's own inbound
/// messages (see `otto-channels/src/slack.rs`'s `bot_id` check — Telegram's
/// `getUpdates` long-poll structurally never returns the bot's own sends), but
/// the binding path checks this too so a relayed/edited copy of our ack can
/// never be mistaken for a chat-binding trigger.
const ACK_PREFIX: &str = "🚀 Started workflow";

/// True when `text` is (the start of) our own ack reply.
fn is_own_ack(text: &str) -> bool {
    text.starts_with(ACK_PREFIX)
}

/// A control command a user can send in a running workflow's chat thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WfControl {
    /// Report the run's current step + progress (non-intrusive; never prompts
    /// the running agent).
    Status,
    /// Skip the currently-running step and continue the run.
    Skip,
    /// Cancel the run and stop its agents.
    Abort,
    /// Print the static command guide (discoverability entry point).
    Help,
}

/// One control operation. THE single source of truth: `parse_wf_control` matches
/// against `synonyms`, and `wf_controls_help` renders `description`/`example` —
/// add a synonym or an op here and both the parser and the guide stay in sync.
struct WfOp {
    canonical: &'static str,
    variant: WfControl,
    synonyms: &'static [&'static str],
    description: &'static str,
    example: &'static str,
}

/// The control taxonomy. `?` maps to Status (a quick "what's up"); `help` is the
/// full guide. Every synonym is unique across ops, so match order is immaterial.
const WF_OPS: &[WfOp] = &[
    WfOp {
        canonical: "status",
        variant: WfControl::Status,
        synonyms: &[
            "status", "status?", "progress", "update", "how's it going",
            "hows it going", "where are we", "what's the status", "whats the status", "?",
        ],
        description: "current step + progress",
        example: "`status` or `progress`",
    },
    WfOp {
        canonical: "skip",
        variant: WfControl::Skip,
        synonyms: &[
            "skip", "skip step", "skip stage", "skip this", "skip this step", "skip it",
            "next", "move on",
        ],
        description: "skip the current step and continue",
        example: "`skip` or `next`",
    },
    WfOp {
        canonical: "abort",
        variant: WfControl::Abort,
        synonyms: &[
            "abort", "cancel", "stop", "halt", "kill", "terminate", "cancel run",
            "stop run", "kill it", "abort run",
        ],
        description: "cancel the run and stop its agents",
        example: "`abort` or `cancel`",
    },
    WfOp {
        canonical: "help",
        variant: WfControl::Help,
        synonyms: &["help", "commands", "usage", "options", "what can you do", "how do i", "?help", "h"],
        description: "this guide",
        example: "`help`",
    },
];

/// The reusable `Action: Workflow` trigger template — the ONE source of truth for
/// the "start a workflow" section of the help guide (and the shape the parser in
/// `parse_workflow_command` reads).
const WF_TRIGGER_TEMPLATE: &str = "@<your-bot>\n\
Action: Workflow\n\
Name: <workflow name>\n\
Msg: <what you want done — instructions for the agents>\n\
Jira ticket: <optional, e.g. PROJ-123>\n\
Working Directory: ~/path/to/repo\n\
Branch: <base branch>, create wt from it\n\
Relevant Info: ~/other/repo, ~/docs\n\
Goals:\n\
  - <goal 1>\n\
  - <goal 2>";

/// Parse a short chat message into a control command. Slack tokens are stripped,
/// whitespace normalized, and only SHORT (≤ 48 char) command-like messages match
/// — a synonym as the whole message or its leading word — so ordinary thread
/// chatter is ignored. `None` ⇒ not a control command.
pub fn parse_wf_control(text: &str) -> Option<WfControl> {
    let norm = strip_slack_tokens(text).to_lowercase();
    let norm = norm.split_whitespace().collect::<Vec<_>>().join(" ");
    if norm.is_empty() || norm.chars().count() > 48 {
        return None;
    }
    for op in WF_OPS {
        for syn in op.synonyms {
            if norm == *syn || norm.starts_with(&format!("{syn} ")) {
                return Some(op.variant);
            }
        }
    }
    None
}

/// The static control guide (no agent): a copy-pasteable trigger template plus
/// every control op with its description + example, all derived from the
/// taxonomy above so it can never drift from what the parser accepts.
pub fn wf_controls_help() -> String {
    let mut s = String::new();
    s.push_str("🛠 *Otto workflow commands*\n\n");
    s.push_str("*1. Start a workflow* — post this (edit the fields):\n```\n");
    s.push_str(WF_TRIGGER_TEMPLATE);
    s.push_str("\n```\n\n");
    s.push_str("*2. Control a running workflow* — reply in the run's thread:\n");
    for op in WF_OPS {
        let others: Vec<&str> =
            op.synonyms.iter().copied().filter(|x| *x != op.canonical).take(4).collect();
        let also = if others.is_empty() {
            String::new()
        } else {
            format!(" (also: {})", others.join(", "))
        };
        s.push_str(&format!("• *{}* — {} · e.g. {}{}\n", op.canonical, op.description, op.example, also));
    }
    s.trim_end().to_string()
}

/// Last 6 chars of a run id — a compact, human-referenceable tag that
/// distinguishes concurrent runs of the same workflow in chat.
fn short_id(id: &str) -> String {
    let chars: Vec<char> = id.chars().collect();
    let start = chars.len().saturating_sub(6);
    chars[start..].iter().collect()
}

/// A parsed `Action: Workflow` command.
#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowCommand {
    pub name: String,
    pub msg: String,
    pub jira_ticket: Option<String>,
    pub working_directory: Option<String>,
    /// Explicit base branch (`Branch:` / `Base:`). When set it becomes the run
    /// input's `base`, which the engine PINS as the worktree/PR/review base —
    /// an explicit user instruction, never overridden by the repo's detected
    /// default branch.
    pub branch: Option<String>,
    pub relevant_info: Vec<String>,
    pub goals: Vec<String>,
    pub raw: String,
}

/// Strip Slack entity tokens so the structured parser sees clean text: `<@U…>`
/// mentions, `<#C…>` channel refs and `<!here>` are removed; `<url|label>` links
/// keep their label. (A leading bot mention is what otherwise breaks the first
/// `Action: Workflow` line.)
fn strip_slack_tokens(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    let bytes = text.as_bytes();
    while i < text.len() {
        if bytes[i] == b'<' {
            if let Some(rel) = text[i..].find('>') {
                let inner = &text[i + 1..i + rel];
                if inner.starts_with('@') || inner.starts_with('#') || inner.starts_with('!') {
                    // mention / channel / special command → drop entirely
                } else if let Some(pipe) = inner.find('|') {
                    out.push_str(&inner[pipe + 1..]); // <url|label> → label
                } else {
                    out.push_str(inner); // <url> → url
                }
                i += rel + 1;
                continue;
            }
        }
        let ch = text[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}

/// Parse a structured workflow command. Returns `None` unless the text declares
/// `Action: Workflow` and carries a non-empty `Name:`. Tolerant of a leading
/// Slack `@bot` mention.
pub fn parse_workflow_command(text: &str) -> Option<WorkflowCommand> {
    let cleaned = strip_slack_tokens(text);
    if !cleaned.to_lowercase().contains("action:") {
        return None;
    }
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut goals: Vec<String> = vec![];
    let mut current_label: Option<String> = None;

    for raw_line in cleaned.lines() {
        let line = raw_line.trim();
        // Bullet line under the Goals label → a goal item.
        let bullet = line
            .strip_prefix('-')
            .or_else(|| line.strip_prefix('*'))
            .or_else(|| line.strip_prefix('•'));
        if let Some(rest) = bullet {
            if current_label.as_deref() == Some("goals") {
                let g = rest.trim();
                if !g.is_empty() {
                    goals.push(g.to_string());
                }
                continue;
            }
        }
        // Label: value
        if let Some((label, val)) = line.split_once(':') {
            let key = label.trim().to_lowercase();
            let is_labelish =
                !key.is_empty() && key.len() <= 24 && key.chars().all(|c| c.is_alphabetic() || c == ' ' || c == '_');
            if is_labelish {
                let v = val.trim().to_string();
                current_label = Some(key.clone());
                if key == "goals" {
                    for g in v.split([',', ';']) {
                        let g = g.trim();
                        if !g.is_empty() {
                            goals.push(g.to_string());
                        }
                    }
                } else {
                    fields.insert(key, v);
                }
                continue;
            }
        }
        if line.is_empty() {
            current_label = None;
        }
    }

    if fields.get("action").map(|s| s.to_lowercase()) != Some("workflow".to_string()) {
        return None;
    }
    let name = fields.get("name").cloned().unwrap_or_default();
    if name.trim().is_empty() {
        return None;
    }
    let pick = |keys: &[&str]| -> Option<String> {
        keys.iter()
            .find_map(|k| fields.get(*k).cloned())
            .filter(|s| !s.trim().is_empty())
    };
    let relevant_info = pick(&["relevant info", "relevant_info", "relevant"])
        .map(|s| {
            s.split(',')
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default();

    Some(WorkflowCommand {
        name,
        msg: pick(&["msg", "message"]).unwrap_or_default(),
        jira_ticket: pick(&["jira ticket", "jira", "jira_ticket", "ticket"]),
        working_directory: pick(&["working directory", "working dir", "workdir", "cwd"]),
        // Only the branch name matters here — trailing guidance like
        // "…, create wt from it" is dropped (the run already creates a worktree);
        // we keep just the part before the first comma.
        branch: pick(&["branch", "base", "base branch", "base_branch"])
            .and_then(|s| s.split(',').next().map(|b| b.trim().to_string()))
            .filter(|s| !s.is_empty()),
        relevant_info,
        goals,
        raw: text.to_string(),
    })
}

/// Run keywords, tried longest-first (so `run workflow X: …` doesn't parse as
/// `workflow` with name `"workflow X"`), case-insensitive. `bool` = explicit
/// (the keyword literally contains "workflow").
const RUN_KEYWORDS: &[(&str, bool)] = &[("run workflow", true), ("workflow", true), ("run", false)];

/// Parse the simplified `run <name>: <prompt>` / `workflow <name>: …` /
/// `run workflow <name>: …` command. Keywords are matched case-insensitively
/// at the start of the (mention-stripped) text and must be followed by
/// whitespace. `name` is the text up to the first `:` on that same line;
/// `prompt` is the remainder of that line plus all following lines. Returns
/// `None` when no keyword matches or the first line carries no `:`.
///
/// `explicit` is `true` when the matched keyword names "workflow" (`workflow`
/// / `run workflow`) — the caller uses it to decide whether an unknown name
/// gets a friendly "no such workflow" reply (explicit) or silently falls
/// through to normal chat (bare `run`, which is also a common English word).
pub fn parse_run_command(text: &str) -> Option<(String, String, bool)> {
    let cleaned = strip_slack_tokens(text);
    let trimmed = cleaned.trim_start();
    let lower = trimmed.to_lowercase();

    for &(kw, explicit) in RUN_KEYWORDS {
        if !lower.starts_with(kw) {
            continue;
        }
        // Keyword must be a whole word — next char (if any) is whitespace.
        match trimmed[kw.len()..].chars().next() {
            Some(c) if c.is_whitespace() => {}
            _ => continue,
        }
        let remainder = trimmed[kw.len()..].trim_start();
        let mut lines = remainder.splitn(2, '\n');
        let first_line = lines.next().unwrap_or("");
        let rest_lines = lines.next().unwrap_or("");

        let Some(colon_idx) = first_line.find(':') else {
            // This keyword matched but the line has no `:` — no other keyword
            // in the list can match the same (already-consumed) prefix either.
            return None;
        };
        let name = first_line[..colon_idx].trim().to_string();
        if name.is_empty() {
            return None;
        }
        let after_colon = first_line[colon_idx + 1..].trim_start();
        let mut prompt = after_colon.to_string();
        if !rest_lines.is_empty() {
            if !prompt.is_empty() {
                prompt.push('\n');
            }
            prompt.push_str(rest_lines);
        }
        return Some((name, prompt, explicit));
    }
    None
}

/// Pure match of a `chat`-kind trigger spec against an inbound message.
/// Spec shape: `{"channel": "slack"|"telegram", "chat": "<id>", "thread"?:
/// "<ts>", "mention_only"?: bool}`. `channel`/`chat` must match exactly; an
/// absent `thread` in the spec matches any thread (including none), a
/// present one requires an exact match; `mention_only` (default false)
/// requires `has_mention`.
pub fn binding_matches(spec: &Value, channel: &str, chat: &str, thread: Option<&str>, has_mention: bool) -> bool {
    if spec.get("channel").and_then(Value::as_str) != Some(channel) {
        return false;
    }
    if spec.get("chat").and_then(Value::as_str) != Some(chat) {
        return false;
    }
    if let Some(spec_thread) = spec.get("thread").and_then(Value::as_str) {
        if thread != Some(spec_thread) {
            return false;
        }
    }
    let mention_only = spec.get("mention_only").and_then(Value::as_bool).unwrap_or(false);
    if mention_only && !has_mention {
        return false;
    }
    true
}

/// Order the enabled `chat`-kind triggers that match the inbound message,
/// best-first: a thread-pinned spec (`spec.thread` set) is preferred over an
/// unpinned one so a thread-scoped automation doesn't lose to a channel-wide
/// one; ties keep the input (oldest-first) order. Pure — matches purely on
/// (channel, chat, thread, mention), same as [`binding_matches`]. `triggers`
/// comes from `TriggersRepo::list_enabled_by_kind("chat")`, which is GLOBAL
/// across every workspace, so the caller (`try_start`) MUST additionally gate
/// each candidate on the owning workflow's `workspace_id` before trusting it
/// — otherwise a channel bound by one workspace's Slack/Telegram integration
/// could leak that channel's content into another workspace's workflow runs.
fn binding_candidates<'a>(
    triggers: &'a [WorkflowTrigger],
    channel: &str,
    chat: &str,
    thread: Option<&str>,
    has_mention: bool,
) -> Vec<&'a WorkflowTrigger> {
    let mut matches: Vec<&WorkflowTrigger> = triggers
        .iter()
        .filter(|t| binding_matches(&t.spec, channel, chat, thread, has_mention))
        .collect();
    // Stable sort: thread-pinned specs sort before unpinned ones; relative
    // order within each group is preserved (matches the old best-match loop,
    // which kept the first pinned/unpinned candidate it saw).
    matches.sort_by_key(|t| std::cmp::Reverse(t.spec.get("thread").and_then(Value::as_str).is_some()));
    matches
}

/// otto-server's implementation of the channel workflow trigger.
pub struct WorkflowChatTriggerImpl {
    pub ctx: ServerCtx,
}

impl WorkflowChatTriggerImpl {
    /// Shared run-start body for all three resolution paths: create the run
    /// row, spawn the workflow engine in the background, and build the ack.
    /// `detail` overrides the default tail sentence (legacy path reports
    /// goals; the simplified/binding paths use the default). `channel`/`chat`
    /// are the inbound message's origin, logged for traceability only (they're
    /// already threaded into `input` by every call site).
    async fn start_named(
        &self,
        wf: Workflow,
        input: Value,
        detail: Option<String>,
        channel: &str,
        chat: &str,
    ) -> Option<WorkflowChatAck> {
        let repo = WorkflowsRepo::new(self.ctx.pool.clone());
        tracing::info!(
            "workflow chat: starting workflow '{}' (id {}, ws {}) from {channel}/{chat}",
            wf.name,
            wf.id,
            wf.workspace_id
        );
        let run = repo.create_run(&wf.id, &wf.workspace_id, &input).await.ok()?;
        let ws = self.ctx.workspaces.get(&wf.workspace_id).await.ok()?;
        let ctx2 = self.ctx.clone();
        let run_id = run.id.clone();
        let wf2 = wf.clone();
        let input2 = input.clone();
        tokio::spawn(async move {
            crate::workflow_engine::run_workflow(ctx2, ws, wf2, run_id, input2, None, false, None).await;
        });

        let tail = detail.unwrap_or_else(|| "Working through the steps now.".to_string());
        Some(WorkflowChatAck {
            reply: format!(
                "{ACK_PREFIX} **{}** (run `{}`). {} Reply `status` · `skip` · `abort` · `help` in this thread to control it.",
                wf.name, run.id, tail
            ),
        })
    }

    /// Find the active (pending|running) run whose input thread matches this
    /// inbound message. Workflows are global, so we scan active runs across all
    /// workspaces and match on the run input's channel/chat/thread (+ origin
    /// workspace). An exact thread match wins; a channel/chat match on the same
    /// workspace is the fallback (newest first), so control still finds the run
    /// when the trigger was a top-level message and the reply is threaded.
    async fn find_active_run_for_thread(
        &self,
        repo: &WorkflowsRepo,
        workspace_id: &str,
        channel: &str,
        chat: &str,
        thread: Option<&str>,
    ) -> Option<WorkflowRun> {
        let ids = repo.list_active_run_ids_global().await.ok()?;
        let mut fallback: Option<WorkflowRun> = None;
        for id in ids {
            let Ok(run) = repo.get_run(&id).await else { continue };
            let i = &run.input;
            let str_at = |k: &str| i.get(k).and_then(Value::as_str);
            let ws_ok = str_at("origin_workspace_id") == Some(workspace_id)
                || run.workspace_id == workspace_id;
            if str_at("channel") == Some(channel) && str_at("chat") == Some(chat) && ws_ok {
                if str_at("thread") == thread {
                    return Some(run); // exact thread → best match
                }
                if fallback.is_none() {
                    fallback = Some(run); // channel/chat match → newest-first fallback
                }
            }
        }
        fallback
    }

    /// A NON-INTRUSIVE status summary built purely from the run's persisted node
    /// states — never prompts a running agent. Overall status, a per-step line,
    /// and the running step's latest log line. `wf_name` is the looked-up
    /// workflow name — only the legacy `Action: Workflow` path stamps `name`
    /// into the run input, so without the fallback every `run <name>:` /
    /// channel-binding run read as a generic "*workflow*".
    fn run_status_summary(&self, run: &WorkflowRun, wf_name: Option<&str>) -> String {
        let short = short_id(&run.id);
        let name = run
            .input
            .get("name")
            .and_then(Value::as_str)
            .or(wf_name)
            .unwrap_or("workflow");
        let mut lines = vec![format!("📊 *{name}* — run `{short}` · {}", run.status.as_str())];
        for n in &run.nodes {
            let icon = match n.status {
                NodeStatus::Success => "✓",
                NodeStatus::Running => "▶",
                NodeStatus::Pending => "·",
                NodeStatus::Error => "✗",
                NodeStatus::Skipped => "⤼",
            };
            let dur = n
                .duration_ms
                .map(|ms| format!(" ({:.1}s)", ms as f64 / 1000.0))
                .unwrap_or_default();
            lines.push(format!("{icon} {}{}", n.node_id, dur));
        }
        if let Some(cur) = run.nodes.iter().find(|n| n.status == NodeStatus::Running) {
            if let Some(last) = cur.logs.last() {
                lines.push(format!("… {}", last.chars().take(160).collect::<String>()));
            }
        }
        lines.join("\n")
    }
}

#[async_trait]
impl WorkflowChatTrigger for WorkflowChatTriggerImpl {
    async fn try_start(
        &self,
        workspace_id: &str,
        channel: &str,
        chat: &str,
        thread: Option<&str>,
        user: &str,
        text: &str,
    ) -> Option<WorkflowChatAck> {
        let repo = WorkflowsRepo::new(self.ctx.pool.clone());

        // (1) Legacy structured `Action: Workflow` command.
        if let Some(cmd) = parse_workflow_command(text) {
            // Workflows are a GLOBAL library: resolve by name across all
            // workspaces, preferring one in the message's own workspace.
            let wf = match repo.find_by_name(&cmd.name, &workspace_id.to_string()).await {
                Ok(Some(w)) => w,
                Ok(None) => {
                    tracing::info!(
                        "workflow chat: parsed Action:Workflow but no workflow named '{}' exists — ignoring",
                        cmd.name
                    );
                    return None;
                }
                Err(e) => {
                    tracing::warn!("workflow chat: lookup for '{}' failed: {e}", cmd.name);
                    return None;
                }
            };

            // The result is reported back via the workspace whose integration
            // received the message (`origin_workspace_id`), not necessarily the
            // workflow's own workspace (workflows are global).
            let input = json!({
                "trigger": "chat",
                "origin_workspace_id": workspace_id,
                "channel": channel,
                "chat": chat,
                "thread": thread,
                "user": user,
                "name": cmd.name,
                "msg": cmd.msg,
                // Belt-and-braces with the engine's normalize_prompt: also
                // set `prompt` explicitly from the parsed `Msg:` field.
                "prompt": cmd.msg,
                "jira_ticket": cmd.jira_ticket,
                "working_directory": cmd.working_directory,
                // Explicit base branch → the engine pins it as the worktree/PR/
                // review base (R14). Absent ⇒ each git step resolves the repo's
                // detected default branch. Honor the user's stated branch; never
                // silently override it.
                "base": cmd.branch,
                "relevant_info": cmd.relevant_info,
                "goals": cmd.goals,
                "raw": cmd.raw,
            });
            let goals_txt = if cmd.goals.is_empty() {
                "none".to_string()
            } else {
                cmd.goals.join("; ")
            };
            let detail = format!("Working through the steps now — goals: {goals_txt}.");
            return self.start_named(wf, input, Some(detail), channel, chat).await;
        }

        // (2) Simplified `run <name>: <prompt>` / `workflow <name>: …` command.
        if let Some((name, prompt, explicit)) = parse_run_command(text) {
            match repo.find_by_name(&name, &workspace_id.to_string()).await {
                Ok(Some(wf)) => {
                    let input = json!({
                        "trigger": "chat",
                        "origin_workspace_id": workspace_id,
                        "channel": channel,
                        "chat": chat,
                        "thread": thread,
                        "user": user,
                        "prompt": prompt,
                        "msg": prompt,
                        "raw": text,
                    });
                    return self.start_named(wf, input, None, channel, chat).await;
                }
                Ok(None) if explicit => {
                    // Explicit `workflow`/`run workflow` keyword names an
                    // unknown workflow — reply, but don't hijack the message
                    // into a chat session either.
                    return Some(WorkflowChatAck {
                        reply: format!(
                            "No workflow named **{name}**. Check `Workflows` for the exact name."
                        ),
                    });
                }
                Ok(None) => {
                    // Bare `run <name>: …` with an unknown name — "run" reads
                    // too much like ordinary English to hijack; fall through
                    // to bindings, then to a normal chat session.
                }
                Err(e) => {
                    tracing::warn!("workflow chat: lookup for '{}' failed: {e}", name);
                    return None;
                }
            }
        }

        // (3) Channel bindings: a `chat`-kind trigger pins a workflow to this
        // channel/chat(/thread) so any (non-own-ack) message there starts it.
        if is_own_ack(text) {
            return None;
        }
        let stripped = strip_slack_tokens(text);
        let stripped = stripped.trim();
        if stripped.is_empty() {
            return None;
        }
        let has_mention = text.contains("<@");
        let triggers_repo = TriggersRepo::new(self.ctx.pool.clone());
        let triggers = match triggers_repo.list_enabled_by_kind("chat").await {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!("workflow chat: list chat bindings failed: {e}");
                return None;
            }
        };
        // `triggers` spans EVERY workspace (list_enabled_by_kind is global), so
        // walk the match candidates in preference order and only accept the
        // first one whose workflow actually belongs to the inbound workspace —
        // otherwise a channel bound by workspace B's Slack/Telegram integration
        // could leak into workspace A's workflow library and vice versa.
        let candidates = binding_candidates(&triggers, channel, chat, thread, has_mention);
        let mut wf = None;
        for trigger in candidates {
            let candidate = match repo.get(&trigger.workflow_id).await {
                Ok(w) => w,
                Err(e) => {
                    tracing::warn!(
                        "workflow chat: binding trigger {} workflow lookup failed: {e}",
                        trigger.id
                    );
                    continue;
                }
            };
            if candidate.workspace_id != workspace_id {
                tracing::info!(
                    "workflow chat: binding trigger {} targets workflow '{}' in workspace {} \
                     — inbound message is from workspace {workspace_id}, skipping",
                    trigger.id,
                    candidate.name,
                    candidate.workspace_id
                );
                continue;
            }
            wf = Some(candidate);
            break;
        }
        let wf = wf?;
        let input = json!({
            "trigger": "chat",
            "origin_workspace_id": workspace_id,
            "channel": channel,
            "chat": chat,
            "thread": thread,
            "user": user,
            "prompt": stripped,
            "msg": stripped,
            "raw": text,
        });
        self.start_named(wf, input, None, channel, chat).await
    }

    async fn try_control(
        &self,
        workspace_id: &str,
        channel: &str,
        chat: &str,
        thread: Option<&str>,
        _user: &str,
        text: &str,
    ) -> Option<WorkflowChatAck> {
        let control = parse_wf_control(text)?;

        // Help is the discoverability entry point — reply with the full guide
        // unconditionally (no active run / workflow binding required).
        if control == WfControl::Help {
            return Some(WorkflowChatAck { reply: wf_controls_help() });
        }

        // status / skip / abort target the run active on THIS thread.
        let repo = WorkflowsRepo::new(self.ctx.pool.clone());
        let run = self
            .find_active_run_for_thread(&repo, workspace_id, channel, chat, thread)
            .await?;
        let short = short_id(&run.id);
        match control {
            WfControl::Status => {
                let wf_name = repo.get(&run.workflow_id).await.ok().map(|w| w.name);
                Some(WorkflowChatAck { reply: self.run_status_summary(&run, wf_name.as_deref()) })
            }
            WfControl::Skip => {
                if let Ok(mut s) = self.ctx.wf_skip_current.lock() {
                    s.insert(run.id.clone());
                }
                Some(WorkflowChatAck {
                    reply: format!("⏭️ Skipping the current step of run `{short}`."),
                })
            }
            WfControl::Abort => {
                // Same as the Cancel button (routes::workflows::cancel_run): flip
                // the run to Canceled + emit. The engine's cancel poll then stops
                // the in-flight node and kills the run's sessions.
                match repo
                    .update_run(&run.id, RunStatus::Canceled, &run.nodes, Some("canceled"), true)
                    .await
                {
                    Ok(rev) => {
                        let _ = self.ctx.events.send(Event::WorkflowRunUpdated {
                            workspace_id: run.workspace_id.clone(),
                            run_id: run.id.clone(),
                            status: "canceled".into(),
                            node_id: None,
                            rev,
                            node: None,
                            nodes_done: 0,
                            nodes_total: 0,
                            waiting_approval: false,
                        });
                    }
                    Err(e) => tracing::warn!("workflow chat abort: update_run failed: {e}"),
                }
                Some(WorkflowChatAck {
                    reply: format!("🛑 Aborting run `{short}` — stopping its agents."),
                })
            }
            WfControl::Help => unreachable!("handled above"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_full_command() {
        let text = "@otto\n\
                    Action: Workflow\n\
                    Name: Implement Feature\n\
                    Msg: please do x y z, follow all relevant rules\n\
                    Jira ticket: PROJ-1111\n\
                    Working Directory: ~/repo\n\
                    Branch: release/base-branch, create wt from it\n\
                    Relevant Info: ~/a, ~/b\n\
                    Goals:\n\
                    - 100% test coverage\n\
                    - under 2 minutes runtime\n";
        let cmd = parse_workflow_command(text).expect("should parse");
        assert_eq!(cmd.name, "Implement Feature");
        assert_eq!(cmd.msg, "please do x y z, follow all relevant rules");
        assert_eq!(cmd.jira_ticket.as_deref(), Some("PROJ-1111"));
        assert_eq!(cmd.working_directory.as_deref(), Some("~/repo"));
        // The explicit base branch is captured; the trailing ", create wt from it"
        // guidance is dropped — only the branch name survives (honored, never overridden).
        assert_eq!(cmd.branch.as_deref(), Some("release/base-branch"));
        assert_eq!(cmd.relevant_info, vec!["~/a", "~/b"]);
        assert_eq!(cmd.goals, vec!["100% test coverage", "under 2 minutes runtime"]);
    }

    #[test]
    fn parses_control_synonyms() {
        use WfControl::*;
        for s in ["status", "STATUS", "progress", "?", "where are we", "status please"] {
            assert_eq!(parse_wf_control(s), Some(Status), "{s}");
        }
        for s in ["skip", "skip step", "next", "move on", "Skip It"] {
            assert_eq!(parse_wf_control(s), Some(Skip), "{s}");
        }
        for s in ["abort", "cancel", "stop", "kill it", "TERMINATE"] {
            assert_eq!(parse_wf_control(s), Some(Abort), "{s}");
        }
        for s in ["help", "commands", "usage", "h", "?help"] {
            assert_eq!(parse_wf_control(s), Some(Help), "{s}");
        }
        // `?` is a quick status, NOT help.
        assert_eq!(parse_wf_control("?"), Some(Status));
    }

    #[test]
    fn ignores_non_control_chatter() {
        // A longer sentence merely CONTAINING a keyword is not a command (length
        // guard + whole-word match), so ordinary thread chatter is left alone.
        assert_eq!(
            parse_wf_control("can you give me a status report on the whole project please"),
            None
        );
        assert_eq!(
            parse_wf_control("I think we should stop overengineering this whole thing honestly"),
            None
        );
        assert_eq!(parse_wf_control("just some normal message"), None);
        assert_eq!(parse_wf_control(""), None);
    }

    #[test]
    fn help_guide_lists_every_op_and_the_trigger_template() {
        let h = wf_controls_help();
        for op in WF_OPS {
            assert!(h.contains(op.canonical), "help missing op {}", op.canonical);
        }
        assert!(h.contains("Action: Workflow"), "help missing trigger template");
        assert!(h.contains("Branch:"), "help template missing Branch field");
    }

    #[test]
    fn tolerates_leading_slack_mention() {
        // Slack prefixes the message with `<@Ubot>` — on the same line as the
        // first field. This previously broke parsing (the run went to a chat
        // session instead of the workflow).
        let inline = "<@U08ABCDEF> Action: Workflow\nName: Write tests for a story\nMsg: go\n";
        let cmd = parse_workflow_command(inline).expect("inline mention should parse");
        assert_eq!(cmd.name, "Write tests for a story");

        let own_line = "<@U08ABCDEF>\nAction: Workflow\nName: Write tests for a story\n";
        assert_eq!(
            parse_workflow_command(own_line).unwrap().name,
            "Write tests for a story"
        );
    }

    #[test]
    fn inline_goals_and_aliases() {
        let text = "Action: Workflow\nName: Tests\nJira: PROJ-9\nGoals: a, b; c\n";
        let cmd = parse_workflow_command(text).unwrap();
        assert_eq!(cmd.jira_ticket.as_deref(), Some("PROJ-9"));
        assert_eq!(cmd.goals, vec!["a", "b", "c"]);
    }

    #[test]
    fn requires_action_workflow_and_name() {
        assert!(parse_workflow_command("just a normal message").is_none());
        assert!(parse_workflow_command("Action: Swarm\nName: x").is_none());
        assert!(parse_workflow_command("Action: Workflow\nMsg: no name").is_none());
    }

    #[test]
    fn run_command_grammar() {
        let (n, p, e) = parse_run_command("run Write tests: do the login story\nwith care").unwrap();
        assert_eq!((n.as_str(), e), ("Write tests", false));
        assert_eq!(p, "do the login story\nwith care");
        let (n, _, e) = parse_run_command("Run Workflow UI flow: go").unwrap();
        assert_eq!((n.as_str(), e), ("UI flow", true)); // longest-first: not name "workflow UI flow"
        let (n, _, e) = parse_run_command("workflow API tests: PROJ-1 please").unwrap();
        assert_eq!((n.as_str(), e), ("API tests", true));
        assert!(parse_run_command("run without colon").is_none());
        assert!(
            parse_run_command("Action: Workflow\nName: x").is_none(),
            "legacy handled elsewhere; 'action' is not a run keyword"
        );
        let (n, _, _) = parse_run_command("<@U08AB> run Tests: go").unwrap(); // mention stripped
        assert_eq!(n, "Tests");
    }

    #[test]
    fn binding_matching() {
        let spec = json!({"channel":"slack","chat":"C123"});
        assert!(binding_matches(&spec, "slack", "C123", None, false));
        assert!(
            binding_matches(&spec, "slack", "C123", Some("t1"), false),
            "unpinned spec matches any thread"
        );
        assert!(!binding_matches(&spec, "telegram", "C123", None, false));
        assert!(!binding_matches(&spec, "slack", "D9", None, false));
        let pinned = json!({"channel":"slack","chat":"C123","thread":"t1"});
        assert!(binding_matches(&pinned, "slack", "C123", Some("t1"), false));
        assert!(!binding_matches(&pinned, "slack", "C123", None, false));
        let m = json!({"channel":"slack","chat":"C123","mention_only":true});
        assert!(binding_matches(&m, "slack", "C123", None, true));
        assert!(!binding_matches(&m, "slack", "C123", None, false));
    }

    #[test]
    fn own_ack_prefix_is_recognized() {
        assert!(is_own_ack(
            "🚀 Started workflow **Foo** (run `abc123`). Working through the steps now."
        ));
        assert!(!is_own_ack("run Foo: bar"));
        assert!(!is_own_ack("please start the workflow"));
    }

    /// The loop guard (`is_own_ack`) and the ack reply built in `start_named`
    /// must never drift apart — both are anchored to `ACK_PREFIX`. Assert the
    /// guard recognizes anything built from the same constant, whatever the
    /// rest of the reply looks like.
    #[test]
    fn own_ack_guard_matches_anything_built_from_the_shared_prefix() {
        assert!(is_own_ack(&format!("{ACK_PREFIX} **Some Workflow** (run `xyz`). blah blah")));
        assert!(is_own_ack(&format!("{ACK_PREFIX} anything at all")));
    }

    fn make_trigger(spec: Value) -> WorkflowTrigger {
        WorkflowTrigger {
            id: otto_core::new_id(),
            workflow_id: otto_core::new_id(),
            kind: "chat".to_string(),
            spec,
            enabled: true,
            created_at: chrono::Utc::now(),
        }
    }

    #[test]
    fn thread_pinned_binding_preferred_over_unpinned() {
        let unpinned = make_trigger(json!({"channel":"slack","chat":"C1"}));
        let pinned = make_trigger(json!({"channel":"slack","chat":"C1","thread":"t1"}));

        let triggers = vec![unpinned.clone(), pinned.clone()];
        let picked = binding_candidates(&triggers, "slack", "C1", Some("t1"), false);
        assert_eq!(
            picked.first().unwrap().id,
            pinned.id,
            "pinned trigger should win when order is unpinned-first"
        );

        // Order shouldn't matter — same result reversed.
        let triggers2 = vec![pinned.clone(), unpinned.clone()];
        let picked2 = binding_candidates(&triggers2, "slack", "C1", Some("t1"), false);
        assert_eq!(picked2.first().unwrap().id, pinned.id);

        // With no thread on the inbound message, only the unpinned spec matches.
        let picked3 = binding_candidates(&triggers, "slack", "C1", None, false);
        assert_eq!(picked3.first().unwrap().id, unpinned.id);
    }

    /// `binding_candidates` returns ALL matches in preference order (not just
    /// the best one) — this is what lets `try_start` walk past a candidate
    /// whose workflow belongs to a different workspace and fall through to
    /// the next-best match instead of just refusing the whole message.
    #[test]
    fn binding_candidates_returns_all_matches_in_preference_order() {
        let unpinned = make_trigger(json!({"channel":"slack","chat":"C1"}));
        let pinned = make_trigger(json!({"channel":"slack","chat":"C1","thread":"t1"}));
        let triggers = vec![unpinned.clone(), pinned.clone()];

        let all = binding_candidates(&triggers, "slack", "C1", Some("t1"), false);
        assert_eq!(all.len(), 2, "both specs match a threaded message");
        assert_eq!(all[0].id, pinned.id, "pinned candidate ranked first");
        assert_eq!(all[1].id, unpinned.id);
    }
}
