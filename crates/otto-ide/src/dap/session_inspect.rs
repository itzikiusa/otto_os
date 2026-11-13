//! Breakpoint synchronization, `stopped` event stack/scope/variable inspection,
//! and IntelliJ-style inline ghost variable value extraction for [`super::session::DapManager`].

use std::collections::{BTreeMap, BTreeSet, HashMap};

use otto_core::{Error, Result};
use serde_json::{json, Value};

use super::read_target_file_for_inline;
use super::session::{ActiveConnection, DapWireClient, SessionEntry};
use crate::types::{
    IdeDebugState, IdeInlineValue, IdeScope, IdeStackFrame, IdeThreadInfo, IdeVariable,
};

pub(super) async fn sync_all_breakpoints(
    entry: &SessionEntry,
    conn: &ActiveConnection,
) -> Result<()> {
    let bps = entry.read_snapshot().breakpoints;
    let temp_rtc = entry.temp_run_to_cursor.lock().ok().and_then(|g| g.clone());

    let mut by_file: BTreeMap<String, Vec<Value>> = BTreeMap::new();
    if let Ok(prev_files) = entry.configured_bp_files.lock() {
        for f in prev_files.iter() {
            by_file.insert(f.clone(), Vec::new());
        }
    }

    for bp in bps.iter().filter(|b| b.enabled) {
        let mut obj = json!({ "line": bp.line });
        if let Some(cond) = bp.condition.as_deref().filter(|s| !s.trim().is_empty()) {
            obj["condition"] = json!(cond);
        }
        if let Some(hit) = bp.hit_condition.as_deref().filter(|s| !s.trim().is_empty()) {
            obj["hitCondition"] = json!(hit);
        }
        if let Some(log) = bp.log_message.as_deref().filter(|s| !s.trim().is_empty()) {
            obj["logMessage"] = json!(log);
        }
        by_file.entry(bp.path.clone()).or_default().push(obj);
    }

    if let Some((rtc_path, rtc_line)) = temp_rtc {
        let list = by_file.entry(rtc_path).or_default();
        let already = list
            .iter()
            .any(|v| v.get("line").and_then(Value::as_u64) == Some(u64::from(rtc_line)));
        if !already {
            list.push(json!({ "line": rtc_line }));
        }
    }

    let mut active_files = BTreeSet::new();
    for (path, file_bps) in by_file {
        if !file_bps.is_empty() {
            active_files.insert(path.clone());
        }
        let _ = conn
            .wire
            .request(
                "setBreakpoints",
                Some(json!({
                    "source": { "path": path },
                    "breakpoints": file_bps,
                })),
            )
            .await;
    }
    if let Ok(mut prev_files) = entry.configured_bp_files.lock() {
        *prev_files = active_files;
    }

    let break_exc = entry
        .break_on_exceptions
        .lock()
        .map(|g| *g)
        .unwrap_or(false);
    let filters: Vec<String> = if break_exc {
        if conn.exception_filters.is_empty() {
            vec!["uncaught".into(), "all".into()]
        } else {
            conn.exception_filters.clone()
        }
    } else {
        Vec::new()
    };
    if break_exc || !conn.exception_filters.is_empty() {
        let _ = conn
            .wire
            .request(
                "setExceptionBreakpoints",
                Some(json!({ "filters": filters })),
            )
            .await;
    }
    Ok(())
}

pub(super) async fn handle_stopped_event(
    entry: &SessionEntry,
    conn: &ActiveConnection,
    reason: String,
    stopped_tid: Option<i64>,
) {
    let had_temp = entry
        .temp_run_to_cursor
        .lock()
        .ok()
        .and_then(|mut g| g.take())
        .is_some();
    if had_temp {
        let _ = sync_all_breakpoints(entry, conn).await;
    }

    let threads = fetch_threads(&conn.wire).await.unwrap_or_default();
    let active_tid = stopped_tid
        .or_else(|| threads.first().map(|t| t.id))
        .unwrap_or(1);

    let frames = fetch_stack_frames(&conn.wire, active_tid)
        .await
        .unwrap_or_default();
    let selected_frame = frames
        .iter()
        .find(|f| f.path.as_deref().is_some_and(|p| !p.is_empty()))
        .or_else(|| frames.first())
        .cloned();

    let (active_fid, paused_file, paused_line, scopes, inline_values) = if let Some(frame) =
        selected_frame
    {
        let scopes = fetch_scopes_with_variables(&conn.wire, frame.id)
            .await
            .unwrap_or_default();
        let inline =
            load_and_compute_inline_values(entry, frame.path.as_deref(), frame.line, &scopes).await;
        (
            Some(frame.id),
            frame.path.clone(),
            Some(frame.line),
            scopes,
            inline,
        )
    } else {
        (None, None, None, Vec::new(), Vec::new())
    };

    entry.mutate_and_broadcast(|snap| {
        snap.state = IdeDebugState::Paused;
        snap.stopped_reason = Some(reason);
        snap.active_thread_id = Some(active_tid);
        snap.active_frame_id = active_fid;
        snap.paused_file = paused_file;
        snap.paused_line = paused_line;
        snap.threads = threads;
        snap.stack_frames = frames;
        snap.scopes = scopes;
        snap.inline_values = inline_values;
    });
}

pub(super) async fn select_frame_and_refresh(
    entry: &SessionEntry,
    conn: &ActiveConnection,
    frame_id: i64,
) -> Result<()> {
    let snap = entry.read_snapshot();
    let frame = snap
        .stack_frames
        .iter()
        .find(|f| f.id == frame_id)
        .cloned()
        .ok_or_else(|| Error::NotFound(format!("stack frame '{frame_id}' not found")))?;
    let scopes = fetch_scopes_with_variables(&conn.wire, frame.id).await?;
    let inline_values =
        load_and_compute_inline_values(entry, frame.path.as_deref(), frame.line, &scopes).await;

    entry.mutate_and_broadcast(|s| {
        s.active_frame_id = Some(frame.id);
        s.paused_file = frame.path;
        s.paused_line = Some(frame.line);
        s.scopes = scopes;
        s.inline_values = inline_values;
    });
    Ok(())
}

async fn fetch_threads(wire: &DapWireClient) -> Result<Vec<IdeThreadInfo>> {
    let resp = wire.request("threads", Some(json!({}))).await?;
    let arr = resp
        .body
        .as_ref()
        .and_then(|b| b.get("threads"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(arr
        .into_iter()
        .filter_map(|t| {
            let id = t.get("id").and_then(Value::as_i64)?;
            let name = t
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("Thread")
                .to_string();
            Some(IdeThreadInfo { id, name })
        })
        .collect())
}

async fn fetch_stack_frames(wire: &DapWireClient, thread_id: i64) -> Result<Vec<IdeStackFrame>> {
    let resp = wire
        .request(
            "stackTrace",
            Some(json!({ "threadId": thread_id, "startFrame": 0, "levels": 32 })),
        )
        .await?;
    let arr = resp
        .body
        .as_ref()
        .and_then(|b| b.get("stackFrames"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(arr
        .into_iter()
        .filter_map(|f| {
            let id = f.get("id").and_then(Value::as_i64)?;
            let name = f
                .get("name")
                .and_then(Value::as_str)
                .unwrap_or("(frame)")
                .to_string();
            let path = f
                .get("source")
                .and_then(|s| s.get("path"))
                .and_then(Value::as_str)
                .filter(|p| !p.is_empty())
                .map(String::from);
            let line = f.get("line").and_then(Value::as_u64).unwrap_or(1) as u32;
            let column = f.get("column").and_then(Value::as_u64).unwrap_or(1) as u32;
            Some(IdeStackFrame {
                id,
                name,
                path,
                line,
                column,
            })
        })
        .collect())
}

async fn fetch_scopes_with_variables(wire: &DapWireClient, frame_id: i64) -> Result<Vec<IdeScope>> {
    let resp = wire
        .request("scopes", Some(json!({ "frameId": frame_id })))
        .await?;
    let arr = resp
        .body
        .as_ref()
        .and_then(|b| b.get("scopes"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    let mut out = Vec::new();
    for sc in arr.into_iter().take(4) {
        let name = sc
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("Locals")
            .to_string();
        let vref = sc
            .get("variablesReference")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        let expensive = sc
            .get("expensive")
            .and_then(Value::as_bool)
            .unwrap_or(false);
        let variables = if vref > 0 && !expensive {
            fetch_variables(wire, vref).await.unwrap_or_default()
        } else {
            Vec::new()
        };
        out.push(IdeScope {
            name,
            variables_reference: vref,
            variables,
        });
    }
    Ok(out)
}

pub(super) async fn fetch_variables(
    wire: &DapWireClient,
    variables_reference: i64,
) -> Result<Vec<IdeVariable>> {
    let resp = wire
        .request(
            "variables",
            Some(json!({ "variablesReference": variables_reference })),
        )
        .await?;
    let arr = resp
        .body
        .as_ref()
        .and_then(|b| b.get("variables"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    Ok(arr
        .into_iter()
        .take(200)
        .filter_map(|v| {
            let name = v.get("name").and_then(Value::as_str)?.to_string();
            let value = v
                .get("value")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let var_type = v.get("type").and_then(Value::as_str).map(String::from);
            let variables_reference = v
                .get("variablesReference")
                .and_then(Value::as_i64)
                .unwrap_or(0);
            let indexed_variables = v
                .get("indexedVariables")
                .and_then(Value::as_u64)
                .map(|n| n as usize);
            Some(IdeVariable {
                name,
                value,
                var_type,
                variables_reference,
                indexed_variables,
            })
        })
        .collect())
}

async fn load_and_compute_inline_values(
    entry: &SessionEntry,
    paused_path: Option<&str>,
    paused_line: u32,
    scopes: &[IdeScope],
) -> Vec<IdeInlineValue> {
    let Some(path) = paused_path.filter(|p| !p.is_empty()) else {
        return Vec::new();
    };
    let (target, ssh, caller_is_root) = {
        let req = entry.orig_req.read().unwrap_or_else(|e| e.into_inner());
        let ctx = entry.orig_ctx.read().unwrap_or_else(|e| e.into_inner());
        (req.target.clone(), ctx.ssh.clone(), ctx.caller_is_root)
    };
    let Some(source) =
        read_target_file_for_inline(&target, ssh.as_ref(), path, caller_is_root).await
    else {
        return Vec::new();
    };
    compute_inline_values(&source, paused_line, scopes)
}

/// Match scope variable identifiers against lines near `paused_line` to produce
/// IntelliJ/GoLand-style inline ghost variable values for the editor.
pub fn compute_inline_values(
    source_text: &str,
    paused_line: u32,
    scopes: &[IdeScope],
) -> Vec<IdeInlineValue> {
    let mut vars_by_name: HashMap<&str, String> = HashMap::new();
    for scope in scopes {
        for var in &scope.variables {
            let name = var.name.trim();
            if name.is_empty() || name.starts_with('_') {
                continue;
            }
            vars_by_name.entry(name).or_insert_with(|| {
                let single_line = var.value.lines().next().unwrap_or("").trim();
                if single_line.chars().count() > 64 {
                    let head: String = single_line.chars().take(61).collect();
                    format!("{head}…")
                } else {
                    single_line.to_string()
                }
            });
        }
    }
    if vars_by_name.is_empty() || paused_line == 0 {
        return Vec::new();
    }

    let start_line = paused_line.saturating_sub(25).max(1) as usize;
    let end_line = paused_line as usize;
    let mut out = Vec::new();

    for (idx, raw_line) in source_text.lines().enumerate() {
        let line_no = idx + 1;
        if line_no < start_line || line_no > end_line {
            continue;
        }
        let trimmed = raw_line.trim_start();
        if trimmed.starts_with("//") || trimmed.starts_with('#') || trimmed.starts_with("/*") {
            continue;
        }
        let code_part = raw_line.split("//").next().unwrap_or(raw_line);
        let mut seen_on_line = BTreeSet::new();
        for tok in code_part.split(|c: char| !(c.is_alphanumeric() || c == '_')) {
            if tok.is_empty() || seen_on_line.len() >= 4 || seen_on_line.contains(tok) {
                continue;
            }
            if let Some(val) = vars_by_name.get(tok) {
                seen_on_line.insert(tok.to_string());
                out.push(IdeInlineValue {
                    line: line_no as u32,
                    name: tok.to_string(),
                    value: val.clone(),
                });
            }
        }
    }
    out
}
