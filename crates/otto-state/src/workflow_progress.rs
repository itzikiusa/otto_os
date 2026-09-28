//! Lightweight workflow projections. Recovery JSON is never rewritten by repair.
use crate::convert::dberr;
use otto_core::workflows::{NodeRunState, WorkflowCheckpoint};
use otto_core::{Error, Id, Result};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqliteConnection};
use crate::DbPool;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock, Weak};
use tokio::sync::{Mutex as AsyncMutex, Semaphore};

fn json_error(error: impl std::fmt::Display) -> Error {
    Error::Internal(format!("workflow projection: {error}"))
}
fn busy() -> Error {
    Error::Conflict("workflow progress busy; retry shortly".into())
}
fn version(value: &Value) -> Result<String> {
    Ok(format!(
        "{:x}",
        Sha256::digest(serde_json::to_vec(value).map_err(json_error)?)
    ))
}
fn clip(value: Option<&Value>, max: usize) -> Value {
    value
        .and_then(Value::as_str)
        .map(|s| Value::String(s.chars().take(max).collect()))
        .unwrap_or(Value::Null)
}

fn project(value: &Value, checkpoint: bool) -> Result<Value> {
    let mut out = serde_json::Map::new();
    for key in [
        "node_id",
        "status",
        "started_at",
        "duration_ms",
        "attempts",
        "sessions",
        "review_ids",
    ] {
        if let Some(value) = value.get(key) {
            out.insert(key.into(), value.clone());
        }
    }
    if checkpoint {
        for key in [
            "loop_id",
            "iteration",
            "step_index",
            "kind",
            "name",
            "updated_at",
        ] {
            if let Some(value) = value.get(key) {
                out.insert(key.into(), value.clone());
            }
        }
        out.insert("input".into(), Value::Null);
    }
    out.insert("error".into(), clip(value.get("error"), 1024));
    out.insert("output".into(), Value::Null);
    out.insert("logs".into(), json!([]));
    out.insert(
        "log_count".into(),
        json!(value
            .get("logs")
            .and_then(Value::as_array)
            .map_or(0, Vec::len)),
    );
    out.insert(
        "has_output".into(),
        json!(value.get("output").is_some_and(|v| !v.is_null())),
    );
    out.insert("detail_version".into(), json!(version(value)?));
    if let Some(activity) = value.get("activity").filter(|v| !v.is_null()) {
        let mut small = activity.clone();
        for key in ["phase", "hold_reason"] {
            small[key] = clip(activity.get(key), 512);
        }
        if let Some(agents) = small.get_mut("subagents").and_then(Value::as_array_mut) {
            agents.truncate(32);
        }
        out.insert("activity".into(), small);
    }
    Ok(Value::Object(out))
}
pub fn nodes_projection(nodes: &[NodeRunState]) -> Result<String> {
    let values = nodes
        .iter()
        .map(|node| {
            serde_json::to_value(node)
                .map_err(json_error)
                .and_then(|v| project(&v, false))
        })
        .collect::<Result<Vec<_>>>()?;
    serde_json::to_string(&values).map_err(json_error)
}
/// Per-node projection memo for [`progress_write`]: run id → per node index
/// `(hash, len)` of the node's serialized JSON → its serialized projection.
/// A live step's log line used to re-project (`to_value` + SHA-256) EVERY node
/// of the run; now only nodes whose JSON changed are re-projected. Bounded to
/// the most recently written runs.
type ProjectionMemo = (
    HashMap<String, Vec<(u64, usize, Arc<str>)>>,
    std::collections::VecDeque<String>,
);
const PROJECTION_MEMO_RUNS: usize = 32;

fn projection_memo() -> &'static Mutex<ProjectionMemo> {
    static MEMO: OnceLock<Mutex<ProjectionMemo>> = OnceLock::new();
    MEMO.get_or_init(Default::default)
}

fn fingerprint(s: &str) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    s.hash(&mut h);
    h.finish()
}

/// `(serde_json::to_string(nodes), nodes_projection(nodes))` — byte-identical
/// — for the run `id`, reusing the memoized projection of every node whose
/// serialized JSON did not change since the run's previous write. Blocking CPU
/// work (serialize + hash): call off the async runtime.
pub fn progress_write(id: &str, nodes: &[NodeRunState]) -> Result<(String, String)> {
    let parts = nodes
        .iter()
        .map(serde_json::to_string)
        .collect::<std::result::Result<Vec<_>, _>>()
        .map_err(|e| Error::Internal(e.to_string()))?;
    let prev = {
        let mut memo = projection_memo().lock().unwrap_or_else(|e| e.into_inner());
        memo.0.remove(id).unwrap_or_default()
    };
    let mut next = Vec::with_capacity(parts.len());
    for (i, part) in parts.iter().enumerate() {
        let key = (fingerprint(part), part.len());
        let projected = match prev.get(i) {
            Some((h, l, p)) if (*h, *l) == key => p.clone(),
            _ => {
                // Exactly `nodes_projection`'s per-node step.
                let value = serde_json::to_value(&nodes[i]).map_err(json_error)?;
                Arc::from(serde_json::to_string(&project(&value, false)?).map_err(json_error)?)
            }
        };
        next.push((key.0, key.1, projected));
    }
    let nodes_json = join_array(parts.iter().map(String::as_str));
    let projection = join_array(next.iter().map(|(_, _, p)| &**p));
    let mut memo = projection_memo().lock().unwrap_or_else(|e| e.into_inner());
    let (runs, order) = &mut *memo;
    order.retain(|r| r != id);
    order.push_back(id.to_string());
    runs.insert(id.to_string(), next);
    while order.len() > PROJECTION_MEMO_RUNS {
        if let Some(old) = order.pop_front() {
            runs.remove(&old);
        }
    }
    Ok((nodes_json, projection))
}

/// `[a,b,…]` from already-serialized elements — exactly what serde_json's
/// compact serializer writes for a sequence.
fn join_array<'a>(parts: impl Iterator<Item = &'a str> + Clone) -> String {
    let cap = parts.clone().map(|p| p.len() + 1).sum::<usize>() + 2;
    let mut out = String::with_capacity(cap);
    out.push('[');
    for (i, p) in parts.enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(p);
    }
    out.push(']');
    out
}

pub fn checkpoint_projection(checkpoint: &WorkflowCheckpoint) -> Result<String> {
    serde_json::to_string(&project(
        &serde_json::to_value(checkpoint).map_err(json_error)?,
        true,
    )?)
    .map_err(json_error)
}
pub async fn publish_nodes(conn: &mut SqliteConnection, id: &str, projection: &str) -> Result<()> {
    sqlx::query("UPDATE workflow_runs SET progress_json=? WHERE id=?")
        .bind(projection)
        .bind(id)
        .execute(conn)
        .await
        .map_err(dberr("publish workflow projection"))?;
    Ok(())
}
pub async fn publish_checkpoint(
    conn: &mut SqliteConnection,
    id: &str,
    node: &str,
    projection: &str,
) -> Result<()> {
    sqlx::query("UPDATE workflow_checkpoints SET summary_json=? WHERE run_id=? AND node_id=?")
        .bind(projection)
        .bind(id)
        .bind(node)
        .execute(conn)
        .await
        .map_err(dberr("publish checkpoint projection"))?;
    Ok(())
}

// Weak keyed gates prevent duplicate repair without retaining a history of runs.
fn repair_gate(id: &str) -> Arc<AsyncMutex<()>> {
    static GATES: OnceLock<Mutex<HashMap<String, Weak<AsyncMutex<()>>>>> = OnceLock::new();
    let mut gates = GATES
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(|e| e.into_inner());
    gates.retain(|_, gate| gate.strong_count() > 0);
    if let Some(gate) = gates.get(id).and_then(Weak::upgrade) {
        return gate;
    }
    let gate = Arc::new(AsyncMutex::new(()));
    gates.insert(id.into(), Arc::downgrade(&gate));
    gate
}
async fn repair(pool: &DbPool, id: &str) -> Result<()> {
    static WORKERS: OnceLock<Arc<Semaphore>> = OnceLock::new();
    let gate = repair_gate(id).try_lock_owned().map_err(|_| busy())?;
    let permit = WORKERS
        .get_or_init(|| Arc::new(Semaphore::new(2)))
        .clone()
        .try_acquire_owned()
        .map_err(|_| busy())?;
    let pool = pool.clone();
    let id = id.to_string();
    // Keep admission through cancellation and SQL publication, not just decode.
    tokio::spawn(async move {
        let _gate=gate; let _permit=permit;
        let row=sqlx::query("SELECT nodes_json,rev,progress_json IS NULL AS missing FROM workflow_runs WHERE id=?").bind(&id).fetch_one(&pool).await.map_err(dberr("read workflow repair"))?;
        if row.get::<bool,_>("missing") {
            let body:String=row.get("nodes_json"); let rev:i64=row.get("rev"); let owned=body.clone();
            let projection=tokio::task::spawn_blocking(move || {
                let nodes:Vec<Value>=serde_json::from_str(&owned).map_err(json_error)?;
                let summary=nodes.iter().map(|node|project(node,false)).collect::<Result<Vec<_>>>()?;
                serde_json::to_string(&summary).map_err(json_error)
            }).await.map_err(json_error)??;
            let updated=sqlx::query("UPDATE workflow_runs SET progress_json=?,rev=rev+1 WHERE id=? AND rev=? AND nodes_json=? AND progress_json IS NULL")
                .bind(projection).bind(&id).bind(rev).bind(body).execute(&pool).await.map_err(dberr("repair workflow projection"))?.rows_affected();
            if updated==0 {return Err(busy());}
        }
        // Cold/import repair holds only one checkpoint body at a time. Bound a
        // request's repair work; another explicit retry continues large imports.
        for _ in 0..200 {
            let row=sqlx::query("SELECT node_id,checkpoint_json FROM workflow_checkpoints WHERE run_id=? AND summary_json IS NULL LIMIT 1").bind(&id).fetch_optional(&pool).await.map_err(dberr("read checkpoint repair"))?;
            let Some(row)=row else {return Ok(())};
            let node:String=row.get("node_id"); let body:String=row.get("checkpoint_json"); let owned=body.clone();
            let projection=tokio::task::spawn_blocking(move || {
                let value:Value=serde_json::from_str(&owned).map_err(json_error)?;
                serde_json::to_string(&project(&value,true)?).map_err(json_error)
            }).await.map_err(json_error)??;
            let mut tx=pool.begin().await.map_err(dberr("begin checkpoint repair"))?;
            let changed=sqlx::query("UPDATE workflow_checkpoints SET summary_json=? WHERE run_id=? AND node_id=? AND checkpoint_json=? AND summary_json IS NULL")
                .bind(projection).bind(&id).bind(node).bind(body).execute(&mut *tx).await.map_err(dberr("repair checkpoint projection"))?.rows_affected();
            if changed>0 {sqlx::query("UPDATE workflow_runs SET rev=rev+1,checkpoint_rev=checkpoint_rev+1 WHERE id=?").bind(&id).execute(&mut *tx).await.map_err(dberr("advance repaired checkpoint revision"))?;}
            tx.commit().await.map_err(dberr("commit checkpoint repair"))?;
        }
        Err(busy())
    }).await.map_err(json_error)?
}

const RUN_COLUMNS: &str="id,workflow_id,workspace_id,status,error,started_at,finished_at,rev,waiting_approval,approval_node_id,approved_by,created_by,approval_note,approved_at,workflow_version,proof_pack_id,resume_attempts,progress_json,checkpoint_rev,checkpoint_generation";
pub async fn progress(pool: &DbPool, id: &Id, after_rev: Option<i64>) -> Result<Value> {
    for attempt in 0..2 {
        let mut tx = pool
            .begin()
            .await
            .map_err(dberr("begin workflow progress"))?;
        let row = sqlx::query(&format!(
            "SELECT {RUN_COLUMNS} FROM workflow_runs WHERE id=?"
        ))
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(dberr("workflow progress"))?;
        let missing:bool=sqlx::query_scalar("SELECT EXISTS(SELECT 1 FROM workflow_checkpoints WHERE run_id=? AND summary_json IS NULL)").bind(id).fetch_one(&mut *tx).await.map_err(dberr("checkpoint projection state"))?;
        let projection: Option<String> = row.get("progress_json");
        if projection.is_none() || missing {
            tx.rollback()
                .await
                .map_err(dberr("close progress snapshot"))?;
            if attempt > 0 {
                return Err(busy());
            }
            repair(pool, id).await?;
            continue;
        }
        let rev: i64 = row.get("rev");
        if after_rev == Some(rev) {
            return Ok(json!({"changed":false,"rev":rev}));
        }
        let counts=sqlx::query("SELECT COUNT(*) AS total, COALESCE(SUM(json_extract(summary_json,'$.status')='success'),0) AS done FROM workflow_checkpoints WHERE run_id=?").bind(id).fetch_one(&mut *tx).await.map_err(dberr("checkpoint counts"))?;
        let mut run = serde_json::Map::new();
        for key in [
            "id",
            "workflow_id",
            "workspace_id",
            "status",
            "error",
            "started_at",
            "finished_at",
            "approval_node_id",
            "approved_by",
            "created_by",
            "approval_note",
            "approved_at",
            "proof_pack_id",
        ] {
            run.insert(
                key.into(),
                row.get::<Option<String>, _>(key)
                    .map_or(Value::Null, Value::String),
            );
        }
        for key in [
            "rev",
            "resume_attempts",
            "checkpoint_rev",
            "checkpoint_generation",
        ] {
            run.insert(key.into(), json!(row.get::<i64, _>(key)));
        }
        run.insert(
            "workflow_version".into(),
            json!(row.get::<Option<i64>, _>("workflow_version")),
        );
        run.insert(
            "waiting_approval".into(),
            json!(row.get::<bool, _>("waiting_approval")),
        );
        run.insert(
            "nodes".into(),
            serde_json::from_str(&projection.unwrap()).map_err(json_error)?,
        );
        run.insert("summary".into(), json!(true));
        run.insert("input".into(), Value::Null);
        run.insert("checkpoints".into(), json!([]));
        run.insert(
            "checkpoint_count".into(),
            json!(counts.get::<i64, _>("total")),
        );
        run.insert(
            "checkpoint_done".into(),
            json!(counts.get::<i64, _>("done")),
        );
        return Ok(json!({"changed":true,"rev":rev,"run":run}));
    }
    Err(busy())
}

/// rowid is assigned for ordinary/raw/restore INSERTs, omitted by archives,
/// and preserved by checkpoint UPSERT DO UPDATE. Deletion/retry change the
/// generation. Otto never VACUUMs or rewrites this table during pagination.
pub async fn checkpoint_page(
    pool: &DbPool,
    id: &str,
    generation: Option<i64>,
    after: i64,
    limit: usize,
) -> Result<Value> {
    // Also repairs archive/direct rows before advertising incomplete summaries.
    let _ = progress(pool, &id.into(), None).await?;
    let mut tx = pool.begin().await.map_err(dberr("begin checkpoint page"))?;
    let row =
        sqlx::query("SELECT checkpoint_generation,checkpoint_rev FROM workflow_runs WHERE id=?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(dberr("checkpoint generation"))?;
    let current: i64 = row.get("checkpoint_generation");
    if generation.is_some_and(|g| g != current) {
        return Err(Error::Conflict(
            "checkpoint list reset; reload checkpoints".into(),
        ));
    }
    let limit = limit.clamp(1, 200);
    let rows=sqlx::query("SELECT rowid,summary_json FROM workflow_checkpoints WHERE run_id=? AND rowid>? ORDER BY rowid LIMIT ?").bind(id).bind(after).bind((limit+1) as i64).fetch_all(&mut *tx).await.map_err(dberr("checkpoint page"))?;
    let more = rows.len() > limit;
    let mut last = after;
    let mut items = Vec::new();
    for row in rows.into_iter().take(limit) {
        last = row.get("rowid");
        items.push(
            serde_json::from_str::<Value>(
                &row.get::<Option<String>, _>("summary_json")
                    .ok_or_else(busy)?,
            )
            .map_err(json_error)?,
        );
    }
    Ok(
        json!({"checkpoint_rev":row.get::<i64,_>("checkpoint_rev"),"generation":current,"items":items,"next_after":if more {Some(last)} else {None}}),
    )
}

pub async fn detail(pool: &DbPool, id: &str, node: &str, checkpoint: bool) -> Result<Value> {
    let mut tx = pool.begin().await.map_err(dberr("begin workflow detail"))?;
    let rev: i64 = sqlx::query_scalar("SELECT rev FROM workflow_runs WHERE id=?")
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(dberr("workflow detail revision"))?;
    let body = if checkpoint {
        let raw: String = sqlx::query_scalar(
            "SELECT checkpoint_json FROM workflow_checkpoints WHERE run_id=? AND node_id=?",
        )
        .bind(id)
        .bind(node)
        .fetch_one(&mut *tx)
        .await
        .map_err(dberr("checkpoint detail"))?;
        serde_json::from_str::<Value>(&raw).map_err(json_error)?
    } else {
        let raw: String = sqlx::query_scalar("SELECT nodes_json FROM workflow_runs WHERE id=?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .map_err(dberr("node detail"))?;
        // Split into raw per-node slices and build a `Value` only for the
        // match: a run's nodes_json can be tens of MB, and this is fetched
        // once per viewed log line.
        #[derive(serde::Deserialize)]
        struct NodeKey<'a> {
            #[serde(borrow, default)]
            node_id: Option<std::borrow::Cow<'a, str>>,
        }
        let nodes: Vec<&serde_json::value::RawValue> =
            serde_json::from_str(&raw).map_err(json_error)?;
        let hit = nodes
            .into_iter()
            .find(|r| {
                serde_json::from_str::<NodeKey>(r.get())
                    .is_ok_and(|k| k.node_id.as_deref() == Some(node))
            })
            .ok_or_else(|| Error::NotFound("workflow node not found".into()))?;
        serde_json::from_str::<Value>(hit.get()).map_err(json_error)?
    };
    Ok(json!({"rev":rev,"detail_version":version(&body)?,"body":body}))
}

/// Run menu rows intentionally omit node/input/checkpoint bodies.
pub async fn run_summaries(pool: &DbPool, workflow: &str) -> Result<Vec<Value>> {
    let rows=sqlx::query("SELECT id,workflow_id,status,started_at,rev FROM workflow_runs WHERE workflow_id=? ORDER BY started_at DESC,id LIMIT 50").bind(workflow).fetch_all(pool).await.map_err(dberr("workflow run summaries"))?;
    Ok(rows.iter().map(|row|json!({"id":row.get::<String,_>("id"),"workflow_id":row.get::<String,_>("workflow_id"),"status":row.get::<String,_>("status"),"started_at":row.get::<String,_>("started_at"),"rev":row.get::<i64,_>("rev")})).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::WorkflowsRepo;
    use otto_core::workflows::{NodeStatus, WorkflowCheckpoint, WorkflowGraph};
    async fn fixture() -> (DbPool, Id) {
        let options = sqlx::sqlite::SqliteConnectOptions::new()
            .in_memory(true)
            .foreign_keys(false);
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect_with(options)
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let repo = WorkflowsRepo::new(pool.clone());
        let wf = repo
            .create(
                &"ws".into(),
                "test",
                "",
                "",
                &WorkflowGraph::default(),
                &"owner".into(),
            )
            .await
            .unwrap();
        let run = repo
            .create_run(
                &wf.id,
                &wf.workspace_id,
                &json!({"large_input":"x".repeat(512*1024)}),
                None,
            )
            .await
            .unwrap();
        (pool.into(), run.id)
    }
    /// r3-07-03: a progress write rewrites the run row ONCE (it was three
    /// times: the UPDATE, the 0130 trigger NULLing the projection, and the
    /// republish), keeps a valid projection, and a raw write that does not
    /// bump `rev` is still invalidated by the trigger.
    #[tokio::test]
    async fn progress_write_rewrites_the_row_once() {
        let (pool, id) = fixture().await;
        let repo = WorkflowsRepo::new(pool.clone());
        sqlx::query("CREATE TABLE run_writes (n INTEGER NOT NULL)")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("INSERT INTO run_writes VALUES (0)").execute(&pool).await.unwrap();
        sqlx::query(
            "CREATE TRIGGER count_run_writes AFTER UPDATE ON workflow_runs BEGIN \
             UPDATE run_writes SET n = n + 1; END",
        )
        .execute(&pool)
        .await
        .unwrap();
        let writes = || async {
            sqlx::query_scalar::<_, i64>("SELECT n FROM run_writes")
                .fetch_one(&pool)
                .await
                .unwrap()
        };
        let rev0: i64 = sqlx::query_scalar("SELECT rev FROM workflow_runs WHERE id=?")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
        let nodes: Vec<NodeRunState> = serde_json::from_value(json!([
            {"node_id":"a","status":"running","logs":["⏳ working"]}
        ]))
        .unwrap();
        let rev = repo.update_run_progress(&id, &nodes).await.unwrap();
        assert_eq!(writes().await, 1, "one row rewrite per progress write");
        assert_eq!(rev, rev0 + 1);
        let stored: Option<String> =
            sqlx::query_scalar("SELECT progress_json FROM workflow_runs WHERE id=?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored, Some(nodes_projection(&nodes).unwrap()));
        // The terminal write path is a single statement too.
        repo.update_run(&id, otto_core::workflows::RunStatus::Success, &nodes, None, true)
            .await
            .unwrap();
        assert_eq!(writes().await, 2);
        // A raw restore-style write (rev untouched) is still invalidated.
        sqlx::query("UPDATE workflow_runs SET nodes_json='[]' WHERE id=?")
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        let (p, r): (Option<String>, i64) =
            sqlx::query_as("SELECT progress_json, rev FROM workflow_runs WHERE id=?")
                .bind(&id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(p.is_none(), "raw body write drops the projection");
        assert_eq!(r, rev0 + 3);
    }

    /// r3-07-02: the projector's narrow read agrees with the full run.
    #[tokio::test]
    async fn run_head_matches_the_full_run_without_parsing_bodies() {
        let (pool, id) = fixture().await;
        let repo = WorkflowsRepo::new(pool.clone());
        let full = repo.get_run(&id).await.unwrap();
        let head = repo.run_head(&id, true).await.unwrap().unwrap();
        assert_eq!(head.id, full.id);
        assert_eq!(head.workflow_id, full.workflow_id);
        assert_eq!(head.workspace_id, full.workspace_id);
        assert_eq!(head.status, full.status.as_str());
        assert_eq!(head.error, full.error);
        assert_eq!(head.node_count, Some(full.nodes.len() as u32));
        assert_eq!(head.workflow_name.as_deref(), Some("test"));
        assert_eq!(repo.run_head(&id, false).await.unwrap().unwrap().node_count, None);
        assert!(repo.run_head(&"nope".into(), true).await.unwrap().is_none());
        assert_eq!(repo.recent_run_ids(&full.workflow_id, 5).await.unwrap(), vec![id]);
    }
    #[tokio::test]
    async fn large_checkpoint_bodies_are_absent_from_progress_and_unchanged_poll() {
        let (pool, id) = fixture().await;
        let repo = WorkflowsRepo::new(pool.clone());
        for n in 0..30 {
            repo.save_checkpoint(
                &id,
                &WorkflowCheckpoint {
                    node_id: format!("loop#{n}"),
                    loop_id: "loop".into(),
                    iteration: n,
                    step_index: 0,
                    kind: "http_request".into(),
                    name: "call".into(),
                    status: NodeStatus::Success,
                    attempts: 1,
                    input: Value::Null,
                    output: Some(json!({"body":"x".repeat(512*1024)})),
                    error: None,
                    logs: vec![],
                    updated_at: chrono::Utc::now(),
                },
            )
            .await
            .unwrap();
        }
        let snapshot = progress(&pool, &id, None).await.unwrap();
        assert!(
            snapshot.to_string().len() < 8192,
            "progress must omit30*512KiB bodies and large run input"
        );
        let rev = snapshot["rev"].as_i64().unwrap();
        assert_eq!(
            progress(&pool, &id, Some(rev)).await.unwrap(),
            json!({"changed":false,"rev":rev})
        );
        assert_eq!(
            repo.checkpoints(&id).await.unwrap()[0]
                .output
                .as_ref()
                .unwrap()["body"]
                .as_str()
                .unwrap()
                .len(),
            512 * 1024
        );
    }
    fn checkpoint(node: &str) -> WorkflowCheckpoint {
        WorkflowCheckpoint {
            node_id: node.into(),
            loop_id: "loop".into(),
            iteration: 1,
            step_index: 0,
            kind: "http_request".into(),
            name: "call".into(),
            status: NodeStatus::Success,
            attempts: 1,
            input: json!({"request":"exact"}),
            output: Some(json!({"body":"original"})),
            error: None,
            logs: vec!["line".into()],
            updated_at: chrono::Utc::now(),
        }
    }
    #[tokio::test]
    async fn raw_insert_and_update_repair_exact_bodies_before_conditional_poll() {
        let (pool, id) = fixture().await;
        let first = progress(&pool, &id, None).await.unwrap();
        let rev = first["rev"].as_i64().unwrap();
        let body =
            json!([{"node_id":"a","status":"success","output":{"exact":"large"},"logs":["saved"]}])
                .to_string();
        sqlx::query("UPDATE workflow_runs SET nodes_json=?,progress_json='[]' WHERE id=?")
            .bind(&body)
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        let next = progress(&pool, &id, Some(rev)).await.unwrap();
        assert_eq!(next["changed"], true);
        assert_eq!(next["run"]["nodes"][0]["node_id"], "a");
        let exact = detail(&pool, &id, "a", false).await.unwrap();
        assert_eq!(exact["body"]["output"]["exact"], "large");
        assert_eq!(
            exact["detail_version"],
            next["run"]["nodes"][0]["detail_version"]
        );
        let stored: String = sqlx::query_scalar("SELECT nodes_json FROM workflow_runs WHERE id=?")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(stored, body);
        let cp = checkpoint("loop#1");
        let raw = serde_json::to_string(&cp).unwrap();
        sqlx::query("INSERT INTO workflow_checkpoints(run_id,node_id,checkpoint_json,summary_json) VALUES (?,?,?,'{}')").bind(&id).bind(&cp.node_id).bind(&raw).execute(&pool).await.unwrap();
        let oldrev = next["rev"].as_i64().unwrap();
        let changed = progress(&pool, &id, Some(oldrev)).await.unwrap();
        assert_eq!(changed["changed"], true);
        assert_eq!(changed["run"]["checkpoint_count"], 1);
        assert_eq!(
            detail(&pool, &id, &cp.node_id, true).await.unwrap()["body"],
            serde_json::to_value(&cp).unwrap()
        );
    }
    #[tokio::test]
    async fn checkpoint_pages_survive_updates_append_and_raw_derived_order_hints() {
        let (pool, id) = fixture().await;
        let repo = WorkflowsRepo::new(pool.clone());
        for node in ["loop#2", "loop#10", "loop#1"] {
            let mut raw = serde_json::to_value(checkpoint(node)).unwrap();
            raw["ordinal"] = json!(7);
            sqlx::query(
                "INSERT INTO workflow_checkpoints(run_id,node_id,checkpoint_json) VALUES (?,?,?)",
            )
            .bind(&id)
            .bind(node)
            .bind(raw.to_string())
            .execute(&pool)
            .await
            .unwrap();
        }
        let first = checkpoint_page(&pool, &id, None, 0, 1).await.unwrap();
        let generation = first["generation"].as_i64().unwrap();
        let after = first["next_after"].as_i64().unwrap();
        assert_eq!(first["items"][0]["node_id"], "loop#2");
        repo.save_checkpoint(&id, &checkpoint("loop#2"))
            .await
            .unwrap();
        repo.update_run_progress(&id, &[]).await.unwrap();
        repo.save_checkpoint(&id, &checkpoint("loop#20"))
            .await
            .unwrap();
        let rest = checkpoint_page(&pool, &id, Some(generation), after, 100)
            .await
            .unwrap();
        assert_eq!(
            rest["items"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v["node_id"].as_str().unwrap())
                .collect::<Vec<_>>(),
            vec!["loop#10", "loop#1", "loop#20"]
        );
        assert_eq!(rest["generation"], generation);
        sqlx::query("DELETE FROM workflow_checkpoints WHERE run_id=? AND node_id='loop#1'")
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(matches!(
            checkpoint_page(&pool, &id, Some(generation), after, 100).await,
            Err(Error::Conflict(_))
        ));
    }
    #[tokio::test]
    async fn invalid_recovery_json_is_never_replaced_by_empty_projection() {
        let (pool, id) = fixture().await;
        sqlx::query("UPDATE workflow_runs SET nodes_json='invalid' WHERE id=?")
            .bind(&id)
            .execute(&pool)
            .await
            .unwrap();
        assert!(progress(&pool, &id, Some(0)).await.is_err());
        let body: String = sqlx::query_scalar("SELECT nodes_json FROM workflow_runs WHERE id=?")
            .bind(&id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(body, "invalid");
    }

    #[tokio::test]
    async fn absent_checkpoint_detail_is_not_found() {
        let (pool, id) = fixture().await;
        assert!(matches!(
            detail(&pool, &id, "missing", true).await,
            Err(Error::NotFound(_))
        ));
    }
}

#[cfg(test)]
mod progress_write_tests {
    use super::*;
    use otto_core::workflows::NodeStatus;

    fn node(id: &str, status: &str, logs: usize) -> NodeRunState {
        serde_json::from_value(json!({
            "node_id": id,
            "status": status,
            "output": {"f": 0.1 + 0.2, "n": -3, "big": u64::MAX, "s": "x\u{2028}é"},
            "logs": (0..logs).map(|i| format!("⏳ line {i}")).collect::<Vec<_>>(),
            "sessions": ["s1"],
        }))
        .unwrap()
    }

    /// `progress_write` is byte-for-byte `(to_string(nodes), nodes_projection)`
    /// on every write, while a node's log grows — the memo only ever returns
    /// the projection of an unchanged node.
    #[test]
    fn progress_write_matches_the_whole_run_projection() {
        let run = "progress-write-test-run";
        let mut nodes: Vec<NodeRunState> = (0..40)
            .map(|i| {
                node(
                    &format!("n{i}"),
                    if i < 10 { "success" } else { "pending" },
                    3,
                )
            })
            .collect();
        for tick in 0..30 {
            nodes[10].logs.push(format!("⏳ tick {tick}"));
            if tick == 12 {
                nodes[10].status = NodeStatus::Success;
                nodes[11].status = NodeStatus::Running;
            }
            let (nodes_json, projection) = progress_write(run, &nodes).unwrap();
            assert_eq!(nodes_json, serde_json::to_string(&nodes).unwrap());
            assert_eq!(projection, nodes_projection(&nodes).unwrap(), "tick {tick}");
        }
        // Shrinking / reordering the run never reuses a stale entry.
        nodes.truncate(5);
        nodes.swap(0, 4);
        let (_, projection) = progress_write(run, &nodes).unwrap();
        assert_eq!(projection, nodes_projection(&nodes).unwrap());
    }
}

#[cfg(test)]
mod review_association_tests {
    #[test]
    fn lightweight_projection_retains_review_association() {
        let full = serde_json::json!({"node_id":"review", "status":"success",
            "review_ids":["review-id"], "sessions":["first"], "output":{"large":"body"}});
        let summary = super::project(&full, false).unwrap();
        assert_eq!(summary["review_ids"], full["review_ids"]);
        assert_eq!(summary["sessions"], full["sessions"]);
        assert!(summary["output"].is_null());
    }
}
