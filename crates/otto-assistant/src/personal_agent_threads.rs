//! Personal agents on the **structured runtime** — their runs and their chat
//! are Threads threads (`otto-agent-protocol`), rendered with the Threads
//! conversation view, for every provider the runtime drives as a Threads
//! thread ([`THREAD_PROVIDERS`]). Any other provider (custom ones)
//! keeps a terminal session (`personal_agent_session`).
//!
//! A thread runs under the same server-owned meta a terminal session would
//! (`personal_agent`, `read_only`, `source`, `run_id`, `browser`, …: stored
//! on the thread, applied by the spawn's safety pipeline and read by the tool
//! policy), so a read-only run is confined the same way.
//!
//! A run: create the thread, send the report-augmented prompt, wait until
//! the thread has settled (`AgentRuntime::run_turn`: no turn running, no
//! background work the agent started still pending, and its own follow-up
//! turn after that work, if it starts one) — take the report from the
//! file the agent wrote, or else its last answer, and close the thread (it
//! stays in History, openable from the run and reopenable). No progress for
//! [`RUN_NO_PROGRESS`] = stuck, with a longer allowance while background
//! work is pending.

use std::path::PathBuf;

use otto_core::agent_runtime::{
    CreateRuntimeSessionReq, RuntimePromptReq, RuntimeProvider, RuntimeSessionStatus,
};
use otto_core::{Error, Id, Result};
use otto_state::PersonalAgent;
use serde_json::{json, Value};
use tracing::warn;

use crate::ctx::{AgentSessionOutcome, AgentSessionRun};
use crate::personal_agents_engine::{self as pa_engine, RunPlan, RUN_NO_PROGRESS};
use crate::AssistantCtx;

/// Providers whose personal-agent runs and chats are Threads threads — the
/// ones Threads starts on the structured runtime (`isRuntimeProvider` in
/// `ui/src/modules/threads/runtime/runtimeThreads.svelte.ts`).
pub const THREAD_PROVIDERS: &[&str] = &[
    "claude", "codex", "gemini", "grok", "agy", "opencode", "pi", "muse",
];

/// The runtime provider an agent's runs and chat use, or `None` for a
/// terminal-session provider. An empty provider is Claude (the default).
pub fn thread_provider(provider: &str) -> Option<RuntimeProvider> {
    let p = match provider.trim() {
        "" => "claude",
        p => p,
    };
    THREAD_PROVIDERS
        .contains(&p)
        .then(|| RuntimeProvider::parse(p))
        .flatten()
}

/// `<agent dir>/<run>.report.md` — where a run's agent writes its report.
pub fn report_path<C: AssistantCtx>(ctx: &C, agent: &PersonalAgent, run_id: &str) -> PathBuf {
    let run_name = otto_core::paths::safe_component(run_id).unwrap_or("invalid");
    pa_engine::default_agent_dir(ctx, &agent.id).join(format!("{run_name}.report.md"))
}

/// The server-owned meta a run's session (thread or terminal) runs under.
/// `personal_agent` is the session→agent identity the room / team tools and
/// the tool policy resolve (`personal_agent_name` is display only — Threads
/// groups and badges by it; `trigger` / `routine` say what started the run); `browser` adds the browser MCP; `work.origin`
/// marks the run ENGINE-owned (`source: "personal_agent"` is deliberately
/// outside `BACKGROUND_SESSION_SOURCES`, so without it the idle sweep would
/// read a terminal run as the user's own and never reclaim it).
/// `read_only` confines the session (daemon tool policy + CLI tool list +
/// forced sandbox); a read-only run gets no browser automation either —
/// Playwright can click and submit, the CLI's own web reading stays.
pub fn run_meta(agent: &PersonalAgent, run_id: &str, plan: &RunPlan) -> Value {
    json!({
        "source": "personal_agent",
        "personal_agent": agent.id,
        "personal_agent_name": agent.name,
        "run_id": run_id,
        "browser": agent.browser && !plan.read_only,
        "agent_mode": plan.mode,
        "read_only": plan.read_only,
        "trigger": plan.trigger,
        "routine": plan.routine,
        "work": { "origin": "personal_agent" },
    })
}

fn model_of(agent: &PersonalAgent) -> Option<String> {
    Some(agent.model.trim().to_string()).filter(|m| !m.is_empty())
}

/// Run a personal agent's fresh visible session: a Threads thread for a
/// runtime provider, else a terminal session. `Err` only for setup failures
/// before the session (e.g. the workspace is gone).
pub async fn run_agent_session<C: AssistantCtx>(
    ctx: &C,
    run: AgentSessionRun<'_>,
) -> Result<AgentSessionOutcome> {
    match thread_provider(&run.agent.provider) {
        Some(provider) => run_thread(ctx, run, provider).await,
        None => crate::personal_agent_session::run_pty_session(ctx, run).await,
    }
}

fn failed(session_id: Option<String>, reason: String) -> AgentSessionOutcome {
    AgentSessionOutcome {
        report: None,
        session_id,
        attempts: 1,
        failure: Some(reason),
    }
}

async fn run_thread<C: AssistantCtx>(
    ctx: &C,
    run: AgentSessionRun<'_>,
    provider: RuntimeProvider,
) -> Result<AgentSessionOutcome> {
    let AgentSessionRun {
        agent,
        run_id,
        owner,
        cwd,
        prompt,
        plan,
        cancel,
    } = run;
    let out_path = report_path(ctx, agent, run_id);
    if let Some(p) = out_path.parent() {
        let _ = tokio::fs::create_dir_all(p).await;
    }
    let _ = std::fs::remove_file(&out_path);
    let augmented = C::augment_report_prompt(prompt, &out_path.to_string_lossy());
    let req = CreateRuntimeSessionReq {
        title_auto: false,
        provider: Some(provider),
        cwd: Some(cwd.to_string()),
        title: Some(format!("Agent: {}", agent.name)),
        model: model_of(agent),
        // The daemon's default mode (Settings), deliberately: a directed run
        // keeps today's behaviour, and the EFFECTIVE mode is persisted on the
        // thread row the run points at (`run.session_id`). A read-only run's
        // confinement never depends on it — `meta.read_only` forces the
        // Seatbelt profile, strips the CLI's shell tools and makes the daemon
        // refuse every mutating tool, whatever the mode.
        access_mode: None,
        meta: Some(run_meta(agent, run_id, plan)),
    };
    // Creation finishes even if the run future is canceled: it publishes the
    // thread id on the run row, then closes the thread when Stop won first.
    let (ctx2, owner2, ws2, run2, stopped) = (
        ctx.clone(),
        owner.to_string(),
        agent.workspace_id.clone(),
        run_id.to_string(),
        cancel.clone(),
    );
    let creation = tokio::spawn(async move {
        let rt = ctx2.agent_runtime();
        let s = rt.create(&owner2, &ws2, req).await?;
        if let Err(e) = pa_engine::repo(&ctx2).set_run_session(&run2, &s.id).await {
            let _ = rt.close(&s.id).await;
            return Err(e);
        }
        if stopped.is_cancelled() {
            let _ = rt.close(&s.id).await;
            return Err(Error::Conflict("run stopped during thread creation".into()));
        }
        Ok(s)
    });
    let thread = match creation
        .await
        .unwrap_or_else(|e| Err(Error::Internal(e.to_string())))
    {
        Ok(s) => s,
        Err(e) => {
            warn!(agent = %agent.id, "personal agent: create thread ({}): {e}", agent.provider);
            return Ok(failed(None, format!("create-failed: {e}")));
        }
    };
    let id = thread.id.clone();
    let result = drive_turn(ctx, &id, &augmented, &out_path, cancel).await;
    // Done (or failed) — nothing of the agent's is still running: the
    // thread stops and stays in History, where it can be reopened.
    let _ = ctx.agent_runtime().close(&id).await;
    let _ = std::fs::remove_file(&out_path);
    Ok(match result {
        Ok(report) => AgentSessionOutcome {
            report: Some(report),
            session_id: Some(id.to_string()),
            attempts: 1,
            failure: None,
        },
        Err(reason) => failed(Some(id.to_string()), reason),
    })
}

/// Send the run's prompt and wait until the thread has settled
/// (`AgentRuntime::run_turn`): the report file if the agent wrote one, else
/// its last answer. `Err(reason)` when the agent could not start, the run was
/// stopped, it ended with nothing to report, or it made no progress for
/// [`RUN_NO_PROGRESS`] (background work it started is not "no progress").
async fn drive_turn<C: AssistantCtx>(
    ctx: &C,
    id: &Id,
    prompt: &str,
    out_path: &std::path::Path,
    cancel: &otto_core::cancel_signal::CancelSignal,
) -> std::result::Result<String, String> {
    let mut opts = otto_agent_protocol::TurnOpts::new(cancel.clone());
    opts.no_progress = Some(RUN_NO_PROGRESS);
    let out = ctx
        .agent_runtime()
        .run_turn(id, &RuntimePromptReq::of(prompt), opts, |_| {})
        .await
        .map_err(|e| format!("create-failed: {e}"))?;
    let file = tokio::fs::read_to_string(out_path).await.ok();
    report(out.ended, file, out.answer, out.turn_status)
}

/// What a settled run reports: the report file wins over the last answer.
fn report(
    ended: otto_agent_protocol::Ended,
    file: Option<String>,
    answer: String,
    status: Option<otto_core::agent_runtime::ProviderTurnStatus>,
) -> std::result::Result<String, String> {
    use otto_agent_protocol::Ended;
    match ended {
        Ended::Settled => {}
        Ended::Cancelled => return Err("stopped".into()),
        Ended::ServerRestart => return Err("exited: Otto restarted during the run".into()),
        Ended::Deadline | Ended::NoProgress | Ended::BackgroundLimit | Ended::LoopGuard => {
            return Err("stuck".into())
        }
    }
    let report = file.filter(|t| !t.trim().is_empty()).unwrap_or(answer);
    if report.trim().is_empty() {
        return Err(format!(
            "exited: the turn ended {} with no report",
            status.map_or("without a status".to_string(), |s| format!("{s:?}")
                .to_lowercase())
        ));
    }
    Ok(report)
}

/// Stop a run's session (Stop / the proactive budget): close its thread, or
/// kill its terminal session.
pub async fn stop_run_session<C: AssistantCtx>(ctx: &C, sid: &str) -> Result<()> {
    let id: Id = sid.to_string();
    match ctx.agent_runtime().get(&id).await {
        Ok(t) if t.status == RuntimeSessionStatus::Closed => Ok(()),
        Ok(_) => ctx.agent_runtime().close(&id).await.map(|_| ()),
        Err(_) => ctx.manager().kill_session(&id).await,
    }
}

/// The caller's chat with `agent` as a Threads thread.
#[derive(Debug, Clone)]
pub struct ChatThread {
    pub session_id: Id,
    pub created: bool,
}

/// Return (creating if absent or closed) `user_id`'s chat thread with
/// `agent`: the pinned one when it is the caller's, else the caller's own
/// newest open chat with this agent, else a new thread in the agent's
/// persona folder (its context snapshot sent as the first message). Only a
/// free pin slot is taken (another member's open chat stays theirs).
/// `Ok(None)` = the agent's provider is not a thread provider, or the
/// workspace needs what threads cannot do yet (the caller opens a terminal
/// chat). The caller serialises per agent.
pub async fn chat_thread<C: AssistantCtx>(
    ctx: &C,
    agent: &PersonalAgent,
    user_id: &str,
) -> Result<Option<ChatThread>> {
    let Some(provider) = thread_provider(&agent.provider) else {
        return Ok(None);
    };
    let rt = ctx.agent_runtime();
    let open =
        |s: &otto_core::agent_runtime::RuntimeSession| s.status != RuntimeSessionStatus::Closed;
    let pinned = match agent.chat_session_id.as_deref().filter(|s| !s.is_empty()) {
        Some(sid) => rt.get(&sid.to_string()).await.ok().filter(open),
        None => None,
    };
    if let Some(t) = pinned.as_ref().filter(|t| t.created_by == user_id) {
        return Ok(Some(ChatThread {
            session_id: t.id.clone(),
            created: false,
        }));
    }
    // The chat is permanent: the caller's own pinned chat that was closed
    // (an app restart, a stray Close) is reopened in place — same history,
    // the provider's session resumed — instead of a fresh, empty thread.
    if pinned.is_none() {
        if let Some(sid) = agent.chat_session_id.as_deref().filter(|s| !s.is_empty()) {
            let id: Id = sid.to_string();
            let mine =
                rt.get(&id).await.ok().filter(|t| {
                    t.created_by == user_id && t.status == RuntimeSessionStatus::Closed
                });
            if mine.is_some() {
                match ctx.reopen_thread(&id).await {
                    Ok(_) => {
                        return Ok(Some(ChatThread {
                            session_id: id,
                            created: false,
                        }))
                    }
                    Err(e) if e.to_string().contains(crate::ctx::RUNTIME_UNSUPPORTED) => {}
                    Err(e) => warn!(agent = %agent.id, "reopening the chat failed: {e}"),
                }
            }
        }
    }
    if pinned.is_some() {
        let own = rt
            .list(&agent.workspace_id, Some(&user_id.to_string()))
            .await
            .unwrap_or_default()
            .into_iter()
            .filter(|t| open(t) && t.meta_flag("personal_agent_chat"))
            .filter(|t| {
                t.meta.as_ref().and_then(|m| m.get("personal_agent")) == Some(&json!(agent.id))
            })
            .max_by(|a, b| a.created_at.cmp(&b.created_at));
        if let Some(t) = own {
            return Ok(Some(ChatThread {
                session_id: t.id,
                created: false,
            }));
        }
    }
    // Snapshot once, before creating the thread: reusing an existing chat
    // above never injects or rewrites its context behind the user's back.
    let (user_context, context_version) = pa_engine::repo(ctx).context(&agent.id).await?;
    let cwd = pa_engine::ensure_agent_workspace(ctx, agent).await?;
    let created = rt
        .create(
            &user_id.to_string(),
            &agent.workspace_id,
            CreateRuntimeSessionReq {
                title_auto: false,
                provider: Some(provider),
                cwd: Some(cwd),
                title: Some(format!("Chat: {}", agent.name)),
                model: model_of(agent),
                access_mode: None,
                meta: Some(json!({
                    "source": "personal_agent",
                    "personal_agent": agent.id,
                    "personal_agent_name": agent.name,
                    "personal_agent_chat": true,
                    "agent_mode": "directed",
                    "personal_agent_context_version": context_version,
                    "browser": agent.browser,
                })),
            },
        )
        .await;
    let thread = created?;
    if !user_context.trim().is_empty() {
        let initial = pa_engine::with_user_context(
            "Use this context for our conversation. Briefly acknowledge that you are ready, then wait for my request.",
            &user_context,
        );
        if let Err(e) = rt.prompt(&thread.id, &initial).await {
            let _ = rt.close(&thread.id).await;
            return Err(Error::Upstream(format!(
                "The chat thread did not accept its context ({e}). Try opening it again."
            )));
        }
    }
    if pinned.is_none() {
        pa_engine::repo(ctx)
            .set_chat_session(&agent.id, Some(&thread.id))
            .await?;
        // An open page must learn the agent now has a chat: a routine's first
        // result opens it, and without this the page kept the intro and the
        // result only showed after a reload.
        pa_engine::emit_changed(ctx, &agent.workspace_id, &agent.id, "agent");
    }
    Ok(Some(ChatThread {
        session_id: thread.id,
        created: true,
    }))
}

/// Close `agent`'s pinned chat (Reset): its thread, or its terminal session.
pub async fn close_chat<C: AssistantCtx>(ctx: &C, agent: &PersonalAgent) {
    if let Some(sid) = agent.chat_session_id.as_deref().filter(|s| !s.is_empty()) {
        let _ = stop_run_session(ctx, sid).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_agent_protocol::Ended;
    use otto_core::agent_runtime::ProviderTurnStatus;

    #[test]
    fn the_report_file_wins_over_the_last_answer_and_each_end_has_its_reason() {
        let done = Some(ProviderTurnStatus::Completed);
        let r = |ended, file: Option<&str>, answer: &str| {
            report(ended, file.map(String::from), answer.into(), done)
        };
        assert_eq!(
            r(Ended::Settled, Some("# Report"), "chat text").as_deref(),
            Ok("# Report")
        );
        assert_eq!(
            r(Ended::Settled, Some("  "), "chat text").as_deref(),
            Ok("chat text")
        );
        assert!(r(Ended::Settled, None, "")
            .unwrap_err()
            .contains("no report"));
        assert_eq!(r(Ended::Cancelled, None, "x"), Err("stopped".into()));
        assert_eq!(r(Ended::NoProgress, None, "x"), Err("stuck".into()));
    }

    #[test]
    fn thread_providers_are_the_runtime_ones() {
        assert_eq!(thread_provider(""), Some(RuntimeProvider::Claude));
        assert_eq!(thread_provider(" codex "), Some(RuntimeProvider::Codex));
        for p in ["gemini", "grok", "agy", "opencode", "pi", "muse"] {
            assert!(thread_provider(p).is_some(), "{p}");
        }
        // Custom providers keep a terminal session.
        assert_eq!(thread_provider("my-cli"), None);
    }

    #[test]
    fn a_read_only_run_gets_no_browser_and_is_confined() {
        let mut agent = otto_state::PersonalAgent {
            admission_generation: 0,
            id: "a1".into(),
            workspace_id: "w".into(),
            name: "A".into(),
            avatar: String::new(),
            soul_md: String::new(),
            provider: "claude".into(),
            model: String::new(),
            cwd: String::new(),
            browser: true,
            delivery: json!({"type": "none"}),
            enabled: true,
            chat_session_id: None,
            created_by: Some("u".into()),
            created_by_agent: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let proactive = RunPlan {
            mode: "proactive",
            read_only: true,
            goal: None,
            max_duration: None,
            trigger: "proactive",
            routine: None,
        };
        let ro = run_meta(&agent, "r1", &proactive);
        assert_eq!(ro["read_only"], true);
        assert_eq!(ro["browser"], false);
        assert_eq!(ro["personal_agent"], "a1");
        assert_eq!(ro["run_id"], "r1");
        assert_eq!(ro["trigger"], "proactive");
        agent.browser = true;
        let rw = run_meta(&agent, "r2", &RunPlan::directed());
        assert_eq!(rw["read_only"], false);
        assert_eq!(rw["browser"], true);
        assert_eq!(rw["work"]["origin"], "personal_agent");
    }

    /// A routine whose permission is `read_only` runs confined in CODE (the
    /// session meta every confinement layer reads), not only in its prompt;
    /// a `directed` one is not.
    #[test]
    fn a_read_only_routine_runs_confined() {
        let agent = otto_state::PersonalAgent {
            admission_generation: 0,
            id: "a1".into(),
            workspace_id: "w".into(),
            name: "A".into(),
            avatar: String::new(),
            soul_md: String::new(),
            provider: "codex".into(),
            model: String::new(),
            cwd: String::new(),
            browser: true,
            delivery: json!({"type": "none"}),
            enabled: true,
            chat_session_id: None,
            created_by: Some("u".into()),
            created_by_agent: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let mut s = otto_state::PersonalAgentSchedule {
            schedule_generation: 0,
            admission_generation: 0,
            id: "s1".into(),
            agent_id: "a1".into(),
            schedule: json!({"cadence": "daily", "at": "09:00"}),
            timezone: "UTC".into(),
            directive: "Morning brief".into(),
            enabled: true,
            last_run_at: None,
            next_run_at: None,
            armed_at: None,
            permission: "read_only".into(),
            delivery: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let ro = run_meta(&agent, "r1", &RunPlan::for_schedule(&s));
        assert_eq!(ro["read_only"], true);
        assert_eq!(ro["browser"], false);
        assert_eq!(ro["trigger"], "schedule");
        s.permission = "directed".into();
        let rw = run_meta(&agent, "r2", &RunPlan::for_schedule(&s));
        assert_eq!(rw["read_only"], false);
    }
}
