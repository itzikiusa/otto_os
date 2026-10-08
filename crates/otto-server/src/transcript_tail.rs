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
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use otto_core::domain::Session;
use otto_core::event::Event;
use otto_core::Id;
use otto_transcript::{Artifact, Folded, Folder, Provider, SubagentScanner, Tailer, Transcript};

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
/// `transcript_appended` payloads above this are first shrunk
/// ([`trim_oversized`]); only a delta still above it after that is sent with
/// `turns: []` (the client then re-fetches the page).
pub const EVENT_CAP: usize = 64 * 1024;
/// An oversized delta keeps this many bytes of each tool result's `text`
/// (then `patch`), marked `elided: true`; the full result is one
/// `GET …/transcript/tool/{tool_id}` away (SA-04).
pub const TRIM_TEXT: usize = 4 * 1024;
/// `transcript_live` drafts are capped to this many bytes (tail kept).
pub const LIVE_CAP: usize = 16 * 1024;

/// The tail's fold state. Owned by a std `Mutex` that is only ever locked
/// inside `spawn_blocking` (the poll step and [`live_page`]), so neither a
/// fold nor a snapshot ever runs on a runtime worker.
struct TailState {
    folder: Folder<'static>,
    tailer: Tailer,
    subagents: SubagentScanner,
    budget: TailBudget,
    retired: bool,
    sidecar_charge: usize,
}

const TAIL_INPUT_CAP: usize = 8 * 1024 * 1024;
const TAIL_RECORD_CAP: usize = 16_384;
const TAIL_BYTES_CAP: usize = 32 * 1024 * 1024;
const TOTAL_TAIL_BYTES: usize = 128 * 1024 * 1024;
/// Oversized transcripts keep a metadata watcher; coalesce invalidations so
/// a fast producer cannot force a whole-file offline fold every 700 ms.
const FALLBACK_REFRESH: Duration = Duration::from_secs(15);
static RETAINED_CHARGE: AtomicUsize = AtomicUsize::new(0);

/// Conservative source/record charge, reserved BEFORE adding to the fold.
/// This is bounded accounting, not a claim about allocator RSS.
#[derive(Default)]
struct TailBudget {
    bytes: usize,
}
impl TailBudget {
    fn reserve(&mut self, bytes: usize) -> std::io::Result<()> {
        if bytes > TAIL_BYTES_CAP {
            return Err(budget_error());
        }
        if bytes <= self.bytes {
            return Ok(());
        }
        let extra = bytes - self.bytes;
        reserve_total(&RETAINED_CHARGE, extra)?;
        self.bytes = bytes;
        Ok(())
    }
}
impl Drop for TailBudget {
    fn drop(&mut self) {
        RETAINED_CHARGE.fetch_sub(self.bytes, Ordering::AcqRel);
    }
}
fn reserve_total(total: &AtomicUsize, extra: usize) -> std::io::Result<()> {
    total
        .try_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            current
                .checked_add(extra)
                .filter(|n| *n <= TOTAL_TAIL_BYTES)
        })
        .map(|_| ())
        .map_err(|_| budget_error())
}
fn budget_error() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::OutOfMemory,
        "transcript live fold budget exceeded",
    )
}
fn fold_charge(bytes: usize, records: usize) -> usize {
    bytes
        .saturating_mul(4)
        .saturating_add(records.saturating_mul(2048))
}

/// What a running tail shares with the read route: the file it folds and its
/// state (`None` until the initial fold lands).
struct Live {
    provider: Provider,
    path: PathBuf,
    state: Mutex<Option<TailState>>,
    /// Flips to `true` once the initial fold settled (landed or failed), so a
    /// read that just armed the tail can wait for it ([`live_page_settled`]).
    settled: tokio::sync::watch::Sender<bool>,
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
    live: Arc<Live>,
}

impl Drop for Slot {
    fn drop(&mut self) {
        let mut reg = lock();
        if reg
            .get(&self.id)
            .is_some_and(|e| Arc::ptr_eq(&e.live, &self.live))
        {
            reg.remove(&self.id);
        }
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
        if e.live.provider == provider && e.live.path == path {
            e.last_touch = Instant::now();
            return;
        }
        // A nested CLI can replace its transcript without changing the shell
        // session ID. Retire that generation and reuse its registry slot.
        e.stop.store(true, Ordering::Relaxed);
    }
    if reg.len() >= MAX_TAILS && !reg.contains_key(&session.id) {
        tracing::debug!(session = %session.id, "transcript tail: cap reached, no live push");
        return;
    }
    let stop = Arc::new(AtomicBool::new(false));
    let live = Arc::new(Live {
        provider,
        path: path.to_path_buf(),
        state: Mutex::new(None),
        settled: tokio::sync::watch::Sender::new(false),
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
        live: live.clone(),
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
fn should_continue(sid: &Id, live: &Arc<Live>) -> bool {
    let mut reg = lock();
    let keep = match reg.get(sid) {
        None => false,
        Some(e) if !Arc::ptr_eq(&e.live, live) => return false,
        Some(e) => !e.stop.load(Ordering::Relaxed) && e.last_touch.elapsed() < IDLE_STOP,
    };
    if !keep {
        reg.remove(sid);
    }
    keep
}

/// Serialize the final ownership check and synchronous publication with
/// replacement. Never run blocking work or await while holding this guard.
fn while_current(sid: &Id, live: &Arc<Live>, publish: impl FnOnce()) -> bool {
    let reg = lock();
    if !reg.get(sid).is_some_and(|e| Arc::ptr_eq(&e.live, live)) {
        return false;
    }
    publish();
    true
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
    before: Option<usize>,
    limit: usize,
) -> Option<Transcript> {
    page_of(live_of(session_id, provider, path)?, before, limit).await
}

/// How long a read waits for a just-armed tail's initial fold before it
/// folds the file itself (a 65 MB transcript folds in ~0.15 s).
pub const SETTLE_WAIT: Duration = Duration::from_secs(15);

/// [`live_page`] for a read that has just [`touch`]ed the tail: waits (at most
/// [`SETTLE_WAIT`]) for the tail's initial fold instead of folding the file a
/// second time on the read path. `None` when no tail runs for this file or
/// its initial fold failed — the caller falls back to the fold cache.
pub async fn live_page_settled(
    session_id: &Id,
    provider: Provider,
    path: &Path,
    before: Option<usize>,
    limit: usize,
) -> Option<Transcript> {
    let live = live_of(session_id, provider, path)?;
    let mut settled = live.settled.subscribe();
    let _ = tokio::time::timeout(SETTLE_WAIT, settled.wait_for(|s| *s)).await;
    page_of(live, before, limit).await
}

/// The running tail of `session_id` when it folds exactly `path`.
fn live_of(session_id: &Id, provider: Provider, path: &Path) -> Option<Arc<Live>> {
    let reg = lock();
    let e = reg.get(session_id)?;
    (e.live.provider == provider && e.live.path == path).then(|| e.live.clone())
}

async fn page_of(live: Arc<Live>, before: Option<usize>, limit: usize) -> Option<Transcript> {
    blocking(move || {
        let mut guard = live.lock();
        let st = guard.as_mut()?;
        if refresh_subagents(live.provider, &live.path, st).is_err() {
            *guard = None;
            return None;
        }
        Some(st.folder.page(
            before,
            limit,
            if before.is_none() {
                st.subagents.tree().to_vec()
            } else {
                vec![]
            },
        ))
    })
    .await
}

/// Tool expansion copies just its containing block, never the whole fold.
pub async fn live_tool(
    session_id: &Id,
    provider: Provider,
    path: &Path,
    tool_id: String,
) -> Option<Option<otto_transcript::Block>> {
    let live = live_of(session_id, provider, path)?;
    blocking(move || {
        let guard = live.lock();
        let st = guard.as_ref()?;
        Some(st.folder.tool_block(&tool_id))
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
    let file = std::fs::File::open(path)?;
    if file.metadata()?.len() > TAIL_INPUT_CAP as u64 {
        return Err(budget_error());
    }
    let mut bytes = Vec::new();
    file.take(TAIL_INPUT_CAP as u64 + 1)
        .read_to_end(&mut bytes)?;
    let records = bytes.iter().filter(|b| **b == b'\n').count();
    if bytes.len() > TAIL_INPUT_CAP || records > TAIL_RECORD_CAP {
        return Err(budget_error());
    }
    let mut budget = TailBudget::default();
    let sidecar_charge = if provider == Provider::Claude {
        otto_transcript::subagent_charge(path)?
    } else {
        0
    };
    budget.reserve(fold_charge(bytes.len(), records).saturating_add(sidecar_charge))?;
    let mut folder = Folder::new(provider, opts);
    let mut subagents = SubagentScanner::new();
    if provider == Provider::Claude {
        subagents.refresh(path);
        folder.set_subagents(subagents.tree().to_vec());
    }
    folder.seed_bytes(&bytes);
    let mut tailer = Tailer::at(path, bytes.len() as u64);
    tailer.partial_line = match bytes.iter().rposition(|b| *b == b'\n') {
        Some(nl) => bytes[nl + 1..].to_vec(),
        None => bytes,
    };
    Ok(TailState {
        folder,
        tailer,
        subagents,
        budget,
        retired: false,
        sidecar_charge,
    })
}

/// Reserve sidecar payload alongside turns before the scanner clones metadata.
fn refresh_subagents(provider: Provider, path: &Path, st: &mut TailState) -> std::io::Result<()> {
    if provider != Provider::Claude || !st.subagents.needs_refresh(path) {
        return Ok(());
    }
    let charge = otto_transcript::subagent_charge(path)?;
    st.budget.reserve(
        fold_charge(st.tailer.offset as usize, st.folder.record_count()).saturating_add(charge),
    )?;
    st.sidecar_charge = charge;
    if st.subagents.refresh(path) {
        st.folder.set_subagents(st.subagents.tree().to_vec());
    }
    Ok(())
}

/// Serialized size of `turns`; `usize::MAX` if they do not serialize.
fn json_size(turns: &[serde_json::Value]) -> usize {
    serde_json::to_vec(turns)
        .map(|v| v.len())
        .unwrap_or(usize::MAX)
}

/// Cut `s` to at most `max` bytes on a char boundary. `true` when cut.
fn cut_str(s: &mut String, max: usize) -> bool {
    if s.len() <= max {
        return false;
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    true
}

/// Elide `key` (`text` / `patch`) of every tool result in `turns` longer than
/// `keep` bytes: cut it and mark the result `elided: true`. `truncated` and
/// `bytes` keep describing the stored result (the 64 KB fold cap and the
/// original size), so the UI can tell "shortened for the live push" (fetch
/// the rest) from "capped on disk" (nothing more to fetch).
fn elide_results(turns: &mut [serde_json::Value], key: &str, keep: usize) {
    for turn in turns {
        let Some(blocks) = turn.get_mut("blocks").and_then(|b| b.as_array_mut()) else {
            continue;
        };
        for block in blocks {
            if block.get("kind").and_then(|k| k.as_str()) != Some("tool_call") {
                continue;
            }
            let Some(result) = block.get_mut("result").and_then(|r| r.as_object_mut()) else {
                continue;
            };
            let cut = match result.get_mut(key) {
                Some(serde_json::Value::String(s)) => cut_str(s, keep),
                _ => false,
            };
            if cut {
                result.insert("elided".into(), serde_json::Value::Bool(true));
            }
        }
    }
}

/// Keep a `transcript_appended` delta under [`EVENT_CAP`] when possible by
/// eliding tool-result bodies in rounds — `text` before `patch` (a diff is
/// worth keeping whole while shorter texts make it fit), at [`TRIM_TEXT`],
/// then 1 KB, 256 B and finally nothing (a Codex answer turn carries every
/// tool call of its user turn, so dozens of 4 KB previews can still
/// overflow). Stops at the first round that fits. Returns the final
/// serialized size — above the cap only when prose/inputs alone exceed it.
fn trim_oversized(turns: &mut [serde_json::Value]) -> usize {
    let mut size = json_size(turns);
    for keep in [TRIM_TEXT, 1024, 256, 0] {
        for key in ["text", "patch"] {
            if size <= EVENT_CAP {
                return size;
            }
            elide_results(turns, key, keep);
            size = json_size(turns);
        }
    }
    size
}

/// The full `tool_call` block `tool_id` from a fold (newest turn first — the
/// lazy load is for a step the live push just elided). `None` when absent.
pub fn find_tool_block(folded: &Folded, tool_id: &str) -> Option<otto_transcript::Block> {
    folded.turns.iter().rev().find_map(|t| {
        t.turn.blocks.iter().find_map(|b| match b {
            otto_transcript::Block::ToolCall { id, .. } if id == tool_id => Some(b.clone()),
            _ => None,
        })
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
    let source_len = match std::fs::metadata(&live.path) {
        Ok(meta) => meta.len(),
        Err(_) => return None,
    };
    // Reserve the incoming source footprint before Tailer allocates/parses
    // the burst. Its prefix read leaves growth after this stat for next time.
    if source_len > TAIL_INPUT_CAP as u64
        || st
            .budget
            .reserve(
                fold_charge(source_len as usize, st.folder.record_count())
                    .saturating_add(st.sidecar_charge),
            )
            .is_err()
    {
        st.retired = true;
        return Some(Step {
            turns: vec![],
            cursor: st.folder.record_count().to_string(),
            oversize: true,
            new_artifacts: vec![],
        });
    }
    let delta = match st.tailer.poll_up_to(
        source_len,
        TAIL_RECORD_CAP.saturating_sub(st.folder.record_count()),
    ) {
        Ok(d) => d,
        Err(e) if e.kind() == std::io::ErrorKind::OutOfMemory => {
            st.retired = true;
            return Some(Step {
                turns: vec![],
                cursor: st.folder.record_count().to_string(),
                oversize: true,
                new_artifacts: vec![],
            });
        }
        Err(e) => {
            tracing::debug!(session = %sid, "transcript tail: poll failed: {e}");
            return None;
        }
    };
    if delta.records.is_empty() && !delta.restarted {
        return None;
    }
    if st
        .budget
        .reserve(
            fold_charge(
                st.tailer.offset as usize,
                st.folder.record_count().saturating_add(delta.records.len()),
            )
            .saturating_add(st.sidecar_charge),
        )
        .is_err()
    {
        st.retired = true;
        return Some(Step {
            turns: vec![],
            cursor: st.folder.record_count().to_string(),
            oversize: true,
            new_artifacts: vec![],
        });
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
                st.retired = true;
                return Some(Step {
                    turns: vec![],
                    cursor: st.folder.record_count().to_string(),
                    oversize: true,
                    new_artifacts: vec![],
                });
            }
        };
    } else if refresh_subagents(live.provider, &live.path, st).is_err() {
        st.retired = true;
        return Some(Step {
            turns: vec![],
            cursor: st.folder.record_count().to_string(),
            oversize: true,
            new_artifacts: vec![],
        });
    }
    let mut turns: Vec<serde_json::Value> = st
        .folder
        .turns_since(prev_count)
        .iter()
        .filter_map(|t| serde_json::to_value(t).ok())
        .collect();
    let cursor = st.folder.record_count().saturating_sub(1).to_string();
    // Size the frame; over the cap, elide tool-result bodies first (the one
    // part of a turn that routinely reaches 64 KB) and only when even that
    // does not fit send `turns: []` so the client re-fetches (served from
    // this tail's memory by `live_page`).
    let size = trim_oversized(&mut turns);
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

/// Poll a degraded tail at a bounded rate. File replacement at equal length
/// still invalidates through mtime; repeated unchanged polls emit nothing.
fn fallback_step(
    path: &Path,
    previous: &mut Option<(u64, Option<std::time::SystemTime>)>,
    last: &mut Instant,
    now: Instant,
) -> Option<Step> {
    if now.duration_since(*last) < FALLBACK_REFRESH {
        return None;
    }
    *last = now;
    let stamp = std::fs::metadata(path)
        .ok()
        .map(|m| (m.len(), m.modified().ok()));
    let changed = stamp != *previous;
    *previous = stamp;
    changed.then(|| Step {
        turns: vec![],
        cursor: stamp.map(|s| s.0).unwrap_or(0).to_string(),
        oversize: true,
        new_artifacts: vec![],
    })
}

async fn run(ctx: ServerCtx, session: Session, live: Arc<Live>) {
    let sid = session.id.clone();
    let wid = session.workspace_id.clone();
    let (cx, lv) = (ctx.clone(), live.clone());
    let permit = crate::transcript_cache::fold_workers()
        .acquire_owned()
        .await;
    if !should_continue(&sid, &live) {
        live.settled.send_replace(true);
        return;
    }
    let known: Option<HashSet<String>> = blocking(move || {
        let _permit = permit;
        let st = refold(&cx, lv.provider, &lv.path).ok()?;
        let known = st.folder.artifacts().iter().map(|a| a.id.clone()).collect();
        *lv.lock() = Some(st);
        Some(known)
    })
    .await;
    // Landed or failed, the reads waiting on it may go (a failure falls back
    // to the fold cache).
    live.settled.send_replace(true);
    if !should_continue(&sid, &live) {
        return;
    }
    let mut known_artifacts = known.unwrap_or_default();
    let mut fallback_stamp = None;
    let mut fallback_at = Instant::now();
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
        if !should_continue(&sid, &live) {
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
            if !should_continue(&sid, &live) {
                break;
            }
            let parts = screen_parts(&h.screen_rows());
            if last_live.as_ref() != Some(&parts) {
                if !while_current(&sid, &live, || {
                    crate::ws_fanout::publish_stream(
                        &ctx.events,
                        Event::TranscriptLive {
                            workspace_id: wid.clone(),
                            session_id: sid.clone(),
                            text: parts.draft.as_str().into(),
                            input: parts.input.as_str().into(),
                            status: parts.status.as_str().into(),
                            branch: branch.clone(),
                        },
                    )
                }) {
                    break;
                }
                last_live = Some(parts);
            }
        }
        // Read + fold + diff + serialize: all of it off the runtime, under
        // the state lock the read route shares.
        let (cx, lv, id) = (ctx.clone(), live.clone(), sid.clone());
        let mut known = std::mem::take(&mut known_artifacts);
        let (out, known, stamp, stamp_at) = blocking(move || {
            let mut guard = lv.lock();
            if guard.is_none() {
                known.clear();
            }
            let out = if let Some(st) = guard.as_mut() {
                let opts = || crate::routes::transcript::fold_opts(&cx, lv.provider, &lv.path);
                let out = step(&opts, &id, &lv, st, &mut known);
                if st.retired {
                    *guard = None;
                    known.clear();
                }
                out
            } else {
                fallback_step(
                    &lv.path,
                    &mut fallback_stamp,
                    &mut fallback_at,
                    Instant::now(),
                )
            };
            (out, known, fallback_stamp, fallback_at)
        })
        .await;
        if !should_continue(&sid, &live) {
            break;
        }
        known_artifacts = known;
        fallback_stamp = stamp;
        fallback_at = stamp_at;
        let Some(out) = out else {
            continue;
        };
        if !while_current(&sid, &live, || {
            crate::ws_fanout::publish_stream(
                &ctx.events,
                Event::TranscriptAppended {
                    workspace_id: wid.clone(),
                    session_id: sid.clone(),
                    cursor: out.cursor,
                    turns: if out.oversize {
                        Vec::new().into()
                    } else {
                        out.turns.into()
                    },
                },
            )
        }) {
            break;
        }
        for a in &out.new_artifacts {
            if !while_current(&sid, &live, || {
                let _ = ctx.events.send(Event::ArtifactAdded {
                    workspace_id: wid.clone(),
                    session_id: sid.clone(),
                    artifact: serde_json::to_value(a).unwrap_or(serde_json::Value::Null),
                });
            }) {
                break;
            }
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
#[path = "../tests/unit/transcript_tail.rs"]
#[allow(clippy::disallowed_methods)] // tests use temporary files
mod tests;
