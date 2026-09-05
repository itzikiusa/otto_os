//! Live transcript tails (design §4.4). Per live agent session whose transcript
//! resolves, a task polls the JSONL every 700 ms (the orchestrator polls the
//! same files at 250 ms), folds the NEW records incrementally
//! (`otto_transcript::Folder`) and broadcasts `transcript_appended`
//! (+ `artifact_added` for new artifacts). A refold from record 0 happens only
//! when the tailer reports a replaced file or the folder says a per-file Codex
//! decision flipped.
//!
//! Lifecycle: a tail starts on the first `GET …/transcript` for a live session
//! and every such GET — plus the open view's `POST …/transcript/touch` ping
//! (every 60 s) — is a subscriber "touch"; it stops 60 s after the session
//! exits or 2 min after the last touch, so only sessions whose chat is
//! actually open are tailed (never archived / idle-for-days ones, never the
//! N parallel review sessions nobody is looking at). Cap 32 concurrent tails —
//! beyond that reads still work, there is just no live push. Each poll also
//! reads the PTY screen (one grid walk) for the `transcript_live` draft. The registry slot is an RAII
//! [`Slot`] held by the task: a panic anywhere in the loop frees it, and the
//! stop decision + removal happen under ONE lock so a `touch` racing the exit
//! either refreshes a live entry or (after removal) starts a fresh tail — never
//! a lost wake-up.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use otto_core::domain::Session;
use otto_core::event::Event;
use otto_core::Id;
use otto_transcript::{read_records, Folder, Provider, Tailer};

use crate::state::ServerCtx;

pub const POLL: Duration = Duration::from_millis(700);
pub const MAX_TAILS: usize = 32;
/// Keep tailing this long after the session exits (late flushes land).
pub const EXIT_GRACE: Duration = Duration::from_secs(60);
/// Stop when nobody touched the transcript for this long. An open chat
/// pings `POST …/transcript/touch` every 60 s, so two minutes means "the
/// view has been closed" — tails never outlive the view that armed them.
pub const IDLE_STOP: Duration = Duration::from_secs(2 * 60);
/// `transcript_appended` payloads above this are sent with `turns: []`.
pub const EVENT_CAP: usize = 64 * 1024;
/// `transcript_live` drafts are capped to this many bytes (tail kept).
pub const LIVE_CAP: usize = 16 * 1024;

struct Entry {
    last_touch: Instant,
    stop: Arc<AtomicBool>,
}

fn registry() -> &'static Mutex<HashMap<Id, Entry>> {
    static R: OnceLock<Mutex<HashMap<Id, Entry>>> = OnceLock::new();
    R.get_or_init(|| Mutex::new(HashMap::new()))
}

fn lock() -> std::sync::MutexGuard<'static, HashMap<Id, Entry>> {
    registry().lock().unwrap_or_else(|p| p.into_inner())
}

/// RAII ownership of one registry slot; dropping it (normal exit, early
/// return, or an unwinding panic) frees the slot.
struct Slot {
    id: Id,
}

impl Drop for Slot {
    fn drop(&mut self) {
        lock().remove(&self.id);
    }
}

/// Number of live tails (diagnostics / tests).
pub fn active() -> usize {
    lock().len()
}

/// A subscriber fetched `session`'s transcript: refresh its tail or start one.
pub fn touch(ctx: &ServerCtx, session: &Session, provider: Provider, path: &Path) {
    let mut reg = lock();
    if let Some(e) = reg.get_mut(&session.id) {
        e.last_touch = Instant::now();
        return;
    }
    if reg.len() >= MAX_TAILS {
        tracing::debug!(session = %session.id, "transcript tail: cap reached, no live push");
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    reg.insert(
        session.id.clone(),
        Entry {
            last_touch: Instant::now(),
            stop,
        },
    );
    drop(reg);
    let slot = Slot {
        id: session.id.clone(),
    };
    let ctx = ctx.clone();
    let session = session.clone();
    let path = path.to_path_buf();
    tokio::spawn(async move {
        let _slot = slot; // freed on every exit path, panics included
        run(ctx, session, provider, path).await;
    });
}

/// Stop a session's tail now (session removed/archived).
pub fn stop(session_id: &Id) {
    if let Some(e) = lock().remove(session_id) {
        e.stop.store(true, Ordering::Relaxed);
    }
}

/// One lock: decide whether to keep going, removing the entry when not.
fn should_continue(sid: &Id) -> bool {
    let mut reg = lock();
    let keep = match reg.get(sid) {
        None => false,
        Some(e) => !e.stop.load(Ordering::Relaxed) && e.last_touch.elapsed() < IDLE_STOP,
    };
    if !keep {
        reg.remove(sid);
    }
    keep
}

/// Refold the whole file from record 0 (initial start, replaced file, Codex
/// era flip). Blocking IO — run via `spawn_blocking`.
fn refold(ctx: &ServerCtx, provider: Provider, path: &Path, opts_subagents: bool) -> std::io::Result<Folder<'static>> {
    let records = read_records(path)?;
    let mut folder = Folder::new(provider, crate::routes::transcript::fold_opts(ctx, provider, path));
    if opts_subagents {
        folder.set_subagents(otto_transcript::read_subagents(path));
    }
    folder.seed(&records);
    Ok(folder)
}

async fn run(ctx: ServerCtx, session: Session, provider: Provider, path: PathBuf) {
    let sid = session.id.clone();
    let wid = session.workspace_id.clone();
    let claude = provider == Provider::Claude;
    let (cx, p) = (ctx.clone(), path.clone());
    let mut folder = match tokio::task::spawn_blocking(move || refold(&cx, provider, &p, claude)).await {
        Ok(Ok(f)) => f,
        Ok(Err(e)) => {
            tracing::debug!(session = %sid, "transcript tail: initial fold failed: {e}");
            return;
        }
        Err(_) => return,
    };
    let mut folded = folder.snapshot();
    let mut known_artifacts: HashSet<String> = folded.artifacts.iter().map(|a| a.id.clone()).collect();
    let mut tailer = Tailer::at(&path, Tailer::current_len(&path));
    let mut exited_since: Option<Instant> = None;
    let mut last_draft = String::new();
    loop {
        tokio::time::sleep(POLL).await;
        if !should_continue(&sid) {
            tracing::debug!(session = %sid, "transcript tail: stopped (no subscriber / stop requested)");
            break;
        }
        if ctx.manager.is_live(&sid) {
            exited_since = None;
        } else if exited_since.get_or_insert_with(Instant::now).elapsed() >= EXIT_GRACE {
            tracing::debug!(session = %sid, "transcript tail: session exited, stopping");
            break;
        }
        // Sub-turn streaming: the provider only writes a transcript record when
        // a block completes, so the in-progress text is read off the terminal
        // screen (plain rows) and pushed whenever it changes — one frame per
        // poll at most. Clients hide it once the folded turn lands.
        if let Some(h) = ctx.manager.live_handle(&sid) {
            let text = live_draft(&h.screen_rows());
            if text != last_draft {
                last_draft = text.clone();
                let _ = ctx.events.send(Event::TranscriptLive {
                    workspace_id: wid.clone(),
                    session_id: sid.clone(),
                    text,
                });
            }
        }
        let delta = match tailer.poll() {
            Ok(d) => d,
            Err(e) => {
                tracing::debug!(session = %sid, "transcript tail: poll failed: {e}");
                continue;
            }
        };
        if delta.records.is_empty() && !delta.restarted {
            continue;
        }
        let prev_count = if delta.restarted { 0 } else { folder.record_count() };
        let mut needs_refold = delta.restarted;
        if !needs_refold {
            for r in &delta.records {
                if folder.push(r) {
                    needs_refold = true;
                    break;
                }
            }
        }
        if needs_refold {
            let (cx, p) = (ctx.clone(), path.clone());
            folder = match tokio::task::spawn_blocking(move || refold(&cx, provider, &p, claude)).await {
                Ok(Ok(f)) => f,
                Ok(Err(e)) => {
                    tracing::debug!(session = %sid, "transcript tail: refold failed: {e}");
                    continue;
                }
                Err(_) => continue,
            };
            // The tailer is now ahead of / behind the refolded file; realign.
            tailer = Tailer::at(&path, Tailer::current_len(&path));
        } else if claude {
            // Sidecars for freshly spawned subagents appear between polls.
            folder.set_subagents(otto_transcript::read_subagents(&path));
        }
        folded = folder.snapshot();
        let turns: Vec<serde_json::Value> = folded
            .turns_since(prev_count)
            .iter()
            .filter_map(|t| serde_json::to_value(t).ok())
            .collect();
        let cursor = folded.record_count.saturating_sub(1).to_string();
        // Size the frame ONCE; over the cap the client re-fetches.
        let size = serde_json::to_vec(&turns).map(|v| v.len()).unwrap_or(usize::MAX);
        let _ = ctx.events.send(Event::TranscriptAppended {
            workspace_id: wid.clone(),
            session_id: sid.clone(),
            cursor,
            turns: if size > EVENT_CAP { Vec::new() } else { turns },
        });
        for a in &folded.artifacts {
            if known_artifacts.insert(a.id.clone()) {
                let _ = ctx.events.send(Event::ArtifactAdded {
                    workspace_id: wid.clone(),
                    session_id: sid.clone(),
                    artifact: serde_json::to_value(a).unwrap_or(serde_json::Value::Null),
                });
                crate::routes::transcript::register_work_artifact(&ctx, &session, a).await;
            }
        }
    }
}

/// The in-progress response as drawn on the agent's screen: everything
/// between the last prompt echo (`> …` / `› …`) and the input box (`❯ …` /
/// `│ > …`, with the rule above it), minus spinner rows. Returns "" when the
/// screen holds no such region. Tolerant by design — a TUI redesign degrades
/// to "the whole screen above the input box", never to garbage.
pub fn live_draft(rows: &[String]) -> String {
    fn is_rule(r: &str) -> bool {
        let t = r.trim();
        t.len() >= 8 && t.chars().all(|c| matches!(c, '─' | '━' | '╌' | '-' | '═'))
    }
    fn is_input(r: &str) -> bool {
        let t = r.trim_start();
        t.starts_with('❯') || t.starts_with("│ >") || t.starts_with("│ ❯") || t.starts_with("╭─") || t.starts_with("╰─")
    }
    fn is_echo(r: &str) -> bool {
        let t = r.trim_start();
        (t.starts_with("> ") || t.starts_with("› ")) && !t.starts_with("> >")
    }
    fn is_spinner(r: &str) -> bool {
        let t = r.trim_start();
        t.contains("esc to interrupt")
            || t.contains("(esc to")
            || t.chars().next().is_some_and(|c| "✻✶✳✢✽⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏".contains(c)) && t.contains('…')
    }
    // Input box: the LAST input row; content is everything above it (and
    // above the rule that frames it).
    let mut end = rows.len();
    if let Some(i) = rows.iter().rposition(|r| is_input(r)) {
        end = i;
        while end > 0 && (is_rule(&rows[end - 1]) || rows[end - 1].trim().is_empty()) {
            end -= 1;
        }
    }
    let content = &rows[..end];
    let start = content.iter().rposition(|r| is_echo(r)).map(|i| i + 1).unwrap_or(0);
    let mut out: Vec<&str> = Vec::new();
    let mut blank_run = 0usize;
    for r in &content[start..] {
        if is_spinner(r) || is_rule(r) {
            continue;
        }
        if r.trim().is_empty() {
            blank_run += 1;
            if blank_run > 1 || out.is_empty() {
                continue;
            }
        } else {
            blank_run = 0;
        }
        out.push(r.as_str());
    }
    while out.last().is_some_and(|r| r.trim().is_empty()) {
        out.pop();
    }
    let mut text = out.join("\n");
    if text.len() > LIVE_CAP {
        let cut = text.len() - LIVE_CAP;
        let at = text
            .char_indices()
            .map(|(i, _)| i)
            .find(|&i| i >= cut)
            .unwrap_or(text.len());
        text = format!("…{}", &text[at..]);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(s: &str) -> Vec<String> {
        s.lines().map(str::to_string).collect()
    }

    #[test]
    fn live_draft_takes_the_region_between_echo_and_input_box() {
        let screen = rows(
            "⏺ earlier answer\n\n> option 2, other services still read GSS_games\n\n⏺ Still exploring. Reading the DAO.\n\n⏺ Bash(cd x && grep -n foo)\n  ⎿  3 lines\n\n✻ Cooking… (esc to interrupt)\n\n────────────────────────────\n❯ \n────────────────────────────\n  -- INSERT --",
        );
        let d = live_draft(&screen);
        assert_eq!(d, "⏺ Still exploring. Reading the DAO.\n\n⏺ Bash(cd x && grep -n foo)\n  ⎿  3 lines");
    }

    #[test]
    fn live_draft_is_empty_right_after_a_prompt_and_tolerates_no_box() {
        assert_eq!(live_draft(&rows("> hi\n\n❯ ")), "");
        assert_eq!(live_draft(&rows("plain output\nmore")), "plain output\nmore");
        assert_eq!(live_draft(&[]), "");
    }

    #[test]
    fn live_draft_caps_to_the_tail() {
        let big: Vec<String> = (0..2000).map(|i| format!("line {i} {}", "x".repeat(20))).collect();
        let d = live_draft(&big);
        assert!(d.len() <= LIVE_CAP + 4);
        assert!(d.starts_with('…'));
        assert!(d.ends_with("line 1999 xxxxxxxxxxxxxxxxxxxx"));
    }

    #[test]
    fn slot_guard_frees_the_registry_entry_on_drop() {
        let id: Id = "tail-test-slot".into();
        lock().insert(
            id.clone(),
            Entry {
                last_touch: Instant::now(),
                stop: Arc::new(AtomicBool::new(false)),
            },
        );
        assert!(should_continue(&id));
        {
            let _slot = Slot { id: id.clone() };
        }
        assert!(lock().get(&id).is_none());
        // A missing entry (removed by `stop`) ends the loop under the same lock.
        assert!(!should_continue(&id));
        assert_eq!(EVENT_CAP, 65536);
        assert_eq!(POLL, Duration::from_millis(700));
        assert_eq!(MAX_TAILS, 32);
    }
}
