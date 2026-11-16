//! JDWP value formatting, watch/REPL expression evaluation, conditional/logpoint
//! breakpoint filters, and line-step request builders for [`super::spawn_java_jdwp_bridge`].

use std::io;

use serde_json::{json, Value};
use tokio::io::AsyncWrite;

use super::inspect::{
    array_length, collect_instance_fields, resolve_frame_locals, resolve_variables,
};
use super::jdwp::{
    JdwpBuf, JdwpClient, JdwpCursor, JdwpValue, EVENT_SINGLE_STEP, STEP_DEPTH_INTO, STEP_SIZE_LINE,
    SUSPEND_ALL,
};
use super::{JdwpBridgeState, VarExpansion};
use crate::dap::wire::DapWriter;

pub(super) async fn format_jdwp_value(
    client: &JdwpClient,
    st: &mut JdwpBridgeState,
    val: JdwpValue,
    sig_hint: Option<&str>,
) -> (String, Option<String>, i64) {
    let hint_ty = sig_hint.map(pretty_java_type);
    match val {
        JdwpValue::Byte(v) => (v.to_string(), Some("byte".into()), 0),
        JdwpValue::Char(v) => (format!("{v:?}"), Some("char".into()), 0),
        JdwpValue::Float(v) => (v.to_string(), Some("float".into()), 0),
        JdwpValue::Double(v) => (v.to_string(), Some("double".into()), 0),
        JdwpValue::Int(v) => (v.to_string(), Some("int".into()), 0),
        JdwpValue::Long(v) => (v.to_string(), Some("long".into()), 0),
        JdwpValue::Short(v) => (v.to_string(), Some("short".into()), 0),
        JdwpValue::Bool(v) => (v.to_string(), Some("boolean".into()), 0),
        JdwpValue::Void => ("void".into(), Some("void".into()), 0),
        JdwpValue::Null => ("null".into(), hint_ty.or(Some("Object".into())), 0),
        JdwpValue::StringObj(id) => {
            let s = client.string_value(id).await.unwrap_or_default();
            (format!("{s:?}"), Some("String".into()), 0)
        }
        JdwpValue::ArrayObj(id) => {
            let len = array_length(client, id).await.unwrap_or(0);
            let r = st.alloc_ref(VarExpansion::ArrayItems(id));
            let base = hint_ty.unwrap_or_else(|| "Object[]".into());
            (format!("{base} (len={len})"), Some(base), r)
        }
        JdwpValue::Object { id } => {
            let rt_sig = object_class_sig(client, id).await;
            if rt_sig.as_deref() == Some("Ljava/lang/String;")
                || sig_hint == Some("Ljava/lang/String;")
            {
                let s = client.string_value(id).await.unwrap_or_default();
                return (format!("{s:?}"), Some("String".into()), 0);
            }
            if let Some(boxed) = unbox_wrapper_value(client, id, rt_sig.as_deref()).await {
                let ty = rt_sig.as_deref().map(pretty_java_type).or(hint_ty);
                return (boxed, ty, 0);
            }
            let ty = rt_sig
                .as_deref()
                .map(pretty_java_type)
                .or(hint_ty)
                .unwrap_or_else(|| "Object".into());
            let short = ty.rsplit('.').next().unwrap_or(&ty);
            let r = st.alloc_ref(VarExpansion::ObjectFields(id));
            (format!("{short}@0x{id:x}"), Some(ty), r)
        }
    }
}

async fn object_class_sig(client: &JdwpClient, obj_id: u64) -> Option<String> {
    let mut b = JdwpBuf::new();
    b.put_id(obj_id, client.sizes.object_id);
    let (ec, data) = client.cmd(9, 1, &b.into_vec()).await.ok()?;
    if ec != 0 {
        return None;
    }
    let mut cur = JdwpCursor::new(&data);
    let _tag = cur.u8().ok()?;
    let tid = cur.id(client.sizes.ref_type_id).ok()?;
    client.class_signature(tid).await
}

async fn unbox_wrapper_value(
    client: &JdwpClient,
    obj_id: u64,
    sig: Option<&str>,
) -> Option<String> {
    let is_wrapper = matches!(
        sig?,
        "Ljava/lang/Integer;"
            | "Ljava/lang/Long;"
            | "Ljava/lang/Boolean;"
            | "Ljava/lang/Double;"
            | "Ljava/lang/Float;"
            | "Ljava/lang/Short;"
            | "Ljava/lang/Byte;"
            | "Ljava/lang/Character;"
    );
    if !is_wrapper {
        return None;
    }
    let fields = collect_instance_fields(client, obj_id).await;
    let vf = fields.iter().find(|f| f.name == "value")?;
    let mut b = JdwpBuf::new();
    b.put_id(obj_id, client.sizes.object_id);
    b.put_i32(1);
    b.put_id(vf.field_id, client.sizes.field_id);
    let (ec, data) = client.cmd(9, 2, &b.into_vec()).await.ok()?;
    if ec != 0 {
        return None;
    }
    let mut cur = JdwpCursor::new(&data);
    let _ = cur.i32().ok()?;
    match cur.tagged_value(&client.sizes).ok()? {
        JdwpValue::Int(v) => Some(v.to_string()),
        JdwpValue::Long(v) => Some(v.to_string()),
        JdwpValue::Bool(v) => Some(v.to_string()),
        JdwpValue::Double(v) => Some(v.to_string()),
        JdwpValue::Float(v) => Some(v.to_string()),
        JdwpValue::Short(v) => Some(v.to_string()),
        JdwpValue::Byte(v) => Some(v.to_string()),
        JdwpValue::Char(v) => Some(format!("{v:?}")),
        _ => None,
    }
}

pub(super) async fn evaluate_java_expr(
    client: &JdwpClient,
    st: &mut JdwpBridgeState,
    fid: i64,
    expr: &str,
) -> Option<(String, Option<String>, i64)> {
    let trimmed = expr.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(n) = trimmed.parse::<i64>() {
        return Some((n.to_string(), Some("long".into()), 0));
    }
    if trimmed == "true" || trimmed == "false" {
        return Some((trimmed.to_string(), Some("boolean".into()), 0));
    }
    if trimmed.starts_with('"') && trimmed.ends_with('"') && trimmed.len() >= 2 {
        return Some((trimmed.to_string(), Some("String".into()), 0));
    }

    for op in ["==", "!=", ">=", "<=", "+", "-", "*", "/", "%", ">", "<"] {
        if let Some((lhs_raw, rhs_raw)) = split_top_level_binop(trimmed, op) {
            let (lv, _, _) = Box::pin(evaluate_java_expr(client, st, fid, lhs_raw)).await?;
            let (rv, _, _) = Box::pin(evaluate_java_expr(client, st, fid, rhs_raw)).await?;
            return eval_binary_values(op, &lv, &rv);
        }
    }

    let locals = resolve_frame_locals(client, st, fid).await;
    let parts: Vec<&str> = trimmed.split('.').map(str::trim).collect();
    let root_name = parts[0];
    let mut cur_var = locals
        .into_iter()
        .find(|v| v.get("name").and_then(Value::as_str) == Some(root_name))?;

    for seg in &parts[1..] {
        let val_s = cur_var.get("value").and_then(Value::as_str).unwrap_or("");
        let cref = cur_var
            .get("variablesReference")
            .and_then(Value::as_i64)
            .unwrap_or(0);
        if *seg == "length()" || *seg == "length" {
            if val_s.starts_with('"') && val_s.ends_with('"') && val_s.len() >= 2 {
                let unq = serde_json::from_str::<String>(val_s).unwrap_or_default();
                return Some((unq.chars().count().to_string(), Some("int".into()), 0));
            }
            if let Some(len_part) = val_s
                .split("len=")
                .nth(1)
                .and_then(|s| s.trim_end_matches(')').parse::<usize>().ok())
            {
                return Some((len_part.to_string(), Some("int".into()), 0));
            }
        }
        if cref <= 0 {
            return None;
        }
        let children = resolve_variables(client, st, cref).await;
        cur_var = children
            .into_iter()
            .find(|c| c.get("name").and_then(Value::as_str) == Some(*seg))?;
    }

    Some((
        cur_var
            .get("value")
            .and_then(Value::as_str)
            .unwrap_or("null")
            .to_string(),
        cur_var
            .get("type")
            .and_then(Value::as_str)
            .map(String::from),
        cur_var
            .get("variablesReference")
            .and_then(Value::as_i64)
            .unwrap_or(0),
    ))
}

fn split_top_level_binop<'a>(expr: &'a str, op: &str) -> Option<(&'a str, &'a str)> {
    let mut in_str = false;
    let mut depth = 0i32;
    let bytes = expr.as_bytes();
    let op_b = op.as_bytes();
    let mut i = 0;
    while i + op_b.len() <= bytes.len() {
        let c = bytes[i];
        if c == b'"' && (i == 0 || bytes[i - 1] != b'\\') {
            in_str = !in_str;
        } else if !in_str {
            match c {
                b'(' | b'[' => depth += 1,
                b')' | b']' => depth -= 1,
                _ => {}
            }
            if depth == 0 && &bytes[i..i + op_b.len()] == op_b {
                let lhs = expr[..i].trim();
                let rhs = expr[i + op_b.len()..].trim();
                if !lhs.is_empty() && !rhs.is_empty() {
                    return Some((lhs, rhs));
                }
            }
        }
        i += 1;
    }
    None
}

fn eval_binary_values(op: &str, lv: &str, rv: &str) -> Option<(String, Option<String>, i64)> {
    let li = lv.parse::<i64>().ok();
    let ri = rv.parse::<i64>().ok();
    if let (Some(a), Some(b)) = (li, ri) {
        return match op {
            "+" => Some(((a + b).to_string(), Some("long".into()), 0)),
            "-" => Some(((a - b).to_string(), Some("long".into()), 0)),
            "*" => Some(((a * b).to_string(), Some("long".into()), 0)),
            "/" if b != 0 => Some(((a / b).to_string(), Some("long".into()), 0)),
            "%" if b != 0 => Some(((a % b).to_string(), Some("long".into()), 0)),
            "==" => Some(((a == b).to_string(), Some("boolean".into()), 0)),
            "!=" => Some(((a != b).to_string(), Some("boolean".into()), 0)),
            ">=" => Some(((a >= b).to_string(), Some("boolean".into()), 0)),
            "<=" => Some(((a <= b).to_string(), Some("boolean".into()), 0)),
            ">" => Some(((a > b).to_string(), Some("boolean".into()), 0)),
            "<" => Some(((a < b).to_string(), Some("boolean".into()), 0)),
            _ => None,
        };
    }
    let clean = |s: &str| {
        serde_json::from_str::<String>(s).unwrap_or_else(|_| s.trim_matches('"').to_string())
    };
    match op {
        "+" => Some((
            format!("{:?}", format!("{}{}", clean(lv), clean(rv))),
            Some("String".into()),
            0,
        )),
        "==" => Some((
            (clean(lv) == clean(rv)).to_string(),
            Some("boolean".into()),
            0,
        )),
        "!=" => Some((
            (clean(lv) != clean(rv)).to_string(),
            Some("boolean".into()),
            0,
        )),
        _ => None,
    }
}

pub(super) async fn should_auto_resume_bp<W: AsyncWrite + Unpin>(
    client: &JdwpClient,
    st: &mut JdwpBridgeState,
    req_id: i32,
    dap_w: &mut DapWriter<W>,
) -> io::Result<bool> {
    let Some((file, line)) = st.req_to_bp.get(&req_id).cloned() else {
        return Ok(false);
    };
    let (cond, hit_cond, log_msg) = {
        let Some(bps) = st.bps_by_file.get_mut(&file) else {
            return Ok(false);
        };
        let Some(bp) = bps.iter_mut().find(|b| b.line == line) else {
            return Ok(false);
        };
        bp.hits += 1;
        (
            bp.condition.clone(),
            bp.hit_condition.map(|hc| (bp.hits, hc)),
            bp.log_message.clone(),
        )
    };

    if let Some(c) = cond {
        match evaluate_java_expr(client, st, 1, &c).await {
            Some((val, _, _)) if val == "false" || val == "0" || val == "null" => {
                return Ok(true);
            }
            _ => {}
        }
    }
    if let Some((hits, want)) = hit_cond {
        if hits < want {
            return Ok(true);
        }
    }
    if let Some(msg_tpl) = log_msg {
        let mut rendered = msg_tpl;
        while let Some(open) = rendered.find('{') {
            let Some(rel_close) = rendered[open + 1..].find('}') else {
                break;
            };
            let close = open + 1 + rel_close;
            let inner = rendered[open + 1..close].trim().to_string();
            let repl = evaluate_java_expr(client, st, 1, &inner)
                .await
                .map(|(v, _, _)| v.trim_matches('"').to_string())
                .unwrap_or_else(|| format!("<{inner}>"));
            rendered.replace_range(open..=close, &repl);
        }
        dap_w
            .send_event(
                "output",
                Some(json!({ "category": "stdout", "output": format!("{rendered}\n") })),
            )
            .await?;
        return Ok(true);
    }
    Ok(false)
}

pub(super) async fn register_step_request(
    client: &JdwpClient,
    thread_id: u64,
    depth: i32,
) -> Option<i32> {
    let excludes: &[&str] = if depth == STEP_DEPTH_INTO {
        &["java.*", "jdk.*", "sun.*", "javax.*"]
    } else {
        &[]
    };
    let mut b = JdwpBuf::new();
    b.put_u8(EVENT_SINGLE_STEP);
    b.put_u8(SUSPEND_ALL);
    b.put_i32(2 + excludes.len() as i32);
    b.put_u8(10);
    b.put_id(thread_id, client.sizes.object_id);
    b.put_i32(STEP_SIZE_LINE);
    b.put_i32(depth);
    for pat in excludes {
        b.put_u8(6);
        b.put_str(pat);
    }
    b.put_u8(1);
    b.put_i32(1);
    let (ec, data) = client.cmd(15, 1, &b.into_vec()).await.ok()?;
    (ec == 0)
        .then(|| JdwpCursor::new(&data).i32().ok())
        .flatten()
}

pub(super) fn jdwp_sig_tag(sig: &str) -> u8 {
    if sig.starts_with('[') {
        b'['
    } else if sig == "Ljava/lang/String;" {
        b'L'
    } else {
        sig.as_bytes().first().copied().unwrap_or(b'L')
    }
}

pub(super) fn pretty_java_type(sig: &str) -> String {
    if let Some(elem) = sig.strip_prefix('[') {
        return format!("{}[]", pretty_java_type(elem));
    }
    if let Some(inner) = sig.strip_prefix('L').and_then(|s| s.strip_suffix(';')) {
        let dotted = inner.replace('/', ".");
        return dotted
            .strip_prefix("java.lang.")
            .unwrap_or(&dotted)
            .to_string();
    }
    match sig {
        "B" => "byte".into(),
        "C" => "char".into(),
        "D" => "double".into(),
        "F" => "float".into(),
        "I" => "int".into(),
        "J" => "long".into(),
        "S" => "short".into(),
        "Z" => "boolean".into(),
        "V" => "void".into(),
        other => other.to_string(),
    }
}
