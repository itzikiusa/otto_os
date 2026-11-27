//! Breakpoint synchronization and `stopped` event stack/scope inspection for
//! [`super::session::DapManager`].

use std::collections::{BTreeMap, BTreeSet};

use futures_util::future::join_all;
use otto_core::{Error, Result};
use serde_json::{json, Value};

use super::adapters::{display_target_path, normalize_target_path};
use super::session::{ActiveConnection, DapWireClient, SessionEntry};
use super::session_inline::{fetch_scopes_with_variables, load_and_compute_inline_values};
use crate::types::{IdeBreakpoint, IdeDebugState, IdeStackFrame, IdeTargetSpec, IdeThreadInfo};

pub(super) async fn sync_all_breakpoints(
    entry: &SessionEntry,
    conn: &ActiveConnection,
) -> Result<()> {
    let mut bps = entry.read_snapshot().breakpoints;
    let target = entry
        .orig_req
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .target
        .clone();
    let temp_rtc = entry.temp_run_to_cursor.lock().ok().and_then(|g| g.clone());

    let mut by_file: BTreeMap<String, Vec<(Option<usize>, Value)>> = BTreeMap::new();
    if let Ok(prev_files) = entry.configured_bp_files.lock() {
        for f in prev_files.iter() {
            by_file.insert(f.clone(), Vec::new());
        }
    }

    for (idx, bp) in bps.iter_mut().enumerate() {
        bp.path = display_target_path(&target, &bp.path);
        if !bp.enabled {
            bp.verified = None;
            bp.message = None;
            continue;
        }
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
        let wire_path = normalize_target_path(&target, &bp.path);
        by_file.entry(wire_path).or_default().push((Some(idx), obj));
    }

    if let Some((rtc_path, rtc_line)) = temp_rtc {
        let list = by_file.entry(rtc_path).or_default();
        let already = list
            .iter()
            .any(|(_, v)| v.get("line").and_then(Value::as_u64) == Some(u64::from(rtc_line)));
        if !already {
            list.push((None, json!({ "line": rtc_line })));
        }
    }

    let snap_before = entry.read_snapshot();
    let need_halt_resume =
        snap_before.state == IdeDebugState::Running && snap_before.adapter == "dlv";
    if need_halt_resume {
        entry
            .suppress_transient_pause
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let halted_fut = entry.halted_notify.notified();
        if conn
            .wire
            .request("pause", Some(json!({ "threadId": 1 })))
            .await
            .is_ok()
        {
            let _ = tokio::time::timeout(std::time::Duration::from_millis(750), halted_fut).await;
        }
    }

    let mut adapter_bp_locs: std::collections::HashMap<i64, (String, u32)> =
        std::collections::HashMap::new();
    // js-debug: every child target binds the breakpoint on its own, so one
    // connection's "not bound (yet)" never unverifies another's binding.
    let keep_verified = entry.targets.enabled();
    let mut active_files = BTreeSet::new();
    let can_parallel =
        !need_halt_resume && snap_before.state == IdeDebugState::Starting && by_file.len() > 1;

    if can_parallel {
        let mut tasks = Vec::with_capacity(by_file.len());
        for (path, indexed_bps) in by_file {
            if !indexed_bps.is_empty() {
                active_files.insert(path.clone());
            }
            let payload_bps: Vec<Value> = indexed_bps.iter().map(|(_, v)| v.clone()).collect();
            let req_payload = json!({
                "source": { "path": path },
                "breakpoints": payload_bps,
            });
            let wire = conn.wire.clone();
            tasks.push(async move {
                let res = wire.request("setBreakpoints", Some(req_payload)).await;
                (indexed_bps, res)
            });
        }
        for (indexed_bps, resp_res) in join_all(tasks).await {
            apply_set_breakpoints_result(
                &mut bps,
                &mut adapter_bp_locs,
                &indexed_bps,
                resp_res,
                keep_verified,
            );
        }
    } else {
        for (path, indexed_bps) in by_file {
            if !indexed_bps.is_empty() {
                active_files.insert(path.clone());
            }
            let payload_bps: Vec<Value> = indexed_bps.iter().map(|(_, v)| v.clone()).collect();
            let req_payload = json!({
                "source": { "path": path },
                "breakpoints": payload_bps,
            });
            let mut resp_res = conn
                .wire
                .request("setBreakpoints", Some(req_payload.clone()))
                .await;
            if let Err(ref e) = resp_res {
                if !need_halt_resume
                    && snap_before.state == IdeDebugState::Running
                    && e.to_string().to_ascii_lowercase().contains("running")
                {
                    entry
                        .suppress_transient_pause
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                    let halted_fut = entry.halted_notify.notified();
                    let _ = conn
                        .wire
                        .request("pause", Some(json!({ "threadId": 1 })))
                        .await;
                    let _ = tokio::time::timeout(std::time::Duration::from_millis(750), halted_fut)
                        .await;
                    resp_res = conn.wire.request("setBreakpoints", Some(req_payload)).await;
                    entry
                        .suppress_transient_pause
                        .store(false, std::sync::atomic::Ordering::SeqCst);
                    let _ = conn
                        .wire
                        .request("continue", Some(json!({ "threadId": 1 })))
                        .await;
                }
            }
            apply_set_breakpoints_result(
                &mut bps,
                &mut adapter_bp_locs,
                &indexed_bps,
                resp_res,
                keep_verified,
            );
        }
    }

    if need_halt_resume {
        entry
            .suppress_transient_pause
            .store(false, std::sync::atomic::Ordering::SeqCst);
        let _ = conn
            .wire
            .request("continue", Some(json!({ "threadId": 1 })))
            .await;
    }
    if let Ok(mut prev_files) = entry.configured_bp_files.lock() {
        *prev_files = active_files;
    }
    if let Ok(mut locs) = conn.wire.bp_ids.lock() {
        *locs = adapter_bp_locs;
    }
    entry.mutate_and_broadcast(|snap| {
        snap.breakpoints = bps;
    });

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
    if snap_before.language == "zig" {
        let fn_bps = if break_exc {
            json!([
                { "name": "debug.defaultPanic" },
                { "name": "builtin.default_panic" },
                { "name": "debug.panicImpl" }
            ])
        } else {
            json!([])
        };
        let _ = conn
            .wire
            .request(
                "setFunctionBreakpoints",
                Some(json!({ "breakpoints": fn_bps })),
            )
            .await;
    } else if matches!(snap_before.language.as_str(), "c" | "cpp")
        && !conn.exception_filters.iter().any(|f| f == "cpp_throw")
    {
        let fn_bps = if break_exc {
            json!([{ "name": "__cxa_throw" }])
        } else {
            json!([])
        };
        let _ = conn
            .wire
            .request(
                "setFunctionBreakpoints",
                Some(json!({ "breakpoints": fn_bps })),
            )
            .await;
    }
    Ok(())
}

/// Send the breakpoints to the session's connection and to each js-debug child
/// target. A child's failure (its process just exited) leaves the others alone.
pub(super) async fn sync_breakpoints_everywhere(entry: &SessionEntry) -> Result<()> {
    let root = entry.active_root();
    for child in entry.targets.connections() {
        let _ = sync_all_breakpoints(entry, &child).await;
    }
    match root {
        Ok(conn) => sync_all_breakpoints(entry, &conn).await,
        Err(_) => Ok(()),
    }
}

fn apply_set_breakpoints_result(
    bps: &mut [IdeBreakpoint],
    adapter_bp_locs: &mut std::collections::HashMap<i64, (String, u32)>,
    indexed_bps: &[(Option<usize>, Value)],
    resp_res: Result<super::wire::DapResponse>,
    keep_verified: bool,
) {
    match resp_res {
        Ok(resp) => {
            let ack_list = resp
                .body
                .as_ref()
                .and_then(|b| b.get("breakpoints"))
                .and_then(Value::as_array)
                .cloned()
                .unwrap_or_default();
            for (pos, (maybe_idx, _)) in indexed_bps.iter().enumerate() {
                let Some(bp_idx) = *maybe_idx else {
                    continue;
                };
                if let Some(ack) = ack_list.get(pos) {
                    let verified = ack.get("verified").and_then(Value::as_bool).unwrap_or(true);
                    if keep_verified && !verified && bps[bp_idx].verified == Some(true) {
                        if let Some(bp_id) = ack.get("id").and_then(Value::as_i64) {
                            adapter_bp_locs
                                .insert(bp_id, (bps[bp_idx].path.clone(), bps[bp_idx].line));
                        }
                        continue;
                    }
                    bps[bp_idx].verified = Some(verified);
                    bps[bp_idx].message = ack
                        .get("message")
                        .and_then(Value::as_str)
                        .filter(|s| !s.is_empty())
                        // js-debug's standalone server sends its untranslated key.
                        .map(|s| match s {
                            "breakpoint.provisionalBreakpoint" => "Unbound breakpoint".into(),
                            other => other.to_string(),
                        });
                    if verified {
                        if let Some(adj_ln) =
                            ack.get("line").and_then(Value::as_u64).filter(|&l| l > 0)
                        {
                            bps[bp_idx].line = adj_ln as u32;
                        }
                    }
                    if let Some(bp_id) = ack.get("id").and_then(Value::as_i64) {
                        adapter_bp_locs.insert(bp_id, (bps[bp_idx].path.clone(), bps[bp_idx].line));
                    }
                } else {
                    bps[bp_idx].verified = Some(true);
                }
            }
        }
        Err(err) => {
            let msg = err.to_string();
            for (maybe_idx, _) in indexed_bps {
                if let Some(bp_idx) = *maybe_idx {
                    if keep_verified && bps[bp_idx].verified == Some(true) {
                        continue;
                    }
                    bps[bp_idx].verified = Some(false);
                    bps[bp_idx].message = Some(msg.clone());
                }
            }
        }
    }
}

/// Apply asynchronous `"breakpoint"` DAP events (e.g. deferred verification in Go/JVM/JS).
pub(super) fn apply_breakpoint_event(
    entry: &SessionEntry,
    bp_ids: &std::sync::Mutex<std::collections::HashMap<i64, (String, u32)>>,
    body: &Value,
) {
    let Some(bp_obj) = body.get("breakpoint") else {
        return;
    };
    let Some(verified) = bp_obj.get("verified").and_then(Value::as_bool) else {
        return;
    };
    let bp_id = bp_obj.get("id").and_then(Value::as_i64);
    let line = bp_obj.get("line").and_then(Value::as_u64).map(|l| l as u32);
    let orig_line = bp_obj
        .get("originalLine")
        .and_then(Value::as_u64)
        .map(|l| l as u32);
    let message = bp_obj
        .get("message")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .map(String::from);
    let raw_path = bp_obj
        .get("source")
        .and_then(|s| s.get("path"))
        .and_then(Value::as_str);
    let target = entry
        .orig_req
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .target
        .clone();
    let norm_path = raw_path.map(|p| normalize_target_path(&target, p));
    let disp_path = raw_path.map(|p| display_target_path(&target, p));
    let tracked_loc = bp_id.and_then(|id| bp_ids.lock().ok().and_then(|m| m.get(&id).cloned()));
    let keep_verified = entry.targets.enabled();

    let mut updated_loc: Option<(String, u32)> = None;
    entry.mutate_and_broadcast(|snap| {
        for bp in &mut snap.breakpoints {
            let matched_by_id = tracked_loc.as_ref().is_some_and(|(tp, tl)| {
                let bp_disp = display_target_path(&target, &bp.path);
                let tp_disp = display_target_path(&target, tp);
                (bp.path == *tp || bp_disp == tp_disp) && bp.line == *tl
            });
            let path_matches = match (disp_path.as_deref(), norm_path.as_deref()) {
                (Some(dp), Some(np)) => bp.path == dp || bp.path == np,
                (Some(dp), None) => bp.path == dp,
                (None, Some(np)) => bp.path == np,
                (None, None) => true,
            };
            let line_matches =
                line.is_some_and(|ln| bp.line == ln) || orig_line.is_some_and(|ol| bp.line == ol);
            if matched_by_id || (path_matches && line_matches) {
                if keep_verified && !verified && bp.verified == Some(true) {
                    continue;
                }
                bp.path = display_target_path(&target, &bp.path);
                bp.verified = Some(verified);
                if verified {
                    if let Some(ln) = line.filter(|&l| l > 0) {
                        bp.line = ln;
                    }
                    bp.message = message.clone();
                } else if message.is_some() {
                    bp.message = message.clone();
                }
                updated_loc = Some((bp.path.clone(), bp.line));
            }
        }
    });
    if let (Some(id), Some(loc)) = (bp_id, updated_loc) {
        if let Ok(mut locs) = bp_ids.lock() {
            locs.insert(id, loc);
        }
    }
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

    let target = entry
        .orig_req
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .target
        .clone();
    let mut threads = fetch_threads(&conn.wire).await.unwrap_or_default();
    let active_tid = stopped_tid
        .or_else(|| threads.first().map(|t| t.id))
        .unwrap_or(1);
    // The stopped thread leads the list: Delve lists parked runtime goroutines first.
    if let Some(i) = threads.iter().position(|t| t.id == active_tid) {
        threads[..=i].rotate_right(1);
    }

    let (frames, top_ip) = fetch_stack_frames(&conn.wire, &target, active_tid)
        .await
        .unwrap_or_default();
    let key = frames
        .first()
        .map(|f| (active_tid, frames.len(), f.path.clone(), f.line));
    let skip = match entry.last_bp_stop.lock() {
        Ok(mut last) => {
            let prev = last.take();
            match (key, top_ip) {
                (Some(key), Some(ip)) if reason == "breakpoint" => {
                    let skip = is_same_line_relocation(prev.as_ref(), &key, ip);
                    *last = Some((key, ip));
                    skip
                }
                _ => false,
            }
        }
        Err(_) => false,
    };
    if skip {
        let _ = conn
            .wire
            .request("continue", Some(json!({ "threadId": active_tid })))
            .await;
        return;
    }
    let selected_frame = top_source_frame(&frames);

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

    if entry.is_terminal() || conn.wire.is_retired() {
        return;
    }
    entry.mutate_and_broadcast(|snap| {
        if matches!(snap.state, IdeDebugState::Terminated | IdeDebugState::Error) {
            return;
        }
        snap.state = IdeDebugState::Paused;
        snap.stop_seq += 1;
        snap.stopped_reason = Some(reason);
        snap.active_thread_id = Some(active_tid);
        snap.active_frame_id = active_fid;
        snap.selected_frame_file = paused_file.clone();
        snap.selected_frame_line = paused_line;
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
    thread_id: Option<i64>,
    frame_id: Option<i64>,
) -> Result<()> {
    if thread_id.is_none() && frame_id.is_none() {
        return Err(Error::Invalid(
            "either 'frame_id' or 'thread_id' is required for 'select_frame'".into(),
        ));
    }
    let target = entry
        .orig_req
        .read()
        .unwrap_or_else(|e| e.into_inner())
        .target
        .clone();
    let snap = entry.read_snapshot();
    let effective_tid = thread_id.or(snap.active_thread_id).unwrap_or(1);
    let thread_changed = snap.active_thread_id != Some(effective_tid);

    let need_refetch_frames = (thread_id.is_some() && thread_id != snap.active_thread_id)
        || snap.stack_frames.is_empty()
        || frame_id.is_some_and(|fid| !snap.stack_frames.iter().any(|f| f.id == fid));

    let frames = if need_refetch_frames {
        fetch_stack_frames(&conn.wire, &target, effective_tid)
            .await?
            .0
    } else {
        snap.stack_frames
    };

    let frame = match frame_id {
        Some(fid) => frames
            .iter()
            .find(|f| f.id == fid)
            .cloned()
            .ok_or_else(|| Error::NotFound(format!("stack frame '{fid}' not found")))?,
        None => top_source_frame(&frames).ok_or_else(|| {
            Error::NotFound(format!(
                "no stack frames available on thread '{effective_tid}'"
            ))
        })?,
    };

    let scopes = fetch_scopes_with_variables(&conn.wire, frame.id).await?;
    let inline_values =
        load_and_compute_inline_values(entry, frame.path.as_deref(), frame.line, &scopes).await;
    // Execution stays where it stopped: only another thread has another stop
    // location (its own top frame). Selecting a caller frame moves the
    // selection, never `paused_file`/`paused_line`.
    let stop_frame = if thread_changed {
        top_source_frame(&frames)
    } else {
        None
    };

    entry.mutate_and_broadcast(|s| {
        s.active_thread_id = Some(effective_tid);
        s.active_frame_id = Some(frame.id);
        s.stack_frames = frames;
        if let Some(top) = stop_frame {
            s.paused_file = top.path;
            s.paused_line = Some(top.line);
        }
        s.selected_frame_file = frame.path;
        s.selected_frame_line = Some(frame.line);
        s.scopes = scopes;
        s.inline_values = inline_values;
    });
    Ok(())
}

/// The first frame that has a source file (else the first frame): where a
/// thread is stopped as far as the editor can show it.
fn top_source_frame(frames: &[IdeStackFrame]) -> Option<IdeStackFrame> {
    frames
        .iter()
        .find(|f| f.path.as_deref().is_some_and(|p| !p.is_empty()))
        .or_else(|| frames.first())
        .cloned()
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

/// Where a breakpoint stop happened: thread, stack depth, top frame file and line.
pub(super) type BpStopKey = (i64, usize, Option<String>, u32);

/// A line holding a macro (`println!`, `assert_eq!`) compiles to several breakpoint
/// locations in one function, and lldb stops at each: Continue looked stuck on that
/// line. A stop is such a relocation when it follows a breakpoint stop at the same
/// place and depth at a HIGHER address; a loop coming back to the line restarts at
/// its first (lowest) address, so it still stops.
pub(super) fn is_same_line_relocation(
    prev: Option<&(BpStopKey, u64)>,
    key: &BpStopKey,
    ip: u64,
) -> bool {
    prev.is_some_and(|(k, prev_ip)| k == key && ip > *prev_ip)
}

/// The thread's frames and the top frame's instruction pointer, when the adapter
/// reports one.
async fn fetch_stack_frames(
    wire: &DapWireClient,
    target: &IdeTargetSpec,
    thread_id: i64,
) -> Result<(Vec<IdeStackFrame>, Option<u64>)> {
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
    let top_ip = arr
        .first()
        .and_then(|f| f.get("instructionPointerReference"))
        .and_then(Value::as_str)
        .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok());
    let frames = arr
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
                .map(|p| display_target_path(target, p));
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
        .collect();
    Ok((frames, top_ip))
}

#[cfg(test)]
mod breakpoint_relocation_tests {
    use super::*;
    use crate::dap::session::DapStartContext;
    use crate::dap::wire::DapResponse;
    use crate::types::{IdeStartDebugReq, IdeTargetSpec};

    #[test]
    fn apply_breakpoint_event_relocates_deferred_unbound_breakpoint_by_id_and_original_line() {
        let bp_path = "/ws/src/app.js".to_string();
        let req = IdeStartDebugReq {
            target: IdeTargetSpec::local("/ws"),
            language: "browser".into(),
            breakpoints: vec![IdeBreakpoint {
                path: bp_path.clone(),
                line: 10,
                enabled: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        let entry = SessionEntry::new(
            1,
            (
                "s1".into(),
                "Browser".into(),
                "browser".into(),
                "node-inspector-dap".into(),
            ),
            Vec::new(),
            req,
            DapStartContext::default(),
        );
        let mut bps = entry.read_snapshot().breakpoints;
        let mut locs = std::collections::HashMap::new();
        let indexed = vec![(Some(0usize), json!({ "line": 10 }))];
        let unbound_resp = DapResponse {
            seq: 1,
            request_seq: 1,
            success: true,
            command: "setBreakpoints".into(),
            message: None,
            body: Some(json!({
                "breakpoints": [{
                    "id": 42,
                    "verified": false,
                    "line": 10,
                    "source": { "path": bp_path },
                    "message": "Unbound: waiting for script"
                }]
            })),
        };
        apply_set_breakpoints_result(&mut bps, &mut locs, &indexed, Ok(unbound_resp), false);
        assert_eq!(bps[0].verified, Some(false));
        assert_eq!(locs.get(&42), Some(&(bp_path.clone(), 10)));
        entry.mutate_and_broadcast(|snap| snap.breakpoints = bps);
        let locs = std::sync::Mutex::new(locs);

        // BUG3-10: Deferred CDP relocation to line 15 arrives with id=42 (no match on line 10!).
        apply_breakpoint_event(
            &entry,
            &locs,
            &json!({
                "reason": "changed",
                "breakpoint": {
                    "id": 42,
                    "verified": true,
                    "line": 15,
                    "source": { "path": bp_path }
                }
            }),
        );
        let snap = entry.read_snapshot();
        assert_eq!(
            snap.breakpoints[0].verified,
            Some(true),
            "BUG3-10: relocated breakpoint must become verified"
        );
        assert_eq!(
            snap.breakpoints[0].line, 15,
            "BUG3-10: relocated breakpoint must update to resolved executable line 15"
        );
        assert_eq!(
            snap.breakpoints[0].message, None,
            "BUG3-10: stale Unbound message must be cleared once verified"
        );
    }
}
