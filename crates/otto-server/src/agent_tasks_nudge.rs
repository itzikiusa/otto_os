//! Board → agent nudges (design §4.5). A task added from the Tasks panel or a
//! Mission Control card is inserted with `nudge_pending = 1`; THIS is the one
//! owner that delivers it — one prompt via `SessionManager::submit_text`
//! (paste + Enter, the reliable "actually send" path):
//!
//! ```text
//! Otto board: new task — "<title>". <description> Add it to your task list and do it next.
//! ```
//!
//! Gates, in order:
//! * only `kind == Agent` sessions on a bracketed-paste CLI (`claude`/`codex`) —
//!   a shell would EXECUTE the line as a command;
//! * the PTY must be ready: spawned ≥ [`UPTIME_FLOOR`] ago, has drawn
//!   something and gone quiet for [`TUI_SETTLE`] (the `wait_for_tui` rule from
//!   `otto-channels`), and no approval/permission prompt on screen (wait, no
//!   override — typing into one would eat the nudge or auto-answer it);
//! * timing: deliver when the session is `Idle`; while `Working`/`Running`
//!   defer, but never past **120 s** measured from when the session was first
//!   seen working after the task appeared — Otto's `Idle` only means "no PTY
//!   output for 5 s", so a spinner would otherwise defer forever (the CLIs
//!   queue typed input during a turn);
//! * an atomic claim (`UPDATE … WHERE nudge_pending = 1`) so two sweeps never
//!   deliver one task twice; a failed PTY write un-claims it.
//!
//! Driven by `Event::SessionStatus` plus a 15 s tick, and kicked directly by
//! `POST /sessions/{id}/tasks`. Independent of transcript tails.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use otto_core::domain::{Session, SessionKind, SessionStatus, TrailKind, TrailLevel, TrailSource};
use otto_core::event::Event;
use otto_core::Id;
use otto_state::{ActivityRepo, NewTrail, PendingNudge};

use crate::state::ServerCtx;

pub const TICK: Duration = Duration::from_secs(15);
/// Max time a nudge waits for `Idle` while the session is busy.
pub const MAX_DEFER: Duration = Duration::from_secs(120);
/// Never type into a PTY younger than this.
pub const UPTIME_FLOOR: Duration = Duration::from_secs(10);
/// The TUI has drawn and been quiet this long → safe to paste.
pub const TUI_SETTLE: Duration = Duration::from_millis(600);

/// The exact prompt the agent receives.
pub fn nudge_text(title: &str, description: Option<&str>) -> String {
    let desc = description
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .map(|d| format!(" {d}"))
        .unwrap_or_default();
    format!("Otto board: new task — \"{title}\".{desc} Add it to your task list and do it next.")
}

/// Board text → one safe line: newlines/tabs become spaces, every other
/// control character (ESC, CR, NUL …) is dropped, whitespace collapsed.
pub fn sanitize_prompt_text(s: &str) -> String {
    let mapped: String = s
        .chars()
        .filter_map(|c| match c {
            '\n' | '\t' | '\r' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Can this session receive a nudge at all? Only an AGENT session whose PTY is
/// running claude/codex has a chat input the line is safe to paste into — its
/// own provider, or the one captured running inside a plain terminal
/// (`meta.nested_provider`, the same resolution the resume feature and the
/// transcript route use). A bare shell stays refused: it would EXECUTE the line.
pub fn nudgeable(session: &Session) -> bool {
    session.kind == SessionKind::Agent
        && matches!(
            crate::routes::transcript::effective_provider(session).as_str(),
            "claude" | "codex"
        )
}

/// For a captured shell (`meta.nested_provider`): is the nested CLI still a
/// live descendant of the PTY's root process (`pid`)? Pure — the caller passes
/// the `ps` snapshot — so both branches are unit-testable. A bare agent
/// session (its own provider is claude/codex) is always "running".
pub fn nested_agent_alive(session: &Session, pid: Option<u32>, table: &[otto_sessions::nested::ProcInfo]) -> bool {
    if matches!(session.provider.as_str(), "claude" | "codex") {
        return true;
    }
    let Some(expected) = session.meta.get("nested_provider").and_then(|v| v.as_str()) else {
        return false;
    };
    let Some(pid) = pid else { return false };
    otto_sessions::nested::find_nested_agent(pid, table).is_some_and(|(_, found)| found == expected)
}

/// Async wrapper: snapshot the process table (a `ps` call, off the runtime
/// threads) and check [`nested_agent_alive`] for `session`'s live PTY.
pub async fn agent_running(ctx: &ServerCtx, session: &Session) -> bool {
    if matches!(session.provider.as_str(), "claude" | "codex") {
        return ctx.manager.is_live(&session.id);
    }
    let Some(pid) = ctx.manager.live_handle(&session.id).and_then(|h| h.pid()) else {
        return false;
    };
    let table = tokio::task::spawn_blocking(otto_sessions::nested::process_table)
        .await
        .unwrap_or_default();
    nested_agent_alive(session, Some(pid), &table)
}

/// When each session was first seen `Working` (since the daemon started); the
/// 120 s defer is measured from here, never from a task's `created_at` alone,
/// so a nudge cannot fire into a PTY that has been alive for milliseconds.
fn first_working() -> &'static Mutex<HashMap<Id, Instant>> {
    static M: OnceLock<Mutex<HashMap<Id, Instant>>> = OnceLock::new();
    M.get_or_init(|| Mutex::new(HashMap::new()))
}

fn note_working(sid: &Id) -> Instant {
    let mut m = first_working().lock().unwrap_or_else(|p| p.into_inner());
    *m.entry(sid.clone()).or_insert_with(Instant::now)
}

fn forget(sid: &Id) {
    first_working().lock().unwrap_or_else(|p| p.into_inner()).remove(sid);
}

/// Start the sweep loop. Returns the task handle (kept alive by the daemon).
pub fn spawn(ctx: ServerCtx) -> tokio::task::JoinHandle<()> {
    let mut rx = ctx.events.subscribe();
    tokio::spawn(async move {
        let mut tick = tokio::time::interval(TICK);
        tick.tick().await; // consume the immediate first tick
        loop {
            tokio::select! {
                ev = rx.recv() => match ev {
                    Ok(Event::SessionStatus { session_id, status, .. }) => match status {
                        SessionStatus::Working => {
                            note_working(&session_id);
                            sweep_session(&ctx, &session_id).await;
                        }
                        SessionStatus::Idle | SessionStatus::Running => sweep_session(&ctx, &session_id).await,
                        SessionStatus::Exited | SessionStatus::Reconnectable => forget(&session_id),
                    },
                    Ok(Event::SessionRemoved { session_id, .. }) => forget(&session_id),
                    Ok(_) => {}
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                },
                _ = tick.tick() => sweep_all(&ctx).await,
            }
        }
    })
}

/// Deliver every due nudge across all sessions.
pub async fn sweep_all(ctx: &ServerCtx) {
    let repo = ActivityRepo::new(ctx.pool.clone());
    let pending = match repo.pending_nudges(None).await {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("nudge sweep: query failed: {e}");
            return;
        }
    };
    let mut sessions: Vec<Id> = pending.iter().map(|p| p.session_id.clone()).collect();
    sessions.sort();
    sessions.dedup();
    for sid in sessions {
        sweep_session(ctx, &sid).await;
    }
}

/// Deliver the due nudges of one session (if any).
pub async fn sweep_session(ctx: &ServerCtx, session_id: &Id) {
    let repo = ActivityRepo::new(ctx.pool.clone());
    let pending = match repo.pending_nudges(Some(session_id)).await {
        Ok(p) if !p.is_empty() => p,
        Ok(_) => return,
        Err(e) => {
            tracing::warn!(session = %session_id, "nudge sweep: query failed: {e}");
            return;
        }
    };
    let Ok(session) = ctx.manager.get(session_id).await else { return };
    if !nudgeable(&session) {
        tracing::debug!(session = %session_id, provider = %session.provider, "nudge sweep: session cannot take a nudge");
        return;
    }
    let Some(handle) = ctx.manager.live_handle(session_id) else {
        return; // delivered when it comes back (status event) or on a later tick
    };
    // A captured shell whose claude/codex has exited would EXECUTE the paste:
    // keep the task pending (no nudge, no `nudged_at`) until the CLI is back.
    if !agent_running(ctx, &session).await {
        tracing::debug!(session = %session_id, "nudge sweep: nested agent not running, keeping task pending");
        return;
    }
    if !tui_ready(&handle) {
        tracing::debug!(session = %session_id, "nudge sweep: PTY not ready yet, waiting");
        return;
    }
    if approval_pending(&handle, &session.provider) {
        tracing::debug!(session = %session_id, "nudge sweep: approval prompt on screen, waiting");
        return;
    }
    let due: Vec<&PendingNudge> = match session.status {
        SessionStatus::Idle => pending.iter().collect(),
        SessionStatus::Working | SessionStatus::Running => {
            let working_since = note_working(session_id);
            let now = chrono::Utc::now();
            pending
                .iter()
                .filter(|p| {
                    // Both clocks must have run 120 s: since the task appeared
                    // AND since the session was first seen working.
                    let since_task = now.signed_duration_since(p.created_at).to_std().unwrap_or_default();
                    since_task >= MAX_DEFER && working_since.elapsed() >= MAX_DEFER
                })
                .collect()
        }
        _ => Vec::new(),
    };
    let mut delivered = false;
    for p in due {
        // Atomic claim: exactly one sweep delivers a task.
        match repo.claim_nudge(&p.task_id).await {
            Ok(true) => {}
            Ok(false) => continue,
            Err(e) => {
                tracing::warn!(session = %session_id, "nudge sweep: claim failed: {e}");
                return;
            }
        }
        let text = nudge_text(&sanitize_prompt_text(&p.title), p.description.as_deref().map(sanitize_prompt_text).as_deref());
        if let Err(e) = ctx.manager.submit_text(session_id, &text).await {
            tracing::warn!(session = %session_id, "nudge sweep: submit failed: {e}");
            let _ = repo.unclaim_nudge(&p.task_id).await;
            return; // retry on the next trigger
        }
        delivered = true;
        let _ = ctx
            .activity()
            .append_trail(NewTrail {
                session_id: session_id.clone(),
                workspace_id: session.workspace_id.clone(),
                source: TrailSource::Otto,
                kind: TrailKind::Task,
                level: TrailLevel::Info,
                summary: format!("Board task sent to agent: {}", otto_transcript::util::clip(&p.title, 120)),
                detail: None,
            })
            .await;
        // Let the CLI consume the paste before the next one.
        tokio::time::sleep(Duration::from_millis(400)).await;
    }
    if delivered {
        if let Ok(tasks) = repo.list_tasks(session_id).await {
            let _ = ctx.events.send(Event::TasksUpdated {
                workspace_id: session.workspace_id.clone(),
                session_id: session_id.clone(),
                tasks,
            });
        }
    }
}

/// `wait_for_tui`'s rule (otto-channels bridge) as a one-shot check, plus an
/// uptime floor: the PTY has lived ≥ 10 s, drawn something, and been quiet
/// for 600 ms.
fn tui_ready(handle: &otto_pty::PtyHandle) -> bool {
    handle.on_exit().borrow().is_none()
        && handle.uptime() >= UPTIME_FLOOR
        && !handle.scrollback(1).is_empty()
        && handle.last_output_at().elapsed() >= TUI_SETTLE
}

/// True when the session's current screen shows a trust/approval prompt.
fn approval_pending(handle: &otto_pty::PtyHandle, provider: &str) -> bool {
    let screen = strip_ansi(&handle.screen_snapshot()).to_lowercase();
    otto_sessions::prompt_guard::detect_approval(provider, &screen).is_some() || looks_like_permission_prompt(&screen)
}

/// Permission prompts the prompt-guard deliberately does NOT auto-answer.
pub fn looks_like_permission_prompt(screen_lower: &str) -> bool {
    const NEEDLES: &[&str] = &[
        "do you want to proceed",
        "do you want to allow",
        "allow this command",
        "yes, allow",
        "yes, and don't ask again",
        "approve this command",
        "allow codex to",
        "(y/n)",
        "[y/n]",
        "esc to cancel",
    ];
    NEEDLES.iter().any(|n| screen_lower.contains(n))
}

/// Drop ESC/CSI/OSC sequences from a screen snapshot, keeping printable text.
pub fn strip_ansi(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == 0x1b {
            i += 1;
            match bytes.get(i) {
                Some(b'[') => {
                    i += 1;
                    while i < bytes.len() && !(0x40..=0x7e).contains(&bytes[i]) {
                        i += 1;
                    }
                    i += 1;
                }
                Some(b']') => {
                    // OSC … BEL or ESC \
                    i += 1;
                    while i < bytes.len() && bytes[i] != 0x07 && !(bytes[i] == 0x1b && bytes.get(i + 1) == Some(&b'\\')) {
                        i += 1;
                    }
                    i += if bytes.get(i) == Some(&0x1b) { 2 } else { 1 };
                }
                Some(_) => i += 1,
                None => {}
            }
            continue;
        }
        if b == b'\n' || b == b'\r' || b == b'\t' {
            out.push(' ');
        } else if b >= 0x20 {
            // Decode the UTF-8 sequence starting here; skip a broken byte.
            let len = match b {
                0x00..=0x7f => 1,
                0xc0..=0xdf => 2,
                0xe0..=0xef => 3,
                _ => 4,
            };
            if let Ok(s) = std::str::from_utf8(&bytes[i..(i + len).min(bytes.len())]) {
                out.push_str(s);
            }
            i += len;
            continue;
        }
        i += 1;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nudge_text_is_the_contract_line() {
        assert_eq!(
            nudge_text("Fix login", Some("see ticket 7")),
            "Otto board: new task — \"Fix login\". see ticket 7 Add it to your task list and do it next."
        );
        assert_eq!(
            nudge_text("Fix login", None),
            "Otto board: new task — \"Fix login\". Add it to your task list and do it next."
        );
        assert_eq!(nudge_text("x", Some("   ")), nudge_text("x", None));
    }

    #[test]
    fn sanitize_strips_control_chars_and_paste_terminators() {
        // ESC is dropped, so the remaining `[201~` can no longer end a paste.
        assert_eq!(sanitize_prompt_text("a\x1b[201~b\nc\td\r\n  e"), "a[201~b c d e");
        assert_eq!(sanitize_prompt_text("\u{0}\u{7}plain"), "plain");
    }

    #[test]
    fn only_claude_codex_agents_are_nudgeable() {
        let mut s = Session {
            id: "s".into(),
            workspace_id: "w".into(),
            kind: SessionKind::Agent,
            provider: "claude".into(),
            title: "t".into(),
            status: SessionStatus::Idle,
            cwd: "/x".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: "u".into(),
            created_at: chrono::Utc::now(),
            last_active_at: chrono::Utc::now(),
            archived: false,
            meta: serde_json::Value::Null,
        };
        assert!(nudgeable(&s));
        s.provider = "shell".into();
        assert!(!nudgeable(&s), "a bare shell would run the line as a command");
        // A terminal in which the user launched claude (captured nested provider)
        // is running a chat TUI → nudgeable, like the transcript/resume paths.
        s.meta = serde_json::json!({ "nested_provider": "claude", "nested_cwd": "/x" });
        assert!(nudgeable(&s), "captured-shell case");
        s.meta = serde_json::json!({ "nested_provider": "python" });
        assert!(!nudgeable(&s));
        s.meta = serde_json::Value::Null;
        s.provider = "codex".into();
        s.kind = SessionKind::Connection;
        assert!(!nudgeable(&s));
    }

    #[test]
    fn captured_shell_nudge_requires_the_nested_cli_to_be_alive() {
        use otto_sessions::nested::ProcInfo;
        let now = std::time::SystemTime::now();
        let proc = |pid, ppid, args: &str| ProcInfo {
            pid,
            ppid,
            started: now,
            args: args.into(),
        };
        let mut s = Session {
            id: "s".into(),
            workspace_id: "w".into(),
            kind: SessionKind::Agent,
            provider: "shell".into(),
            title: "t".into(),
            status: SessionStatus::Idle,
            cwd: "/x".into(),
            provider_session_id: None,
            connection_id: None,
            created_by: "u".into(),
            created_at: chrono::Utc::now(),
            last_active_at: chrono::Utc::now(),
            archived: false,
            meta: serde_json::json!({ "nested_provider": "claude", "nested_pid": 300 }),
        };
        // Alive: PTY root 100 → zsh 200 → claude 300.
        let alive = [
            proc(100, 1, "/bin/zsh -l"),
            proc(200, 100, "zsh"),
            proc(300, 200, "node /usr/local/lib/node_modules/@anthropic-ai/claude-code/cli.js"),
        ];
        assert!(nested_agent_alive(&s, Some(100), &alive));
        // Gone: only the shell remains → refused (the paste would run as a command).
        let gone = [proc(100, 1, "/bin/zsh -l"), proc(200, 100, "zsh")];
        assert!(!nested_agent_alive(&s, Some(100), &gone));
        // A different agent than the captured one does not count either.
        let other = [proc(100, 1, "zsh"), proc(300, 100, "codex")];
        assert!(!nested_agent_alive(&s, Some(100), &other));
        assert!(!nested_agent_alive(&s, None, &alive), "no live PTY");
        // A first-class claude session never needs the scan.
        s.provider = "claude".into();
        assert!(nested_agent_alive(&s, None, &gone));
    }

    #[test]
    fn ansi_is_stripped_and_prompts_detected() {
        let screen = b"\x1b[2J\x1b[H\x1b[1mDo you want to proceed?\x1b[0m\r\n> Yes\r\n  No, esc to cancel";
        let plain = strip_ansi(screen).to_lowercase();
        assert!(plain.contains("do you want to proceed?"));
        assert!(looks_like_permission_prompt(&plain));
        assert!(!looks_like_permission_prompt("compiling otto-server v0.1.0"));
        assert_eq!(MAX_DEFER, Duration::from_secs(120));
    }
}
