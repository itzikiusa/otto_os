//! JDWP breakpoint binding, thread listing, stack frames, and variable slot expansion
//! for [`super::spawn_java_jdwp_bridge`].

use std::collections::HashSet;
use std::io;
use std::path::Path;

use serde_json::{json, Value};
use tokio::io::AsyncWrite;

use super::eval::{format_jdwp_value, jdwp_sig_tag, pretty_java_type};
use super::jdwp::{
    JdwpBuf, JdwpClient, JdwpCursor, JdwpFieldInfo, JdwpLocation, JdwpValue, EVENT_BREAKPOINT,
    EVENT_CLASS_PREPARE,
};
use super::{
    find_line_in_methods, infer_java_fqcn, set_class_prepare_filter, set_location_breakpoint,
    CachedFrame, JavaBpSpec, JdwpBridgeState, VarExpansion,
};
use crate::dap::adapters::normalize_target_path;
use crate::dap::wire::DapWriter;

pub(super) async fn apply_java_breakpoints<W: AsyncWrite + Unpin>(
    req_seq: i64,
    args: Option<&Value>,
    client: &JdwpClient,
    st: &mut JdwpBridgeState,
    dap_w: &mut DapWriter<W>,
) -> io::Result<()> {
    let raw_path = args
        .and_then(|a| a.get("source"))
        .and_then(|s| s.get("path"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let path = normalize_target_path(&st.target, raw_path);
    if let Some(fname) = Path::new(&path).file_name().and_then(|n| n.to_str()) {
        st.source_map.insert(fname.to_string(), path.clone());
    }
    if let Some(parent) = Path::new(&path).parent().and_then(|p| p.to_str()) {
        let src_root = crate::dap::adapters_jvm_js::infer_java_source_root(&path, parent);
        if !st.source_roots.contains(&src_root) {
            st.source_roots.push(src_root);
        }
    }

    if let Some(old_reqs) = st.file_req_ids.remove(&path) {
        for (ek, rid) in old_reqs {
            client.clear_event(ek, rid).await;
            st.req_to_bp.remove(&rid);
        }
    }

    let fqcn = infer_java_fqcn(&st.target, &path).await;
    let bps = args
        .and_then(|a| a.get("breakpoints"))
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();

    let mut specs = Vec::with_capacity(bps.len());
    for bp in &bps {
        let line = bp.get("line").and_then(Value::as_u64).unwrap_or(1) as u32;
        let condition = bp
            .get("condition")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from);
        let hit_condition = bp
            .get("hitCondition")
            .and_then(Value::as_str)
            .and_then(|s| {
                s.trim()
                    .trim_start_matches(|c: char| !c.is_ascii_digit())
                    .parse::<u32>()
                    .ok()
            })
            .filter(|&n| n > 0);
        let log_message = bp
            .get("logMessage")
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .map(String::from);
        specs.push(JavaBpSpec {
            path: path.clone(),
            fqcn: fqcn.clone(),
            line,
            condition,
            hit_condition,
            log_message,
            hits: 0,
        });
    }

    let mut req_ids: Vec<(u8, i32)> = Vec::new();
    let mut verified_lines: HashSet<u32> = HashSet::new();

    if !specs.is_empty() {
        if let Some(rid) = set_class_prepare_filter(client, &fqcn).await {
            req_ids.push((EVENT_CLASS_PREPARE, rid));
        }
        if let Some(rid) = set_class_prepare_filter(client, &format!("{fqcn}$*")).await {
            req_ids.push((EVENT_CLASS_PREPARE, rid));
        }

        let slash = fqcn.replace('.', "/");
        let exact_sig = format!("L{slash};");
        let inner_prefix = format!("L{slash}$");
        if let Ok(classes) = client.all_classes().await {
            for (tag, cid, sig, status) in classes {
                if (status & 4) == 0 || (sig != exact_sig && !sig.starts_with(&inner_prefix)) {
                    continue;
                }
                let methods = client.methods(cid).await.unwrap_or_default();
                for sp in &specs {
                    if let Some((mid, code_idx, _)) =
                        find_line_in_methods(client, cid, &methods, sp.line).await
                    {
                        let loc = JdwpLocation {
                            type_tag: tag,
                            class_id: cid,
                            method_id: mid,
                            index: code_idx,
                        };
                        if let Some(rid) = set_location_breakpoint(client, &loc).await {
                            req_ids.push((EVENT_BREAKPOINT, rid));
                            st.req_to_bp.insert(rid, (path.clone(), sp.line));
                            verified_lines.insert(sp.line);
                        }
                    }
                }
            }
        }
    }

    let resp_bps: Vec<Value> = specs
        .iter()
        .map(|sp| {
            let is_ver = verified_lines.contains(&sp.line) || !req_ids.is_empty();
            json!({
                "verified": is_ver,
                "line": sp.line,
                "source": { "path": sp.path }
            })
        })
        .collect();

    st.file_req_ids.insert(path.clone(), req_ids);
    st.bps_by_file.insert(path, specs);

    dap_w
        .send_response(
            req_seq,
            "setBreakpoints",
            Ok(Some(json!({ "breakpoints": resp_bps }))),
        )
        .await?;
    Ok(())
}

pub(super) async fn list_jdwp_threads(client: &JdwpClient) -> Vec<Value> {
    let Ok((ec, data)) = client.cmd(1, 4, &[]).await else {
        return vec![json!({ "id": 1, "name": "main" })];
    };
    if ec != 0 {
        return vec![json!({ "id": 1, "name": "main" })];
    }
    let mut cur = JdwpCursor::new(&data);
    let Ok(cnt) = cur.i32() else {
        return vec![json!({ "id": 1, "name": "main" })];
    };
    let mut all = Vec::new();
    let mut user = Vec::new();
    for _ in 0..(cnt.max(0) as usize).min(128) {
        let Ok(tid) = cur.id(client.sizes.object_id) else {
            break;
        };
        let mut nb = JdwpBuf::new();
        nb.put_id(tid, client.sizes.object_id);
        let name = client
            .cmd(11, 1, &nb.into_vec())
            .await
            .ok()
            .and_then(|(ec, d)| {
                (ec == 0)
                    .then(|| JdwpCursor::new(&d).string().ok())
                    .flatten()
            })
            .unwrap_or_else(|| format!("Thread-{tid}"));
        let entry = json!({ "id": tid as i64, "name": name });
        let is_sys = matches!(
            name.as_str(),
            "Reference Handler"
                | "Finalizer"
                | "Signal Dispatcher"
                | "Common-Cleaner"
                | "Notification Thread"
                | "Attach Listener"
        ) || name.starts_with("JDWP ");
        if !is_sys {
            user.push(entry.clone());
        }
        all.push(entry);
    }
    if !user.is_empty() {
        user
    } else if !all.is_empty() {
        all
    } else {
        vec![json!({ "id": 1, "name": "main" })]
    }
}

pub(super) async fn populate_stopped_frames(
    client: &JdwpClient,
    st: &mut JdwpBridgeState,
    thread_id: u64,
) {
    st.frames.clear();
    st.ordered_frames.clear();
    st.var_refs.clear();

    let mut b = JdwpBuf::new();
    b.put_id(thread_id, client.sizes.object_id);
    b.put_i32(0);
    b.put_i32(-1);
    let Ok((ec, data)) = client.cmd(11, 6, &b.into_vec()).await else {
        return;
    };
    if ec != 0 {
        return;
    }
    let mut cur = JdwpCursor::new(&data);
    let Ok(cnt) = cur.i32() else { return };
    for idx in 0..(cnt.max(0) as usize).min(48) {
        let Ok(jdwp_fid) = cur.id(client.sizes.frame_id) else {
            break;
        };
        let Ok(loc) = cur.location(&client.sizes) else {
            break;
        };
        let class_sig = client
            .class_signature(loc.class_id)
            .await
            .unwrap_or_default();
        let methods = client.methods(loc.class_id).await.unwrap_or_default();
        let method_name = methods
            .iter()
            .find(|m| m.method_id == loc.method_id)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| "<method>".into());
        let line = client
            .line_table(loc.class_id, loc.method_id)
            .await
            .ok()
            .and_then(|tbl| {
                tbl.iter()
                    .filter(|(ci, _)| *ci <= loc.index)
                    .max_by_key(|(ci, _)| *ci)
                    .or_else(|| tbl.first())
                    .map(|(_, ln)| *ln)
            })
            .unwrap_or(1);
        let src_file = client.source_file(loc.class_id).await;
        let path = st.resolve_source_path(&class_sig, src_file.as_deref());
        let short_cls = pretty_java_type(&class_sig);
        let short_cls = short_cls.rsplit('.').next().unwrap_or(&short_cls);
        let fid = (idx + 1) as i64;
        st.frames.insert(
            fid,
            CachedFrame {
                thread_id,
                jdwp_frame_id: jdwp_fid,
                loc,
                class_sig: class_sig.clone(),
            },
        );
        let source_val = path.as_ref().map(|p| {
            let name = Path::new(p)
                .file_name()
                .and_then(|s| s.to_str())
                .unwrap_or(p.as_str());
            json!({ "name": name, "path": p })
        });
        st.ordered_frames.push(json!({
            "id": fid,
            "name": format!("{short_cls}.{method_name}"),
            "source": source_val,
            "line": line,
            "column": 1
        }));
    }
}

pub(super) async fn resolve_variables(
    client: &JdwpClient,
    st: &mut JdwpBridgeState,
    vref: i64,
) -> Vec<Value> {
    let Some(exp) = st.var_refs.get(&vref).cloned() else {
        return Vec::new();
    };
    match exp {
        VarExpansion::FrameLocals(fid) => resolve_frame_locals(client, st, fid).await,
        VarExpansion::ObjectFields(oid) => resolve_object_fields(client, st, oid).await,
        VarExpansion::ArrayItems(aid) => resolve_array_items(client, st, aid).await,
    }
}

pub(super) async fn resolve_frame_locals(
    client: &JdwpClient,
    st: &mut JdwpBridgeState,
    fid: i64,
) -> Vec<Value> {
    let Some(cf) = st.frames.get(&fid).cloned() else {
        return Vec::new();
    };
    let mut out = Vec::new();

    if let Some(this_id) = fetch_this_object(client, &cf).await {
        let (val, vt, cref) = format_jdwp_value(
            client,
            st,
            JdwpValue::Object { id: this_id },
            Some(&cf.class_sig),
        )
        .await;
        out.push(json!({
            "name": "this",
            "value": val,
            "type": vt,
            "variablesReference": cref
        }));
    }

    let table = client
        .variable_table(cf.loc.class_id, cf.loc.method_id)
        .await
        .unwrap_or_default();
    let mut seen_slots = HashSet::new();
    let mut active = Vec::new();
    for v in table {
        if v.name == "this" {
            continue;
        }
        let end = v.code_index.saturating_add(u64::from(v.length));
        if cf.loc.index >= v.code_index && cf.loc.index < end && seen_slots.insert(v.slot) {
            active.push(v);
        }
    }
    if active.is_empty() {
        return out;
    }

    let mut b = JdwpBuf::new();
    b.put_id(cf.thread_id, client.sizes.object_id);
    b.put_id(cf.jdwp_frame_id, client.sizes.frame_id);
    b.put_i32(active.len() as i32);
    for v in &active {
        b.put_i32(v.slot);
        b.put_u8(jdwp_sig_tag(&v.signature));
    }
    let Ok((ec, data)) = client.cmd(16, 1, &b.into_vec()).await else {
        return out;
    };
    if ec != 0 {
        return out;
    }
    let mut cur = JdwpCursor::new(&data);
    let Ok(cnt) = cur.i32() else { return out };
    for v in active.iter().take(cnt.max(0) as usize) {
        let Ok(jval) = cur.tagged_value(&client.sizes) else {
            break;
        };
        let (val_s, vt, cref) = format_jdwp_value(client, st, jval, Some(&v.signature)).await;
        out.push(json!({
            "name": v.name,
            "value": val_s,
            "type": vt,
            "variablesReference": cref
        }));
    }
    out
}

async fn fetch_this_object(client: &JdwpClient, cf: &CachedFrame) -> Option<u64> {
    let mut b = JdwpBuf::new();
    b.put_id(cf.thread_id, client.sizes.object_id);
    b.put_id(cf.jdwp_frame_id, client.sizes.frame_id);
    let (ec, data) = client.cmd(16, 3, &b.into_vec()).await.ok()?;
    if ec != 0 {
        return None;
    }
    match JdwpCursor::new(&data).tagged_value(&client.sizes).ok()? {
        JdwpValue::Object { id } | JdwpValue::StringObj(id) | JdwpValue::ArrayObj(id)
            if id != 0 =>
        {
            Some(id)
        }
        _ => None,
    }
}

pub(super) async fn collect_instance_fields(
    client: &JdwpClient,
    obj_id: u64,
) -> Vec<JdwpFieldInfo> {
    let mut b = JdwpBuf::new();
    b.put_id(obj_id, client.sizes.object_id);
    let Ok((ec, data)) = client.cmd(9, 1, &b.into_vec()).await else {
        return Vec::new();
    };
    if ec != 0 {
        return Vec::new();
    }
    let mut cur = JdwpCursor::new(&data);
    let Ok(mut ref_tag) = cur.u8() else {
        return Vec::new();
    };
    let Ok(mut type_id) = cur.id(client.sizes.ref_type_id) else {
        return Vec::new();
    };

    let mut fields = Vec::new();
    let mut seen_names = HashSet::new();
    for _ in 0..5 {
        if type_id == 0 {
            break;
        }
        for f in client.fields(type_id).await.unwrap_or_default() {
            if (f.mod_bits & 8) == 0 && seen_names.insert(f.name.clone()) {
                fields.push(f);
            }
        }
        if ref_tag != 1 {
            break;
        }
        let mut sb = JdwpBuf::new();
        sb.put_id(type_id, client.sizes.ref_type_id);
        let Ok((sec, sdata)) = client.cmd(3, 1, &sb.into_vec()).await else {
            break;
        };
        if sec != 0 {
            break;
        }
        let Ok(super_id) = JdwpCursor::new(&sdata).id(client.sizes.ref_type_id) else {
            break;
        };
        type_id = super_id;
        ref_tag = 1;
    }
    fields
}

async fn resolve_object_fields(
    client: &JdwpClient,
    st: &mut JdwpBridgeState,
    obj_id: u64,
) -> Vec<Value> {
    let inst_fields = collect_instance_fields(client, obj_id).await;
    if inst_fields.is_empty() {
        return Vec::new();
    }
    let mut b = JdwpBuf::new();
    b.put_id(obj_id, client.sizes.object_id);
    b.put_i32(inst_fields.len() as i32);
    for f in &inst_fields {
        b.put_id(f.field_id, client.sizes.field_id);
    }
    let Ok((ec, data)) = client.cmd(9, 2, &b.into_vec()).await else {
        return Vec::new();
    };
    if ec != 0 {
        return Vec::new();
    }
    let mut cur = JdwpCursor::new(&data);
    let Ok(cnt) = cur.i32() else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for f in inst_fields.iter().take(cnt.max(0) as usize) {
        let Ok(jval) = cur.tagged_value(&client.sizes) else {
            break;
        };
        let (val_s, vt, cref) = format_jdwp_value(client, st, jval, Some(&f.signature)).await;
        out.push(json!({
            "name": f.name,
            "value": val_s,
            "type": vt,
            "variablesReference": cref
        }));
    }
    out
}

async fn resolve_array_items(
    client: &JdwpClient,
    st: &mut JdwpBridgeState,
    arr_id: u64,
) -> Vec<Value> {
    let len = array_length(client, arr_id).await.unwrap_or(0);
    let count = len.clamp(0, 64);
    if count == 0 {
        return Vec::new();
    }
    let mut b = JdwpBuf::new();
    b.put_id(arr_id, client.sizes.object_id);
    b.put_i32(0);
    b.put_i32(count);
    let Ok((ec, data)) = client.cmd(13, 2, &b.into_vec()).await else {
        return Vec::new();
    };
    if ec != 0 {
        return Vec::new();
    }
    let mut cur = JdwpCursor::new(&data);
    let Ok(tag) = cur.u8() else {
        return Vec::new();
    };
    let Ok(n) = cur.i32() else {
        return Vec::new();
    };
    let is_ref = matches!(tag, b'L' | b'[' | b's' | b't' | b'g' | b'l' | b'c');
    let mut out = Vec::new();
    for idx in 0..(n.max(0) as usize) {
        let jval = if is_ref {
            cur.tagged_value(&client.sizes)
        } else {
            cur.untagged_value(tag, &client.sizes)
        };
        let Ok(jv) = jval else { break };
        let (val_s, vt, cref) = format_jdwp_value(client, st, jv, None).await;
        out.push(json!({
            "name": format!("[{idx}]"),
            "value": val_s,
            "type": vt,
            "variablesReference": cref
        }));
    }
    out
}

pub(super) async fn array_length(client: &JdwpClient, arr_id: u64) -> Option<i32> {
    let mut b = JdwpBuf::new();
    b.put_id(arr_id, client.sizes.object_id);
    let (ec, data) = client.cmd(13, 1, &b.into_vec()).await.ok()?;
    (ec == 0)
        .then(|| JdwpCursor::new(&data).i32().ok())
        .flatten()
}
