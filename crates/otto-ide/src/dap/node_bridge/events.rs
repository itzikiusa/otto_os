//! V8 Inspector CDP event and response translation into DAP messages.

use std::io;

use serde_json::{json, Value};
use tokio::io::AsyncWrite;

use super::cdp::InspectorWs;
use super::{file_url_to_path_in_cwd, NodeBridgeState};
use crate::dap::wire::DapWriter;

pub(super) fn format_remote_object(
    st: &mut NodeBridgeState,
    obj: &Value,
) -> (String, Option<String>, i64) {
    let vtype = obj.get("type").and_then(Value::as_str).map(String::from);
    if obj.get("subtype").and_then(Value::as_str) == Some("null") {
        return ("null".into(), Some("null".into()), 0);
    }
    let display = if let Some(val) = obj.get("value") {
        match val {
            Value::String(s) => format!("{s:?}"),
            other => other.to_string(),
        }
    } else if let Some(desc) = obj.get("description").and_then(Value::as_str) {
        desc.to_string()
    } else {
        "undefined".into()
    };
    let child_ref = if vtype.as_deref() == Some("object") {
        obj.get("objectId")
            .and_then(Value::as_str)
            .map(|oid| st.alloc_obj_ref(oid.to_string()))
            .unwrap_or(0)
    } else {
        0
    };
    (display, vtype, child_ref)
}

pub(super) async fn handle_node_cdp_msg<W: AsyncWrite + Unpin>(
    msg: Value,
    st: &mut NodeBridgeState,
    ws: &mut InspectorWs,
    dap_w: &mut DapWriter<W>,
) -> io::Result<bool> {
    if let Some(id) = msg.get("id").and_then(Value::as_i64) {
        if let Some((req_seq, kind, meta)) = st.pending_cdp.remove(&id) {
            handle_node_cdp_reply(req_seq, &kind, meta, msg.get("result"), st, dap_w).await?;
        }
        return Ok(false);
    }

    let method = msg.get("method").and_then(Value::as_str).unwrap_or("");
    let params = msg.get("params");
    match method {
        "Debugger.scriptParsed" => {
            if let (Some(sid), Some(url)) = (
                params
                    .and_then(|p| p.get("scriptId"))
                    .and_then(Value::as_str),
                params
                    .and_then(|p| p.get("url"))
                    .and_then(Value::as_str)
                    .filter(|u| !u.is_empty()),
            ) {
                st.script_urls.insert(sid.to_string(), url.to_string());
            }
        }
        "Debugger.breakpointResolved" => {
            if let Some(bid) = params
                .and_then(|p| p.get("breakpointId"))
                .and_then(Value::as_str)
            {
                if let Some((path, orig_line)) = st.bp_meta_by_id.get(bid).cloned() {
                    let bound_line = params
                        .and_then(|p| p.get("location"))
                        .and_then(|l| l.get("lineNumber"))
                        .and_then(Value::as_u64)
                        .map(|n| n as u32 + 1)
                        .unwrap_or(orig_line);
                    dap_w
                        .send_event(
                            "breakpoint",
                            Some(json!({
                                "reason": "changed",
                                "breakpoint": {
                                    "verified": true,
                                    "line": bound_line,
                                    "source": { "path": path }
                                }
                            })),
                        )
                        .await?;
                }
            }
        }
        "Debugger.paused" => {
            let reason_raw = params
                .and_then(|p| p.get("reason"))
                .and_then(Value::as_str)
                .unwrap_or("other");
            let hit_bps = params
                .and_then(|p| p.get("hitBreakpoints"))
                .and_then(Value::as_array)
                .map(Vec::len)
                .unwrap_or(0);
            let call_frames = params
                .and_then(|p| p.get("callFrames"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();

            if !st.initial_brk_skipped && hit_bps == 0 {
                st.initial_brk_skipped = true;
                let top_cf = call_frames.first();
                let resolved_url = top_cf
                    .and_then(|cf| cf.get("url"))
                    .and_then(Value::as_str)
                    .filter(|u| !u.is_empty())
                    .map(String::from)
                    .or_else(|| {
                        let sid = top_cf
                            .and_then(|cf| cf.get("location"))
                            .and_then(|l| l.get("scriptId"))
                            .and_then(Value::as_str)?;
                        st.script_urls.get(sid).cloned()
                    })
                    .unwrap_or_default();
                let top_line = top_cf
                    .and_then(|cf| cf.get("location"))
                    .and_then(|l| l.get("lineNumber"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0) as u32
                    + 1;
                let has_bp_at_entry = file_url_to_path_in_cwd(&resolved_url, &st.workspace_cwd)
                    .is_some_and(|top_path| {
                        let top_canon = std::fs::canonicalize(&top_path).ok();
                        st.user_bp_lines.iter().any(|(bp_file, lines)| {
                            let same_file = bp_file == &top_path
                                || top_canon.as_ref().is_some_and(|tc| {
                                    std::fs::canonicalize(bp_file).ok().as_ref() == Some(tc)
                                });
                            same_file && lines.contains(&top_line)
                        })
                    });
                if !has_bp_at_entry {
                    let cid = st.alloc_cdp();
                    ws.send_json(&json!({ "id": cid, "method": "Debugger.resume" }))
                        .await?;
                    return Ok(false);
                }
            }
            st.initial_brk_skipped = true;
            st.frames = call_frames;
            let dap_reason = if hit_bps > 0 {
                "breakpoint"
            } else if reason_raw.contains("exception") || reason_raw == "promiseRejection" {
                "exception"
            } else {
                "step"
            };
            dap_w
                .send_event(
                    "stopped",
                    Some(json!({
                        "reason": dap_reason,
                        "threadId": 1,
                        "allThreadsStopped": true
                    })),
                )
                .await?;
        }
        // Node drops a `runIfWaitingForDebugger` dispatched before its main thread
        // enters `waitForFrontend` (the connect races Node's startup; under load the
        // window is wide), then waits forever. This event marks that wait: release it.
        "NodeRuntime.waitingForDebugger" if st.config_done => {
            let cid = st.alloc_cdp();
            ws.send_json(&json!({ "id": cid, "method": "Runtime.runIfWaitingForDebugger" }))
                .await?;
        }
        "Inspector.detached"
        | "Runtime.executionContextDestroyed"
        | "Runtime.executionContextsCleared" => {
            let ctx_id = params
                .and_then(|p| p.get("executionContextId"))
                .and_then(Value::as_i64);
            if matches!(ctx_id, None | Some(1)) {
                return Ok(true);
            }
        }
        _ => {}
    }
    Ok(false)
}

async fn handle_node_cdp_reply<W: AsyncWrite + Unpin>(
    req_seq: i64,
    kind: &str,
    meta: String,
    result: Option<&Value>,
    st: &mut NodeBridgeState,
    dap_w: &mut DapWriter<W>,
) -> io::Result<()> {
    match kind {
        "setBp" => {
            if let Some(bid) = result
                .and_then(|r| r.get("breakpointId"))
                .and_then(Value::as_str)
            {
                st.bp_ids_by_file
                    .entry(meta)
                    .or_default()
                    .push(bid.to_string());
            }
        }
        "setBpBrowser" => {
            let (idx, path) = meta
                .split_once(':')
                .and_then(|(i, p)| i.parse::<usize>().ok().map(|idx| (idx, p.to_string())))
                .unwrap_or((0, meta));
            let bid = result
                .and_then(|r| r.get("breakpointId"))
                .and_then(Value::as_str)
                .map(String::from);
            let locations = result
                .and_then(|r| r.get("locations"))
                .and_then(Value::as_array);
            let first_loc = locations.and_then(|arr| arr.first());
            let is_bound = first_loc.is_some();
            if let Some(ref b) = bid {
                st.bp_ids_by_file
                    .entry(path.clone())
                    .or_default()
                    .push(b.clone());
            }
            let finished = if let Some(batch) = st.pending_browser_bp_batches.get_mut(&req_seq) {
                let req_line = batch.requested_lines.get(idx).copied().unwrap_or(1);
                if let Some(ref b) = bid {
                    st.bp_meta_by_id.insert(b.clone(), (path.clone(), req_line));
                }
                let entry = if is_bound {
                    let bound_line = first_loc
                        .and_then(|l| l.get("lineNumber"))
                        .and_then(Value::as_u64)
                        .map(|n| n as u32 + 1)
                        .unwrap_or(req_line);
                    json!({
                        "verified": true,
                        "line": bound_line,
                        "source": { "path": path }
                    })
                } else {
                    json!({
                        "verified": false,
                        "line": req_line,
                        "source": { "path": path },
                        "message": "Unbound: browser has not bound a script location for this source path (source maps are not resolved in the built-in browser CDP bridge)"
                    })
                };
                if let Some(slot) = batch.results.get_mut(idx) {
                    *slot = Some(entry);
                }
                batch.remaining = batch.remaining.saturating_sub(1);
                batch.remaining == 0
            } else {
                false
            };
            if finished {
                if let Some(batch) = st.pending_browser_bp_batches.remove(&req_seq) {
                    let out: Vec<Value> = batch
                        .results
                        .into_iter()
                        .enumerate()
                        .map(|(i, v)| {
                            v.unwrap_or_else(|| {
                                let line = batch.requested_lines.get(i).copied().unwrap_or(1);
                                json!({
                                    "verified": false,
                                    "line": line,
                                    "source": { "path": batch.path }
                                })
                            })
                        })
                        .collect();
                    dap_w
                        .send_response(
                            batch.req_seq,
                            "setBreakpoints",
                            Ok(Some(json!({ "breakpoints": out }))),
                        )
                        .await?;
                }
            }
        }
        "variables" => {
            let mut vars = Vec::new();
            if let Some(props) = result
                .and_then(|r| r.get("result"))
                .and_then(Value::as_array)
            {
                for p in props {
                    let Some(name) = p.get("name").and_then(Value::as_str) else {
                        continue;
                    };
                    if name == "__proto__" {
                        continue;
                    }
                    if let Some(vobj) = p.get("value") {
                        let (val, vt, cref) = format_remote_object(st, vobj);
                        vars.push(json!({
                            "name": name,
                            "value": val,
                            "type": vt,
                            "variablesReference": cref
                        }));
                    }
                }
            }
            dap_w
                .send_response(req_seq, "variables", Ok(Some(json!({ "variables": vars }))))
                .await?;
        }
        "evaluate" => {
            if let Some(r_obj) = result.and_then(|r| r.get("result")) {
                let (val, vt, cref) = format_remote_object(st, r_obj);
                dap_w
                    .send_response(
                        req_seq,
                        "evaluate",
                        Ok(Some(json!({
                            "result": val,
                            "type": vt,
                            "variablesReference": cref
                        }))),
                    )
                    .await?;
            } else {
                dap_w
                    .send_response(req_seq, "evaluate", Err("evaluation failed".into()))
                    .await?;
            }
        }
        _ => {}
    }
    Ok(())
}
