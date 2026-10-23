//! History rows (recording, caps, retention) and the storage gauge cache.

use super::*;

// ===========================================================================
// Storage gauge + on-demand retention (perf N2)
// ===========================================================================

/// How long one workspace's storage gauge is reused (the counts are an
/// indexed scan, but the History list mounts often).
pub(crate) const STORAGE_GAUGE_TTL: Duration = Duration::from_secs(60);

pub(crate) fn storage_gauge_cache() -> &'static StdMutex<HashMap<Id, (Instant, ApiClientStorage)>> {
    static CACHE: OnceLock<StdMutex<HashMap<Id, (Instant, ApiClientStorage)>>> = OnceLock::new();
    CACHE.get_or_init(|| StdMutex::new(HashMap::new()))
}

/// Drop `wid`'s cached gauge — after a prune, a clear or a retention pass.
pub fn invalidate_storage_gauge(wid: &Id) {
    storage_gauge_cache()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .remove(wid);
}

/// `wid`'s history / run-report sizes, reused for [`STORAGE_GAUGE_TTL`].
pub async fn storage_gauge(repo: &ApiClientRepo, wid: &Id) -> EngineResult<ApiClientStorage> {
    if let Some((at, stats)) = storage_gauge_cache()
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .get(wid)
    {
        if at.elapsed() < STORAGE_GAUGE_TTL {
            return Ok(stats.clone());
        }
    }
    let stats = repo.storage_stats(wid).await?;
    let mut cache = storage_gauge_cache()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    cache.retain(|_, (at, _)| at.elapsed() < STORAGE_GAUGE_TTL);
    cache.insert(wid.clone(), (Instant::now(), stats.clone()));
    Ok(stats)
}

/// Insert a history row best-effort, stamp its source metadata, then notify
/// workspace clients. A failed insert emits no event.
pub(crate) async fn record_history<C: ApiClientCtx>(
    ctx: &C,
    repo: &ApiClientRepo,
    mut history: NewApiHistory,
    source: &Value,
    session_id: Option<Id>,
    request_id: Option<Id>,
) -> Option<Id> {
    if !history.request.is_object() {
        history.request = Value::Object(serde_json::Map::new());
    }
    let request = history
        .request
        .as_object_mut()
        .expect("history request normalized to an object");
    request.insert(
        "request_id".into(),
        request_id.clone().map_or(Value::Null, Value::String),
    );
    request.entry("name").or_insert(Value::Null);
    request.insert("source".into(), source.clone());
    cap_history_request(request);
    cap_history_body(&mut history.response);
    let entry = repo.insert_history(history).await.ok()?;
    // Retention (runtime cap, no migration) runs OFF the response path and at
    // most every PRUNE_MIN_INTERVAL per workspace (perf F5: a workspace read
    // plus a DELETE scan used to precede every `/execute` reply).
    schedule_history_prune(ctx, repo, &entry.workspace_id);
    let source_kind = source
        .get("kind")
        .and_then(Value::as_str)
        .unwrap_or("human")
        .to_string();
    ctx.broadcast(Event::ApiHistoryAppended {
        workspace_id: entry.workspace_id,
        entry_id: entry.id.clone(),
        source: source_kind,
        session_id,
        request_id,
    });
    Some(entry.id)
}

/// Minimum gap between two background history-retention passes for one
/// workspace. Rows over a limit survive at most this long (or until the next
/// send after it); nothing is pruned when no limit is set.
pub(crate) const PRUNE_MIN_INTERVAL: Duration = Duration::from_secs(30);

/// Run the workspace's history retention in the background, throttled to one
/// pass per [`PRUNE_MIN_INTERVAL`]. Best-effort, like the inline prune it
/// replaced.
pub(crate) fn schedule_history_prune<C: ApiClientCtx>(ctx: &C, repo: &ApiClientRepo, wid: &Id) {
    static LAST: OnceLock<StdMutex<HashMap<Id, Instant>>> = OnceLock::new();
    {
        let mut last = LAST
            .get_or_init(|| StdMutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|p| p.into_inner());
        if last
            .get(wid)
            .is_some_and(|at| at.elapsed() < PRUNE_MIN_INTERVAL)
        {
            return;
        }
        last.insert(wid.clone(), Instant::now());
    }
    let (ctx, repo, wid) = (ctx.clone(), repo.clone(), wid.clone());
    tokio::spawn(async move {
        let (max_rows, max_days) = history_retention(&ctx, &wid).await;
        if (max_rows > 0 || max_days > 0)
            && repo
                .prune_history(&wid, max_rows, max_days)
                .await
                .is_ok_and(|n| n > 0)
        {
            invalidate_storage_gauge(&wid);
        }
    });
}

/// Request body kept in a history row (perf F5: it was uncapped — up to the
/// 2 MB request limit, base64 multipart files included).
pub(crate) const HISTORY_REQUEST_BODY_MAX: usize = 64 * 1024;

/// Shrink a history row's `request` snapshot: multipart `file` parts keep
/// their `filename` and decoded `size` but drop the base64 `value`, and the
/// `body` is then capped at [`HISTORY_REQUEST_BODY_MAX`] (UTF-8 safe) with
/// `request_truncated: true`. The live send already used the full body.
pub(crate) fn cap_history_request(request: &mut serde_json::Map<String, Value>) {
    let multipart = matches!(
        request.get("body_mode").and_then(Value::as_str),
        Some("multipart" | "form-data" | "formdata")
    );
    let mut cut = false;
    if let Some(Value::String(body)) = request.get_mut("body") {
        if multipart && body.trim_start().starts_with('[') {
            if let Ok(Value::Array(mut fields)) = serde_json::from_str::<Value>(body) {
                let mut changed = false;
                for field in fields.iter_mut().filter_map(Value::as_object_mut) {
                    if field.get("type").and_then(Value::as_str) != Some("file") {
                        continue;
                    }
                    if let Some(Value::String(b64)) = field.get("value") {
                        let encoded = b64.trim();
                        let size = (encoded.len() / 4 * 3).saturating_sub(
                            encoded
                                .chars()
                                .rev()
                                .take_while(|c| *c == '=')
                                .count()
                                .min(2),
                        );
                        field.insert("value".into(), Value::String(String::new()));
                        field.insert("size".into(), json!(size));
                        field.insert("missing_content".into(), Value::Bool(true));
                        cut = true;
                        changed = true;
                    }
                }
                if changed {
                    *body = Value::Array(fields).to_string();
                }
            }
        }
        if body.len() > HISTORY_REQUEST_BODY_MAX {
            truncate_string(body, HISTORY_REQUEST_BODY_MAX);
            cut = true;
        }
    }
    if cut {
        request.insert("request_truncated".into(), Value::Bool(true));
    }
}

/// Default history retention per workspace: newest rows kept …
///
/// `0` (no limit) by default — retention is opt-in. Pruning deletes the
/// user's recorded runs, so a workspace only loses history after someone
/// picks a limit in the History list's retention control.
pub const HISTORY_KEEP_ROWS_DEFAULT: i64 = 0;
/// … and maximum age in days (`0` = no limit, same opt-in rule).
pub const HISTORY_KEEP_DAYS_DEFAULT: i64 = 0;
/// Response body kept in a history row. The live response already carried the
/// full body (up to the 512 KB display cap); history keeps a preview.
pub(crate) const HISTORY_BODY_MAX: usize = 64 * 1024;

/// The workspace's history retention `(max_rows, max_days)` from
/// `settings.api_client.history_max_rows` / `history_max_days` (non-negative
/// integers; `0` disables that limit), defaulting to
/// [`HISTORY_KEEP_ROWS_DEFAULT`] / [`HISTORY_KEEP_DAYS_DEFAULT`].
pub async fn history_retention<C: ApiClientCtx>(ctx: &C, wid: &Id) -> (i64, i64) {
    retention_from_settings(ctx.workspace_settings(wid).await.as_ref())
}

/// Longest honoured "keep N days" (~100 years). A larger value (someone typing
/// `99999999` to mean "forever") is clamped here; `prune_history` additionally
/// treats an unrepresentable cutoff as "no age limit" (backend-correctness-2-01:
/// `now - days(n)` used to panic every prune).
pub const HISTORY_KEEP_DAYS_MAX: i64 = 36_500;

/// [`history_retention`] over an already-loaded settings object.
pub fn retention_from_settings(settings: Option<&Value>) -> (i64, i64) {
    let limit = |key: &str, default: i64| -> i64 {
        settings
            .and_then(|s| s.get("api_client"))
            .and_then(|a| a.get(key))
            .and_then(Value::as_i64)
            .filter(|v| *v >= 0)
            .unwrap_or(default)
    };
    (
        limit("history_max_rows", HISTORY_KEEP_ROWS_DEFAULT),
        limit("history_max_days", HISTORY_KEEP_DAYS_DEFAULT).min(HISTORY_KEEP_DAYS_MAX),
    )
}

/// Cap a history row's stored response `body` at [`HISTORY_BODY_MAX`] (UTF-8
/// safe), flagging it `truncated`, so polling automations / agent loops can't
/// balloon the state DB with full bodies.
pub(crate) fn cap_history_body(response: &mut Value) {
    let mut cut = false;
    if let Some(Value::String(body)) = response.get_mut("body") {
        if body.len() > HISTORY_BODY_MAX {
            truncate_string(body, HISTORY_BODY_MAX);
            cut = true;
        }
    }
    if cut {
        if let Some(obj) = response.as_object_mut() {
            obj.insert("truncated".into(), Value::Bool(true));
        }
    }
}
