//! Multi-target / parameterised runs ("Run on…"): the SAME script against
//! several targets (connection × database/schema) and/or once per parameter
//! value, each run with its own status, result and error.
//!
//! This module holds the wire contract and the pure planning steps:
//!
//! - [`params`] — the editor's placeholder syntax (`:name` / `{name}` /
//!   `{{name}}`), typed and escaped server-side;
//! - [`cluster`] — ClickHouse cluster detection + `ON CLUSTER` injection into
//!   DDL (never DML);
//! - [`expand_values`] — the parameter value matrix (cartesian product);
//! - [`plan_hash`] — a digest of every planned (target, statement) pair, so a
//!   start request can prove it runs EXACTLY what the preview showed.
//!
//! The async orchestration (resolving connections, probing clusters, running
//! the job, cancel) lives in `service/multirun.rs`; each run executes through
//! the ordinary [`crate::service::DbViewerService::run`] path, so the
//! write-guard, access enforcement, native cancel and query history apply to
//! every single run exactly as for a run from the editor.

pub mod cluster;
pub mod params;

use otto_core::domain::Environment;
use otto_core::{Error, Id, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::types::{Engine, QueryResult};
pub use cluster::ClusterSource;
pub use params::VarType;

/// Most targets one multi-run may address.
pub const MAX_TARGETS: usize = 50;
/// Most runs (targets × value combinations) one multi-run may expand to.
pub const MAX_RUNS: usize = 200;
/// Most values one parameter may sweep.
pub const MAX_VALUES: usize = 200;
/// Highest allowed parallelism (default 1 = sequential).
pub const MAX_CONCURRENCY: usize = 8;
/// Default per-run row cap (each run's result is kept in daemon memory).
pub const DEFAULT_RUN_ROWS: usize = 500;
/// Highest per-run row cap.
pub const MAX_RUN_ROWS: usize = 10_000;

/// Per-target ClickHouse cluster choice.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClusterMode {
    /// Detect the cluster and inject `ON CLUSTER` into DDL when there is one.
    #[default]
    Auto,
    /// Never inject.
    Off,
    /// Inject `ON CLUSTER <cluster_name>` (the user's override).
    Custom,
}

/// One target: a connection, optionally scoped to a database / schema /
/// Redis keyspace (`node`, same semantics as `QueryRequest.node`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct MultiRunTarget {
    pub connection_id: Id,
    #[serde(default)]
    pub node: Option<String>,
    /// ClickHouse only; ignored elsewhere.
    #[serde(default)]
    pub cluster_mode: ClusterMode,
    /// Required when `cluster_mode` is `custom`.
    #[serde(default)]
    pub cluster_name: Option<String>,
}

fn yes() -> bool {
    true
}

/// A placeholder and the values it takes. One value = a fixed variable; more
/// = a sweep (one run per value, combined across parameters as a cartesian
/// product, and with every target).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiRunParam {
    pub name: String,
    pub values: Vec<String>,
    #[serde(default, rename = "type")]
    pub var_type: VarType,
    /// Escape quotes/backslashes inside a `string` value (default on).
    #[serde(default = "yes")]
    pub escape: bool,
}

/// What to run and where — the body of `POST /db/multi-run/plan`, and the
/// core of a start request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiRunSpec {
    /// The script template (placeholders allowed).
    pub statement: String,
    pub targets: Vec<MultiRunTarget>,
    #[serde(default)]
    pub params: Vec<MultiRunParam>,
    /// Per-run row cap (default [`DEFAULT_RUN_ROWS`], clamped to [`MAX_RUN_ROWS`]).
    #[serde(default)]
    pub max_rows: Option<usize>,
    /// Per-statement timeout in ms (as `QueryRequest.timeout_ms`).
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// Redact result cells server-side (as `QueryRequest.mask`).
    #[serde(default)]
    pub mask: Option<bool>,
    /// Refuse any write/DDL and run in the engine's native read-only mode (as
    /// `QueryRequest.read_only`) — what an agent-driven multi-run sets.
    #[serde(default)]
    pub read_only: bool,
}

/// Body of `POST /db/multi-runs`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StartMultiRunReq {
    #[serde(flatten)]
    pub spec: MultiRunSpec,
    /// Parallel runs (1 = sequential, the default; max [`MAX_CONCURRENCY`]).
    #[serde(default)]
    pub concurrency: Option<usize>,
    /// Stop dispatching after the first failed run (default true). Runs
    /// already in flight finish; the rest are `skipped`.
    #[serde(default = "yes")]
    pub stop_on_error: bool,
    /// Explicit acknowledgement of the writes/DDL on guarded (production /
    /// read-only) targets, after the UI's typed confirmation listing every
    /// target and final statement. Without it a plan with any such run is
    /// refused up front (409 `write_blocked:`) — nothing runs.
    #[serde(default)]
    pub confirm_write: bool,
    /// The `plan_hash` of the preview the user approved. When set and the
    /// re-computed plan differs (a cluster changed, a connection was edited),
    /// the start is refused with 409 so nothing unreviewed runs.
    #[serde(default)]
    pub plan_hash: Option<String>,
}

/// A parameter value of one run.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ParamValue {
    pub name: String,
    pub value: String,
}

/// ClickHouse cluster facts for one target, as planned.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetCluster {
    pub mode: ClusterMode,
    /// The auto-detected cluster (`None` = not a cluster / ambiguous / not probed).
    pub detected: Option<String>,
    pub source: ClusterSource,
    /// Multi-host clusters seen in `system.clusters` (for the override picker).
    pub candidates: Vec<String>,
    /// `system.databases.engine` of the target database, when probed.
    pub database_engine: Option<String>,
    /// The cluster actually injected (`None` = no `ON CLUSTER`).
    pub applied: Option<String>,
    pub note: Option<String>,
}

/// One planned target.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedTarget {
    pub index: usize,
    pub connection_id: Id,
    pub connection_name: String,
    pub environment: Environment,
    pub read_only: bool,
    /// Production or read-only: writes need the typed confirmation.
    pub guarded: bool,
    pub node: Option<String>,
    /// `connection · node` (or just the connection name).
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cluster: Option<TargetCluster>,
}

/// One planned run: a target × one value combination, with the FINAL statement
/// that will be sent (placeholders substituted, `ON CLUSTER` injected).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlannedRun {
    pub index: usize,
    /// Index into [`MultiRunPlan::targets`].
    pub target: usize,
    pub values: Vec<ParamValue>,
    pub label: String,
    pub statement: String,
    /// Statements that got `ON CLUSTER` (object descriptions).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_cluster: Vec<String>,
    /// DDL deliberately left without `ON CLUSTER`, with the reason.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub cluster_skipped: Vec<String>,
    /// Classified as a write/DDL (the conservative write-guard classifier).
    pub is_write: bool,
    /// Needs the typed confirmation (a write — or Redis `KEYS` — on a guarded target).
    pub needs_confirm: bool,
}

/// Response of `POST /db/multi-run/plan` — the preview.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiRunPlan {
    pub engine: Engine,
    /// Placeholder names the script references.
    pub placeholders: Vec<String>,
    pub targets: Vec<PlannedTarget>,
    pub runs: Vec<PlannedRun>,
    pub write_count: usize,
    /// Any run needs the typed confirmation.
    pub needs_confirm: bool,
    pub warnings: Vec<String>,
    /// Digest of every (target, final statement) pair — echo it on start.
    pub plan_hash: String,
}

/// Status of one run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RunStatus {
    Pending,
    Running,
    Ok,
    Failed,
    /// Not started because an earlier run failed (stop-on-error).
    Skipped,
    /// Stopped or never started because the multi-run was cancelled.
    Cancelled,
}

/// Status of the whole multi-run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Running,
    /// Every run reached a final state (see the summary for failures).
    Done,
    Cancelled,
}

/// Counts per run status.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MultiRunSummary {
    pub total: usize,
    pub ok: usize,
    pub failed: usize,
    pub running: usize,
    pub pending: usize,
    pub skipped: usize,
    pub cancelled: usize,
}

impl MultiRunSummary {
    pub fn of(statuses: impl Iterator<Item = RunStatus>) -> Self {
        let mut s = Self::default();
        for st in statuses {
            s.total += 1;
            match st {
                RunStatus::Pending => s.pending += 1,
                RunStatus::Running => s.running += 1,
                RunStatus::Ok => s.ok += 1,
                RunStatus::Failed => s.failed += 1,
                RunStatus::Skipped => s.skipped += 1,
                RunStatus::Cancelled => s.cancelled += 1,
            }
        }
        s
    }
}

/// One run as reported by `GET /db/multi-runs/{rid}` (no rows, a statement
/// preview only — fetch the item for its full statement + result).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiRunItemView {
    pub index: usize,
    pub target: usize,
    pub label: String,
    pub values: Vec<ParamValue>,
    pub statement_preview: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub on_cluster: Vec<String>,
    pub is_write: bool,
    pub status: RunStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub duration_ms: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub row_count: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rows_affected: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// A result is retained and can be fetched.
    pub has_result: bool,
    /// The result was dropped to keep the multi-run within its memory budget
    /// (counts and status are still accurate).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub result_dropped: bool,
}

/// `GET /db/multi-runs/{rid}` / the start response.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiRunJobView {
    pub id: String,
    pub status: JobStatus,
    pub engine: Engine,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    pub concurrency: usize,
    pub stop_on_error: bool,
    pub read_only: bool,
    pub statement_preview: String,
    pub summary: MultiRunSummary,
    pub targets: Vec<PlannedTarget>,
    pub items: Vec<MultiRunItemView>,
    /// Change counter: pass it back as `?since=` to get only what changed.
    #[serde(default)]
    pub seq: u64,
    /// `items` holds only the runs changed after `since` and `targets` is
    /// empty — merge into the previous view by `index`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub partial: bool,
}

/// `GET /db/multi-runs` — the caller's recent multi-runs (no items).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiRunJobBrief {
    pub id: String,
    pub status: JobStatus,
    pub engine: Engine,
    pub created_at: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<String>,
    pub statement_preview: String,
    pub target_count: usize,
    pub summary: MultiRunSummary,
}

/// `GET /db/multi-runs/{rid}/items/{index}` — one run in full.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MultiRunItemDetail {
    pub item: MultiRunItemView,
    pub connection_id: Id,
    pub node: Option<String>,
    pub statement: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<QueryResult>,
}

/// Validate the parameters and expand them into every value combination
/// (cartesian product, first parameter outermost). No parameters = one empty
/// combination.
pub fn expand_values(params: &[MultiRunParam]) -> Result<Vec<Vec<ParamValue>>> {
    let mut seen: Vec<&str> = Vec::new();
    for p in params {
        if !params::valid_name(&p.name) {
            return Err(Error::Invalid(format!(
                "parameter name `{}` is not a valid placeholder name",
                params::clip(&p.name, 40)
            )));
        }
        if seen.contains(&p.name.as_str()) {
            return Err(Error::Invalid(format!(
                "parameter `{}` is listed twice",
                p.name
            )));
        }
        seen.push(&p.name);
        if p.values.is_empty() {
            return Err(Error::Invalid(format!(
                "parameter `{}` has no values",
                p.name
            )));
        }
        if p.values.len() > MAX_VALUES {
            return Err(Error::Invalid(format!(
                "parameter `{}` has {} values (max {MAX_VALUES})",
                p.name,
                p.values.len()
            )));
        }
    }
    let mut combos: Vec<Vec<ParamValue>> = vec![Vec::new()];
    for p in params {
        let mut next = Vec::with_capacity(combos.len() * p.values.len());
        for combo in &combos {
            for v in &p.values {
                let mut c = combo.clone();
                c.push(ParamValue {
                    name: p.name.clone(),
                    value: v.clone(),
                });
                next.push(c);
                if next.len() > MAX_RUNS {
                    return Err(Error::Invalid(format!(
                        "the parameter values expand to more than {MAX_RUNS} runs"
                    )));
                }
            }
        }
        combos = next;
    }
    Ok(combos)
}

/// `brand=1, region=eu` (values clipped) — empty for no parameters.
pub fn values_label(values: &[ParamValue]) -> String {
    values
        .iter()
        .map(|v| format!("{}={}", v.name, params::clip(&v.value, 32)))
        .collect::<Vec<_>>()
        .join(", ")
}

/// Digest of the plan's (connection, node, final statement) triples, in order.
pub fn plan_hash(targets: &[PlannedTarget], runs: &[PlannedRun]) -> String {
    let mut h = Sha256::new();
    for r in runs {
        let t = &targets[r.target];
        h.update(t.connection_id.as_bytes());
        h.update([0]);
        h.update(t.node.as_deref().unwrap_or("").as_bytes());
        h.update([0]);
        h.update(r.statement.as_bytes());
        h.update([0xff]);
    }
    h.finalize()
        .iter()
        .take(16)
        .map(|b| format!("{b:02x}"))
        .collect()
}

/// Short single-line statement preview for job views.
pub fn preview(statement: &str) -> String {
    params::clip(statement, 160)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(name: &str, values: &[&str]) -> MultiRunParam {
        MultiRunParam {
            name: name.into(),
            values: values.iter().map(|s| s.to_string()).collect(),
            var_type: VarType::String,
            escape: true,
        }
    }

    #[test]
    fn expands_a_cartesian_product_first_param_outermost() {
        let combos = expand_values(&[p("brand", &["1", "2"]), p("env", &["a", "b", "c"])]).unwrap();
        assert_eq!(combos.len(), 6);
        assert_eq!(values_label(&combos[0]), "brand=1, env=a");
        assert_eq!(values_label(&combos[5]), "brand=2, env=c");
        assert_eq!(expand_values(&[]).unwrap(), vec![Vec::<ParamValue>::new()]);
    }

    #[test]
    fn rejects_bad_params() {
        assert!(expand_values(&[p("1x", &["a"])]).is_err());
        assert!(expand_values(&[p("a", &[])]).is_err());
        assert!(expand_values(&[p("a", &["1"]), p("a", &["2"])]).is_err());
        let many: Vec<String> = (0..20).map(|i| i.to_string()).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        // 20 × 20 = 400 > MAX_RUNS.
        assert!(expand_values(&[p("a", &refs), p("b", &refs)]).is_err());
    }

    #[test]
    fn job_wire_shapes_are_snake_case() {
        let s = MultiRunSummary::of(
            [
                RunStatus::Ok,
                RunStatus::Failed,
                RunStatus::Ok,
                RunStatus::Skipped,
            ]
            .into_iter(),
        );
        assert_eq!((s.total, s.ok, s.failed, s.skipped), (4, 2, 1, 1));
        assert_eq!(
            serde_json::to_value(RunStatus::Cancelled).unwrap(),
            serde_json::json!("cancelled")
        );
        assert_eq!(
            serde_json::to_value(ClusterSource::ReplicatedDatabase).unwrap(),
            serde_json::json!("replicated_database")
        );
        let req: StartMultiRunReq = serde_json::from_value(serde_json::json!({
            "statement": "SELECT :b",
            "targets": [{"connection_id": "c1", "node": "shop"}],
            "params": [{"name": "b", "values": ["1"], "type": "number"}]
        }))
        .unwrap();
        assert!(req.stop_on_error, "stop-on-error defaults on");
        assert!(!req.confirm_write);
        assert_eq!(req.spec.targets[0].cluster_mode, ClusterMode::Auto);
        assert_eq!(req.spec.params[0].var_type, VarType::Number);
        assert!(req.spec.params[0].escape);
    }
}
