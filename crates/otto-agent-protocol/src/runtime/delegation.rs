//! Delegation rules: what a Threads agent is told about subagents.
//!
//! After T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.)
//! `packages/provider-core/src/server/orchestrationInstructions.ts`: short
//! rules ride a thread's first prompt (every provider takes a prompt; not
//! every one takes a system prompt), so the agent delegates from one sentence
//! ("do this with a Codex subagent"), picks the provider and model from
//! `otto_thread_capabilities`, and waits to be woken instead of polling. Only
//! the driver sees them: the timeline keeps the person's own words. A
//! subagent gets its own rules in its task header instead
//! ([`task_header`]).

use std::borrow::Cow;

use otto_core::agent_runtime::{RuntimePromptReq, RuntimeSession};

use super::{Inner, State};

/// Opens and closes the rules block (the fake CLIs strip it).
pub const RULES_OPEN: &str = "<otto_instructions>";
pub const RULES_CLOSE: &str = "</otto_instructions>";

/// The rules a thread's agent gets once per conversation.
pub const RULES: &str = "## Otto subagents
Ordinary tasks need none of this. When the user asks for a subagent, another provider or model (Codex, Gemini, Claude, …), or parallel parts:
- Delegate with otto_thread_open: one subagent per part, `prompt` = a self-contained task (it does not see this conversation; no secrets). For your own provider and model your native subagent tool is fine.
- Call otto_thread_capabilities once before the first otto_thread_open: the providers, models and efforts enabled here. A provider or model that is missing: say so; never substitute another.
- Completion wakes you: each subagent's reply arrives here when it finishes. Do not poll it (otto_thread_read, sleep, loops). After delegating, do the rest of your own work, then end your turn.
- A subagent that failed: tell the user why (its reply carries the error); do not retry it or open another unasked.
- otto_thread_message sends a finished subagent follow-up work; otto_thread_close stops one you no longer need.";

/// What the driver sends for `req` in `st`'s thread: a provider switch's
/// context handoff (`fresh::with_handoff`), then [`with_rules`] on a
/// conversation's first turn — no turn yet, or a fresh context (a switch
/// included, so the new provider gets the rules too).
pub(super) fn for_driver<'a>(
    inner: &Inner,
    st: &State,
    req: &'a RuntimePromptReq,
) -> Cow<'a, RuntimePromptReq> {
    let first = st.proj.next_turn_ordinal == 0 || req.fresh_context;
    let teaches = inner.host.teaches_delegation();
    match super::fresh::with_handoff(st, req) {
        Some(switched) => {
            Cow::Owned(with_rules(teaches, &st.session, &switched, first).into_owned())
        }
        None => with_rules(teaches, &st.session, req, first),
    }
}

/// The rules ahead of the person's words on a conversation's first turn
/// (`first`) of a top-level thread whose host teaches delegation. A
/// subagent, a compared thread and a slash command (it must stay first) go
/// as they are.
fn with_rules<'a>(
    teaches: bool,
    session: &RuntimeSession,
    req: &'a RuntimePromptReq,
    first: bool,
) -> Cow<'a, RuntimePromptReq> {
    let plain = !teaches
        || !first
        || session.child_of().is_some()
        || session.council().is_some()
        || req.text.trim_start().starts_with('/');
    if plain {
        return Cow::Borrowed(req);
    }
    Cow::Owned(RuntimePromptReq {
        text: format!("{RULES_OPEN}\n{RULES}\n{RULES_CLOSE}\n\n{}", req.text),
        ..req.clone()
    })
}

/// The header a subagent's task arrives under: who delegated it, that it does
/// the work itself, and that its reply goes back on its own.
pub(super) fn task_header(from: &RuntimeSession) -> String {
    format!(
        "[Subagent task from thread \"{}\" ({}, id {}). Do the work yourself; start no \
         subagents unless this task says to. When your turn ends your reply goes back to that \
         thread on its own — do not message it.]",
        from.title,
        from.provider_slug(),
        from.id
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::agent_runtime::{
        ChildOpener, RuntimeAccessMode, RuntimeProvider, RuntimeSessionStatus,
    };

    fn session(meta: Option<serde_json::Value>) -> RuntimeSession {
        RuntimeSession {
            title_auto: false,
            id: "lead".into(),
            workspace_id: "w".into(),
            provider: RuntimeProvider::Claude,
            title: "Share links".into(),
            cwd: "/tmp".into(),
            model: None,
            effort: None,
            native_thread_id: None,
            status: RuntimeSessionStatus::Idle,
            access_mode: RuntimeAccessMode::ApprovalRequired,
            created_by: "u".into(),
            created_at: chrono::Utc::now(),
            last_active_at: chrono::Utc::now(),
            last_sequence: 0,
            parent_id: None,
            ui_control: None,
            background_tasks: Vec::new(),
            capabilities: Default::default(),
            meta,
            last_turn_status: None,
            last_error_class: None,
        }
    }

    #[test]
    fn the_first_turn_of_a_thread_carries_the_rules_once() {
        let s = session(None);
        let req = RuntimePromptReq::of("do this with a Codex subagent");
        let sent = with_rules(true, &s, &req, true);
        assert!(sent.text.starts_with(RULES_OPEN));
        assert!(sent.text.ends_with("\n\ndo this with a Codex subagent"));
        assert!(sent.text.contains("otto_thread_capabilities"));
        assert!(sent.text.contains("Do not poll"));
        // Later turns, a host that does not teach, a slash command: as typed.
        assert_eq!(with_rules(true, &s, &req, false).text, req.text);
        assert_eq!(with_rules(false, &s, &req, true).text, req.text);
        let slash = RuntimePromptReq::of("/review");
        assert_eq!(with_rules(true, &s, &slash, true).text, "/review");
    }

    #[test]
    fn a_subagent_gets_its_task_header_not_the_rules() {
        let child = session(Some(super::super::children::child_meta(
            &"lead".into(),
            ChildOpener::Agent,
        )));
        let req = RuntimePromptReq::of("look");
        assert_eq!(with_rules(true, &child, &req, true).text, "look");
        let h = task_header(&session(None));
        assert!(h.starts_with("[Subagent task from thread \"Share links\" (claude, id lead). "));
        assert!(h.contains("Do the work yourself"));
        assert!(!h.contains("otto_thread_message tool"));
    }
}
