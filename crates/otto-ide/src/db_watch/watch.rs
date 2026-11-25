//! The daemon-owned DB Watch engine (`/api/v1/ide/db-watch/watches`).
//!
//! A watch reads 1–8 tables / collections / Redis key patterns of ONE target
//! at each checkpoint and keeps, per item, the baseline (checkpoint 0), the
//! previous and the latest snapshot — raw, in daemon memory only. Diffs run on
//! the raw values; only what an agent reads is masked. A watch bound to a
//! debug session checkpoints on every stop (`DapManager::watch_stops`); one
//! bound to a run, when the run leaves `running`.
//!
//! Checkpoints are single-flight per watch: a stop that arrives while one is
//! running waits in ONE pending slot (the newest wins), so 20 stops in 2 s
//! against a slow database become a few checkpoints, never a queue, never out
//! of order. Items read with at most 4 queries in flight on a local target and
//! 2 on a saved one (its pool is shared with DB Explorer).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use futures_util::{stream, StreamExt};
use otto_core::auth::{request_credential, with_request_credential, BoxFuture, RequestCredential};
use otto_core::domain::User;
use otto_core::event::Event;
use otto_core::{Error, Result};
use serde_json::Value;
use tokio::sync::{broadcast, watch};

use super::diff::{diff_snapshots, DiffOpts};
use super::{
    build_watch_query, DbWatchExecOutcome, DbWatchOp, DbWatchQueryExec, DbWatchQueryTarget,
    IdeDbEngine, IdeDbFieldChange, IdeDbRowDiff, IdeDbWatch, IdeDbWatchBind,
    IdeDbWatchCheckpointInfo, IdeDbWatchCheckpointReason, IdeDbWatchCompare, IdeDbWatchItemDiff,
    IdeDbWatchItemInfo, IdeDbWatchItemSpec, IdeDbWatchReadResp, IdeDbWatchSnapshotItem,
    IdeDbWatchTargetInfo, MAX_DB_WATCH_LIMIT,
};
use crate::dap::DapStopSignal;
use crate::types::{IdeRunState, IdeRunStatus};

/// Items one watch reads.
pub const MAX_WATCH_ITEMS: usize = 8;
/// Watches one user keeps (the least recently used goes).
const MAX_WATCHES_PER_USER: usize = 4;
/// A watch outlives its ended debug session / run this long.
const ENDED_TTL: Duration = Duration::from_secs(600);
/// Queries in flight per checkpoint: local target / saved (shared pool).
const LOCAL_PARALLEL: usize = 4;
const SAVED_PARALLEL: usize = 2;
/// Outer bound of one item read (the server-side cap is 10 s).
const ITEM_TIMEOUT: Duration = Duration::from_secs(15);
/// Redis items enrich at most this many keys (`REDIS_WATCH_ENRICH_CAP`).
const REDIS_WINDOW: usize = 50;
/// Longer text cells are kept as a prefix + a hash of the whole value.
const MAX_CELL_BYTES: usize = 4096;

/// What the engine needs from the daemon: the read-only executor and the
/// event bus ([`crate::IdeCtx`] in production, a fake in tests).
pub trait WatchHost: Send + Sync + 'static {
    fn exec<'a>(
        &'a self,
        user: &'a User,
        exec: DbWatchQueryExec,
    ) -> BoxFuture<'a, Result<DbWatchExecOutcome>>;
    fn emit(&self, event: Event);
}

/// A resolved watch target: what the UI sees and what the executor runs on.
#[derive(Clone)]
pub struct WatchTarget {
    pub info: IdeDbWatchTargetInfo,
    pub exec: DbWatchQueryTarget,
}

/// A checkpoint request.
#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub reason: IdeDbWatchCheckpointReason,
    pub label: Option<String>,
    pub file: Option<String>,
    pub line: Option<u32>,
}

impl Checkpoint {
    pub fn new(reason: IdeDbWatchCheckpointReason) -> Self {
        Self {
            reason,
            label: None,
            file: None,
            line: None,
        }
    }
}

#[derive(Clone)]
struct Snap {
    seq: u32,
    data: Arc<IdeDbWatchSnapshotItem>,
    window_full: bool,
}

struct Item {
    spec: IdeDbWatchItemSpec,
    info: IdeDbWatchItemInfo,
    described: bool,
    query: String,
    base: Option<Snap>,
    prev: Option<Snap>,
    latest: Option<Snap>,
    /// The latest read failed (its snapshots are older than the checkpoint).
    read_error: Option<String>,
}

impl Item {
    fn new(spec: IdeDbWatchItemSpec, engine: IdeDbEngine) -> Self {
        let info = IdeDbWatchItemInfo {
            object: spec.object.clone(),
            kind: engine.object_kind(),
            filter: spec.filter.clone(),
            primary_key: Vec::new(),
            row_estimate: None,
            error: None,
        };
        Self {
            spec,
            info,
            described: false,
            query: String::new(),
            base: None,
            prev: None,
            latest: None,
            read_error: None,
        }
    }
}

struct State {
    target: WatchTarget,
    database: Option<String>,
    items: Vec<Item>,
    bind: Option<IdeDbWatchBind>,
    bound_ended_at_ms: Option<i64>,
    checkpoint_count: u32,
    latest: Option<IdeDbWatchCheckpointInfo>,
    busy: bool,
    last_used: Instant,
}

/// One user's watch of one project root.
pub struct Watch {
    pub id: String,
    user: User,
    /// The creator's request credential: background checkpoints read as it.
    cred: RequestCredential,
    root: String,
    created_at_ms: i64,
    run: tokio::sync::Mutex<()>,
    pending: Mutex<Option<Checkpoint>>,
    state: Mutex<State>,
    binder: Mutex<Option<tokio::task::AbortHandle>>,
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// 1..=8 distinct, non-empty objects.
fn check_items(items: &[IdeDbWatchItemSpec]) -> Result<()> {
    if items.is_empty() || items.len() > MAX_WATCH_ITEMS {
        return Err(Error::Invalid(format!(
            "a watch reads 1 to {MAX_WATCH_ITEMS} tables (got {})",
            items.len()
        )));
    }
    let mut seen = std::collections::HashSet::new();
    for i in items {
        if i.object.trim().is_empty() || !seen.insert(i.object.trim()) {
            return Err(Error::Invalid(format!(
                "each watched table appears once (\"{}\")",
                i.object
            )));
        }
    }
    Ok(())
}

impl Watch {
    /// The caller's request credential is kept for background checkpoints.
    pub fn new(
        user: &User,
        root: &str,
        target: WatchTarget,
        database: Option<String>,
        items: Vec<IdeDbWatchItemSpec>,
    ) -> Result<Arc<Self>> {
        check_items(&items)?;
        let engine = target.info.engine;
        Ok(Arc::new(Self {
            id: format!("dbw_{}", uuid::Uuid::new_v4().simple()),
            user: user.clone(),
            cred: request_credential(),
            root: root.to_string(),
            created_at_ms: now_ms(),
            run: tokio::sync::Mutex::new(()),
            pending: Mutex::new(None),
            state: Mutex::new(State {
                target,
                database,
                items: items.into_iter().map(|s| Item::new(s, engine)).collect(),
                bind: None,
                bound_ended_at_ms: None,
                checkpoint_count: 0,
                latest: None,
                busy: false,
                last_used: Instant::now(),
            }),
            binder: Mutex::new(None),
        }))
    }

    pub fn root(&self) -> &str {
        &self.root
    }

    pub fn target(&self) -> WatchTarget {
        lock(&self.state).target.clone()
    }

    /// Replace the item list; items kept by `object` keep their snapshots.
    /// New ones get their baseline at once ([`Self::baseline_new_items`]).
    pub async fn set_items(&self, items: Vec<IdeDbWatchItemSpec>) -> Result<()> {
        check_items(&items)?;
        let _run = self.run.lock().await;
        let mut st = lock(&self.state);
        let engine = st.target.info.engine;
        let mut old: Vec<Item> = std::mem::take(&mut st.items);
        st.items = items
            .into_iter()
            .map(|spec| match old.iter().position(|i| i.spec == spec) {
                Some(idx) => old.swap_remove(idx),
                None => Item::new(spec, engine),
            })
            .collect();
        Ok(())
    }

    /// Bind to a debug session or a run, dropping any earlier binding.
    pub fn set_binding(
        &self,
        bind: Option<IdeDbWatchBind>,
        task: Option<tokio::task::AbortHandle>,
    ) {
        if let Some(old) = std::mem::replace(&mut *lock(&self.binder), task) {
            old.abort();
        }
        let mut st = lock(&self.state);
        st.bind = bind;
        st.bound_ended_at_ms = None;
    }

    fn mark_ended(&self) {
        lock(&self.state).bound_ended_at_ms = Some(now_ms());
    }

    fn expired(&self) -> bool {
        lock(&self.state)
            .bound_ended_at_ms
            .is_some_and(|t| now_ms() - t > ENDED_TTL.as_millis() as i64)
    }

    pub fn stop(&self) {
        if let Some(task) = lock(&self.binder).take() {
            task.abort();
        }
    }

    /// Request a checkpoint. Single-flight: when one is running, this one
    /// waits in the one pending slot (replacing an older pending request) and
    /// the running loop takes it next. `wait`: return only after it ran.
    pub async fn checkpoint(
        self: &Arc<Self>,
        host: &Arc<dyn WatchHost>,
        ck: Checkpoint,
        wait: bool,
    ) {
        *lock(&self.pending) = Some(ck);
        loop {
            let guard = if wait {
                self.run.lock().await
            } else {
                match self.run.try_lock() {
                    Ok(g) => g,
                    Err(_) => return,
                }
            };
            loop {
                let next = lock(&self.pending).take();
                let Some(next) = next else { break };
                self.take(host, Some(next)).await;
            }
            drop(guard);
            // A request that raced the release is taken by this caller.
            if lock(&self.pending).is_none() {
                return;
            }
        }
    }

    /// Read only the items that have no snapshot yet (added by an update),
    /// as part of the latest checkpoint: no new `seq`, no event.
    pub async fn baseline_new_items(self: &Arc<Self>, host: &Arc<dyn WatchHost>) {
        let _run = self.run.lock().await;
        self.take(host, None).await;
    }

    async fn take(self: &Arc<Self>, host: &Arc<dyn WatchHost>, ck: Option<Checkpoint>) {
        let started = Instant::now();
        let (target, database, plan, parallel) = {
            let mut st = lock(&self.state);
            st.busy = true;
            let parallel = if st.target.info.local {
                LOCAL_PARALLEL
            } else {
                SAVED_PARALLEL
            };
            let plan: Vec<(usize, IdeDbWatchItemSpec, Option<IdeDbWatchItemInfo>)> = st
                .items
                .iter()
                .enumerate()
                .filter(|(_, i)| ck.is_some() || i.latest.is_none())
                .map(|(idx, i)| (idx, i.spec.clone(), i.described.then(|| i.info.clone())))
                .collect();
            (st.target.clone(), st.database.clone(), plan, parallel)
        };
        let engine = target.info.engine;
        let user = &self.user;
        let reads: Vec<(usize, ItemRead)> = with_request_credential(
            self.cred,
            stream::iter(plan)
                .map(|(idx, spec, info)| {
                    let (target, database) = (&target.exec, database.as_deref());
                    async move {
                        let read =
                            read_item(&**host, user, target, engine, database, &spec, info).await;
                        (idx, read)
                    }
                })
                .buffered(parallel)
                .collect(),
        )
        .await;

        let mut st = lock(&self.state);
        st.busy = false;
        st.last_used = Instant::now();
        let seq = match &ck {
            Some(_) => st.checkpoint_count,
            None => st.checkpoint_count.saturating_sub(1),
        };
        let (mut changed_rows, mut changed_items, mut failed_items) = (0, 0, 0);
        for (idx, read) in reads {
            let Some(item) = st.items.get_mut(idx) else {
                continue;
            };
            if let Some(info) = read.described {
                item.info.primary_key = info.0;
                item.info.row_estimate = info.1;
                item.described = true;
            }
            item.info.error = read.describe_error;
            if let Some(q) = read.query {
                item.query = q;
            }
            match read.result {
                Ok(data) => {
                    let window = if engine == IdeDbEngine::Redis {
                        REDIS_WINDOW
                    } else {
                        MAX_DB_WATCH_LIMIT
                    };
                    let snap = Snap {
                        seq,
                        window_full: data.rows.len() >= window,
                        data: Arc::new(data),
                    };
                    item.read_error = None;
                    if ck.is_none() || item.latest.is_none() {
                        item.base.get_or_insert_with(|| snap.clone());
                        item.latest = Some(snap);
                        continue;
                    }
                    let prev = item.latest.replace(snap);
                    if let (Some(prev), Some(latest)) = (&prev, &item.latest) {
                        let d = diff_snapshots(&prev.data, &latest.data, &item_opts(item, latest));
                        let n = d.added_count + d.modified_count + d.removed_count;
                        changed_rows += n;
                        changed_items += usize::from(n > 0);
                    }
                    item.prev = prev;
                }
                Err(e) => {
                    failed_items += 1;
                    item.read_error = Some(e.to_string());
                }
            }
        }
        let Some(ck) = ck else { return };
        st.checkpoint_count += 1;
        let info = IdeDbWatchCheckpointInfo {
            seq,
            reason: ck.reason,
            label: ck.label,
            file: ck.file,
            line: ck.line,
            captured_at_ms: now_ms(),
            duration_ms: started.elapsed().as_millis() as u64,
            changed_rows,
            changed_items,
            failed_items,
        };
        st.latest = Some(info);
        drop(st);
        host.emit(Event::IdeDbWatchCheckpoint {
            user_id: self.user.id.clone(),
            watch_id: self.id.clone(),
            root: self.root.clone(),
            seq,
            reason: ck.reason.as_str().into(),
            changed_rows,
            failed_items,
        });
    }

    pub fn view(&self) -> IdeDbWatch {
        let st = lock(&self.state);
        IdeDbWatch {
            id: self.id.clone(),
            root: self.root.clone(),
            target: st.target.info.clone(),
            database: st.database.clone(),
            items: st.items.iter().map(|i| i.info.clone()).collect(),
            bind: st.bind.clone(),
            bound_ended_at_ms: st.bound_ended_at_ms,
            checkpoint_count: st.checkpoint_count,
            latest: st.latest.clone(),
            busy: st.busy,
            created_at_ms: self.created_at_ms,
        }
    }

    /// The diff per item. Values are raw unless `for_agent`.
    pub fn read(
        &self,
        compare: IdeDbWatchCompare,
        rows: bool,
        for_agent: bool,
    ) -> IdeDbWatchReadResp {
        let watch = self.view();
        let (engine, items) = {
            let mut st = lock(&self.state);
            st.last_used = Instant::now();
            let engine = st.target.info.engine;
            let items: Vec<_> = st
                .items
                .iter()
                .map(|i| {
                    let from = match compare {
                        IdeDbWatchCompare::PreviousStop => i.prev.clone(),
                        IdeDbWatchCompare::Start => i.base.clone(),
                    };
                    let from = from.filter(|f| i.latest.as_ref().is_some_and(|l| l.seq != f.seq));
                    (
                        i.info.clone(),
                        i.query.clone(),
                        from,
                        i.latest.clone(),
                        i.read_error.clone(),
                    )
                })
                .collect();
            (engine, items)
        };
        let (mut from_seq, mut to_seq) = (None, None);
        let items = items
            .into_iter()
            .map(|(info, query, from, latest, read_error)| {
                if let (Some(f), Some(l)) = (&from, &latest) {
                    from_seq = from_seq.or(Some(f.seq));
                    to_seq = to_seq.or(Some(l.seq));
                }
                item_diff(
                    engine, info, query, from, latest, read_error, rows, for_agent,
                )
            })
            .collect();
        IdeDbWatchReadResp {
            watch,
            compare,
            from_seq,
            to_seq,
            items,
        }
    }
}

fn item_opts(item: &Item, latest: &Snap) -> DiffOpts {
    DiffOpts {
        primary_key: item.info.primary_key.clone(),
        window_full: latest.window_full,
    }
}

struct ItemRead {
    /// Freshly described: (primary key, row estimate).
    described: Option<(Vec<String>, Option<i64>)>,
    describe_error: Option<String>,
    query: Option<String>,
    result: Result<IdeDbWatchSnapshotItem>,
}

async fn read_item(
    host: &dyn WatchHost,
    user: &User,
    target: &DbWatchQueryTarget,
    engine: IdeDbEngine,
    database: Option<&str>,
    spec: &IdeDbWatchItemSpec,
    described: Option<IdeDbWatchItemInfo>,
) -> ItemRead {
    let exec = |op: DbWatchOp, statement: String| DbWatchQueryExec {
        target: target.clone(),
        engine,
        database: database.map(str::to_string),
        object_name: spec.object.clone(),
        statement,
        is_custom_query: false,
        limit: MAX_DB_WATCH_LIMIT,
        op,
    };
    let bounded = |e: DbWatchQueryExec| async move {
        tokio::time::timeout(ITEM_TIMEOUT, host.exec(user, e))
            .await
            .unwrap_or_else(|_| Err(Error::Upstream("the read took longer than 15 s".into())))
    };
    let mut out = ItemRead {
        described: None,
        describe_error: None,
        query: None,
        result: Err(Error::Internal("not read".into())),
    };
    let primary_key = match described {
        Some(info) => info.primary_key,
        None => match bounded(exec(DbWatchOp::Describe, String::new())).await {
            Ok(d) => {
                out.described = Some((d.primary_key.clone(), d.row_estimate));
                d.primary_key
            }
            Err(e) => {
                let msg = refusal_text(&e, &spec.object, database);
                out.describe_error = Some(msg.clone());
                out.result = Err(Error::Invalid(msg));
                return out;
            }
        },
    };
    let query = match build_watch_query(
        engine,
        database,
        &spec.object,
        &primary_key,
        spec.filter.as_ref(),
    ) {
        Ok(q) => q,
        Err(e) => {
            out.result = Err(e);
            return out;
        }
    };
    out.query = Some(query.clone());
    out.result = bounded(exec(DbWatchOp::Read, query))
        .await
        .map(|o| IdeDbWatchSnapshotItem {
            columns: o.columns,
            rows: o
                .rows
                .into_iter()
                .map(|r| r.into_iter().map(cap_cell).collect())
                .collect(),
        });
    out
}

/// IDE wording for an executor refusal (the executor speaks DB Explorer / MCP).
fn refusal_text(e: &Error, object: &str, database: Option<&str>) -> String {
    let raw = e.to_string();
    let lower = raw.to_ascii_lowercase();
    let place = database.map(|d| format!(" in {d}")).unwrap_or_default();
    if lower.contains("not found")
        || lower.contains("doesn't exist")
        || lower.contains("does not exist")
    {
        format!("{object}: not found{place}")
    } else if lower.contains("read-only") || lower.contains("read only") {
        format!("{object}: DB Watch only reads; this statement was refused")
    } else {
        format!("{object}: {raw}")
    }
}

/// Keep a long text cell as its prefix + a hash of the WHOLE value, so a
/// change past the prefix still shows as a change.
fn cap_cell(v: Value) -> Value {
    match v {
        Value::String(s) if s.len() > MAX_CELL_BYTES => {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            s.hash(&mut h);
            let mut end = MAX_CELL_BYTES;
            while !s.is_char_boundary(end) {
                end -= 1;
            }
            Value::String(format!(
                "{}… ({} bytes, #{:016x})",
                &s[..end],
                s.len(),
                h.finish()
            ))
        }
        other => other,
    }
}

fn compact(n: i64) -> String {
    match n {
        n if n >= 1_000_000 => format!("{:.1}M", n as f64 / 1e6),
        n if n >= 1_000 => format!("{:.1}k", n as f64 / 1e3),
        n => n.to_string(),
    }
}

/// The honest description of what a read covers.
fn window_note(engine: IdeDbEngine, info: &IdeDbWatchItemInfo, latest: Option<&Snap>) -> String {
    let Some(snap) = latest else {
        return "not read yet".into();
    };
    let n = snap.data.rows.len();
    if engine == IdeDbEngine::Redis {
        return if snap.window_full {
            format!(
                "first {n} keys matching {} — others not compared",
                info.object
            )
        } else {
            format!("all {n} keys matching {}", info.object)
        };
    }
    let matching = if info.filter.is_some() {
        " matching"
    } else {
        ""
    };
    if !snap.window_full {
        return format!("all {n}{matching} rows");
    }
    let of = info
        .row_estimate
        .map(|e| format!(" of ~{}", compact(e)))
        .unwrap_or_default();
    if info.primary_key.is_empty() {
        format!("{n}{matching} rows{of}, no primary key (order not stable) — others not compared")
    } else {
        format!(
            "the {n}{matching} rows with the highest {}{of} — older rows not compared",
            info.primary_key.join(", ")
        )
    }
}

#[allow(clippy::too_many_arguments)]
fn item_diff(
    engine: IdeDbEngine,
    info: IdeDbWatchItemInfo,
    query: String,
    from: Option<Snap>,
    latest: Option<Snap>,
    read_error: Option<String>,
    rows: bool,
    for_agent: bool,
) -> IdeDbWatchItemDiff {
    let columns = latest
        .as_ref()
        .map(|l| l.data.columns.clone())
        .unwrap_or_default();
    let mut out = IdeDbWatchItemDiff {
        object: info.object.clone(),
        engine,
        executed_query: query,
        window_note: window_note(engine, &info, latest.as_ref()),
        window_full: latest.as_ref().is_some_and(|l| l.window_full),
        columns: columns.clone(),
        added_count: 0,
        modified_count: 0,
        removed_count: 0,
        unchanged_count: latest.as_ref().map_or(0, |l| l.data.rows.len()),
        diffs: Vec::new(),
        more_count: 0,
        current: None,
        error: read_error.or_else(|| info.error.clone()),
    };
    if let (Some(from), Some(l)) = (&from, &latest) {
        let opts = DiffOpts {
            primary_key: info.primary_key.clone(),
            window_full: l.window_full,
        };
        let d = diff_snapshots(&from.data, &l.data, &opts);
        out.added_count = d.added_count;
        out.modified_count = d.modified_count;
        out.removed_count = d.removed_count;
        out.unchanged_count = d.unchanged_count;
        out.more_count =
            (d.added_count + d.modified_count + d.removed_count).saturating_sub(d.diffs.len());
        out.diffs = d.diffs;
    }
    if rows || from.is_none() {
        out.current = latest.map(|l| (*l.data).clone());
    }
    if for_agent {
        mask_for_agent(&mut out);
    }
    out
}

// ── Masking (what an agent reads) ────────────────────────────────────────────
// Through the shared redaction (`otto_core::redact`): a cell is masked when
// the NAME it is stored under is sensitive (its column; a Redis `value`'s
// key, `auth:token:42`) or its value looks like a secret.

/// The name a cell is stored under: its column, or a Redis value's key.
fn cell_name<'a>(columns: &'a [String], row: &'a [Value], i: usize) -> &'a str {
    let redis = columns.len() == 4 && columns[0] == "key" && columns[2] == "value";
    match row.first().and_then(Value::as_str) {
        Some(key) if redis && i == 2 => key,
        _ => columns.get(i).map(String::as_str).unwrap_or(""),
    }
}

/// `(masked value, was masked)`.
fn mask_cell(name: &str, v: &Value) -> (Value, bool) {
    let r = otto_core::redact::redact_named(name, v);
    let hit = !r.hits.is_empty();
    (r.value, hit)
}

fn mask_row(columns: &[String], row: &[Value]) -> (Vec<Value>, bool) {
    let mut any = false;
    let out = row
        .iter()
        .enumerate()
        .map(|(i, v)| {
            let (m, hit) = mask_cell(cell_name(columns, row, i), v);
            any |= hit;
            m
        })
        .collect();
    (out, any)
}

fn mask_for_agent(out: &mut IdeDbWatchItemDiff) {
    let columns = out.columns.clone();
    for d in &mut out.diffs {
        mask_diff(&columns, d);
    }
    if let Some(cur) = &mut out.current {
        for row in &mut cur.rows {
            *row = mask_row(&columns, row).0;
        }
    }
}

fn mask_diff(columns: &[String], d: &mut IdeDbRowDiff) {
    let raw = d.row.clone();
    let (row, mut any) = mask_row(columns, &d.row);
    d.row = row;
    if let Some(prev) = &d.previous_row {
        let (p, hit) = mask_row(columns, prev);
        any |= hit;
        d.previous_row = Some(p);
    }
    for c in &mut d.changes {
        let i = columns
            .iter()
            .position(|col| *col == c.column)
            .unwrap_or(usize::MAX);
        let name = cell_name(columns, &raw, i).to_string();
        let IdeDbFieldChange {
            before,
            after,
            masked,
            ..
        } = c;
        let (b, bh) = mask_cell(&name, before);
        let (a, ah) = mask_cell(&name, after);
        *masked = bh || ah;
        any |= *masked;
        *before = b;
        *after = a;
    }
    d.key = otto_core::redact::redact_text(&d.key).value;
    d.masked = any;
}

// ── Bindings ────────────────────────────────────────────────────────────────

/// Checkpoint on every new stop of a debug session; a final one when it
/// ends, then the watch expires [`ENDED_TTL`] later. `locate` reads where
/// the session stopped.
pub fn bind_debug(
    watch: &Arc<Watch>,
    host: Arc<dyn WatchHost>,
    mut stops: watch::Receiver<DapStopSignal>,
    locate: impl Fn() -> (Option<String>, Option<u32>) + Send + 'static,
) -> tokio::task::AbortHandle {
    let w = Arc::clone(watch);
    let mut last = stops.borrow_and_update().stop_seq;
    let already_ended = stops.borrow().ended;
    tokio::spawn(with_request_credential(watch.cred, async move {
        loop {
            let ended = already_ended || stops.changed().await.is_err() || stops.borrow().ended;
            let signal = *stops.borrow_and_update();
            if ended {
                w.checkpoint(
                    &host,
                    Checkpoint::new(IdeDbWatchCheckpointReason::SessionEnd),
                    true,
                )
                .await;
                w.mark_ended();
                return;
            }
            if signal.stop_seq != last {
                last = signal.stop_seq;
                let (file, line) = locate();
                let ck = Checkpoint {
                    file,
                    line,
                    ..Checkpoint::new(IdeDbWatchCheckpointReason::Stop)
                };
                w.checkpoint(&host, ck, false).await;
            }
        }
    }))
    .abort_handle()
}

/// Checkpoint when a run leaves `running`, then expire. `status` re-reads
/// the run after the broadcast lagged.
pub fn bind_run(
    watch: &Arc<Watch>,
    host: Arc<dyn WatchHost>,
    initial: IdeRunState,
    mut events: broadcast::Receiver<IdeRunState>,
    status: impl Fn() -> Option<IdeRunStatus> + Send + 'static,
) -> tokio::task::AbortHandle {
    let w = Arc::clone(watch);
    tokio::spawn(with_request_credential(watch.cred, async move {
        let mut running = initial.status == IdeRunStatus::Running;
        while running {
            running = match events.recv().await {
                Ok(s) => s.status == IdeRunStatus::Running,
                Err(broadcast::error::RecvError::Lagged(_)) => {
                    status() == Some(IdeRunStatus::Running)
                }
                Err(broadcast::error::RecvError::Closed) => false,
            };
        }
        w.checkpoint(
            &host,
            Checkpoint::new(IdeDbWatchCheckpointReason::RunEnd),
            true,
        )
        .await;
        w.mark_ended();
    }))
    .abort_handle()
}

// ── Registry ────────────────────────────────────────────────────────────────

/// Every live watch, by id (in memory: a watch does not survive a restart).
#[derive(Default)]
pub struct WatchRegistry {
    watches: Mutex<HashMap<String, Arc<Watch>>>,
}

impl WatchRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    fn purge(map: &mut HashMap<String, Arc<Watch>>) {
        map.retain(|_, w| {
            let keep = !w.expired();
            if !keep {
                w.stop();
            }
            keep
        });
    }

    /// Add `watch`: it replaces its user's watch of the same root, and the
    /// user's least recently used one past [`MAX_WATCHES_PER_USER`].
    pub fn insert(&self, watch: Arc<Watch>) {
        let mut map = lock(&self.watches);
        Self::purge(&mut map);
        let uid = watch.user.id.clone();
        let same: Vec<String> = map
            .values()
            .filter(|w| w.user.id == uid && w.root == watch.root)
            .map(|w| w.id.clone())
            .collect();
        for id in same {
            if let Some(w) = map.remove(&id) {
                w.stop();
            }
        }
        let mut mine: Vec<(Instant, String)> = map
            .values()
            .filter(|w| w.user.id == uid)
            .map(|w| (lock(&w.state).last_used, w.id.clone()))
            .collect();
        mine.sort();
        while mine.len() >= MAX_WATCHES_PER_USER {
            let (_, id) = mine.remove(0);
            if let Some(w) = map.remove(&id) {
                w.stop();
            }
        }
        map.insert(watch.id.clone(), watch);
    }

    /// The caller's own watch (another user's reads as not found).
    pub fn get(&self, user: &User, id: &str) -> Result<Arc<Watch>> {
        let mut map = lock(&self.watches);
        Self::purge(&mut map);
        map.get(id)
            .filter(|w| w.user.id == user.id)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("database watch '{id}'")))
    }

    pub fn list(&self, user: &User, root: Option<&str>) -> Vec<IdeDbWatch> {
        let mut map = lock(&self.watches);
        Self::purge(&mut map);
        let mut out: Vec<IdeDbWatch> = map
            .values()
            .filter(|w| w.user.id == user.id && root.is_none_or(|r| w.root == r))
            .map(|w| w.view())
            .collect();
        out.sort_by_key(|w| std::cmp::Reverse(w.created_at_ms));
        out
    }

    pub fn remove(&self, user: &User, id: &str) -> Result<()> {
        let w = self.get(user, id)?;
        w.stop();
        lock(&self.watches).remove(id);
        Ok(())
    }
}

#[cfg(test)]
#[path = "watch_tests.rs"]
mod tests;
