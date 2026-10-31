//! Submitting a prompt into an agent TUI: the ONE paste → Enter path every
//! engine uses (architecture-4-01).
//!
//! A provider TUI (Claude Code, Codex, agy …) takes a prompt in a fixed
//! sequence: wait for it to draw, take the per-session submit lock, bracketed
//! paste, settle, Enter, and re-Enter once if the submit visibly didn't
//! dispatch. The verified variant also waits for the paste to ECHO (re-pasting
//! once after a Ctrl-U when a late redraw ate it) before pressing Enter.
//!
//! This used to live in the code-review crate, so automation, product,
//! insights, personal agents and the channel bridge each re-assembled the
//! sequence from host primitives — and four of those copies never took the
//! submit lock, so a broadcast or an agent→agent message could land between
//! their paste and Enter (one merged prompt plus an empty one). Every copy now
//! calls [`SessionManager::submit_prompt`] (or [`submit_prompt`], the verified
//! path); `scripts/check.sh` refuses a new `bracketed_paste(` or raw `b"\r"`
//! PTY write outside this crate.

use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_core::Id;

use crate::SessionManager;

// Generous: several CLIs cold-start concurrently (a review fans out), so claude
// can take >30s to draw its TUI; injecting before it's ready loses the prompt.
const TUI_STARTUP_WAIT: Duration = Duration::from_secs(90);
const TUI_POLL: Duration = Duration::from_millis(250);
const TUI_SETTLE: Duration = Duration::from_millis(600);
/// Settle time between a paste and the Enter that submits it.
pub const PASTE_TO_ENTER: Duration = Duration::from_millis(250);
/// After a paste, how long to wait for the pasted text to actually echo in the
/// TUI before pressing Enter. A late redraw (claude's "N MCP servers need
/// authentication" banner lands well after the prompt box is drawn) can wipe
/// the input box, so an unverified Enter submits an empty line and the agent
/// idles for the whole timeout.
const PASTE_ECHO_WAIT: Duration = Duration::from_secs(8);
const PASTE_ECHO_POLL: Duration = Duration::from_millis(250);
/// Extra time a paste that has not echoed yet gets while the TUI is still
/// busy drawing (output advanced within [`PASTE_BUSY_QUIET`]). A 46–70 KB
/// reviewer prompt can take longer than [`PASTE_ECHO_WAIT`] to land, and
/// re-pasting on top of a slow-but-successful paste submitted it TWICE.
const PASTE_LATE_WAIT: Duration = Duration::from_secs(20);
const PASTE_BUSY_QUIET: Duration = Duration::from_millis(1500);
/// After Enter, confirm the agent actually started (output advanced); if not,
/// re-send Enter once — a freshly-spawned CLI under load can drop the first.
const DISPATCH_WAIT: Duration = Duration::from_secs(6);
const DISPATCH_POLL: Duration = Duration::from_millis(250);
/// Hard cap on [`SubmitOpts::extra_settle`]'s wait for a quiet TUI.
const EXTRA_SETTLE_CAP: Duration = Duration::from_secs(5);
const EXTRA_SETTLE_POLL: Duration = Duration::from_millis(100);
/// Ctrl-U: clears the partial input line before a re-paste.
const CLEAR_LINE: &[u8] = b"\x15";
const ENTER: &[u8] = b"\r";

/// How [`SessionManager::submit_prompt`] submits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SubmitOpts {
    /// Verified submit: refuse a TUI that drew nothing by the deadline (the
    /// strict wait), and wait for the paste to echo — re-pasting once — before
    /// Enter. `false` is the legacy submit (lenient wait, paste, settle, Enter)
    /// for callers that run their own "did the prompt land" watch.
    pub echo_wait: bool,
    /// After the TUI is ready, also wait (capped at 5 s) until its output has
    /// been quiet this long before pasting — Codex keeps drawing a
    /// model-loading burst after its TUI first settles and drops a paste typed
    /// into it.
    pub extra_settle: Option<Duration>,
}

impl SubmitOpts {
    /// The verified submit ([`submit_prompt`]'s options).
    pub const VERIFIED: Self = Self {
        echo_wait: true,
        extra_settle: None,
    };
}

/// "Is the run this submit belongs to still active?" (architecture-4-01r). A
/// run-scoped caller (a swarm run, a stoppable engine step) passes one to
/// [`SessionManager::submit_prompt_while`]; it is checked while waiting for
/// the TUI and, under the submit lock, before the paste and before every
/// re-inject (re-paste, Enter, re-Enter), so a run stopped mid-submit never
/// types into a session it no longer owns. The check is ASYNC — the swarm's
/// is a DB status read plus its cancel flag
/// (`otto_swarm::runtime::engine::run_guard`). Kept off [`SubmitOpts`] so
/// that stays a plain `Copy` value its callers build with a struct literal.
#[derive(Clone)]
pub struct RunGuard(Arc<ActiveFn>);

type ActiveFn =
    dyn Fn() -> std::pin::Pin<Box<dyn std::future::Future<Output = bool> + Send>> + Send + Sync;

impl RunGuard {
    /// `active` answers "is the run still active?" each time it is called.
    pub fn new<F, Fut>(active: F) -> Self
    where
        F: Fn() -> Fut + Send + Sync + 'static,
        Fut: std::future::Future<Output = bool> + Send + 'static,
    {
        Self(Arc::new(move || Box::pin(active())))
    }

    /// One check of the run.
    pub async fn is_active(&self) -> bool {
        (self.0)().await
    }

    /// No guard (an unscoped submit) is always active.
    async fn active(guard: Option<&Self>) -> bool {
        match guard {
            Some(g) => g.is_active().await,
            None => true,
        }
    }
}

impl std::fmt::Debug for RunGuard {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RunGuard")
    }
}

/// `text` wrapped in bracketed-paste markers. The prompt carries external text
/// (Jira descriptions, chat messages, user context): a raw `ESC[201~` in it
/// would end the paste early and type the rest as keystrokes, so control
/// characters other than newline and tab are stripped first (the same rule as
/// `otto_orchestrator::claude_pty::sanitize_paste`).
pub fn bracketed_paste(text: &str) -> Vec<u8> {
    let mut v = Vec::with_capacity(text.len() + 12);
    v.extend_from_slice(b"\x1b[200~");
    for c in text
        .chars()
        .filter(|c| !c.is_control() || matches!(c, '\n' | '\t'))
    {
        let mut buf = [0u8; 4];
        v.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
    }
    v.extend_from_slice(b"\x1b[201~");
    v
}

/// One poll of the TUI wait: `Some(true)` ready to paste into, `Some(false)`
/// give up, `None` keep polling. The two rules differ only on the DEADLINE: a
/// `strict` caller treats a TUI that has drawn NOTHING as NOT ready — pasting
/// into it loses the prompt, and the caller then waits out the agent's entire
/// grace window for a turn that was never started — while the legacy rule
/// pastes anyway. Pure so both deadline rules are testable without a PTY.
pub(crate) fn tui_ready(
    scrollback_empty: bool,
    settled: bool,
    deadline_hit: bool,
    strict: bool,
) -> Option<bool> {
    if !scrollback_empty && settled {
        return Some(true);
    }
    if deadline_hit {
        return Some(!(strict && scrollback_empty));
    }
    None
}

/// Wait for the agent's TUI to draw and settle. LEGACY rule: on the deadline it
/// reports ready even for a blank TUI, so its callers paste and fall back to
/// their own watch loop. Callers with a fail-fast path for "the prompt never
/// landed" submit with [`SubmitOpts::VERIFIED`] instead, whose strict wait
/// treats a TUI that drew NOTHING by the deadline as not ready.
pub async fn wait_for_tui(manager: &SessionManager, sid: &Id) -> bool {
    wait_for_tui_with(manager, sid, false, None).await
}

/// The TUI wait; a `run` that ends while it polls stops the wait (not ready).
async fn wait_for_tui_with(
    manager: &SessionManager,
    sid: &Id,
    strict: bool,
    run: Option<&RunGuard>,
) -> bool {
    let deadline = Instant::now() + TUI_STARTUP_WAIT;
    loop {
        let Some(handle) = manager.live_handle(sid) else {
            return false;
        };
        if !RunGuard::active(run).await {
            tracing::info!("submit: run ended while waiting for session {sid}'s TUI");
            return false;
        }
        if handle.on_exit().borrow().is_some() {
            return false;
        }
        let verdict = tui_ready(
            handle.scrollback(1).is_empty(),
            handle.last_output_at().elapsed() >= TUI_SETTLE,
            Instant::now() >= deadline,
            strict,
        );
        if let Some(ready) = verdict {
            if !ready {
                tracing::warn!("submit: agent TUI never drew in session {sid}");
            }
            return ready;
        }
        tokio::time::sleep(TUI_POLL).await;
    }
}

/// True if the session produced fresh output after `before` within
/// [`DISPATCH_WAIT`] — i.e. the submitted prompt was accepted and the agent
/// started working.
pub async fn dispatched(manager: &SessionManager, sid: &Id, before: Option<Instant>) -> bool {
    let Some(before) = before else { return false };
    let deadline = Instant::now() + DISPATCH_WAIT;
    loop {
        match manager.live_handle(sid) {
            Some(h) if h.last_output_at() > before => return true,
            None => return false,
            _ => {}
        }
        if Instant::now() >= deadline {
            return false;
        }
        tokio::time::sleep(DISPATCH_POLL).await;
    }
}

/// Paste `prompt` into the session and press Enter through the verified path
/// ([`SubmitOpts::VERIFIED`]). Returns false when the session died (or its TUI
/// never drew) before the prompt could be sent.
pub async fn submit_prompt(manager: &SessionManager, sid: &Id, prompt: &str) -> bool {
    manager
        .submit_prompt(sid, prompt, SubmitOpts::VERIFIED)
        .await
}

impl SessionManager {
    /// Submit `prompt` into the session's TUI — the one submit path. Waits for
    /// the TUI (strict when `opts.echo_wait`), applies `opts.extra_settle`,
    /// then HOLDS the per-session submit lock (the one [`Self::submit_text`],
    /// the human composer, the channel bridge and swarm runs take) through
    /// paste, echo wait, Enter and re-Enter. Returns false when the session
    /// was gone (or, verified, never drew) before the prompt could be sent.
    pub async fn submit_prompt(&self, sid: &Id, prompt: &str, opts: SubmitOpts) -> bool {
        self.submit_prompt_at(sid, prompt, opts).await.is_some()
    }

    /// [`Self::submit_prompt`], returning the session's last-output instant
    /// taken right BEFORE the paste (`None` inside when it had no handle) —
    /// the baseline a caller's own "did the prompt land" watch measures from.
    /// `None` = not submitted.
    pub async fn submit_prompt_at(
        &self,
        sid: &Id,
        prompt: &str,
        opts: SubmitOpts,
    ) -> Option<Option<Instant>> {
        self.submit_guarded(sid, prompt, opts, None).await
    }

    /// [`Self::submit_prompt_at`] for a run-scoped caller: `run` is checked
    /// during the TUI wait, then under the submit lock before the paste, the
    /// re-paste, Enter and the re-Enter; once it reports the run over, nothing
    /// more is typed (`None` when that happens before Enter — the caller then
    /// asks `run` itself whether the run stopped or the session never drew).
    pub async fn submit_prompt_while(
        &self,
        sid: &Id,
        prompt: &str,
        opts: SubmitOpts,
        run: RunGuard,
    ) -> Option<Option<Instant>> {
        self.submit_guarded(sid, prompt, opts, Some(&run)).await
    }

    async fn submit_guarded(
        &self,
        sid: &Id,
        prompt: &str,
        opts: SubmitOpts,
        run: Option<&RunGuard>,
    ) -> Option<Option<Instant>> {
        if !wait_for_tui_with(self, sid, opts.echo_wait, run).await {
            return None;
        }
        if let Some(quiet) = opts.extra_settle {
            self.wait_quiet(sid, quiet).await;
        }
        let lock = self.submit_lock(sid);
        let _submit = lock.lock().await;
        if !RunGuard::active(run).await {
            tracing::info!("submit: run ended before the paste into session {sid}");
            return None;
        }
        let pre_paste = self.live_handle(sid).map(|h| h.last_output_at());
        // A paste that could not be written (the session died, or a room
        // driver refused automation input) is "not submitted".
        let pasted = if opts.echo_wait {
            self.paste_until_echoed(sid, prompt, run).await
        } else {
            let ok = self.input(sid, &bracketed_paste(prompt)).await.is_ok();
            tokio::time::sleep(PASTE_TO_ENTER).await;
            ok
        };
        if !pasted {
            tracing::warn!("submit: could not paste the prompt into session {sid}");
            return None;
        }
        if !RunGuard::active(run).await {
            tracing::info!("submit: run ended before Enter in session {sid}");
            return None;
        }
        self.enter_until_dispatched(sid, run).await;
        Some(pre_paste)
    }

    /// Re-paste and re-submit `prompt` with no TUI wait — for a caller whose
    /// own watch decided the first submit was dropped (product runs). Takes
    /// the submit lock for this one paste → Enter → re-Enter group only, so a
    /// caller polling between groups never blocks other submitters.
    pub async fn resubmit_prompt(&self, sid: &Id, prompt: &str) {
        let lock = self.submit_lock(sid);
        let _submit = lock.lock().await;
        let _ = self.input(sid, &bracketed_paste(prompt)).await;
        tokio::time::sleep(PASTE_TO_ENTER).await;
        self.enter_until_dispatched(sid, None).await;
    }

    /// Enter, then Enter once more when the submit didn't visibly dispatch
    /// (and `run`, if any, is still active). Caller holds the submit lock.
    async fn enter_until_dispatched(&self, sid: &Id, run: Option<&RunGuard>) {
        let before = self.live_handle(sid).map(|h| h.last_output_at());
        let _ = self.input(sid, ENTER).await;
        if !dispatched(self, sid, before).await && RunGuard::active(run).await {
            let _ = self.input(sid, ENTER).await;
        }
    }

    /// Wait (capped at [`EXTRA_SETTLE_CAP`]) until the session's output has
    /// been quiet for `quiet`. Returns at once when it already has, or when
    /// the session has no handle.
    async fn wait_quiet(&self, sid: &Id, quiet: Duration) {
        let deadline = Instant::now() + EXTRA_SETTLE_CAP;
        loop {
            let elapsed = self
                .live_handle(sid)
                .map(|h| h.last_output_at().elapsed())
                .unwrap_or(quiet);
            if elapsed >= quiet || Instant::now() >= deadline {
                return;
            }
            tokio::time::sleep(EXTRA_SETTLE_POLL).await;
        }
    }

    /// Paste, then WAIT FOR THE PASTE TO ECHO before Enter, re-pasting once
    /// (after a Ctrl-U clear) if a late redraw wiped it. Caller holds the
    /// submit lock — without it the Ctrl-U erased another submitter's paste.
    /// False only when the first paste could not be written at all.
    async fn paste_until_echoed(&self, sid: &Id, prompt: &str, run: Option<&RunGuard>) -> bool {
        let probe = paste_probe(prompt);
        for attempt in 0..2 {
            if attempt == 1 {
                if !RunGuard::active(run).await {
                    // The run ended while the first paste was settling: leave
                    // the half-landed text rather than re-typing for a run
                    // that no longer owns the session.
                    return true;
                }
                // Clear whatever partial line a half-landed paste left, so a
                // re-paste never concatenates onto it.
                let _ = self.input(sid, CLEAR_LINE).await;
                tokio::time::sleep(PASTE_TO_ENTER).await;
            }
            let wrote = self.input(sid, &bracketed_paste(prompt)).await.is_ok();
            if attempt == 0 && !wrote {
                return false;
            }
            tokio::time::sleep(PASTE_TO_ENTER).await;
            let deadline = Instant::now() + PASTE_ECHO_WAIT;
            let late_deadline = deadline + PASTE_LATE_WAIT;
            loop {
                if paste_echoed(self, sid, &probe) {
                    break;
                }
                let now = Instant::now();
                if now >= late_deadline || (now >= deadline && !tui_busy(self, sid)) {
                    break;
                }
                tokio::time::sleep(PASTE_ECHO_POLL).await;
            }
            if paste_echoed(self, sid, &probe) {
                return true;
            }
            if attempt == 0 {
                // Let whatever redraw ate the paste finish, then look ONE more
                // time before re-sending — a paste that landed during the
                // settle must not be pasted again.
                tokio::time::sleep(Duration::from_secs(2)).await;
                if paste_echoed(self, sid, &probe) {
                    return true;
                }
                tracing::warn!("prompt paste did not echo in session {sid}; re-pasting once");
            }
        }
        true
    }
}

/// A short, distinctive, whitespace-normalized probe from the prompt's first
/// non-empty line — what the TUI must echo back for the paste to count.
pub(crate) fn paste_probe(prompt: &str) -> String {
    let line = prompt.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
    let norm: String = line.split_whitespace().collect::<Vec<_>>().join(" ");
    // TUIs wrap/truncate long lines; 40 chars is short enough to survive that
    // and long enough not to match boilerplate.
    norm.chars().take(40).collect()
}

/// True once the screen contains `probe` (whitespace-normalized) or a paste
/// placeholder — i.e. the pasted prompt is really sitting in the input box.
///
/// Matched against ANSI-FREE text: the emulator's current screen (the vt100
/// grid — what a user would see) plus the escape-stripped scrollback tail.
/// Matching the RAW PTY bytes missed pastes whose echo was interleaved with
/// colour/cursor sequences, and every miss re-pasted a prompt that had in fact
/// landed — 23 of 201 re-pasted review transcripts held the prompt twice.
fn paste_echoed(manager: &SessionManager, sid: &Id, probe: &str) -> bool {
    if probe.is_empty() {
        return true;
    }
    let Some(h) = manager.live_handle(sid) else {
        return false;
    };
    let rows = h.screen_rows();
    let tail = strip_ansi(&h.scrollback(200));
    [rows.join(" "), rows.concat(), tail]
        .iter()
        .any(|text| screen_shows_paste(&normalize_ws(text), probe))
}

/// Escape-free printable text of a screen snapshot (control bytes dropped,
/// line breaks and tabs as spaces).
fn strip_ansi(bytes: &[u8]) -> String {
    otto_core::text::strip_ansi_bytes(bytes)
        .chars()
        .filter_map(|c| match c {
            '\n' | '\r' | '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect()
}

fn normalize_ws(s: &str) -> String {
    s.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// True while the TUI has drawn something within [`PASTE_BUSY_QUIET`] — a
/// large paste still being ingested/rendered.
fn tui_busy(manager: &SessionManager, sid: &Id) -> bool {
    manager
        .live_handle(sid)
        .is_some_and(|h| h.last_output_at().elapsed() < PASTE_BUSY_QUIET)
}

/// The paste counts as echoed when the screen shows the probe text OR a
/// large-paste placeholder. Codex collapses any sizeable paste into
/// `[Pasted Content N chars]` and Claude Code into `[Pasted text #1 +N lines]`,
/// so a worker prompt never echoes verbatim; treating that as "not echoed"
/// re-pasted the prompt and the worker received it twice.
pub(crate) fn screen_shows_paste(norm_screen: &str, probe: &str) -> bool {
    if norm_screen.contains(probe) {
        return true;
    }
    let lower = norm_screen.to_lowercase();
    lower.contains("[pasted content ") || lower.contains("[pasted text #")
}

impl SessionManager {
    /// [`Self::submit_text`] for a session room's driver: the same submit
    /// lock, control-byte stripping and paste-then-Enter timing, with every
    /// write re-checked against the room grant (`room_input`). `submit: false`
    /// pastes only (a pasted image's path the driver keeps typing around).
    pub async fn room_paste_text(
        &self,
        id: &Id,
        room: &str,
        member: &str,
        epoch: u64,
        text: &str,
        submit: bool,
    ) -> otto_core::Result<()> {
        let lock = self.submit_lock(id);
        let _submit = lock.lock().await;
        self.room_input(id, room, member, epoch, &bracketed_paste(text))
            .await?;
        if !submit {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
        self.room_input(id, room, member, epoch, b"\r").await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_probe_is_short_normalized_and_skips_blank_lines() {
        let p = paste_probe("\n\n  You are   reviewing\tthe skill package at /tmp/x — do it carefully and thoroughly\nmore");
        assert_eq!(p.chars().count(), 40);
        assert!(p.starts_with("You are reviewing the skill package"));
        assert_eq!(paste_probe(""), "");
    }

    #[test]
    fn paste_counts_as_echoed_when_the_tui_collapses_it_to_a_placeholder() {
        let probe = "You are a Codex worker on /repo (branch";
        // Codex: any sizeable paste is shown as a placeholder, never verbatim.
        assert!(screen_shows_paste(
            "› [Pasted Content 2614 chars] gpt-5 high",
            probe
        ));
        // Claude Code's placeholder.
        assert!(screen_shows_paste("> [Pasted text #1 +38 lines]", probe));
        // Verbatim echo still counts.
        assert!(screen_shows_paste(
            "> You are a Codex worker on /repo (branch main)",
            probe
        ));
        // An empty input box does not.
        assert!(!screen_shows_paste("› Ask Codex to do anything", probe));
    }

    #[test]
    fn styled_echo_is_detected_once_escapes_are_stripped() {
        // Raw PTY bytes interleave the echo with SGR/cursor sequences; the
        // raw-byte match missed it and the prompt was pasted a second time.
        let probe = "You are a Codex worker on /repo (branch";
        let raw = b"\x1b[2m> \x1b[22mYou are a\x1b[1C Codex worker\x1b[0m on /repo (branch main)";
        let plain = strip_ansi(raw);
        assert!(screen_shows_paste(&normalize_ws(&plain), probe));
        let placeholder = strip_ansi(b"\x1b[38;5;246m[Pasted text #1 +812 lines]\x1b[39m");
        assert!(screen_shows_paste(&normalize_ws(&placeholder), probe));
    }

    #[test]
    fn bracketed_paste_cannot_be_closed_early() {
        let v = bracketed_paste("a\x1b[201~\r!rm -rf ~\r");
        let s = String::from_utf8(v).unwrap();
        assert_eq!(s.matches("\x1b[201~").count(), 1);
        assert!(s.ends_with("\x1b[201~"));
        assert!(!s.contains('\r'));
    }

    #[test]
    fn wait_for_tui_false_on_deadline_without_output() {
        // Drawn and settled ⇒ ready, whenever that happens.
        assert_eq!(tui_ready(false, true, false, true), Some(true));
        assert_eq!(tui_ready(false, true, true, true), Some(true));
        // Drawn but still repainting ⇒ keep polling until the deadline, then
        // accept it (there IS a TUI to paste into).
        assert_eq!(tui_ready(false, false, false, true), None);
        assert_eq!(tui_ready(false, false, true, true), Some(true));
        // NOTHING drawn by the deadline ⇒ give up. Pasting into a blank TUI
        // loses the prompt and the reviewer then idles out its whole grace.
        assert_eq!(tui_ready(true, false, true, true), Some(false));
        assert_eq!(tui_ready(true, false, false, true), None);
        // The LEGACY rule (`wait_for_tui`, `SubmitOpts { echo_wait: false }`)
        // pastes into a blank TUI on the deadline rather than giving up — they
        // have no fail-fast path and would otherwise never send their prompt.
        assert_eq!(tui_ready(true, false, true, false), Some(true));
        assert_eq!(tui_ready(true, false, false, false), None);
    }
}
