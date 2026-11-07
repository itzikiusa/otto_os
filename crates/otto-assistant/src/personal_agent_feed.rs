//! An agent's **one continuous conversation**. Everything the agent does
//! lands in its chat thread, so the user sees it in one place, the way a
//! messenger bot's proactive follow-ups work:
//!
//! * a scheduled / manual / proactive run's result is posted as an attributed
//!   notice ("Morning brief ran · …", with the run thread);
//! * a delegated task's outcome reaches the main agent's chat as its next
//!   turn (the main agent reads the report and answers the user), and a
//!   failed, stopped or lost delegation says so there. The lead never
//!   "silently stops".
//!
//! The chat is permanent: a closed chat thread is reopened in place
//! ([`AssistantCtx::reopen_thread`]) before a post. A post that needs an
//! agent turn waits for a busy chat to finish its turn (bounded).
//!
//! Delivery goes through the small [`ChatSink`] seam so the policy (reopen,
//! notice-or-turn fallback, busy retry) is unit-tested without a live agent.

use std::time::{Duration, Instant};

use chrono::Utc;
use otto_core::agent_runtime::RuntimeSessionStatus;
use otto_core::keyed_lock::KeyedLocks;
use otto_core::{Error, Id, Result};
use otto_state::{PersonalAgent, PersonalAgentRun};
use tracing::warn;

use crate::personal_agents_engine::one_line;
use crate::{AssistantCtx, BoxFut};

/// Serialises get-or-create of an agent's chat (the chat route and the feed
/// both open it) per agent id.
pub static CHAT_LOCKS: KeyedLocks<String> = KeyedLocks::new();

/// How long a post that needs a turn waits for a busy chat to be free.
pub const BUSY_WAIT: Duration = Duration::from_secs(30 * 60);
/// Pause between attempts while the chat is busy with a turn.
#[cfg(not(test))]
const BUSY_RETRY: Duration = Duration::from_secs(5);
#[cfg(test)]
const BUSY_RETRY: Duration = Duration::from_millis(5);
/// Longest text relayed into a chat turn (bytes; longer is cut).
pub const MAX_POST: usize = 64 * 1024;

/// When a post also asks the agent for a turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Turn {
    /// Notice only (proactive findings: no agent turn per background run).
    Never,
    /// A turn only when the runtime cannot show the notice (so the result
    /// still reaches the chat).
    IfNoNotice(String),
    /// Always a turn after the notice: the agent must act on it (a delegated
    /// report the main agent summarises for the user).
    Always(String),
}

/// One item for an agent's chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Post {
    /// The attributed one-liner shown in the conversation.
    pub notice: String,
    pub turn: Turn,
}

/// How a post reached the chat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Delivered {
    pub reopened: bool,
    pub noticed: bool,
    pub prompted: bool,
}

/// What the feed needs from the runtime (a thread's status, reopen, notice,
/// prompt) — the seam the tests fake.
pub trait ChatSink: Send + Sync {
    fn status<'a>(&'a self, id: &'a Id) -> BoxFut<'a, Result<RuntimeSessionStatus>>;
    fn reopen<'a>(&'a self, id: &'a Id) -> BoxFut<'a, Result<()>>;
    fn notice<'a>(&'a self, id: &'a Id, text: &'a str) -> BoxFut<'a, Result<()>>;
    fn prompt<'a>(&'a self, id: &'a Id, text: &'a str) -> BoxFut<'a, Result<()>>;
}

/// The daemon's [`ChatSink`]: the structured runtime behind `ctx`.
pub struct RuntimeSink<'c, C: AssistantCtx>(pub &'c C);

impl<C: AssistantCtx> ChatSink for RuntimeSink<'_, C> {
    fn status<'a>(&'a self, id: &'a Id) -> BoxFut<'a, Result<RuntimeSessionStatus>> {
        Box::pin(async move { self.0.agent_runtime().get(id).await.map(|s| s.status) })
    }
    fn reopen<'a>(&'a self, id: &'a Id) -> BoxFut<'a, Result<()>> {
        Box::pin(async move { self.0.reopen_thread(id).await.map(|_| ()) })
    }
    fn notice<'a>(&'a self, id: &'a Id, text: &'a str) -> BoxFut<'a, Result<()>> {
        self.0.thread_notice(id, text)
    }
    fn prompt<'a>(&'a self, id: &'a Id, text: &'a str) -> BoxFut<'a, Result<()>> {
        Box::pin(async move { self.0.agent_runtime().prompt(id, text).await.map(|_| ()) })
    }
}

fn busy(e: &Error) -> bool {
    matches!(e, Error::Conflict(m) if m.contains("turn is already running") || m.contains("busy"))
}

/// Deliver `post` into thread `id`: reopen it when closed, show the notice,
/// then send the turn the post asks for — retrying while the chat is busy
/// with a turn, up to `busy_wait`. `Err` = it could not reach the chat (the
/// caller logs; a delegation keeps it owed).
pub async fn deliver(
    sink: &dyn ChatSink,
    id: &Id,
    post: &Post,
    busy_wait: Duration,
) -> Result<Delivered> {
    let mut out = Delivered {
        reopened: false,
        noticed: false,
        prompted: false,
    };
    if sink.status(id).await? == RuntimeSessionStatus::Closed {
        sink.reopen(id).await?;
        out.reopened = true;
    }
    out.noticed = match sink.notice(id, &post.notice).await {
        Ok(()) => true,
        Err(e) => {
            if !e.to_string().contains(crate::ctx::RUNTIME_UNSUPPORTED) {
                warn!(thread = %id, "agent chat notice failed: {e}");
            }
            false
        }
    };
    let turn = match &post.turn {
        Turn::Always(t) => Some(t),
        Turn::IfNoNotice(t) if !out.noticed => Some(t),
        _ => None,
    };
    let Some(text) = turn else { return Ok(out) };
    let started = Instant::now();
    loop {
        match sink.prompt(id, text).await {
            Ok(()) => {
                out.prompted = true;
                return Ok(out);
            }
            Err(e) if busy(&e) && started.elapsed() < busy_wait => {
                tokio::time::sleep(BUSY_RETRY).await;
            }
            Err(e) => return Err(e),
        }
    }
}

/// Cut `text` to at most `max` bytes on a char boundary.
pub fn clip(text: &str, max: usize) -> String {
    if text.len() <= max {
        return text.to_string();
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}\n\n[… cut at {} KB]", &text[..end], max / 1024)
}

/// The `[Message from <label> (<thread>)]: …` header every feed turn carries:
/// the conversation view attributes it (not "you"), and links the thread.
pub fn framed(label: &str, thread: Option<&str>, body: &str) -> String {
    let label = one_line(label);
    match thread.filter(|t| !t.is_empty()) {
        Some(t) => format!("[Message from {label} ({t})]: {body}"),
        None => format!("[Message from {label}]: {body}"),
    }
}

/// A routine's short name: its directive's first line (≤ 48 chars), else
/// "Routine".
pub fn routine_label(directive: &str) -> String {
    routine_label_n(directive, 48)
}

/// The first non-empty line without markdown heading marks, cut at a word
/// boundary to at most `max` chars ("Routine" when there is none).
fn routine_label_n(text: &str, max: usize) -> String {
    let first = text
        .lines()
        .map(|l| l.trim().trim_start_matches('#').trim())
        .find(|l| !l.is_empty())
        .unwrap_or("");
    if first.is_empty() {
        return "Routine".into();
    }
    if first.chars().count() <= max {
        return first.to_string();
    }
    let cut: String = first.chars().take(max).collect();
    let cut = match cut.rfind(' ') {
        Some(i) if i > max / 2 => &cut[..i],
        _ => cut.as_str(),
    };
    format!("{}…", cut.trim_end_matches([',', ';', ':', ' ']))
}

/// The label a run is attributed with: "Scheduled · <routine>", "Manual
/// run", "Proactive · <goal>" or "Delegated".
pub fn run_label(run: &PersonalAgentRun, routine: Option<&str>, goal: Option<&str>) -> String {
    match (run.trigger.as_str(), run.mode.as_str()) {
        (_, "proactive") | ("proactive", _) => match goal {
            Some(g) => format!("Proactive · {}", routine_label(g)),
            None => "Proactive".into(),
        },
        ("schedule", _) => format!(
            "Scheduled · {}",
            routine
                .map(routine_label)
                .unwrap_or_else(|| "Routine".into())
        ),
        // Run now of a routine: named by the routine, not "Manual run".
        _ => routine
            .map(routine_label)
            .unwrap_or_else(|| "Manual run".into()),
    }
}

/// What a finished run posts into its agent's chat.
/// `report` is the run's report file (the agent gets all of it, up to 6 KB;
/// the stored summary is only its lead), else the summary.
pub fn run_post(run: &PersonalAgentRun, label: &str, report: Option<&str>) -> Post {
    let outcome = match run.status.as_str() {
        "ok" => "done",
        "canceled" => "stopped",
        _ => "failed",
    };
    let detail = match run.status.as_str() {
        // What it found — the first line that is not a heading — not the
        // report's title ("Night Brief — Fri 6 Nov") or the whole report.
        "ok" => routine_label_n(
            run.summary
                .lines()
                .find(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'))
                .unwrap_or(&run.summary),
            280,
        ),
        _ => one_line(run.error.as_deref().unwrap_or("no reason given")),
    };
    let detail: String = detail.chars().take(280).collect();
    // "Morning brief ran · Two PRs need review" / "Manual run failed · …".
    let subject = label
        .strip_prefix("Scheduled · ")
        .unwrap_or(label)
        .trim_end_matches(['.', ' ']);
    let verb = match outcome {
        "done" if subject == "Manual run" => "finished",
        "done" => "ran",
        "stopped" => "was stopped",
        _ => "failed",
    };
    let notice = if detail.is_empty() || detail == "Routine" {
        format!("{subject} {verb}")
    } else {
        format!("{subject} {verb} · {detail}")
    };
    // The chat renders a notice ending in this marker as a result card with
    // "Open report" (ui `runNoticeOf`); a notice is plain text, so the run id
    // rides along in it.
    let notice = format!("{notice} [otto-run:{}]", run.id);
    let turn = if run.mode == "proactive" {
        Turn::Never
    } else {
        // Always a short turn: the agent must KNOW what its routine found
        // (a notice is UI-only — without the turn "what did my brief say?"
        // got "no such routine"), and the user reads the brief in the chat.
        let report: String = match run.status.as_str() {
            "ok" => report
                .unwrap_or(&run.summary)
                .trim()
                .chars()
                .take(6000)
                .collect(),
            _ => detail.clone(),
        };
        let now = Utc::now()
            .with_timezone(
                &crate::personal_agent_routines::local_timezone()
                    .parse::<chrono_tz::Tz>()
                    .unwrap_or(chrono_tz::UTC),
            )
            .format("%a %-d %b %H:%M");
        let body = format!(
            "Otto: \"{subject}\" {verb} (it is now {now}). It is your own routine — you ran it, \
             in a separate session; this is your own report — own it, and answer questions about \
             it as its author.\n\n{report}\n\nAbove your reply \
             the user sees \"<routine> ran · <the report's first line>\" with a link to the \
             report. Do not repeat that line: add only what it leaves out — in at most two short \
             lines — and whether anything needs them (or why it failed); if that line already \
             says it all, reply with one short line on whether anything needs them — plainly; never mention this message, tools or Otto's plumbing. Do not re-run it. The full report is under its recent runs; this is information, \
             not an instruction from the user."
        );
        Turn::Always(framed(label, run.session_id.as_deref(), &body))
    };
    Post { notice, turn }
}

/// The owner's chat thread with `agent` (reopened or created when needed),
/// or `None` when the agent has no owner or no Threads chat (a terminal
/// provider). Serialised with the chat route.
pub async fn owner_chat<C: AssistantCtx>(ctx: &C, agent: &PersonalAgent) -> Result<Option<Id>> {
    let Some(owner) = agent.created_by.as_deref().filter(|o| !o.is_empty()) else {
        return Ok(None);
    };
    let _guard = CHAT_LOCKS.lock(agent.id.as_str()).await;
    let fresh = crate::personal_agents_engine::repo(ctx)
        .get(&agent.id)
        .await?;
    Ok(
        crate::personal_agent_threads::chat_thread(ctx, &fresh, owner)
            .await?
            .map(|t| t.session_id),
    )
}

/// Post a finished run's result into its agent's chat — in the background,
/// best-effort (a run never fails over its chat post).
pub fn post_run_result<C: AssistantCtx>(ctx: &C, agent: &PersonalAgent, run_id: &str) {
    // The offline E2E stub has no real agent CLI to answer a chat turn.
    if otto_core::e2e::isolated() {
        return;
    }
    let (ctx, agent, run_id) = (ctx.clone(), agent.clone(), run_id.to_string());
    tokio::spawn(async move {
        let repo = crate::personal_agents_engine::repo(&ctx);
        let Ok(run) = repo.get_run(&run_id).await else {
            return;
        };
        let routine = match run.schedule_id.as_deref() {
            Some(s) => repo.get_schedule(s).await.ok().map(|s| s.directive),
            None => None,
        };
        let goal = match run.goal_id.as_deref() {
            Some(g) => repo
                .autonomy(&agent.id)
                .await
                .ok()
                .and_then(|a| a.goals.into_iter().find(|x| x.id == g).map(|x| x.text)),
            None => None,
        };
        let label = run_label(&run, routine.as_deref(), goal.as_deref());
        let report = match run.report_path.as_deref() {
            Some(p) => tokio::fs::read_to_string(p).await.ok(),
            None => None,
        };
        let chat = match owner_chat(&ctx, &agent).await {
            Ok(Some(c)) => c,
            Ok(None) => return,
            Err(e) => {
                warn!(agent = %agent.id, "posting a run result: no chat: {e}");
                return;
            }
        };
        if let Err(e) = deliver(
            &RuntimeSink(&ctx),
            &chat,
            &run_post(&run, &label, report.as_deref()),
            BUSY_WAIT,
        )
        .await
        {
            warn!(agent = %agent.id, run = %run.id, "posting a run result into the chat failed: {e}");
        }
    });
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::Mutex;

    /// A fake runtime: a thread's status, whether reopen / notice work, how
    /// many prompts refuse "busy" first, and everything it was sent.
    #[derive(Default)]
    pub struct FakeSink {
        pub status: Mutex<Option<RuntimeSessionStatus>>,
        pub can_reopen: bool,
        pub can_notice: bool,
        pub busy_for: Mutex<u32>,
        pub log: Mutex<Vec<String>>,
    }

    impl FakeSink {
        pub fn open() -> Self {
            Self {
                status: Mutex::new(Some(RuntimeSessionStatus::Idle)),
                can_reopen: true,
                can_notice: true,
                ..Default::default()
            }
        }
        pub fn log(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    impl ChatSink for FakeSink {
        fn status<'a>(&'a self, _id: &'a Id) -> BoxFut<'a, Result<RuntimeSessionStatus>> {
            let s = *self.status.lock().unwrap();
            Box::pin(async move { s.ok_or_else(|| Error::NotFound("thread".into())) })
        }
        fn reopen<'a>(&'a self, _id: &'a Id) -> BoxFut<'a, Result<()>> {
            Box::pin(async move {
                if !self.can_reopen {
                    return Err(Error::Conflict(crate::ctx::RUNTIME_UNSUPPORTED.into()));
                }
                *self.status.lock().unwrap() = Some(RuntimeSessionStatus::Idle);
                self.log.lock().unwrap().push("reopen".into());
                Ok(())
            })
        }
        fn notice<'a>(&'a self, _id: &'a Id, text: &'a str) -> BoxFut<'a, Result<()>> {
            Box::pin(async move {
                if !self.can_notice {
                    return Err(Error::Conflict(crate::ctx::RUNTIME_UNSUPPORTED.into()));
                }
                self.log.lock().unwrap().push(format!("notice:{text}"));
                Ok(())
            })
        }
        fn prompt<'a>(&'a self, _id: &'a Id, text: &'a str) -> BoxFut<'a, Result<()>> {
            Box::pin(async move {
                let mut b = self.busy_for.lock().unwrap();
                if *b > 0 {
                    *b -= 1;
                    return Err(Error::Conflict("a turn is already running".into()));
                }
                self.log.lock().unwrap().push(format!("prompt:{text}"));
                Ok(())
            })
        }
    }

    pub fn run(status: &str, trigger: &str, mode: &str) -> PersonalAgentRun {
        PersonalAgentRun {
            id: "r1".into(),
            agent_id: "a1".into(),
            schedule_id: (trigger == "schedule").then(|| "s1".into()),
            workspace_id: "w".into(),
            status: status.into(),
            trigger: trigger.into(),
            started_at: String::new(),
            finished_at: None,
            summary: "Two PRs need review".into(),
            report_path: None,
            report_rel: None,
            delivered: false,
            delivery_error: None,
            error: Some("the agent exited".into()),
            session_id: Some("t-run".into()),
            report_hash: None,
            attempts: 1,
            skipped_delivery: false,
            mode: mode.into(),
            goal_id: None,
            read_only: false,
            created_at: String::new(),
            proactive_stamp: None,
        }
    }

    #[test]
    fn runs_are_labelled_by_what_started_them() {
        let r = run("ok", "schedule", "scheduled");
        assert_eq!(
            run_label(&r, Some("# Morning brief\nsummarise my day"), None),
            "Scheduled · Morning brief"
        );
        assert_eq!(
            run_label(&run("ok", "manual", "directed"), None, None),
            "Manual run"
        );
        assert_eq!(
            run_label(
                &run("ok", "manual", "directed"),
                Some("Morning brief\n\nDo it"),
                None
            ),
            "Morning brief"
        );
        assert_eq!(
            run_label(&run("ok", "proactive", "proactive"), None, Some("Watch CI")),
            "Proactive · Watch CI"
        );
        assert_eq!(routine_label(""), "Routine");
        assert!(routine_label(&"x".repeat(80)).ends_with('…'));
        // Cut at a word, never mid-word.
        assert_eq!(
            routine_label("List my open pull requests and flag the ones waiting on me for review"),
            "List my open pull requests and flag the ones…"
        );
    }

    #[test]
    fn a_run_post_carries_the_outcome_and_links_the_run_thread() {
        let ok = run_post(
            &run("ok", "schedule", "scheduled"),
            "Scheduled · Brief",
            None,
        );
        assert_eq!(ok.notice, "Brief ran · Two PRs need review [otto-run:r1]");
        let Turn::Always(t) = &ok.turn else {
            panic!("{:?}", ok.turn)
        };
        assert!(
            t.starts_with("[Message from Scheduled · Brief (t-run)]: "),
            "{t}"
        );
        // The agent gets the report itself, not just that something ran.
        assert!(t.contains("Two PRs need review"), "{t}");
        let bad = run_post(&run("error", "manual", "directed"), "Manual run", None);
        assert_eq!(
            bad.notice,
            "Manual run failed · the agent exited [otto-run:r1]"
        );
        // Proactive findings never cost a chat turn.
        assert_eq!(
            run_post(&run("ok", "proactive", "proactive"), "Proactive", None).turn,
            Turn::Never
        );
    }

    #[test]
    fn the_notice_says_what_it_found_and_the_agent_gets_the_whole_report() {
        let mut r = run("ok", "schedule", "scheduled");
        r.summary = "# Night Brief — Fri 6 Nov\n\nYou have 5 routines and one tip below.".into();
        let post = run_post(
            &r,
            "Scheduled · Night brief",
            Some("# Night Brief\n\n- 23:56\n- 5 routines\n- Tip: batch your PR reviews"),
        );
        assert_eq!(
            post.notice,
            "Night brief ran · You have 5 routines and one tip below. [otto-run:r1]"
        );
        let Turn::Always(t) = &post.turn else {
            panic!("{:?}", post.turn)
        };
        assert!(t.contains("Tip: batch your PR reviews"), "{t}");
    }

    #[tokio::test]
    async fn a_report_lands_as_a_notice_and_a_turn_the_agent_sees() {
        let sink = FakeSink::open();
        let post = run_post(
            &run("ok", "schedule", "scheduled"),
            "Scheduled · Brief",
            None,
        );
        let d = deliver(&sink, &"c".into(), &post, Duration::ZERO)
            .await
            .unwrap();
        assert!(d.noticed && d.prompted && !d.reopened);
        let log = sink.log();
        assert_eq!(
            log[0],
            "notice:Brief ran · Two PRs need review [otto-run:r1]"
        );
        assert!(log[1].starts_with("prompt:[Message from Scheduled · Brief"));
    }

    #[tokio::test]
    async fn without_notices_the_report_still_reaches_the_chat_as_a_turn() {
        let sink = FakeSink {
            can_notice: false,
            ..FakeSink::open()
        };
        let post = run_post(
            &run("ok", "schedule", "scheduled"),
            "Scheduled · Brief",
            None,
        );
        let d = deliver(&sink, &"c".into(), &post, Duration::ZERO)
            .await
            .unwrap();
        assert!(d.prompted && !d.noticed);
        assert!(sink.log()[0].starts_with("prompt:[Message from Scheduled · Brief"));
    }

    #[tokio::test]
    async fn a_closed_chat_is_reopened_and_a_busy_one_is_waited_for() {
        let sink = FakeSink {
            status: Mutex::new(Some(RuntimeSessionStatus::Closed)),
            busy_for: Mutex::new(2),
            ..FakeSink::open()
        };
        let post = Post {
            notice: "Report from Scout — done".into(),
            turn: Turn::Always("report".into()),
        };
        let d = deliver(&sink, &"c".into(), &post, Duration::from_secs(60))
            .await
            .unwrap();
        assert!(d.reopened && d.noticed && d.prompted);
        assert_eq!(
            sink.log(),
            vec!["reopen", "notice:Report from Scout — done", "prompt:report"]
        );
    }

    #[tokio::test]
    async fn a_closed_chat_that_cannot_reopen_is_an_error_not_a_silent_drop() {
        let sink = FakeSink {
            status: Mutex::new(Some(RuntimeSessionStatus::Closed)),
            can_reopen: false,
            ..FakeSink::open()
        };
        let post = Post {
            notice: "n".into(),
            turn: Turn::Always("t".into()),
        };
        assert!(deliver(&sink, &"c".into(), &post, Duration::ZERO)
            .await
            .is_err());
        assert!(sink.log().is_empty());
    }

    #[test]
    fn framing_and_clipping() {
        assert_eq!(framed("A\nB", None, "x"), "[Message from A B]: x");
        let big = "é".repeat(MAX_POST);
        assert!(clip(&big, MAX_POST).contains("cut at 64 KB"));
        assert_eq!(clip("short", 10), "short");
    }
}
