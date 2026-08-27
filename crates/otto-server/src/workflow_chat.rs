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
Jira ticket: <optional, e.g. PROJ-123 — fetched IN FULL: description, fields and all comments>\n\
Working Directory: /abs/path/to/repo, /abs/path/to/other-repo\n\
PR: <optional — 15, or repo-a#15, repo-b#340 when several repos>\n\
Branch: <optional BASE branch the PR merges into, e.g. develop — NOT the feature branch>\n\
Relevant Info: /abs/path/to/extra-repo, /abs/path/to/docs\n\
Goals:\n\
  - <goal 1>\n\
  - <goal 2>";

/// Per-field guidance rendered under the template in the help guide — the
/// explanations users actually need to fill it in right (semantics that have
/// bitten in practice: `Branch:` is the review BASE, not the feature branch).
const WF_TRIGGER_FIELD_NOTES: &str = "\
• *Name* — must match a workflow from the list below exactly.\n\
• *Working Directory* — absolute path(s) (no `~`); comma-separate SEVERAL repos to \
review them as one run (cross-repo interactions included).\n\
• *PR* — which pull request to work on. `PR: 15` for one repo, or `repo-a#15, repo-b#340` \
to name one per repo. The ONLY unambiguous handle when a repo has several open PRs — \
and it supplies both branches (source + destination) on its own, so `Branch:` can be \
left out entirely. Omit it and the PR is matched by the Jira key; if that matches \
two open PRs the run stops on that repo rather than guessing.\n\
• *Branch* — OPTIONAL, and it means the branch the PR merges INTO (e.g. `develop`) — the \
run cuts an isolated worktree from it and reviews the diff against it. The feature \
branch does NOT go here: it is discovered from the PR. A value naming your ticket \
(e.g. `feature/PROJ-123`) is taken as the PR's source branch instead, and you get a \
note in the thread saying so.\n\
• *Jira ticket* — fetched in full (description, fields, every comment) into the run \
context for all agents; also how the PR is found when `PR:` is omitted.\n\
• *Msg / Goals* — free-text instructions and goal list; agents read them verbatim.";

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
    s.push_str("\n```\n");
    s.push_str(WF_TRIGGER_FIELD_NOTES);
    s.push_str("\n\n*2. Control a running workflow* — reply in the run's thread:\n");
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
    /// The PR(s) to work on (`PR:` / `Pull Request:`), verbatim. One repo:
    /// `15`. Several: `repo-a#15, repo-b#340` — the comma shape
    /// `Working Directory:` already uses. An explicit PR number is the only
    /// UNAMBIGUOUS handle when a repo has more than one open PR, and it yields
    /// both branches (source AND destination) from the provider API, so it also
    /// makes `Branch:` unnecessary.
    pub pr: Option<String>,
    /// The PR's SOURCE (feature) branch, when the user named it. Never the diff
    /// base. Filled either from an explicit `PR branch:`/`Source:` field, or by
    /// rescuing a `Branch:` value that is plainly the feature branch — see
    /// [`split_branch_fields`].
    pub pr_branch: Option<String>,
    /// Set when `Branch:` was reinterpreted as `pr_branch`. The caller echoes it
    /// into the chat ack: a silent reinterpretation of an explicit user field is
    /// worse than the mistake it fixes.
    pub branch_note: Option<String>,
    pub relevant_info: Vec<String>,
    pub goals: Vec<String>,
    pub raw: String,
}

/// Resolve `Jira ticket:` to an issue KEY.
///
/// Slack rewrites a pasted link as `<url|title>`, and token-stripping keeps the
/// human-readable label — so `Jira ticket: https://…/browse/GS-123` arrived as
/// the page's TITLE. That was then used as the key: the fetch 404'd and the run
/// continued with no ticket at all, silently. The key is still in the original
/// message, so look for one in the stripped value first, then in the raw text
/// (`/browse/KEY`, or a bare `KEY` token). Only when nothing looks like a key is
/// the value passed through unchanged.
fn resolve_jira_key(value: Option<String>, raw: &str) -> Option<String> {
    let value = value?;
    if let Some(k) = find_jira_key(&value) {
        return Some(k);
    }
    // The label lost it; the raw message still has `<https://…/browse/KEY|…>`.
    if let Some(rest) = raw.split("/browse/").nth(1) {
        if let Some(k) = find_jira_key(rest) {
            return Some(k);
        }
    }
    Some(value)
}

/// First `ABC-123`-shaped token in `s`: uppercase letters, `-`, then digits.
fn find_jira_key(s: &str) -> Option<String> {
    let b = s.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if !b[i].is_ascii_uppercase() {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && b[i].is_ascii_uppercase() {
            i += 1;
        }
        if i < b.len() && b[i] == b'-' {
            let mut j = i + 1;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            // Reject `ABC-12X`: a key ends at the digits.
            let ends_cleanly = j > i + 1 && b.get(j).is_none_or(|c| !c.is_ascii_alphanumeric());
            if ends_cleanly {
                return Some(s[start..j].to_string());
            }
        }
    }
    None
}

/// Decide what a declared `Branch:` value actually is, given the run's Jira key.
///
/// `Branch:` is documented as the BASE — the branch the PR merges INTO. Users
/// reliably put the FEATURE branch there instead, and that used to be silently
/// fatal: the run cut its worktree from the feature branch and reviewed it
/// against itself, so `git diff` was empty, zero reviewer agents ran, and the
/// review reported a perfect score on a PR nobody read.
///
/// A branch carrying the run's Jira key is the feature branch — no base branch
/// is ever named after a ticket. Route it to `pr_branch` (where it usefully
/// disambiguates WHICH open PR to review) and leave the base unset so it is
/// detected from the PR / the repo default. Anything else is taken at face value.
///
/// Returns `(base, pr_branch, note)`.
fn split_branch_fields(
    branch: Option<String>,
    jira_ticket: Option<&str>,
) -> (Option<String>, Option<String>, Option<String>) {
    let Some(b) = branch else {
        return (None, None, None);
    };
    let key = jira_ticket.map(str::trim).filter(|k| !k.is_empty());
    let looks_like_feature_branch = key
        .map(|k| b.to_lowercase().contains(&k.to_lowercase()))
        .unwrap_or(false);
    if !looks_like_feature_branch {
        return (Some(b), None, None);
    }
    let note = format!(
        "`Branch: {b}` names the ticket, so it is the PR's SOURCE branch, not the base it merges \
         into — using it to pick the PR and detecting the base from the PR itself. \
         Put the destination (e.g. `develop`) in `Branch:`, or name the PR with `PR: <number>`."
    );
    (None, Some(b), Some(note))
}

/// Strip Slack entity tokens, then decode Slack's HTML escapes, so the
/// structured parser sees the text the user actually typed: `<@U…>` mentions,
/// `<#C…>` channel refs and `<!here>` are removed; `<url|label>` links keep
/// their label; `&amp;`/`&lt;`/`&gt;` become `&`/`<`/`>`.
///
/// The ORDER matters and is the whole point of doing both here. Slack escapes
/// `&`, `<` and `>` in message text, so a `<` that survives to the token scan
/// can only be a real Slack token — a literal one the user typed arrives as
/// `&lt;`. Stripping first and decoding after therefore can't turn a user's
/// `<foo>` into a token, and can't leave an entity inside a link label.
///
/// Without the decode, `&` in a workflow name was fatal: `Name: A &amp; B` never
/// matched the workflow actually called `A & B` (the lookup is an exact
/// `name = ? COLLATE NOCASE`), so `try_start` returned `None` and the message
/// fell through to a plain chat session with no error anywhere.
fn strip_slack_tokens(text: &str) -> String {
    decode_html_entities(&strip_entity_tokens(text))
}

/// Slack's `&amp;` / `&lt;` / `&gt;` → `&` / `<` / `>`. Single left-to-right
/// pass, so `&amp;lt;` decodes to the literal `&lt;` rather than to `<`.
/// Slack escapes exactly these three characters and nothing else, so this
/// deliberately does NOT implement general HTML entity decoding — inventing
/// extra entities would corrupt text Slack never escaped.
fn decode_html_entities(text: &str) -> String {
    const ENTITIES: &[(&str, char)] = &[("&amp;", '&'), ("&lt;", '<'), ("&gt;", '>')];
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find('&') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        match ENTITIES.iter().find(|(pat, _)| tail.starts_with(pat)) {
            Some((pat, ch)) => {
                out.push(*ch);
                rest = &tail[pat.len()..];
            }
            None => {
                out.push('&');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// The token pass of [`strip_slack_tokens`] — see there for why it runs first.
fn strip_entity_tokens(text: &str) -> String {
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

    let jira_ticket = resolve_jira_key(pick(&["jira ticket", "jira", "jira_ticket", "ticket"]), text);
    // Only the branch name matters here — trailing guidance like
    // "…, create wt from it" is dropped (the run already creates a worktree);
    // we keep just the part before the first comma.
    let declared_branch = pick(&["branch", "base", "base branch", "base_branch"])
        .and_then(|s| s.split(',').next().map(|b| b.trim().to_string()))
        .filter(|s| !s.is_empty());
    let (branch, rescued_pr_branch, branch_note) =
        split_branch_fields(declared_branch, jira_ticket.as_deref());
    // An explicit source-branch field always wins over a rescued `Branch:`.
    let pr_branch = pick(&["pr branch", "source", "source branch", "feature branch"])
        .or(rescued_pr_branch);

    Some(WorkflowCommand {
        name,
        msg: pick(&["msg", "message"]).unwrap_or_default(),
        jira_ticket,
        working_directory: pick(&["working directory", "working dir", "workdir", "cwd"]),
        branch,
        // Kept verbatim (commas and all): the multi-repo form is
        // `repo-a#15, repo-b#340`, and only the step that resolves repos knows
        // which entry each `repo#number` belongs to.
        pr: pick(&["pr", "prs", "pull request", "pull_request", "pr number"]),
        pr_branch,
        branch_note,
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
        crate::workflow_engine::spawn_run(
            self.ctx.clone(),
            ws,
            wf.clone(),
            run.id.clone(),
            input.clone(),
            None,
            false,
            None,
        );

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
                // Which PR(s) to work on, and the PR's source branch when named.
                // Both are HANDLES for the step that resolves the PR — never a
                // diff base. `pr` is the only unambiguous one when a repo has
                // several open PRs.
                "pr": cmd.pr,
                "pr_branch": cmd.pr_branch,
                "relevant_info": cmd.relevant_info,
                "goals": cmd.goals,
                "raw": cmd.raw,
            });
            let goals_txt = if cmd.goals.is_empty() {
                "none".to_string()
            } else {
                cmd.goals.join("; ")
            };
            let mut detail = format!("Working through the steps now — goals: {goals_txt}.");
            // Never reinterpret an explicit field in silence.
            if let Some(note) = &cmd.branch_note {
                detail.push_str(&format!("\n⚠️ {note}"));
            }
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
        // unconditionally (no active run / workflow binding required), plus the
        // workspace's actual workflows so users see what `Name:` can be.
        if control == WfControl::Help {
            let repo = WorkflowsRepo::new(self.ctx.pool.clone());
            let mut reply = wf_controls_help();
            if let Ok(wfs) = repo.list(&workspace_id.to_string()).await {
                if !wfs.is_empty() {
                    reply.push_str("\n\n*3. Available workflows* (use as `Name:`):\n");
                    for w in wfs {
                        // First line of the description as a one-line hint.
                        let hint = w.description.lines().next().unwrap_or("").trim().to_string();
                        if hint.is_empty() {
                            reply.push_str(&format!("• *{}*\n", w.name));
                        } else {
                            reply.push_str(&format!("• *{}* — {}\n", w.name, hint));
                        }
                    }
                }
            }
            return Some(WorkflowChatAck { reply: reply.trim_end().to_string() });
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
        // A real base branch is left alone, and names nothing else.
        assert_eq!(cmd.pr, None);
        assert_eq!(cmd.pr_branch, None);
        assert_eq!(cmd.branch_note, None);
    }

    #[test]
    fn feature_branch_in_the_base_field_becomes_a_pr_hint() {
        // The failure this prevents: `Branch:` holding the PR's own branch made
        // the run diff that branch against itself — empty diff, no reviewers, a
        // perfect score on an unreviewed PR.
        let text = "Action: Workflow\n\
                    Name: PR Reviewer\n\
                    Jira ticket: PROJ-5282\n\
                    Working Directory: /r\n\
                    Branch: feature/PROJ-5282\n";
        let cmd = parse_workflow_command(text).expect("should parse");
        assert_eq!(cmd.branch, None, "a ticket-named branch must not become the base");
        assert_eq!(cmd.pr_branch.as_deref(), Some("feature/PROJ-5282"));
        let note = cmd.branch_note.expect("the reinterpretation must be announced");
        assert!(note.contains("feature/PROJ-5282") && note.contains("SOURCE"), "{note}");

        // Case-insensitive on the key, and a lone `Base:` alias behaves the same.
        let lower = parse_workflow_command(
            "Action: Workflow\nName: X\nJira ticket: PROJ-5282\nBase: FEATURE/proj-5282\n",
        )
        .unwrap();
        assert_eq!(lower.branch, None);
        assert_eq!(lower.pr_branch.as_deref(), Some("FEATURE/proj-5282"));

        // No ticket to match against ⇒ take the field at face value (the
        // resolve_base HEAD guard is the backstop there).
        let no_key =
            parse_workflow_command("Action: Workflow\nName: X\nBranch: feature/PROJ-5282\n").unwrap();
        assert_eq!(no_key.branch.as_deref(), Some("feature/PROJ-5282"));
        assert_eq!(no_key.pr_branch, None);
        assert_eq!(no_key.branch_note, None);
    }

    #[test]
    fn jira_ticket_survives_a_slack_link_unfurl() {
        // What Slack actually delivered: the pasted Jira URL rewritten as
        // `<url|title>`. Keeping the label made the "key" the page title, the
        // fetch 404'd, and the run reviewed with no ticket.
        let text = "Action: Workflow\nName: PR Reviewer\n\
                    Jira ticket: <https://acme.atlassian.net/browse/GS-16578|TMX Update on ABC \
                    (Imperial wins) - Org Separation, Affiliate ID, DOB Format>\n\
                    Working Directory: /r\n";
        let cmd = parse_workflow_command(text).expect("should parse");
        assert_eq!(cmd.jira_ticket.as_deref(), Some("GS-16578"));

        // A bare key, a bare URL, and a key with surrounding words all resolve.
        for (input, want) in [
            ("Jira ticket: GS-1", "GS-1"),
            ("Jira ticket: https://acme.atlassian.net/browse/ABC-42", "ABC-42"),
            ("Jira ticket: see PROJ-77 please", "PROJ-77"),
        ] {
            let c = parse_workflow_command(&format!("Action: Workflow\nName: X\n{input}\n")).unwrap();
            assert_eq!(c.jira_ticket.as_deref(), Some(want), "{input}");
        }

        // Nothing key-shaped anywhere → passed through untouched, as before.
        let free = parse_workflow_command("Action: Workflow\nName: X\nJira ticket: none yet\n").unwrap();
        assert_eq!(free.jira_ticket.as_deref(), Some("none yet"));
    }

    #[test]
    fn parses_pr_field_single_and_multi_repo() {
        let one = parse_workflow_command("Action: Workflow\nName: X\nPR: 15\n").unwrap();
        assert_eq!(one.pr.as_deref(), Some("15"));

        // Multi-repo keeps the whole `repo#number` list verbatim — only the step
        // that resolves repos can map each entry.
        let many = parse_workflow_command(
            "Action: Workflow\nName: X\nPull Request: repo-a#15, repo-b#340\n",
        )
        .unwrap();
        assert_eq!(many.pr.as_deref(), Some("repo-a#15, repo-b#340"));

        // An explicit source-branch field beats a rescued `Branch:`.
        let both = parse_workflow_command(
            "Action: Workflow\nName: X\nJira ticket: PROJ-1\nBranch: feature/PROJ-1\nSource: feature/PROJ-1-real\n",
        )
        .unwrap();
        assert_eq!(both.pr_branch.as_deref(), Some("feature/PROJ-1-real"));
        assert_eq!(both.branch, None);
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
        assert!(h.contains("PR:"), "help template missing PR field");
        // The two semantics users get wrong must be stated, not implied.
        assert!(h.contains("merges INTO"), "help must define Branch as the destination");
        assert!(
            h.contains("several open PRs"),
            "help must say why PR: exists"
        );
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
    fn slack_html_escapes_are_decoded_in_field_values() {
        // Slack escapes `&`, `<` and `>` in message text, so a workflow whose
        // name contains an ampersand arrived as `A &amp; B` and never matched
        // the row named `A & B` (an exact `name = ? COLLATE NOCASE` lookup) —
        // try_start returned None and the trigger silently became a chat
        // session. Every `… fetch &amp; review` workflow was unreachable from
        // Slack because of this.
        let text =
            "Action: Workflow\nName: Game Attributes &amp; Games Update\nMsg: a &lt;b&gt; c\n";
        let cmd = parse_workflow_command(text).unwrap();
        assert_eq!(cmd.name, "Game Attributes & Games Update");
        assert_eq!(cmd.msg, "a <b> c");
    }

    #[test]
    fn entity_decode_runs_after_token_stripping() {
        // A `<` the user actually typed reaches us as `&lt;`, so it must never
        // be re-scanned as a Slack token: decoding strictly after the token
        // pass is what guarantees that. `&amp;lt;` is a literal `&lt;`, not `<`.
        assert_eq!(decode_html_entities("&amp;lt;"), "&lt;");
        assert_eq!(strip_slack_tokens("&lt;@U123&gt;"), "<@U123>");
        assert_eq!(strip_slack_tokens("<@U123> hi"), " hi");
        // A bare `&` (Slack does escape it, but be forgiving) survives intact.
        assert_eq!(strip_slack_tokens("A & B &nbsp; C"), "A & B &nbsp; C");
        // Entities inside a `<url|label>` label are decoded too.
        assert_eq!(strip_slack_tokens("<https://x/y|A &amp; B>"), "A & B");
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
