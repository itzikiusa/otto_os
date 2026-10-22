//! The `product_analyze` / `product_rewrite` / `product_plan` workflow node, moved verbatim out of
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
    let kind = node.kind.clone();
    let story_id = p
        .get("story_id")
        .and_then(Value::as_str)
        .or_else(|| input.get("story_id").and_then(Value::as_str))
        .map(str::to_string);
    let (skill_name, instruction, persist_kind) = match kind.as_str() {
                "product_analyze" => (
                    "grill",
                    "Analyze this product story: surface scope gaps, ambiguities, non-testable acceptance criteria, unhandled edge cases, and risks. Be specific and evidence-based.",
                    None,
                ),
                "product_rewrite" => (
                    "jira-story-writer",
                    "Rewrite this story so it is clear, valuable, and testable. Reply with the rewritten story in Markdown.",
                    Some("suggested"),
                ),
                _ => (
                    "story-task-breakdown",
                    "Break this story into an ordered implementation plan of small, independently-verifiable tasks. Reply in Markdown.",
                    Some("plan"),
                ),
            };
    let skill = crate::modules::resolve_skill_inline(&ctx.context_library, skill_name);
    let context = match &story_id {
        Some(sid) => ctx
            .product
            .build_agent_context(sid, None)
            .await
            .unwrap_or_else(|_| truncate(&input.to_string(), 6000)),
        None => truncate(&input.to_string(), 6000),
    };
    let extra = p.get("instruction").and_then(Value::as_str).unwrap_or("");
    // Same file-based handoff as agent_prompt: context dir + the step
    // file this node must write (engine fallback covers omission).
    let preamble = env.files.preamble_for(
        scope.step_no,
        node_display_name(node),
        scope.iter,
        scope.inner_idx,
    );
    let prompt = format!(
        "{preamble}{skill}\n\n# Task\n{instruction}\n{extra}\n\n# Story context\n{}",
        truncate(&context, 8000)
    );
    let acwd = node_cwd(node, &input, run_cwd);
    let provider = p
        .get("provider")
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
        .unwrap_or(env.default_provider.as_str());
    let model = p.get("model").and_then(Value::as_str);
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
        &prompt,
        &acwd,
        done,
        session_tx,
        log_tx,
        activity_tx,
    )
    .await?;
    let mut out = serde_json::Map::new();
    out.insert("story_id".into(), json!(story_id));
    out.insert("session_id".into(), json!(sid));
    match kind.as_str() {
        "product_analyze" => {
            out.insert("analysis".into(), json!(reply));
        }
        "product_rewrite" => {
            out.insert("body_md".into(), json!(reply));
        }
        _ => {
            out.insert("plan_md".into(), json!(reply));
        }
    }
    // Optional: persist as a product version.
    let persist = p.get("persist").and_then(Value::as_bool).unwrap_or(false);
    if let (Some(pk), Some(sid_story)) = (persist_kind, story_id.as_ref()) {
        if persist {
            let nv = otto_state::NewVersion {
                story_id: sid_story.clone(),
                kind: pk.into(),
                title: format!("Workflow {kind}"),
                body_md: reply.clone(),
                raw_json: None,
                change_notes: Some(format!("from workflow node {}", node.id)),
                created_by: user.id.clone(),
            };
            let v = persisted_version(ctx.product_repo.add_version(nv).await)?;
            out.insert("version_id".into(), json!(v));
        }
    }
    Ok((Value::Object(out), vec![format!("{kind}: complete")]))
}
