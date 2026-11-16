//! Variable expansion, scope loading, restart/teardown helpers, and IntelliJ-style
//! inline ghost variable value computation for [`super::session::DapManager`].

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use otto_core::{Error, Result};
use serde_json::{json, Value};

use super::adapters::{display_target_path, resolve_dap_adapter_with_manager};
use super::read_target_file_for_inline;
use super::session::{DapManager, DapWireClient, SessionEntry};
use super::session_transport::boot_session_connection;
use crate::types::{
    IdeDebugEvaluateReq, IdeDebugEvaluateResp, IdeDebugSessionSnapshot, IdeDebugState,
    IdeInlineValue, IdeScope, IdeStartDebugReq, IdeVariable,
};

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

pub(super) async fn load_and_compute_inline_values(
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

pub(super) async fn fetch_scopes_with_variables(
    wire: &DapWireClient,
    frame_id: i64,
) -> Result<Vec<IdeScope>> {
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

impl DapManager {
    pub(super) async fn restart_entry(
        &self,
        entry: &Arc<SessionEntry>,
    ) -> Result<IdeDebugSessionSnapshot> {
        use futures_util::FutureExt;

        let my_gen = entry.boot_generation.fetch_add(1, Ordering::SeqCst) + 1;
        entry.restarting.store(true, Ordering::SeqCst);
        teardown_connection(entry).await;
        let lifecycle_guard = entry.lifecycle_lock.lock().await;
        if entry.boot_generation.load(Ordering::SeqCst) != my_gen {
            entry.restarting.store(false, Ordering::SeqCst);
            return Ok(entry.read_snapshot());
        }
        tokio::task::yield_now().await;

        if let Ok(mut files) = entry.configured_bp_files.lock() {
            files.clear();
        }
        if let Ok(mut rtc) = entry.temp_run_to_cursor.lock() {
            *rtc = None;
        }
        entry
            .suppress_transient_pause
            .store(false, Ordering::SeqCst);
        let _ = entry.initialized_notify.notified().now_or_never();
        let _ = entry.halted_notify.notified().now_or_never();

        let (mut orig_req, mut orig_ctx) = {
            let mut r = entry
                .orig_req
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            r.breakpoints = entry.read_snapshot().breakpoints;
            let c = entry
                .orig_ctx
                .read()
                .unwrap_or_else(|e| e.into_inner())
                .clone();
            (r, c)
        };
        if orig_ctx.netbridge.is_none() {
            if let Some(ref dormant) = orig_ctx.dormant_netbridge {
                if let Some(reacquired) = dormant.reacquire().await {
                    orig_ctx.netbridge = Some(Arc::clone(&reacquired));
                    entry
                        .orig_ctx
                        .write()
                        .unwrap_or_else(|e| e.into_inner())
                        .netbridge = Some(reacquired);
                }
            }
        }
        for bp in &mut orig_req.breakpoints {
            bp.path = display_target_path(&orig_req.target, &bp.path);
        }
        entry.mutate_and_broadcast(|snap| {
            snap.config_id = orig_req.config_id.clone();
            snap.launch = Some(orig_req.launch.clone());
            snap.state = IdeDebugState::Starting;
            snap.stopped_reason = None;
            snap.paused_file = None;
            snap.paused_line = None;
            snap.threads.clear();
            snap.stack_frames.clear();
            snap.scopes.clear();
            snap.inline_values.clear();
            snap.error = None;
        });
        entry.restarting.store(false, Ordering::SeqCst);
        drop(lifecycle_guard);

        let resolved = match resolve_dap_adapter_with_manager(
            &self.data_dir,
            &orig_req,
            &orig_ctx,
            Some(&self.environments),
        )
        .await
        {
            Ok(res) => res,
            Err(err) => {
                let _guard = entry.lifecycle_lock.lock().await;
                if entry.boot_generation.load(Ordering::SeqCst) == my_gen && !entry.is_terminal() {
                    entry.fail_and_evict(&self.sessions, err.to_string());
                }
                return Err(err);
            }
        };
        let _guard = entry.lifecycle_lock.lock().await;
        if entry.boot_generation.load(Ordering::SeqCst) != my_gen || entry.is_terminal() {
            return Ok(entry.read_snapshot());
        }
        for item in &resolved.initial_output {
            entry.append_output(&item.category, item.text.clone());
        }
        entry.mutate_and_broadcast(|snap| {
            snap.language = resolved.language.clone();
            snap.adapter = resolved.adapter_name.clone();
            snap.config_id = orig_req.config_id.clone();
            snap.launch = Some(orig_req.launch.clone());
        });
        if let Err(err) =
            boot_session_connection(&self.sessions, entry, resolved, &orig_req, &orig_ctx).await
        {
            if entry.boot_generation.load(Ordering::SeqCst) == my_gen && !entry.is_terminal() {
                entry.fail_and_evict(&self.sessions, err.to_string());
            }
            return Err(err);
        }
        Ok(entry.read_snapshot())
    }

    /// Expand child properties/elements of a compound variable (`variablesReference > 0`).
    pub async fn expand_variables(
        &self,
        id: &str,
        variables_reference: i64,
    ) -> Result<Vec<IdeVariable>> {
        if variables_reference <= 0 {
            return Ok(Vec::new());
        }
        let conn = self.get_entry(id)?.active_conn()?;
        fetch_variables(&conn.wire, variables_reference).await
    }

    /// Evaluate an expression in the active (or specified) stack frame.
    pub async fn evaluate(
        &self,
        id: &str,
        req: IdeDebugEvaluateReq,
    ) -> Result<IdeDebugEvaluateResp> {
        if req.expression.trim().is_empty() {
            return Err(Error::Invalid("expression must not be empty".into()));
        }
        let entry = self.get_entry(id)?;
        let conn = entry.active_conn()?;
        let frame_id = req
            .frame_id
            .or_else(|| entry.read_snapshot().active_frame_id);
        let context = req.context.unwrap_or_else(|| "repl".to_string());
        let mut args = json!({ "expression": req.expression, "context": context });
        if let Some(fid) = frame_id {
            args["frameId"] = json!(fid);
        }
        let resp = conn.wire.request("evaluate", Some(args)).await?;
        let body = resp.body.unwrap_or_else(|| json!({}));
        Ok(IdeDebugEvaluateResp {
            result: body
                .get("result")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
            var_type: body.get("type").and_then(Value::as_str).map(String::from),
            variables_reference: body
                .get("variablesReference")
                .and_then(Value::as_i64)
                .unwrap_or(0),
        })
    }
}

pub(super) fn build_session_title(req: &IdeStartDebugReq, adapter: &str) -> String {
    if let Some(tname) = req.launch.test_name.as_deref().filter(|s| !s.is_empty()) {
        return format!("Debug {tname} ({adapter})");
    }
    let prog_label = std::path::Path::new(&req.launch.program)
        .file_name()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(&req.language);
    format!("Debug {prog_label} ({adapter})")
}

pub(super) fn mark_resumed(entry: &SessionEntry) {
    if entry.is_terminal() {
        return;
    }
    entry.mutate_and_broadcast(|snap| {
        if matches!(snap.state, IdeDebugState::Terminated | IdeDebugState::Error) {
            return;
        }
        snap.state = IdeDebugState::Running;
        snap.stopped_reason = None;
        snap.paused_file = None;
        snap.paused_line = None;
        snap.inline_values.clear();
    });
}

async fn teardown_connection(entry: &SessionEntry) {
    let old = entry.conn.write().unwrap_or_else(|e| e.into_inner()).take();
    if let Some(conn) = old {
        conn.wire.retire();
        if conn.supports_terminate {
            let _ = tokio::time::timeout(
                Duration::from_millis(800),
                conn.wire.request("terminate", Some(json!({}))),
            )
            .await;
        }
        let _ = tokio::time::timeout(
            Duration::from_millis(900),
            conn.wire
                .request("disconnect", Some(json!({ "terminateDebuggee": true }))),
        )
        .await;
        conn.wire.abort_pending();
        conn.guard.kill_all();
    }
}

pub(super) async fn stop_entry(entry: &SessionEntry) {
    entry.boot_generation.fetch_add(1, Ordering::SeqCst);
    teardown_connection(entry).await;
    let _guard = entry.lifecycle_lock.lock().await;
    teardown_connection(entry).await;
    entry.mark_terminated();
}
