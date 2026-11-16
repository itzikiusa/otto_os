//! Working / idle for a live PTY session — the decision the per-session status
//! task ([`super::SessionManager::start_status_task`]) makes every
//! [`super::STATUS_TICK`]. Split out of `manager.rs` (LOC ratchet) and kept
//! pure where it decides, so every rule below has a unit test.
//!
//! "Any byte in the last 5 s" was the old rule, and it lied: an idle TUI that
//! repaints (a spinner, an elapsed-time counter, Claude's footer while a
//! background task runs) read as `Working` forever. Now:
//!
//! 1. **Turn signals first** (claude / codex only): the hooks Otto already
//!    receives (claude `UserPromptSubmit` opens a turn, `Stop` closes it; codex
//!    `agent-turn-complete` closes it) and the provider transcript's own turn
//!    fold ([`otto_transcript::turn::tail_open`]), whichever is newer. A closed
//!    turn is `Idle` however the TUI repaints; an open one is `Working` through
//!    a quiet tool call — unless the screen shows an approval prompt (the agent
//!    is waiting on a person, and the monitor must be able to say so).
//! 2. **Otherwise the screen content**: the visible text, with spinner glyphs
//!    and clock / elapsed-time digits masked ([`crate::screen::mask_volatile`]),
//!    must have changed — a repaint changes nothing.
//!
//! Consumers that stop processes (idle sweep, live-session cap, the CLI
//! auto-update's restart deferral) keep their own PTY-output clocks, so an
//! `Idle` here never makes them stop a process that is still printing.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime};

use dashmap::DashMap;
use otto_core::domain::{Session, SessionStatus};
use otto_core::Id;
use otto_pty::PtyHandle;
use otto_state::SessionsRepo;

use super::{read_tail, SessionManager, REAP_UNRESUMABLE_GRACE, WORKING_WINDOW};
use crate::screen::{awaits_approval, busy_prints, row_prints};

/// An OPEN turn keeps a session `Working` only while its TUI printed
/// something this recently. Every agent CLI animates while a turn runs (that
/// is how a long tool call looks), so a turn record that says "open" over a
/// silent screen is stale — an interrupt (`Esc` fires no `Stop` hook), a
/// crash — and the screen rule decides instead.
const OPEN_TURN_SILENCE: Duration = Duration::from_secs(120);

/// After real output stops, a session whose TUI keeps repainting idle
/// decoration (bytes, a clock, but no content change) stays `Working` this
/// long before it reads `Idle`.
const REPAINT_LINGER: Duration = Duration::from_secs(30);

/// A BUSY animation (a spinner frame, an elapsed-time counter ticking — see
/// [`crate::screen::busy_prints`]) keeps a `Working` row working while it
/// ticks, up to this long after the last real content change: an agent
/// without a turn record thinking behind a spinner is busy, and must not read
/// `Idle` (a false needs-you, a nudge typed into it). It never STARTS
/// `Working`, and stops on an approval prompt or when the bytes stop.
const BUSY_ANIMATION_CAP: Duration = Duration::from_secs(30 * 60);

/// How much of the transcript the status probe folds. Smaller than the idle
/// sweep's window: it runs every tick the file moved. A tail that holds no
/// parseable message (one huge tool result) is "unknown", never "idle".
const STATUS_TAIL_BYTES: u64 = 64 * 1024;

/// How often a session whose transcript is not located yet (codex mints its
/// id a few seconds after spawn) retries the lookup.
const LOCATE_RETRY: Duration = Duration::from_secs(60);

/// A turn verdict and the moment it was true.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct TurnMark {
    pub open: bool,
    pub at: Instant,
}

/// The per-session turn marks the agent hooks leave (see
/// [`SessionManager::note_claude_hook`]); read by the status task.
pub(super) type TurnMarks = Arc<DashMap<Id, TurnMark>>;

impl SessionManager {
    /// A Claude Code hook for `id` arrived (`/ingest/claude`):
    /// `UserPromptSubmit` opens a turn, `Stop` closes it. Other events say
    /// nothing about the turn boundary (`PostToolUse` can land after `Stop`
    /// on the wire, `Notification` is mid-turn) and are ignored.
    pub fn note_claude_hook(&self, id: &Id, event: &str) {
        if let Some(open) = claude_hook_turn(event) {
            let at = Instant::now();
            self.turn_marks.insert(id.clone(), TurnMark { open, at });
        }
    }

    /// A Codex `notify` payload for `id` arrived (`/ingest/codex`):
    /// `agent-turn-complete` closes the turn. Codex has no "turn started"
    /// notify, so the next input or the rollout re-opens it.
    pub fn note_codex_notify(&self, id: &Id, kind: &str) {
        if kind == "agent-turn-complete" {
            let at = Instant::now();
            let mark = TurnMark { open: false, at };
            self.turn_marks.insert(id.clone(), mark);
        }
    }
}

/// What a Claude hook event says about the turn (see
/// [`SessionManager::note_claude_hook`]).
fn claude_hook_turn(event: &str) -> Option<bool> {
    match event {
        "UserPromptSubmit" => Some(true),
        "Stop" => Some(false),
        _ => None,
    }
}

/// The newer of the hook mark and the transcript mark.
pub(super) fn newer(hook: Option<TurnMark>, transcript: Option<TurnMark>) -> Option<TurnMark> {
    match (hook, transcript) {
        (Some(h), Some(t)) => Some(if t.at > h.at { t } else { h }),
        (h, t) => h.or(t),
    }
}

/// The newest turn mark ([`newer`]) as "is the turn open?"
/// (`None` = unknown). A CLOSED verdict is void once input reached the PTY
/// after it when `input_reopens` (codex: no hook announces the next turn);
/// an OPEN one is void once nothing renewed it for [`REAP_UNRESUMABLE_GRACE`]
/// (the idle sweep's freshness bound for the same record).
pub(super) fn effective_turn(
    mark: Option<TurnMark>,
    last_input: Option<Instant>,
    input_reopens: bool,
    now: Instant,
) -> Option<bool> {
    let mark = mark?;
    if mark.open {
        (now.saturating_duration_since(mark.at) < REAP_UNRESUMABLE_GRACE).then_some(true)
    } else if input_reopens && last_input.is_some_and(|i| i > mark.at) {
        None
    } else {
        Some(false)
    }
}

/// Everything one tick knows (see [`classify`]).
#[derive(Clone, Copy, Debug)]
pub(super) struct Observation {
    /// The status the row has now.
    pub current: SessionStatus,
    /// [`effective_turn`].
    pub turn: Option<bool>,
    /// The last screen sample shows an approval / permission prompt.
    pub awaiting_approval: bool,
    /// Since the PTY printed any byte.
    pub quiet_for: Duration,
    /// Since the masked screen text last changed (`None` = never seen).
    pub content_age: Option<Duration>,
    /// This tick's content change is confirmed real output ([`ScreenTracker`]).
    pub burst: bool,
    /// Since a spinner / elapsed-time cell last ticked (`None` = never seen).
    pub busy_age: Option<Duration>,
}

/// The working / idle decision, pure.
pub(super) fn classify(o: &Observation) -> SessionStatus {
    use otto_core::domain::SessionStatus::{Idle, Working};
    match o.turn {
        Some(true) if !o.awaiting_approval && o.quiet_for < OPEN_TURN_SILENCE => return Working,
        Some(false) => return Idle,
        _ => {}
    }
    let content_within = |w: Duration| o.content_age.is_some_and(|a| a < w);
    if o.current == Working {
        // Stays while content moves; while only a busy animation ticks, stays
        // (bounded); while only idle decoration repaints, lingers then lets go.
        let bytes = o.quiet_for < WORKING_WINDOW;
        let busy = bytes
            && !o.awaiting_approval
            && o.busy_age.is_some_and(|a| a < WORKING_WINDOW)
            && content_within(BUSY_ANIMATION_CAP);
        let repainting = bytes && content_within(REPAINT_LINGER);
        if content_within(WORKING_WINDOW) || busy || repainting {
            return Working;
        }
        return Idle;
    }
    // Idle → Working only on confirmed output (debounced: a periodic
    // one-row redraw never flips the row).
    if o.burst && content_within(WORKING_WINDOW) {
        Working
    } else {
        Idle
    }
}

/// Row prints of the last screen sample and when they last changed.
#[derive(Debug, Default)]
pub(super) struct ScreenTracker {
    size: (u16, u16),
    prints: Option<Vec<u64>>,
    busy: Option<Vec<u64>>,
    pub changed_at: Option<Instant>,
    /// Last tick a spinner / elapsed-time cell changed ([`busy_prints`]).
    pub busy_at: Option<Instant>,
    prev_changed: bool,
    pub awaiting_approval: bool,
}

impl ScreenTracker {
    /// Fold one screen sample (taken because new bytes arrived). Returns
    /// whether the change is CONFIRMED output: two or more rows changed, a
    /// blank row filled (a line was appended), or rows changed on two ticks
    /// in a row. A single row redrawn in place once is a repaint until it
    /// repeats. The first sample, and the first after a resize (re-wrapped
    /// text, not output), only set the baseline.
    pub fn observe(&mut self, size: (u16, u16), prints: Vec<u64>, now: Instant) -> bool {
        let blank = row_prints(&[String::new()])[0];
        let (mut changed, mut appended) = (0usize, false);
        if let Some(prev) = self.prints.as_ref().filter(|_| self.size == size) {
            for i in 0..prev.len().max(prints.len()) {
                let (a, b) = (prev.get(i).copied(), prints.get(i).copied());
                if a != b {
                    changed += 1;
                    appended |= a.is_none_or(|a| a == blank);
                }
            }
        }
        self.size = size;
        self.prints = Some(prints);
        let any = changed > 0;
        if any {
            self.changed_at = Some(now);
        }
        let burst = changed >= 2 || appended || (any && self.prev_changed);
        self.prev_changed = any;
        burst
    }

    /// Fold the same sample's [`busy_prints`] (after [`Self::observe`], which
    /// owns the size check): any difference is a busy animation tick.
    pub fn observe_busy(&mut self, busy: Vec<u64>, now: Instant) {
        let same_grid = self.busy.as_ref().is_some_and(|b| b.len() == busy.len());
        if same_grid && self.busy.as_ref() != Some(&busy) {
            self.busy_at = Some(now);
        }
        self.busy = Some(busy);
    }

    /// A tick with no new bytes: nothing changed on screen.
    pub fn quiet_tick(&mut self) {
        self.prev_changed = false;
    }
}

/// The provider transcript's turn verdict, re-read only when the file moved.
pub(super) struct TranscriptProbe {
    family: String,
    session: Session,
    roots: otto_transcript::roots::TranscriptRoots,
    repo: SessionsRepo,
    path: Option<PathBuf>,
    next_locate: Instant,
    mtime: Option<SystemTime>,
    mark: Option<TurnMark>,
}

impl TranscriptProbe {
    async fn mark(&mut self) -> Option<TurnMark> {
        if self.path.is_none() {
            if Instant::now() < self.next_locate {
                return None;
            }
            self.next_locate = Instant::now() + LOCATE_RETRY;
            self.path = self.locate().await;
        }
        let path = self.path.clone()?;
        let seen = self.mtime;
        let probe = tokio::task::spawn_blocking(move || {
            let mtime = std::fs::metadata(&path).and_then(|m| m.modified()).ok()?;
            if Some(mtime) == seen {
                return Some((mtime, None));
            }
            let tail = read_tail(&path, STATUS_TAIL_BYTES).ok();
            Some((mtime, tail))
        })
        .await
        .ok()
        .flatten();
        let Some((mtime, tail)) = probe else {
            // Gone (a `/clear` rotates claude's file): locate again later.
            (self.path, self.mtime, self.mark) = (None, None, None);
            return None;
        };
        if let Some(tail) = tail {
            self.mtime = Some(mtime);
            let at = Instant::now()
                .checked_sub(mtime.elapsed().unwrap_or_default())
                .unwrap_or_else(Instant::now);
            self.mark = otto_transcript::turn::tail_open(&self.family, &tail)
                .map(|open| TurnMark { open, at });
        }
        self.mark
    }

    /// [`SessionManager::activity_artifact`] without the manager: the
    /// provider id may have been captured since the task started.
    async fn locate(&mut self) -> Option<PathBuf> {
        if self.session.provider_session_id.is_none() {
            self.session = self.repo.get(&self.session.id).await.ok()?;
        }
        let psid = self.session.provider_session_id.clone()?;
        if psid.is_empty() || psid.contains(['/', '\\', '\0']) || psid.contains("..") {
            return None;
        }
        let (session, family, roots) = (
            self.session.clone(),
            self.family.clone(),
            self.roots.clone(),
        );
        tokio::task::spawn_blocking(move || {
            SessionManager::locate_activity_artifact(&session, &family, &psid, &roots)
        })
        .await
        .ok()
        .flatten()
    }
}

/// One status task's classifier state (see the module doc).
pub(super) struct StatusClassifier {
    id: Id,
    handle: Arc<PtyHandle>,
    marks: TurnMarks,
    input_reopens: bool,
    screen: ScreenTracker,
    seen_output: Option<Instant>,
    transcript: Option<TranscriptProbe>,
    /// An open mark already outlived [`OPEN_TURN_SILENCE`]: ignored from then
    /// on, so a later stray repaint cannot revive it (a flap every repaint).
    spent_open: Option<Instant>,
}

impl StatusClassifier {
    /// A classifier for `session`'s new process `handle`. A new process starts
    /// with no hook mark (a resume's old turn is not this process's).
    pub fn new(manager: &SessionManager, session: &Session, handle: Arc<PtyHandle>) -> Self {
        manager.turn_marks.remove(&session.id);
        let family = manager.cli_family(&session.provider);
        let transcript = matches!(family.as_str(), "claude" | "codex").then(|| TranscriptProbe {
            roots: manager.transcript_roots_for(session),
            session: session.clone(),
            repo: manager.repo.clone(),
            path: None,
            next_locate: Instant::now(),
            mtime: None,
            mark: None,
            family: family.clone(),
        });
        Self {
            id: session.id.clone(),
            handle,
            marks: Arc::clone(&manager.turn_marks),
            input_reopens: family != "claude",
            screen: ScreenTracker::default(),
            seen_output: None,
            transcript,
            spent_open: None,
        }
    }

    /// This tick's status for a row that currently reads `current`. Samples
    /// the screen only when new bytes arrived since the last tick (one grid
    /// walk on the blocking pool), and the transcript only when it moved.
    pub async fn next(&mut self, current: SessionStatus) -> SessionStatus {
        let now = Instant::now();
        let last_out = self.handle.last_output_at();
        let mut burst = false;
        if self.seen_output != Some(last_out) {
            self.seen_output = Some(last_out);
            let h = Arc::clone(&self.handle);
            let sample =
                tokio::task::spawn_blocking(move || (h.screen_size(), h.screen_rows())).await;
            if let Ok((size, rows)) = sample {
                self.screen.awaiting_approval = awaits_approval(&rows);
                burst = self.screen.observe(size, row_prints(&rows), now);
                self.screen.observe_busy(busy_prints(&rows), now);
            }
        } else {
            self.screen.quiet_tick();
        }
        let transcript = match self.transcript.as_mut() {
            Some(p) => p.mark().await,
            None => None,
        };
        let hook = self.marks.get(&self.id).map(|m| *m.value());
        let spent = self.spent_open;
        let mark = newer(hook, transcript).filter(|m| !(m.open && Some(m.at) == spent));
        let quiet_for = last_out.elapsed();
        if let Some(m) = mark.filter(|m| m.open && quiet_for >= OPEN_TURN_SILENCE) {
            self.spent_open = Some(m.at);
        }
        let last_input = self.handle.last_input_at();
        classify(&Observation {
            current,
            turn: effective_turn(mark, last_input, self.input_reopens, now),
            awaiting_approval: self.screen.awaiting_approval,
            quiet_for,
            content_age: self
                .screen
                .changed_at
                .map(|t| now.saturating_duration_since(t)),
            burst,
            busy_age: self
                .screen
                .busy_at
                .map(|t| now.saturating_duration_since(t)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::domain::SessionStatus::{Idle, Running, Working};

    fn obs(current: SessionStatus) -> Observation {
        Observation {
            current,
            turn: None,
            awaiting_approval: false,
            quiet_for: Duration::from_secs(1),
            content_age: None,
            burst: false,
            busy_age: None,
        }
    }

    #[test]
    fn status_classify_repaint_without_content_change_is_idle() {
        // Bytes every second (a spinner / clock), masked text unchanged.
        let o = Observation {
            content_age: Some(Duration::from_secs(600)),
            ..obs(Idle)
        };
        assert_eq!(classify(&o), Idle);
        assert_eq!(
            classify(&Observation {
                content_age: None,
                ..o
            }),
            Idle
        );
        // A repainting row that WAS working lingers, then lets go.
        let lingering = Observation {
            current: Working,
            content_age: Some(Duration::from_secs(10)),
            ..o
        };
        assert_eq!(classify(&lingering), Working);
        let done = Observation {
            content_age: Some(REPAINT_LINGER),
            ..lingering
        };
        assert_eq!(classify(&done), Idle);
        // No bytes at all: idle after the plain window, no linger.
        let silent = Observation {
            quiet_for: WORKING_WINDOW,
            ..lingering
        };
        assert_eq!(classify(&silent), Idle);
    }

    #[test]
    fn status_classify_confirmed_output_is_working_within_a_tick() {
        let o = Observation {
            content_age: Some(Duration::ZERO),
            burst: true,
            ..obs(Idle)
        };
        assert_eq!(classify(&o), Working);
        assert_eq!(
            classify(&Observation {
                current: Running,
                ..o
            }),
            Working
        );
        // One unconfirmed in-place change does not flip an idle row.
        assert_eq!(classify(&Observation { burst: false, ..o }), Idle);
    }

    #[test]
    fn status_classify_turn_signals_win_over_the_screen() {
        // Closed turn + a moving screen (background-task footer): idle.
        let closed = Observation {
            turn: Some(false),
            content_age: Some(Duration::ZERO),
            burst: true,
            ..obs(Working)
        };
        assert_eq!(classify(&closed), Idle);
        // Open turn + a static screen (a long tool call): working.
        let open = Observation {
            turn: Some(true),
            content_age: Some(Duration::from_secs(300)),
            quiet_for: Duration::from_secs(1),
            ..obs(Idle)
        };
        assert_eq!(classify(&open), Working);
        // ...but not over a silent screen (an interrupt fires no Stop).
        let stale = Observation {
            quiet_for: OPEN_TURN_SILENCE,
            ..open
        };
        assert_eq!(classify(&stale), Idle);
        // ...and not while it waits on an approval prompt.
        let asking = Observation {
            awaiting_approval: true,
            ..open
        };
        assert_eq!(classify(&asking), Idle);
    }

    #[test]
    fn status_classify_newer_turn_mark_wins_and_input_reopens_codex() {
        let now = Instant::now();
        let t0 = now - Duration::from_secs(60);
        let t1 = now - Duration::from_secs(30);
        let closed_hook = TurnMark {
            open: false,
            at: t1,
        };
        let open_file = TurnMark { open: true, at: t0 };
        assert_eq!(
            effective_turn(newer(Some(closed_hook), Some(open_file)), None, false, now),
            Some(false)
        );
        // A later transcript line (a task notification woke the agent).
        let woke = TurnMark {
            open: true,
            at: now,
        };
        assert_eq!(
            effective_turn(newer(Some(closed_hook), Some(woke)), None, false, now),
            Some(true)
        );
        assert_eq!(effective_turn(None, None, false, now), None);
        // Typing after codex's turn-complete: unknown, the screen decides.
        let typed = Some(now - Duration::from_secs(5));
        assert_eq!(effective_turn(Some(closed_hook), typed, true, now), None);
        // Claude announces its next turn itself: input does not void Stop.
        assert_eq!(
            effective_turn(Some(closed_hook), typed, false, now),
            Some(false)
        );
        // An open mark nothing renewed for the sweep's bound is stale.
        let old = TurnMark {
            open: true,
            at: now - REAP_UNRESUMABLE_GRACE,
        };
        assert_eq!(effective_turn(Some(old), None, false, now), None);
    }

    #[test]
    fn status_classify_hook_events_mark_turn_boundaries() {
        assert_eq!(claude_hook_turn("UserPromptSubmit"), Some(true));
        assert_eq!(claude_hook_turn("Stop"), Some(false));
        // Mid-turn events (or ones that can race past Stop) say nothing.
        for e in ["PostToolUse", "Notification", "SessionStart", ""] {
            assert_eq!(claude_hook_turn(e), None, "{e}");
        }
    }

    /// (a) Real content, then only a spinner + elapsed counter ticking for two
    /// minutes (an agent thinking with no turn record, e.g. Muse): Working.
    #[test]
    fn status_classify_spinner_after_content_keeps_working() {
        let thinking = Observation {
            content_age: Some(Duration::from_secs(120)),
            busy_age: Some(Duration::from_secs(1)),
            ..obs(Working)
        };
        assert_eq!(classify(&thinking), Working);
        // ...but not on an approval prompt, not once the bytes stop, and not
        // past the cap.
        let asking = Observation {
            awaiting_approval: true,
            ..thinking
        };
        assert_eq!(classify(&asking), Idle);
        let stopped = Observation {
            quiet_for: WORKING_WINDOW,
            ..thinking
        };
        assert_eq!(classify(&stopped), Idle);
        let capped = Observation {
            content_age: Some(BUSY_ANIMATION_CAP),
            ..thinking
        };
        assert_eq!(classify(&capped), Idle);
    }

    /// (b) Real content, then only a wall clock ticking: Idle after ~30 s.
    #[test]
    fn status_classify_clock_after_content_goes_idle() {
        let t0 = Instant::now();
        let mut t = ScreenTracker::default();
        let at = |secs: u64| t0 + Duration::from_secs(secs);
        let mut feed = |text: &str, clock: u64, when: Instant| {
            let rows = vec![format!("{text} · 09:00:{clock:02}"), "> ".to_string()];
            t.observe((80, 2), row_prints(&rows), when);
            t.observe_busy(busy_prints(&rows), when);
            (t.changed_at, t.busy_at)
        };
        feed("building", 0, at(0));
        feed("built ok", 1, at(1));
        let mut last = (None, None);
        for s in (2..=40).step_by(2) {
            last = feed("built ok", s, at(s));
        }
        assert_eq!(
            last,
            (Some(at(1)), Some(at(1))),
            "a clock is neither content nor busy"
        );
        let observe = |now: Instant| Observation {
            content_age: last.0.map(|c| now.saturating_duration_since(c)),
            busy_age: last.1.map(|b| now.saturating_duration_since(b)),
            ..obs(Working)
        };
        assert_eq!(classify(&observe(at(20))), Working, "lingers");
        assert_eq!(classify(&observe(at(40))), Idle);
    }

    /// (c) An idle screen where a spinner starts, no content change: Idle.
    #[test]
    fn status_classify_spinner_never_starts_working() {
        let now = Instant::now();
        let mut t = ScreenTracker::default();
        for (i, frame) in ["⠋", "⠙", "⠹", "⠸"].iter().enumerate() {
            let rows = vec![format!("{frame} idle (3s)"), "> ".to_string()];
            let at = now + Duration::from_secs(2 * i as u64);
            assert!(!t.observe((80, 2), row_prints(&rows), at), "no burst");
            t.observe_busy(busy_prints(&rows), at);
        }
        assert!(t.changed_at.is_none() && t.busy_at.is_some());
        let o = Observation {
            busy_age: Some(Duration::ZERO),
            ..obs(Idle)
        };
        assert_eq!(classify(&o), Idle);
    }

    fn prints(rows: &[&str]) -> Vec<u64> {
        row_prints(&rows.iter().map(|r| r.to_string()).collect::<Vec<_>>())
    }

    #[test]
    fn status_classify_tracker_debounces_in_place_redraws() {
        let now = Instant::now();
        let mut t = ScreenTracker::default();
        assert!(
            !t.observe((80, 3), prints(&["⠹ a (0s)", "b 10:00", ""]), now),
            "baseline"
        );
        assert!(t.changed_at.is_none());
        // Spinner frame + elapsed tick: masked, no change at all.
        assert!(!t.observe((80, 3), prints(&["⠋ a (1s)", "b 10:01", ""]), now));
        assert!(!t.observe((80, 3), prints(&["⠙ a (3s)", "b 10:02", ""]), now));
        // One row redrawn in place once: changed, not confirmed.
        assert!(!t.observe((80, 3), prints(&["⠙ a (3s)", "c 10:02", ""]), now));
        assert!(t.changed_at.is_some());
        // ...a quiet tick in between resets the streak.
        t.quiet_tick();
        assert!(!t.observe((80, 3), prints(&["⠙ a (3s)", "d", ""]), now));
        // Same row again on the very next tick: streaming, confirmed.
        assert!(t.observe((80, 3), prints(&["⠙ a (3s)", "de", ""]), now));
    }

    #[test]
    fn status_classify_tracker_confirms_appends_and_scrolls_but_not_resizes() {
        let now = Instant::now();
        let mut t = ScreenTracker::default();
        t.observe((80, 3), prints(&["$ make", "", ""]), now);
        assert!(
            t.observe((80, 3), prints(&["$ make", "cc a.o", ""]), now),
            "append"
        );
        assert!(
            t.observe((80, 3), prints(&["cc a.o", "cc b.o", "cc c.o"]), now),
            "scroll"
        );
        // A resize re-wraps the same text: a new baseline, not output.
        assert!(!t.observe((40, 3), prints(&["cc", "a.o", "cc"]), now));
    }
}
