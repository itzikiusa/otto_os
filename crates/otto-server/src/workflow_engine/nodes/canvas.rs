//! The `canvas` workflow node, moved verbatim out of
//! `execute_node`'s match (see `nodes/mod.rs`).

use super::super::*;
use super::NodeCall;

pub(in crate::workflow_engine) async fn execute(
    call: NodeCall<'_>,
) -> Result<(Value, Vec<String>)> {
    let NodeCall {
        ctx,
        ws,
        user,
        node,
        input,
        env,
        session_tx,
        log_tx,
        activity_tx,
        ..
    } = call;
    let run_id: &Id = &env.run_id;
    let run_cwd: &str = env.run_cwd.as_str();
    let p = &node.params;
    let prompt_in = p
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or("Create a clear diagram of the system/flow described in the input.");
    let mode = p.get("mode").and_then(Value::as_str).unwrap_or("mermaid");
    let full = format!(
                "Produce a {mode} diagram. {prompt_in}\nReply with ONLY a ```{mode} code block.\n\n[context]\n{}",
                truncate(&input.to_string(), 4000)
            );
    let acwd = node_cwd(node, &input, run_cwd);
    let provider = p
        .get("provider")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(env.default_provider.as_str());
    let model = p.get("model").and_then(Value::as_str);
    let (reply, sid) = run_node_agent(
        ctx,
        ws,
        user,
        node,
        &env.wf_name,
        &env.run_id,
        provider,
        model,
        &full,
        &acwd,
        None,
        session_tx,
        log_tx,
        activity_tx,
    )
    .await?;
    let diagram = extract_code_block(&reply, mode).unwrap_or_else(|| reply.clone());
    // Write under the data dir (never the user's repo working tree).
    let ext = canvas_node_ext(mode);
    let rel = canvas_rel(run_id, &node.id, ext);
    let path = ctx.data_dir.join(&rel);
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let written = std::fs::write(&path, &diagram).is_ok();
    Ok((
        json!({ "scene_id": rel, "path": path.to_string_lossy(), "mode": mode,
                        "diagram": diagram, "written": written, "session_id": sid }),
        vec![format!("canvas: {mode} diagram ({} bytes)", diagram.len())],
    ))
}
