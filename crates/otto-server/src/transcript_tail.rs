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
//! N parallel review sessions nobody is looking at). Cap 64 concurrent tails —
//! beyond that reads still work, there is just no live push. Each poll also
//! reads the PTY screen (one grid walk) for the `transcript_live` draft. The registry slot is an RAII
//! [`Slot`] held by the task: a panic anywhere in the loop frees it, and the
//! stop decision + removal happen under ONE lock so a `touch` racing the exit
//! either refreshes a live entry or (after removal) starts a fresh tail — never
//! a lost wake-up.

use std::collections::{HashMap, HashSet};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use otto_core::domain::Session;
use otto_core::event::Event;
use otto_core::Id;
use otto_transcript::{
    parse_records, Artifact, Folded, Folder, Provider, SubagentMeta, SubagentScanner, Tailer,
};

use crate::offload::blocking;
use crate::state::ServerCtx;

pub const POLL: Duration = Duration::from_millis(700);
pub const MAX_TAILS: usize = 64;
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

/// The tail's fold state. Owned by a std `Mutex` that is only ever locked
/// inside `spawn_blocking` (the poll step and [`live_page`]), so neither a
/// fold nor a snapshot ever runs on a runtime worker.
struct TailState {
    folder: Folder<'static>,
    tailer: Tailer,
    subagents: SubagentScanner,
    /// Full snapshot for `GET …/transcript`, built on demand and dropped
    /// whenever the fold moves.
    snap: Option<Arc<Folded>>,
}

/// What a running tail shares with the read route: the file it folds and its
/// state (`None` until the initial fold lands).
struct Live {
    provider: Provider,
    path: PathBuf,
    state: Mutex<Option<TailState>>,
}

impl Live {
    fn lock(&self) -> std::sync::MutexGuard<'_, Option<TailState>> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }
}

struct Entry {
    last_touch: Instant,
    stop: Arc<AtomicBool>,
    live: Arc<Live>,
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
    let live = Arc::new(Live {
        provider,
        path: path.to_path_buf(),
        state: Mutex::new(None),
    });
    reg.insert(
        session.id.clone(),
        Entry {
            last_touch: Instant::now(),
            stop,
            live: live.clone(),
        },
    );
    drop(reg);
    let slot = Slot {
        id: session.id.clone(),
    };
    let ctx = ctx.clone();
    let session = session.clone();
    tokio::spawn(async move {
        let _slot = slot; // freed on every exit path, panics included
        run(ctx, session, live).await;
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

/// The live tail's current fold of `path`, for `GET …/transcript` — served
/// from memory instead of re-reading and re-folding the whole JSONL (which,
/// on a 65 MB transcript, cost ~150 ms per refetch and raced the writer into
/// "transcript busy"). `None` when no tail runs for this exact file yet (the
/// caller falls back to the disk fold). The snapshot is at most one poll
/// behind the file, and everything after it arrives as `transcript_appended`
/// — the same contract a disk fold taken mid-poll has.
pub async fn live_page(
    session_id: &Id,
    provider: Provider,
    path: &Path,
) -> Option<(Arc<Folded>, Vec<SubagentMeta>)> {
    let live = {
        let reg = lock();
        let e = reg.get(session_id)?;
        (e.live.provider == provider && e.live.path == path).then(|| e.live.clone())?
    };
    blocking(move || {
        let mut guard = live.lock();
        let st = guard.as_mut()?;
        if live.provider == Provider::Claude && st.subagents.refresh(&live.path) {
            st.folder.set_subagents(st.subagents.tree().to_vec());
            st.snap = None;
        }
        let snap = st
            .snap
            .get_or_insert_with(|| Arc::new(st.folder.snapshot()))
            .clone();
        Some((snap, st.subagents.tree().to_vec()))
    })
    .await
}

/// Refold the whole file from record 0 (initial start, replaced file, Codex
/// era flip). The tailer resumes exactly where this read stopped — the bytes
/// of a line still being written are carried as its partial line — so a
/// record appended between the read and the resume is never skipped.
/// Blocking IO — call off the runtime.
fn refold(ctx: &ServerCtx, provider: Provider, path: &Path) -> std::io::Result<TailState> {
    refold_with(
        provider,
        path,
        crate::routes::transcript::fold_opts(ctx, provider, path),
    )
}

/// [`refold`] with explicit fold options (the server's knobs, or defaults in
/// tests).
fn refold_with(
    provider: Provider,
    path: &Path,
    opts: otto_transcript::FoldOpts<'static>,
) -> std::io::Result<TailState> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.read_to_end(&mut bytes)?;
    let records = parse_records(&bytes);
    let mut folder = Folder::new(provider, opts);
    let mut subagents = SubagentScanner::new();
    if provider == Provider::Claude {
        subagents.refresh(path);
        folder.set_subagents(subagents.tree().to_vec());
    }
    folder.seed(&records);
    let mut tailer = Tailer::at(path, bytes.len() as u64);
    tailer.partial_line = match bytes.iter().rposition(|b| *b == b'\n') {
        Some(nl) => bytes[nl + 1..].to_vec(),
        None => bytes,
    };
    Ok(TailState {
        folder,
        tailer,
        subagents,
        snap: None,
    })
}

/// One poll's worth of transcript work, produced off the runtime.
struct Step {
    turns: Vec<serde_json::Value>,
    cursor: String,
    oversize: bool,
    new_artifacts: Vec<Artifact>,
}

/// Poll the file and fold what appeared. `None` = nothing new (or a read
/// error, logged). Runs under the state lock inside `spawn_blocking`.
fn step(
    opts: &dyn Fn() -> otto_transcript::FoldOpts<'static>,
    sid: &Id,
    live: &Live,
    st: &mut TailState,
    known_artifacts: &mut HashSet<String>,
) -> Option<Step> {
    let delta = match st.tailer.poll() {
        Ok(d) => d,
        Err(e) => {
            tracing::debug!(session = %sid, "transcript tail: poll failed: {e}");
            return None;
        }
    };
    if delta.records.is_empty() && !delta.restarted {
        return None;
    }
    let prev_count = if delta.restarted {
        0
    } else {
        st.folder.record_count()
    };
    let needs_refold = delta.restarted || delta.records.iter().any(|r| st.folder.push(r));
    if needs_refold {
        *st = match refold_with(live.provider, &live.path, opts()) {
            Ok(fresh) => fresh,
            Err(e) => {
                tracing::debug!(session = %sid, "transcript tail: refold failed: {e}");
                return None;
            }
        };
    } else if live.provider == Provider::Claude && st.subagents.refresh(&live.path) {
        // Sidecars for freshly spawned subagents appear between polls; the
        // scanner only re-reads the ones that are new or changed.
        st.folder.set_subagents(st.subagents.tree().to_vec());
    }
    st.snap = None;
    let turns: Vec<serde_json::Value> = st
        .folder
        .turns_since(prev_count)
        .iter()
        .filter_map(|t| serde_json::to_value(t).ok())
        .collect();
    let cursor = st.folder.record_count().saturating_sub(1).to_string();
    // Size the frame ONCE; over the cap the client re-fetches (served from
    // this tail's memory by `live_page`).
    let size = serde_json::to_vec(&turns)
        .map(|v| v.len())
        .unwrap_or(usize::MAX);
    let new_artifacts = st
        .folder
        .artifacts()
        .iter()
        .filter(|a| known_artifacts.insert(a.id.clone()))
        .cloned()
        .collect();
    Some(Step {
        turns,
        cursor,
        oversize: size > EVENT_CAP,
        new_artifacts,
    })
}

async fn run(ctx: ServerCtx, session: Session, live: Arc<Live>) {
    let sid = session.id.clone();
    let wid = session.workspace_id.clone();
    let (cx, lv) = (ctx.clone(), live.clone());
    let known: Option<HashSet<String>> = blocking(move || {
        let st = refold(&cx, lv.provider, &lv.path).ok()?;
        let known = st.folder.artifacts().iter().map(|a| a.id.clone()).collect();
        *lv.lock() = Some(st);
        Some(known)
    })
    .await;
    let Some(mut known_artifacts) = known else {
        tracing::debug!(session = %sid, "transcript tail: initial fold failed");
        return;
    };
    let mut exited_since: Option<Instant> = None;
    let mut last_live: Option<ScreenParts> = None;
    let mut last_branch_at = Instant::now() - BRANCH_EVERY;
    let mut branch: Option<String> = None;
    let cwd = PathBuf::from(
        session
            .meta
            .get("nested_cwd")
            .and_then(|v| v.as_str())
            .unwrap_or(&session.cwd),
    );
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
            if last_branch_at.elapsed() >= BRANCH_EVERY {
                last_branch_at = Instant::now();
                let cwd = cwd.clone();
                branch = blocking(move || git_branch(&cwd)).await;
            }
            let parts = screen_parts(&h.screen_rows());
            if last_live.as_ref() != Some(&parts) {
                let _ = ctx.events.send(Event::TranscriptLive {
                    workspace_id: wid.clone(),
                    session_id: sid.clone(),
                    text: parts.draft.clone(),
                    input: parts.input.clone(),
                    status: parts.status.clone(),
                    branch: branch.clone(),
                });
                last_live = Some(parts);
            }
        }
        // Read + fold + diff + serialize: all of it off the runtime, under
        // the state lock the read route shares.
        let (cx, lv, id) = (ctx.clone(), live.clone(), sid.clone());
        let mut known = std::mem::take(&mut known_artifacts);
        let (out, known) = blocking(move || {
            let mut guard = lv.lock();
            let out = guard.as_mut().and_then(|st| {
                let opts = || crate::routes::transcript::fold_opts(&cx, lv.provider, &lv.path);
                step(&opts, &id, &lv, st, &mut known)
            });
            (out, known)
        })
        .await;
        known_artifacts = known;
        let Some(out) = out else {
            continue;
        };
        let _ = ctx.events.send(Event::TranscriptAppended {
            workspace_id: wid.clone(),
            session_id: sid.clone(),
            cursor: out.cursor,
            turns: if out.oversize { Vec::new() } else { out.turns },
        });
        for a in &out.new_artifacts {
            let _ = ctx.events.send(Event::ArtifactAdded {
                workspace_id: wid.clone(),
                session_id: sid.clone(),
                artifact: serde_json::to_value(a).unwrap_or(serde_json::Value::Null),
            });
            crate::routes::transcript::register_work_artifact(&ctx, &session, a).await;
        }
    }
}

/// Re-read `.git/HEAD` this often (a file read, no subprocess).
pub const BRANCH_EVERY: Duration = Duration::from_secs(10);

/// Current branch of `cwd` from `.git/HEAD` (walking up to the repo root;
/// worktrees' `.git` FILE is followed to its `gitdir`). Detached HEAD → the
/// short sha. No git → None.
pub fn git_branch(cwd: &Path) -> Option<String> {
    let mut dir = Some(cwd);
    let mut git_dir: Option<PathBuf> = None;
    while let Some(d) = dir {
        let g = d.join(".git");
        if g.is_dir() {
            git_dir = Some(g);
            break;
        }
        if g.is_file() {
            let s = std::fs::read_to_string(&g).ok()?;
            let target = s.trim().strip_prefix("gitdir:")?.trim();
            let p = PathBuf::from(target);
            git_dir = Some(if p.is_absolute() { p } else { d.join(p) });
            break;
        }
        dir = d.parent();
    }
    let head = std::fs::read_to_string(git_dir?.join("HEAD")).ok()?;
    let head = head.trim();
    Some(match head.strip_prefix("ref: refs/heads/") {
        Some(b) => b.to_string(),
        None => head.chars().take(8).collect(),
    })
}

/// What the tail reads off the screen each poll.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ScreenParts {
    /// The in-progress response (see [`live_draft`]).
    pub draft: String,
    /// Unsent text in the input box.
    pub input: String,
    /// Status rows below the input box, joined by " · ".
    pub status: String,
}

/// Split the screen into the response draft, the input box text and the
/// status rows. Input rows start at the last `❯`/`│ >` row and run until the
/// closing rule (or the first status-looking row); everything after is status.
pub fn screen_parts(rows: &[String]) -> ScreenParts {
    fn is_rule(r: &str) -> bool {
        let t = r.trim();
        t.len() >= 8 && t.chars().all(|c| matches!(c, '─' | '━' | '╌' | '-' | '═'))
    }
    fn is_input(r: &str) -> bool {
        let t = r.trim_start();
        t.starts_with('❯') || t.starts_with("│ >") || t.starts_with("│ ❯")
    }
    fn strip_prompt(r: &str) -> &str {
        let t = r.trim_start();
        t.trim_start_matches('│')
            .trim_start()
            .trim_start_matches(['❯', '>'])
            .trim_end_matches('│')
            .trim()
    }
    let draft = live_draft(rows);
    let Some(i) = rows.iter().rposition(|r| is_input(r)) else {
        return ScreenParts {
            draft,
            ..Default::default()
        };
    };
    // Input: the prompt row plus continuation rows until a rule / box edge /
    // blank row.
    let mut input_lines = vec![strip_prompt(&rows[i]).to_string()];
    let mut j = i + 1;
    while j < rows.len() {
        let r = &rows[j];
        if is_rule(r) || r.trim().is_empty() || r.trim_start().starts_with('╰') {
            break;
        }
        input_lines.push(strip_prompt(r).to_string());
        j += 1;
    }
    let input = input_lines.join("\n").trim().to_string();
    let status: Vec<String> = rows[j..]
        .iter()
        .map(|r| r.trim())
        .filter(|r| !r.is_empty() && !is_rule(r) && !r.starts_with('╰'))
        .map(|r| r.split_whitespace().collect::<Vec<_>>().join(" "))
        .collect();
    ScreenParts {
        draft,
        input,
        status: status.join(" · "),
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
        t.starts_with('❯')
            || t.starts_with("│ >")
            || t.starts_with("│ ❯")
            || t.starts_with("╭─")
            || t.starts_with("╰─")
    }
    fn is_echo(r: &str) -> bool {
        let t = r.trim_start();
        (t.starts_with("> ") || t.starts_with("› ")) && !t.starts_with("> >")
    }
    // Ephemeral rows: spinners, elapsed timers ("Running… (7m 34s · timeout
    // 10m)"), key hints and tips. They change every second and would make
    // the draft re-render (and the chat jump) without carrying content.
    fn has_timer(t: &str) -> bool {
        let b = t.as_bytes();
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'(' {
                let mut j = i + 1;
                while j < b.len() && b[j].is_ascii_digit() {
                    j += 1;
                }
                if j > i + 1 && j < b.len() && matches!(b[j], b'h' | b'm' | b's') {
                    return true;
                }
            }
            i += 1;
        }
        false
    }
    fn is_spinner(r: &str) -> bool {
        let t = r.trim_start();
        t.contains("esc to interrupt")
            || t.contains("(esc to")
            || t.contains("ctrl+b to run in background")
            || t.starts_with("Tip:")
            || t.starts_with("※ Tip:")
            || (t.contains('…') && has_timer(t))
            || t.chars()
                .next()
                .is_some_and(|c| "✻✶✳✢✽⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏".contains(c))
                && t.contains('…')
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
    let start = content
        .iter()
        .rposition(|r| is_echo(r))
        .map(|i| i + 1)
        .unwrap_or(0);
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
        assert_eq!(
            d,
            "⏺ Still exploring. Reading the DAO.\n\n⏺ Bash(cd x && grep -n foo)\n  ⎿  3 lines"
        );
    }

    #[test]
    fn live_draft_drops_timer_hint_and_tip_rows() {
        let screen = rows(
            "> go\n\n⏺ Bash(cargo test)\n  ⎿  Running… (7m 34s · timeout 10m)\n     (ctrl+b to run in background)\n\n  Tip: Use /clear to start fresh\n\n────────────────\n❯ ",
        );
        assert_eq!(live_draft(&screen), "⏺ Bash(cargo test)");
    }

    #[test]
    fn live_draft_is_empty_right_after_a_prompt_and_tolerates_no_box() {
        assert_eq!(live_draft(&rows("> hi\n\n❯ ")), "");
        assert_eq!(
            live_draft(&rows("plain output\nmore")),
            "plain output\nmore"
        );
        assert_eq!(live_draft(&[]), "");
    }

    #[test]
    fn screen_parts_splits_input_and_status_rows() {
        let screen = rows(
            "> hi\n\n⏺ working\n\n────────────────────────────\n❯ option 2, other services   \n────────────────────────────\n  ~ | Fable 5.1 | ▓▓░░ 11%\n  -- INSERT --  ▶▶ bypass permissions on",
        );
        let p = screen_parts(&screen);
        assert_eq!(p.draft, "⏺ working");
        assert_eq!(p.input, "option 2, other services");
        assert_eq!(
            p.status,
            "~ | Fable 5.1 | ▓▓░░ 11% · -- INSERT -- ▶▶ bypass permissions on"
        );
        // No input box → draft only.
        let p = screen_parts(&rows("just text"));
        assert_eq!(p.input, "");
        assert_eq!(p.status, "");
    }

    #[test]
    fn git_branch_reads_head_and_follows_worktree_gitdir() {
        let tmp = tempfile::tempdir().unwrap();
        let repo = tmp.path().join("repo");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::write(repo.join(".git/HEAD"), "ref: refs/heads/feat/x\n").unwrap();
        let sub = repo.join("crates/a");
        std::fs::create_dir_all(&sub).unwrap();
        assert_eq!(git_branch(&sub).as_deref(), Some("feat/x"));
        // Worktree: `.git` is a file pointing at the gitdir.
        let wt = tmp.path().join("wt");
        let gd = tmp.path().join("gitdir");
        std::fs::create_dir_all(&wt).unwrap();
        std::fs::create_dir_all(&gd).unwrap();
        std::fs::write(wt.join(".git"), format!("gitdir: {}\n", gd.display())).unwrap();
        std::fs::write(gd.join("HEAD"), "0123456789abcdef\n").unwrap();
        assert_eq!(git_branch(&wt).as_deref(), Some("01234567"));
        assert_eq!(git_branch(tmp.path()), None);
    }

    #[test]
    fn live_draft_caps_to_the_tail() {
        let big: Vec<String> = (0..2000)
            .map(|i| format!("line {i} {}", "x".repeat(20)))
            .collect();
        let d = live_draft(&big);
        assert!(d.len() <= LIVE_CAP + 4);
        assert!(d.starts_with('…'));
        assert!(d.ends_with("line 1999 xxxxxxxxxxxxxxxxxxxx"));
    }

    fn copy_dir(from: &Path, to: &Path) {
        std::fs::create_dir_all(to).unwrap();
        for e in std::fs::read_dir(from).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                copy_dir(&p, &to.join(e.file_name()));
            } else {
                std::fs::copy(&p, to.join(e.file_name())).unwrap();
            }
        }
    }

    /// The tail folds a file that grows in odd-sized appends (lines split
    /// anywhere, the initial fold landing mid-line) and must end exactly where
    /// a one-shot fold of the finished file does — every record once, the
    /// partial line carried, the delta cursor contiguous, sub-agent sidecars
    /// attached — without re-reading the file.
    #[test]
    fn stepping_a_growing_file_matches_a_whole_file_fold() {
        use std::io::Write;
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../otto-transcript/fixtures");
        for (provider, sub) in [(Provider::Claude, "claude"), (Provider::Codex, "codex-new")] {
            let mut files: Vec<PathBuf> = std::fs::read_dir(root.join(sub))
                .unwrap()
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|x| x == "jsonl"))
                .collect();
            files.sort();
            for src in files {
                let dir = tempfile::tempdir().unwrap();
                let path = dir.path().join(src.file_name().unwrap());
                let side = src.with_extension("");
                if side.is_dir() {
                    copy_dir(&side, &path.with_extension(""));
                }
                let bytes = std::fs::read(&src).unwrap();
                let head = bytes.len() / 3;
                std::fs::write(&path, &bytes[..head]).unwrap();
                let live = Live {
                    provider,
                    path: path.clone(),
                    state: Mutex::new(None),
                };
                let mut st = refold_with(provider, &path, Default::default()).unwrap();
                let mut known: HashSet<String> =
                    st.folder.artifacts().iter().map(|a| a.id.clone()).collect();
                let opts = || otto_transcript::FoldOpts::default();
                let mut f = std::fs::OpenOptions::new()
                    .append(true)
                    .open(&path)
                    .unwrap();
                let mut at = head;
                for n in [1usize, 97, 4096, 13, 20_000, 7].iter().cycle() {
                    if at >= bytes.len() {
                        break;
                    }
                    let end = (at + n).min(bytes.len());
                    f.write_all(&bytes[at..end]).unwrap();
                    f.flush().unwrap();
                    at = end;
                    let before = st.folder.record_count();
                    if let Some(out) = step(&opts, &"t".into(), &live, &mut st, &mut known) {
                        assert_eq!(
                            out.cursor,
                            st.folder.record_count().saturating_sub(1).to_string()
                        );
                        assert!(st.folder.record_count() > before || out.turns.is_empty());
                    }
                }
                let want = otto_transcript::fold_file(provider, &path, {
                    let mut o = otto_transcript::FoldOpts::default();
                    if provider == Provider::Claude {
                        o.subagents = otto_transcript::read_subagents(&path);
                    }
                    o
                })
                .unwrap();
                let got = st.folder.snapshot();
                assert_eq!(got.record_count, want.record_count, "{}", src.display());
                assert_eq!(
                    serde_json::to_value(got.turns_since(0)).unwrap(),
                    serde_json::to_value(want.turns_since(0)).unwrap(),
                    "{}",
                    src.display()
                );
                let ids = |f: &Folded| f.artifacts.iter().map(|a| a.id.clone()).collect::<Vec<_>>();
                assert_eq!(ids(&got), ids(&want));
                assert_eq!(known.len(), want.artifacts.len());
            }
        }
    }

    #[test]
    fn slot_guard_frees_the_registry_entry_on_drop() {
        let id: Id = "tail-test-slot".into();
        lock().insert(
            id.clone(),
            Entry {
                last_touch: Instant::now(),
                stop: Arc::new(AtomicBool::new(false)),
                live: Arc::new(Live {
                    provider: Provider::Claude,
                    path: PathBuf::from("/nonexistent.jsonl"),
                    state: Mutex::new(None),
                }),
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
        assert_eq!(MAX_TAILS, 64);
    }
}
