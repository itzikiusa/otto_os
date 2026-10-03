//! Multi-target / parameterised runs — planning and the in-memory job runner
//! (see [`crate::multirun`] for the contract and the pure pieces).
//!
//! Every run goes through [`DbViewerService::run`] with its own `query_id`, so
//! each one is write-guarded, access-checked, natively cancellable and lands
//! in the connection's ordinary query history, exactly like a run from the
//! editor. Jobs live in daemon memory only: a finished job (and its retained
//! results, bounded per job) is dropped [`JOB_TTL`] after it ends.

use super::*;
use crate::multirun::cluster::{self, ClusterRow};
use crate::multirun::params::{self, PlaceholderMode};
use crate::multirun::{
    expand_values, plan_hash, preview, values_label, ClusterMode, ClusterSource, JobStatus,
    MultiRunItemDetail, MultiRunItemView, MultiRunJobBrief, MultiRunJobView, MultiRunPlan,
    MultiRunSpec, MultiRunSummary, PlannedRun, PlannedTarget, RunStatus, StartMultiRunReq,
    TargetCluster, DEFAULT_RUN_ROWS, MAX_CONCURRENCY, MAX_RUNS, MAX_RUN_ROWS, MAX_TARGETS,
};
use crate::types::Scope;
use tokio::sync::watch;

/// How long a finished multi-run (and its results) stays retrievable.
const JOB_TTL: Duration = Duration::from_secs(30 * 60);
/// Multi-runs retained at once (running + recently finished).
const MAX_JOBS: usize = 10;
/// Estimated size of the results ONE multi-run may retain; past it a run's
/// rows are dropped (status and counts stay accurate).
const JOB_MAX_BYTES: usize = 32 * 1024 * 1024;
/// Topology probes in flight at once while planning.
const PROBE_PARALLELISM: usize = 4;

/// Retained multi-run jobs, by id.
#[derive(Default)]
pub(super) struct MultiRunStore {
    jobs: HashMap<String, JobHandle>,
}

type JobHandle = Arc<std::sync::Mutex<Job>>;

struct JobItem {
    view: MultiRunItemView,
    connection_id: Id,
    node: Option<String>,
    statement: String,
    query_id: String,
    result: Option<QueryResult>,
}

struct Job {
    id: String,
    user_id: Id,
    engine: Engine,
    created_at: chrono::DateTime<chrono::Utc>,
    finished_at: Option<chrono::DateTime<chrono::Utc>>,
    status: JobStatus,
    concurrency: usize,
    stop_on_error: bool,
    read_only: bool,
    statement_preview: String,
    targets: Vec<PlannedTarget>,
    items: Vec<JobItem>,
    /// Estimated bytes of the retained results.
    bytes: usize,
    /// A run failed under stop-on-error: dispatch no more.
    stopping: bool,
    cancel_tx: watch::Sender<bool>,
}

impl Job {
    fn summary(&self) -> MultiRunSummary {
        MultiRunSummary::of(self.items.iter().map(|i| i.view.status))
    }

    fn view(&self) -> MultiRunJobView {
        MultiRunJobView {
            id: self.id.clone(),
            status: self.status,
            engine: self.engine,
            created_at: self.created_at.to_rfc3339(),
            finished_at: self.finished_at.map(|t| t.to_rfc3339()),
            concurrency: self.concurrency,
            stop_on_error: self.stop_on_error,
            read_only: self.read_only,
            statement_preview: self.statement_preview.clone(),
            summary: self.summary(),
            targets: self.targets.clone(),
            items: self.items.iter().map(|i| i.view.clone()).collect(),
        }
    }

    fn brief(&self) -> MultiRunJobBrief {
        MultiRunJobBrief {
            id: self.id.clone(),
            status: self.status,
            engine: self.engine,
            created_at: self.created_at.to_rfc3339(),
            finished_at: self.finished_at.map(|t| t.to_rfc3339()),
            statement_preview: self.statement_preview.clone(),
            target_count: self.targets.len(),
            summary: self.summary(),
        }
    }
}

/// The per-run request knobs shared by every run of a job.
#[derive(Clone)]
struct RunOpts {
    max_rows: usize,
    timeout_ms: Option<u64>,
    mask: Option<bool>,
    read_only: bool,
    confirm_write: bool,
}

fn lock(job: &JobHandle) -> std::sync::MutexGuard<'_, Job> {
    job.lock().unwrap_or_else(|p| p.into_inner())
}

/// A result cell as text (ClickHouse may send UInt64 as a JSON string).
fn cell_str(v: Option<&Value>) -> Option<String> {
    match v? {
        Value::String(s) => Some(s.clone()),
        Value::Null => None,
        other => Some(other.to_string()),
    }
}

fn cell_u64(v: Option<&Value>) -> u64 {
    match v {
        Some(Value::Number(n)) => n.as_u64().unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// Whether running `statement` on `conn` needs the typed confirmation — the
/// planning-time mirror of `guard_write` (which still gates every run).
fn needs_confirm(conn: &Connection, engine: Engine, statement: &str) -> bool {
    conn.is_write_guarded()
        && (statement_is_write(engine, statement)
            || (engine == Engine::Redis && crate::types::redis_uses_keys(statement)))
}

impl DbViewerService {
    /// Build the preview: validate the spec, resolve every target, render the
    /// placeholders, detect ClickHouse clusters where the script has DDL, and
    /// produce the FINAL statement of every run. Runs nothing but the
    /// read-only topology probes (native read-only mode, not recorded in
    /// history).
    pub async fn multi_run_plan(&self, user_id: &Id, spec: &MultiRunSpec) -> Result<MultiRunPlan> {
        let template = spec.statement.as_str();
        if template.trim().is_empty() {
            return Err(Error::Invalid("empty statement".into()));
        }
        if spec.targets.is_empty() {
            return Err(Error::Invalid("pick at least one target".into()));
        }
        if spec.targets.len() > MAX_TARGETS {
            return Err(Error::Invalid(format!(
                "{} targets — a multi-run may address at most {MAX_TARGETS}",
                spec.targets.len()
            )));
        }

        // Resolve every target's connection; one engine for the whole run.
        let mut conns: Vec<Connection> = Vec::new();
        let mut engine: Option<Engine> = None;
        let mut seen: Vec<(Id, Option<String>)> = Vec::new();
        for t in &spec.targets {
            let conn = self.connections.get(&t.connection_id).await?;
            let e = Engine::from_kind(conn.kind).ok_or_else(|| {
                Error::Invalid(format!(
                    "connection '{}' is not a queryable database",
                    conn.name
                ))
            })?;
            match engine {
                None => engine = Some(e),
                Some(first) if first != e => {
                    return Err(Error::Invalid(format!(
                        "all targets must use the same engine — '{}' is {} but the first target is {}",
                        conn.name,
                        e.as_str(),
                        first.as_str()
                    )));
                }
                _ => {}
            }
            let key = (
                t.connection_id.clone(),
                crate::access::canonical_node(t.node.as_deref()),
            );
            if seen.contains(&key) {
                return Err(Error::Invalid(format!(
                    "target '{}' is listed twice",
                    target_label(&conn, key.1.as_deref())
                )));
            }
            seen.push(key);
            if t.cluster_mode == ClusterMode::Custom {
                let name = t.cluster_name.as_deref().map(str::trim).unwrap_or("");
                if !cluster::valid_cluster_name(name) {
                    return Err(Error::Invalid(format!(
                        "target '{}': cluster name must be letters, digits, _ - . {{ }} (got `{}`)",
                        conn.name,
                        params::clip(name, 40)
                    )));
                }
            }
            conns.push(conn);
        }
        let engine = engine.expect("at least one target");

        // Placeholders: every one referenced needs a value; a swept parameter
        // the script never uses would just repeat the same statement.
        let mode = PlaceholderMode::for_engine(engine);
        let placeholders = params::placeholder_names(template, mode);
        let missing: Vec<String> = placeholders
            .iter()
            .filter(|n| !spec.params.iter().any(|p| &p.name == *n))
            .map(|n| format!(":{n}"))
            .collect();
        if !missing.is_empty() {
            return Err(Error::Invalid(format!(
                "no value for placeholder(s) {}",
                missing.join(", ")
            )));
        }
        let mut warnings = Vec::new();
        for p in &spec.params {
            if !placeholders.contains(&p.name) {
                if p.values.len() > 1 {
                    return Err(Error::Invalid(format!(
                        "parameter `{}` is not used by the script — sweeping it would run the \
                         same statement {} times",
                        p.name,
                        p.values.len()
                    )));
                }
                warnings.push(format!("parameter `{}` is not used by the script", p.name));
            }
        }
        let combos = expand_values(&spec.params)?;
        let total = combos.len() * spec.targets.len();
        if total > MAX_RUNS {
            return Err(Error::Invalid(format!(
                "{} target(s) × {} value combination(s) = {total} runs (max {MAX_RUNS})",
                spec.targets.len(),
                combos.len()
            )));
        }
        let mut rendered: Vec<String> = Vec::with_capacity(combos.len());
        for combo in &combos {
            let mut values = HashMap::new();
            for v in combo {
                let p = spec
                    .params
                    .iter()
                    .find(|p| p.name == v.name)
                    .expect("combo names come from params");
                values.insert(
                    v.name.clone(),
                    params::render_value(&v.name, &v.value, p.var_type, p.escape, engine)?,
                );
            }
            rendered.push(params::substitute(template, mode, &values));
        }

        // ClickHouse: probe the topology only where an `auto` target's script
        // actually has DDL that would get ON CLUSTER.
        let has_ddl =
            engine == Engine::Clickhouse && rendered.iter().any(|s| cluster::has_injectable_ddl(s));
        // Bounded fan-out without a borrowing closure (keeps the handler
        // future `Send` for every lifetime).
        let mut probes: Vec<Option<TargetCluster>> = Vec::new();
        for chunk in spec.targets.chunks(PROBE_PARALLELISM) {
            let mut futs = Vec::with_capacity(chunk.len());
            for t in chunk {
                futs.push(self.plan_target_cluster(t, user_id, engine, has_ddl));
            }
            probes.extend(futures_util::future::join_all(futs).await);
        }

        let targets: Vec<PlannedTarget> = spec
            .targets
            .iter()
            .zip(conns.iter())
            .zip(probes)
            .enumerate()
            .map(|(index, ((t, conn), cluster))| {
                let node = crate::access::canonical_node(t.node.as_deref());
                PlannedTarget {
                    index,
                    connection_id: conn.id.clone(),
                    connection_name: conn.name.clone(),
                    environment: conn.environment,
                    read_only: conn.read_only,
                    guarded: conn.is_write_guarded(),
                    label: target_label(conn, node.as_deref()),
                    node,
                    cluster,
                }
            })
            .collect();

        let mut runs = Vec::with_capacity(total);
        for (ti, target) in targets.iter().enumerate() {
            let conn = &conns[ti];
            let applied = target.cluster.as_ref().and_then(|c| c.applied.clone());
            for (ci, combo) in combos.iter().enumerate() {
                let mut statement = rendered[ci].clone();
                let (mut on_cluster, mut cluster_skipped) = (Vec::new(), Vec::new());
                if let Some(c) = applied.as_deref() {
                    let r = cluster::rewrite_script(&statement, c);
                    statement = r.statement;
                    on_cluster = r.injected;
                    cluster_skipped = r.skipped;
                }
                let is_write = statement_is_write(engine, &statement);
                let values_text = values_label(combo);
                runs.push(PlannedRun {
                    index: runs.len(),
                    target: ti,
                    values: combo.clone(),
                    label: if values_text.is_empty() {
                        target.label.clone()
                    } else {
                        format!("{} · {values_text}", target.label)
                    },
                    needs_confirm: !spec.read_only && needs_confirm(conn, engine, &statement),
                    statement,
                    on_cluster,
                    cluster_skipped,
                    is_write,
                });
            }
        }
        let write_count = runs.iter().filter(|r| r.is_write).count();
        if spec.read_only && write_count > 0 {
            warnings.push(format!(
                "this multi-run is read-only, but {write_count} run(s) are classified as \
                 writes/DDL — it will be refused"
            ));
        }
        // Access-enforced production / read-only connections refuse direct
        // mutations whatever the confirmation (they go through the reviewed-
        // change flow) — say so in the preview instead of failing at run time.
        if !spec.read_only {
            for (ti, t) in targets.iter().enumerate() {
                let writes = runs.iter().any(|r| r.target == ti && r.is_write);
                if writes && t.guarded && self.is_enforced(&t.connection_id).await? {
                    warnings.push(format!(
                        "{}: access-enforced {} connection — its writes will be refused \
                         (production / read-only changes need an approved reviewed change)",
                        t.label,
                        if t.read_only {
                            "read-only"
                        } else {
                            "production"
                        }
                    ));
                }
            }
        }
        for t in &targets {
            if let Some(c) = &t.cluster {
                if c.source == ClusterSource::Ambiguous || c.source == ClusterSource::ProbeFailed {
                    warnings.push(format!(
                        "{}: no ON CLUSTER — {}",
                        t.label,
                        c.note.clone().unwrap_or_default()
                    ));
                }
            }
        }
        Ok(MultiRunPlan {
            engine,
            placeholders,
            needs_confirm: runs.iter().any(|r| r.needs_confirm),
            write_count,
            warnings,
            plan_hash: plan_hash(&targets, &runs),
            targets,
            runs,
        })
    }

    /// The ClickHouse cluster facts for one target (`None` for other engines):
    /// the user's `off` / `custom` choice as given, or — for `auto` when the
    /// script has DDL to rewrite — the detected topology.
    async fn plan_target_cluster(
        &self,
        t: &crate::multirun::MultiRunTarget,
        user_id: &Id,
        engine: Engine,
        has_ddl: bool,
    ) -> Option<TargetCluster> {
        if engine != Engine::Clickhouse {
            return None;
        }
        let base = TargetCluster {
            mode: t.cluster_mode,
            detected: None,
            source: ClusterSource::NotNeeded,
            candidates: Vec::new(),
            database_engine: None,
            applied: None,
            note: None,
        };
        Some(match t.cluster_mode {
            ClusterMode::Off => TargetCluster {
                note: Some("ON CLUSTER turned off for this target".into()),
                ..base
            },
            ClusterMode::Custom => TargetCluster {
                applied: t.cluster_name.as_deref().map(|s| s.trim().to_string()),
                note: Some("cluster chosen by you".into()),
                ..base
            },
            ClusterMode::Auto if !has_ddl => base,
            ClusterMode::Auto => {
                let node = crate::access::canonical_node(t.node.as_deref());
                match self
                    .probe_clickhouse_cluster(&t.connection_id, user_id, node.as_deref())
                    .await
                {
                    Ok((macro_cluster, rows, db_engine)) => {
                        let d =
                            cluster::decide(macro_cluster.as_deref(), &rows, db_engine.as_deref());
                        TargetCluster {
                            detected: d.cluster.clone(),
                            applied: d.cluster,
                            source: d.source,
                            candidates: d.candidates,
                            database_engine: db_engine,
                            note: d.note,
                            ..base
                        }
                    }
                    Err(e) => TargetCluster {
                        source: ClusterSource::ProbeFailed,
                        note: Some(format!(
                            "could not read the cluster topology: {}",
                            params::clip(&e.to_string(), 200)
                        )),
                        ..base
                    },
                }
            }
        })
    }

    /// Read a ClickHouse target's cluster topology: the `{cluster}` macro,
    /// `system.clusters` aggregated per cluster, and the target database's
    /// engine. Read-only native mode, never recorded in history.
    async fn probe_clickhouse_cluster(
        &self,
        conn_id: &Id,
        user_id: &Id,
        node: Option<&str>,
    ) -> Result<(Option<String>, Vec<ClusterRow>, Option<String>)> {
        let probe = |statement: String| QueryRequest {
            statement,
            node: node.map(str::to_string),
            max_rows: Some(1000),
            ..QueryRequest::default()
        };
        let macros = self
            .run_with(
                conn_id,
                user_id,
                &probe("SELECT macro, substitution FROM system.macros".into()),
                true,
                false,
            )
            .await?;
        let macro_cluster = macros
            .rows
            .iter()
            .find(|r| cell_str(r.first()).as_deref() == Some("cluster"))
            .and_then(|r| cell_str(r.get(1)));
        let clusters = self
            .run_with(
                conn_id,
                user_id,
                &probe(
                    "SELECT cluster, count() AS hosts, countIf(is_local) AS local_hosts, \
                     countIf(NOT (host_address LIKE '127.%' OR host_address = '::1' \
                     OR host_name = 'localhost')) AS remote_hosts \
                     FROM system.clusters GROUP BY cluster ORDER BY cluster"
                        .into(),
                ),
                true,
                false,
            )
            .await?;
        let rows = clusters
            .rows
            .iter()
            .filter_map(|r| {
                Some(ClusterRow {
                    name: cell_str(r.first())?,
                    hosts: cell_u64(r.get(1)),
                    local_hosts: cell_u64(r.get(2)),
                    remote_hosts: cell_u64(r.get(3)),
                })
            })
            .collect();
        let db = Scope::parse(node).and_then(|s| s.database().map(str::to_string));
        let where_db = match db {
            Some(name) => format!("'{}'", name.replace('\\', "\\\\").replace('\'', "\\'")),
            None => "currentDatabase()".into(),
        };
        let engine = self
            .run_with(
                conn_id,
                user_id,
                &probe(format!(
                    "SELECT engine FROM system.databases WHERE name = {where_db}"
                )),
                true,
                false,
            )
            .await?;
        let db_engine = engine.rows.first().and_then(|r| cell_str(r.first()));
        Ok((macro_cluster, rows, db_engine))
    }

    /// Start a multi-run: re-plan (so what runs is exactly what a fresh
    /// preview shows — and, with `plan_hash`, exactly what the user approved),
    /// refuse up front when a guarded write lacks `confirm_write` (nothing
    /// runs partially), then execute in the background. Returns the job and
    /// the plan it runs.
    pub async fn multi_run_start(
        &self,
        user_id: &Id,
        req: &StartMultiRunReq,
    ) -> Result<(MultiRunJobView, MultiRunPlan)> {
        let plan = self.multi_run_plan(user_id, &req.spec).await?;
        if let Some(expected) = req.plan_hash.as_deref().filter(|h| !h.is_empty()) {
            if expected != plan.plan_hash {
                return Err(Error::Conflict(
                    "plan_changed: the final statements changed since the preview — review \
                     them again before running"
                        .into(),
                ));
            }
        }
        if req.spec.read_only && plan.write_count > 0 {
            return Err(Error::Forbidden(format!(
                "{READ_ONLY_PREFIX}this multi-run was requested read-only; {} run(s) are \
                 classified as writes/DDL",
                plan.write_count
            )));
        }
        if plan.needs_confirm && !req.confirm_write {
            let mut names: Vec<&str> = plan
                .runs
                .iter()
                .filter(|r| r.needs_confirm)
                .map(|r| plan.targets[r.target].label.as_str())
                .collect();
            names.dedup();
            return Err(Error::Conflict(format!(
                "{WRITE_BLOCKED_PREFIX}{} run(s) write to production / read-only targets ({}); \
                 confirm the writes to run them",
                plan.runs.iter().filter(|r| r.needs_confirm).count(),
                names.join(", ")
            )));
        }
        let opts = RunOpts {
            max_rows: req
                .spec
                .max_rows
                .unwrap_or(DEFAULT_RUN_ROWS)
                .clamp(1, MAX_RUN_ROWS),
            timeout_ms: req.spec.timeout_ms,
            mask: req.spec.mask,
            read_only: req.spec.read_only,
            confirm_write: req.confirm_write,
        };
        let concurrency = req.concurrency.unwrap_or(1).clamp(1, MAX_CONCURRENCY);
        let id = otto_core::new_id();
        let (cancel_tx, cancel_rx) = watch::channel(false);
        let items = plan
            .runs
            .iter()
            .map(|r| {
                let t = &plan.targets[r.target];
                JobItem {
                    view: MultiRunItemView {
                        index: r.index,
                        target: r.target,
                        label: r.label.clone(),
                        values: r.values.clone(),
                        statement_preview: preview(&r.statement),
                        on_cluster: r.on_cluster.clone(),
                        is_write: r.is_write,
                        status: RunStatus::Pending,
                        duration_ms: None,
                        row_count: None,
                        rows_affected: None,
                        message: None,
                        error: None,
                        has_result: false,
                        result_dropped: false,
                    },
                    connection_id: t.connection_id.clone(),
                    node: t.node.clone(),
                    statement: r.statement.clone(),
                    query_id: format!("multirun-{id}-{}", r.index),
                    result: None,
                }
            })
            .collect();
        let job = Arc::new(std::sync::Mutex::new(Job {
            id: id.clone(),
            user_id: user_id.clone(),
            engine: plan.engine,
            created_at: chrono::Utc::now(),
            finished_at: None,
            status: JobStatus::Running,
            concurrency,
            stop_on_error: req.stop_on_error,
            read_only: req.spec.read_only,
            statement_preview: preview(&req.spec.statement),
            targets: plan.targets.clone(),
            items,
            bytes: 0,
            stopping: false,
            cancel_tx,
        }));
        {
            let mut store = self.multi_runs.lock().unwrap_or_else(|p| p.into_inner());
            if store.jobs.len() >= MAX_JOBS {
                // Evict the oldest FINISHED job; never a running one.
                let oldest = store
                    .jobs
                    .iter()
                    .filter_map(|(k, j)| {
                        let j = lock(j);
                        (j.status != JobStatus::Running).then(|| (k.clone(), j.created_at))
                    })
                    .min_by_key(|(_, at)| *at)
                    .map(|(k, _)| k);
                match oldest {
                    Some(k) => {
                        store.jobs.remove(&k);
                    }
                    None => {
                        return Err(Error::Conflict(format!(
                            "{MAX_JOBS} multi-runs are already running — wait for one to finish"
                        )))
                    }
                }
            }
            store.jobs.insert(id.clone(), Arc::clone(&job));
        }
        let view = lock(&job).view();
        let svc = self.clone();
        let runner_user = user_id.clone();
        tokio::spawn(async move {
            svc.run_job(job, runner_user, opts, concurrency, cancel_rx)
                .await;
        });
        Ok((view, plan))
    }

    /// Drive every run of `job` (at most `concurrency` at once, in order),
    /// then settle the job and schedule its expiry.
    async fn run_job(
        &self,
        job: JobHandle,
        user_id: Id,
        opts: RunOpts,
        concurrency: usize,
        cancel_rx: watch::Receiver<bool>,
    ) {
        let n = lock(&job).items.len();
        // `concurrency` workers pull the next run index in order — with one
        // worker that is strictly sequential.
        let next = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut workers = Vec::with_capacity(concurrency);
        for _ in 0..concurrency.min(n.max(1)) {
            let (job, user_id, opts, cancel_rx, next) = (
                Arc::clone(&job),
                user_id.clone(),
                opts.clone(),
                cancel_rx.clone(),
                Arc::clone(&next),
            );
            let svc = self.clone();
            workers.push(async move {
                loop {
                    let idx = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if idx >= n {
                        break;
                    }
                    svc.run_job_item(&job, idx, &user_id, &opts, cancel_rx.clone())
                        .await;
                }
            });
        }
        futures_util::future::join_all(workers).await;
        let id = {
            let mut j = lock(&job);
            let cancelled = *j.cancel_tx.borrow();
            let stopping = j.stopping;
            // Every run was visited above; this only settles a straggler.
            for it in &mut j.items {
                if it.view.status == RunStatus::Pending {
                    it.view.status = if stopping && !cancelled {
                        RunStatus::Skipped
                    } else {
                        RunStatus::Cancelled
                    };
                }
            }
            j.status = if cancelled {
                JobStatus::Cancelled
            } else {
                JobStatus::Done
            };
            j.finished_at = Some(chrono::Utc::now());
            j.id.clone()
        };
        let weak = Arc::downgrade(&self.multi_runs);
        tokio::spawn(async move {
            tokio::time::sleep(JOB_TTL).await;
            if let Some(store) = weak.upgrade() {
                store
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .jobs
                    .remove(&id);
            }
        });
    }

    async fn run_job_item(
        &self,
        job: &JobHandle,
        idx: usize,
        user_id: &Id,
        opts: &RunOpts,
        mut cancel: watch::Receiver<bool>,
    ) {
        let (conn_id, query_id, req) = {
            let mut j = lock(job);
            let cancelled = *cancel.borrow();
            let stopping = j.stopping;
            let it = &mut j.items[idx];
            if it.view.status != RunStatus::Pending {
                return;
            }
            if cancelled {
                it.view.status = RunStatus::Cancelled;
                return;
            }
            if stopping {
                it.view.status = RunStatus::Skipped;
                return;
            }
            it.view.status = RunStatus::Running;
            (
                it.connection_id.clone(),
                it.query_id.clone(),
                QueryRequest {
                    statement: it.statement.clone(),
                    node: it.node.clone(),
                    max_rows: Some(opts.max_rows),
                    timeout_ms: opts.timeout_ms,
                    mask: opts.mask,
                    read_only: opts.read_only,
                    confirm_write: opts.confirm_write,
                    query_id: Some(it.query_id.clone()),
                    ..QueryRequest::default()
                },
            )
        };
        let started = Instant::now();
        let outcome = tokio::select! {
            r = self.run(&conn_id, user_id, &req) => Some(r),
            _ = async {
                // A dropped sender never cancels (the job owns it while running).
                if cancel.wait_for(|c| *c).await.is_err() {
                    std::future::pending::<()>().await;
                }
            } => None,
        };
        let outcome = match outcome {
            Some(r) => r,
            None => {
                // The run future is dropped: if it had not yet registered its
                // query it never will; if it had, stop the detached execution.
                let _ = self.cancel(&conn_id, user_id, &query_id).await;
                Err(Error::Conflict("cancelled".into()))
            }
        };
        let mut j = lock(job);
        let cancelled = *j.cancel_tx.borrow();
        let elapsed = started.elapsed().as_millis() as u64;
        let mut failed = false;
        let mut keep: Option<QueryResult> = None;
        {
            let it = &mut j.items[idx];
            it.view.duration_ms = Some(elapsed);
            match outcome {
                Ok(res) => {
                    let sets = || std::iter::once(&res).chain(res.more_results.iter());
                    it.view.row_count = Some(sets().map(|r| r.stats.row_count).sum());
                    let affected: Vec<u64> = sets().filter_map(|r| r.rows_affected).collect();
                    it.view.rows_affected = (!affected.is_empty()).then(|| affected.iter().sum());
                    if let Some(bad) = sets().find(|r| r.errored) {
                        failed = true;
                        it.view.status = RunStatus::Failed;
                        it.view.error = Some(
                            bad.message
                                .clone()
                                .unwrap_or_else(|| "a statement in the batch failed".into()),
                        );
                    } else {
                        it.view.status = RunStatus::Ok;
                        it.view.message = res.message.clone();
                    }
                    keep = Some(res);
                }
                Err(e) => {
                    if cancelled {
                        it.view.status = RunStatus::Cancelled;
                    } else {
                        failed = true;
                        it.view.status = RunStatus::Failed;
                    }
                    it.view.error = Some(e.to_string());
                }
            }
        }
        if let Some(res) = keep {
            // Retain the rows within the job's memory budget; past it only the
            // status and counts are kept.
            let wrapped: std::result::Result<QueryResult, String> = Ok(res);
            let bytes = outcome_bytes(&wrapped);
            if let Ok(res) = wrapped {
                if j.bytes + bytes <= JOB_MAX_BYTES {
                    j.bytes += bytes;
                    let it = &mut j.items[idx];
                    it.result = Some(res);
                    it.view.has_result = true;
                } else {
                    j.items[idx].view.result_dropped = true;
                }
            }
        }
        if failed && j.stop_on_error {
            j.stopping = true;
        }
    }

    /// The caller's job, or 404 (another user's job is invisible; root sees all).
    fn multi_run_job(&self, user_id: &Id, is_root: bool, run_id: &str) -> Result<JobHandle> {
        let store = self.multi_runs.lock().unwrap_or_else(|p| p.into_inner());
        let job = store
            .jobs
            .get(run_id)
            .cloned()
            .ok_or_else(|| Error::NotFound("multi-run".into()))?;
        drop(store);
        if !is_root && lock(&job).user_id != *user_id {
            return Err(Error::NotFound("multi-run".into()));
        }
        Ok(job)
    }

    /// `GET /db/multi-runs/{rid}`.
    pub fn multi_run_get(
        &self,
        user_id: &Id,
        is_root: bool,
        run_id: &str,
    ) -> Result<MultiRunJobView> {
        Ok(lock(&self.multi_run_job(user_id, is_root, run_id)?).view())
    }

    /// `GET /db/multi-runs` — the caller's retained multi-runs, newest first
    /// (root: everyone's).
    pub fn multi_run_list(&self, user_id: &Id, is_root: bool) -> Vec<MultiRunJobBrief> {
        let jobs: Vec<JobHandle> = self
            .multi_runs
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .jobs
            .values()
            .cloned()
            .collect();
        let mut out: Vec<(chrono::DateTime<chrono::Utc>, MultiRunJobBrief)> = jobs
            .iter()
            .filter_map(|j| {
                let j = lock(j);
                (is_root || j.user_id == *user_id).then(|| (j.created_at, j.brief()))
            })
            .collect();
        out.sort_by_key(|a| std::cmp::Reverse(a.0));
        out.into_iter().map(|(_, b)| b).collect()
    }

    /// `GET /db/multi-runs/{rid}/items/{index}` — one run with its full final
    /// statement and retained result. The HTTP layer re-checks the caller's
    /// role on the run's connection before returning it.
    pub fn multi_run_item(
        &self,
        user_id: &Id,
        is_root: bool,
        run_id: &str,
        index: usize,
    ) -> Result<MultiRunItemDetail> {
        let job = self.multi_run_job(user_id, is_root, run_id)?;
        let j = lock(&job);
        let it = j
            .items
            .get(index)
            .ok_or_else(|| Error::NotFound("multi-run item".into()))?;
        Ok(MultiRunItemDetail {
            item: it.view.clone(),
            connection_id: it.connection_id.clone(),
            node: it.node.clone(),
            statement: it.statement.clone(),
            result: it.result.clone(),
        })
    }

    /// `POST /db/multi-runs/{rid}/cancel` — stop dispatching, mark the pending
    /// runs cancelled and signal the running ones (each issues the
    /// engine-native cancel for its own query). Idempotent; a finished job is
    /// returned unchanged.
    pub fn multi_run_cancel(
        &self,
        user_id: &Id,
        is_root: bool,
        run_id: &str,
    ) -> Result<MultiRunJobView> {
        let job = self.multi_run_job(user_id, is_root, run_id)?;
        let mut j = lock(&job);
        if j.status == JobStatus::Running {
            let _ = j.cancel_tx.send(true);
            for it in &mut j.items {
                if it.view.status == RunStatus::Pending {
                    it.view.status = RunStatus::Cancelled;
                }
            }
        }
        Ok(j.view())
    }
}

/// `connection · node`, or the connection name when no node is set.
fn target_label(conn: &Connection, node: Option<&str>) -> String {
    match Scope::parse(node) {
        Some(Scope::Database(db)) => format!("{} · {db}", conn.name),
        Some(Scope::Keyspace(n)) => format!("{} · db{n}", conn.name),
        None => conn.name.clone(),
    }
}

#[cfg(test)]
#[path = "multirun_tests.rs"]
mod tests;
