//! The `agent_prompt` workflow node, moved verbatim out of
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
        scope,
        session_tx,
        log_tx,
        activity_tx,
        ..
    } = call;
    let run_cwd: &str = env.run_cwd.as_str();
    let p = &node.params;
    let prompt = p
        .get("prompt")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    if prompt.trim().is_empty() {
        return Err(otto_core::Error::Invalid("agent node: empty prompt".into()));
    }
    // Run as a real, openable session (reusing the shared session runner)
    // so the run view can watch/inspect it — not the headless PTY.
    let provider = p
        .get("provider")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(env.default_provider.as_str());
    let model = p.get("model").and_then(Value::as_str);
    // File-based handoff (design 2026-07-02): point the agent at the
    // run's context dir (instruction, repos.json, prior step files)
    // and name the step file IT must write. The inline [input data]
    // excerpt stays as a glance — the files are the complete channel.
    let preamble = env.files.preamble_for(
        scope.step_no,
        node_display_name(node),
        scope.iter,
        scope.inner_idx,
    );
    // Inject any per-step skills (`skill`/`skills`) ahead of the prompt so
    // the step runs a specific method "via prompt".
    let full = prepend_skills(
        ctx,
        p,
        &format!(
            "{preamble}{prompt}\n\n[input data]\n{}",
            truncate(&input.to_string(), 4000)
        ),
    );
    let acwd = node_cwd(node, &input, run_cwd);
    let done = env
        .files
        .step_md_path(&crate::workflow_context::step_base_name(
            scope.step_no,
            node_display_name(node),
            scope.iter,
            scope.inner_idx,
        ));
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
        done,
        session_tx,
        log_tx,
        activity_tx,
    )
    .await?;
    // Publish WHERE the implementer worked (+ thread the ambient repo/base)
    // so a downstream review/PR is aware of exactly this directory — even
    // when the agent ran in its own per-node cwd. This is what carries the
    // reference from the implementer to the reviewer and the PR.
    let mut out = serde_json::Map::new();
    out.insert("reply".into(), json!(reply));
    out.insert("session_id".into(), json!(sid));
    out.insert("working_directory".into(), json!(acwd));
    // `repos`/`worktree` forward too so data-flow inspection still
    // shows them — though the run-level registry (RunEnv.files), not
    // input threading, is what downstream git steps rely on.
    for k in ["repo_id", "base", "repos", "worktree", "pr", "pr_branch"] {
        if let Some(v) = input.get(k) {
            if !v.is_null() {
                out.insert(k.into(), v.clone());
            }
        }
    }
    Ok((Value::Object(out), vec!["agent turn complete".into()]))
}
