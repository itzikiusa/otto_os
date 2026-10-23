//! Per-agent retry: re-run a single failed / stuck analysis lens agent on its
//! EXISTING row (the user's Retry, and the boot reaper's auto-resume).

use otto_core::Id;
use tracing::warn;

use super::analysis::{findings_json, LENS_TIMEOUT};
use super::context::{skill_body, story_context_md, story_work};
use super::prompts::{augment_with_out_path, build_analysis_prompt, extract_json_as};
use super::scratch::{session_cwd, trust_session_cwd};
use super::{Findings, SessionAppearance, SUPERSEDED};
use crate::host::ProductRunHost;

/// Re-run a single analysis lens agent by id.  Mirrors one iteration of the
/// `run_analysis` per-agent loop but operates on an EXISTING agent row rather
/// than creating a new one.  The summarizer is NOT re-run; only the one lens.
///
/// All errors are isolated — nothing panics.  The caller (the HTTP handler)
/// spawns this as a background task and returns 202 immediately.
pub async fn retry_analysis_agent<C: ProductRunHost>(
    ctx: C,
    ws: otto_core::domain::Workspace,
    user_id: Id,
    analysis_id: Id,
    agent_id: Id,
) {
    let Ok(owner) = crate::agent_attempt::claim(&agent_id) else {
        return;
    };
    retry_analysis_agent_owned(ctx, ws, user_id, analysis_id, agent_id, owner).await;
}

fn retry_model(model: &str) -> Option<&str> {
    let model = model.trim();
    (!model.is_empty()).then_some(model)
}

pub(crate) async fn retry_analysis_agent_owned<C: ProductRunHost>(
    ctx: C,
    ws: otto_core::domain::Workspace,
    user_id: Id,
    analysis_id: Id,
    agent_id: Id,
    _owner: tokio::sync::OwnedMutexGuard<()>,
) {
    // 1. Load the agent row.
    let agent = match ctx.product_repo().get_analysis_agent(&agent_id).await {
        Ok(a) => a,
        Err(e) => {
            warn!("product_run(retry): get_analysis_agent {agent_id}: {e}");
            return;
        }
    };

    // 2. Verify the agent belongs to the requested analysis.
    if agent.analysis_id != analysis_id {
        warn!("product_run(retry): agent {agent_id} does not belong to analysis {analysis_id}");
        return;
    }

    // 3. Load the analysis → story (for context_path + cwd).
    let analysis = match ctx.product_repo().get_analysis(&analysis_id).await {
        Ok(a) => a,
        Err(e) => {
            warn!("product_run(retry): get_analysis {analysis_id}: {e}");
            let _ = ctx
                .product_repo()
                .set_agent_status(
                    &agent_id,
                    "error",
                    None,
                    Some("retry: analysis not found"),
                    true,
                )
                .await;
            return;
        }
    };

    let story = match ctx.product_repo().get_story(&analysis.story_id).await {
        Ok(s) => s,
        Err(e) => {
            warn!("product_run(retry): get_story {}: {e}", analysis.story_id);
            let _ = ctx
                .product_repo()
                .set_agent_status(
                    &agent_id,
                    "error",
                    None,
                    Some("retry: story not found"),
                    true,
                )
                .await;
            return;
        }
    };

    // 4. Mark the agent row "running" and clear prior error.
    let clear_err: Option<&str> = None;
    if let Err(e) = ctx
        .product_repo()
        .set_agent_status(&agent_id, "running", None, clear_err, false)
        .await
    {
        warn!("product_run(retry): set_agent_status running {agent_id}: {e}");
    }
    // Clear the error field explicitly by re-setting to None via a raw update.
    // (set_agent_status merges existing error; we want a clean slate.)
    // We do this by passing empty-string error and relying on the merge — the
    // empty string is better than the old error text for the UI.
    // (No separate "clear_error" API; the empty string approach is idiomatic here.)

    // 5. Pre-trust provider. As in the fan-out, a missing story cwd falls back to
    // the shared temp dir; rewrite that to a unique per-session subdir so codex
    // usage attributes this retry 1:1 by cwd. A real story cwd passes unchanged.
    let cwd = session_cwd(
        &story
            .cwd
            .clone()
            .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().to_string()),
    );
    trust_session_cwd(&agent.provider, &cwd);

    // 6. Rebuild context file (shared with this analysis's context path, or
    // fresh). Unlike the fan-out (which embeds daemon-minted ids), the retry ids
    // arrive as route params — confine the temp-dir joins so a hostile id can't
    // steer the scratch files (rust/path-injection).
    let context_path = match otto_core::paths::confine_join(
        &std::env::temp_dir(),
        &format!("otto-product-{analysis_id}-context.md"),
    ) {
        Some(p) => p,
        None => {
            warn!("product_run(retry): unsafe analysis id {analysis_id}");
            let _ = ctx
                .product_repo()
                .set_agent_status(
                    &agent_id,
                    "error",
                    None,
                    Some("retry: unsafe analysis id"),
                    true,
                )
                .await;
            return;
        }
    };
    if !context_path.exists() {
        // Context file was cleaned up; rebuild it.
        let context_md = story_context_md(
            &ctx,
            &analysis.story_id,
            &story.title,
            None,
            "product_run(retry)",
        )
        .await;
        if let Err(e) = std::fs::write(&context_path, &context_md) {
            warn!("product_run(retry): write context file: {e}");
        }
    }
    let context_path_str = context_path.to_string_lossy().to_string();

    // 7. Resolve skill body.
    let skill_body = skill_body(&ctx, &agent.skill);

    // 8. Build prompt + unique out_path (use agent_id for uniqueness). Same
    // route-param confinement as the context file above.
    let out_path = match otto_core::paths::confine_join(
        &std::env::temp_dir(),
        &format!(
            "otto-product-{analysis_id}-retry-{agent_id}-{}.json",
            uuid::Uuid::new_v4()
        ),
    ) {
        Some(p) => p,
        None => {
            warn!("product_run(retry): unsafe agent id {agent_id}");
            let _ = ctx
                .product_repo()
                .set_agent_status(
                    &agent_id,
                    "error",
                    None,
                    Some("retry: unsafe agent id"),
                    true,
                )
                .await;
            return;
        }
    };
    let base_prompt = build_analysis_prompt(&skill_body, &context_path_str, None);
    let prompt = augment_with_out_path(&base_prompt, &out_path.to_string_lossy());

    // 9. Run the session.  Attribute cost to the source story.
    let retry_work = story_work(&analysis.story_id);
    let result = ctx
        .run_agent_with_recovery(
            &ws,
            &user_id,
            &agent.provider,
            retry_model(&agent.model),
            retry_work,
            &cwd,
            &prompt,
            &out_path,
            LENS_TIMEOUT,
            Some(&agent_id),
            &SessionAppearance::analysis(&agent.provider),
            None,
        )
        .await;

    // 10. Record the new session id.
    if let Some(ref sid) = result.session_id {
        if let Err(e) = ctx.product_repo().set_agent_session(&agent_id, sid).await {
            warn!("product_run(retry): set_agent_session {agent_id}: {e}");
        }
    }

    // 11. Parse + persist outcome — mirrors the run_analysis per-agent logic.
    let parsed = result.raw.as_deref().and_then(extract_json_as::<Findings>);

    match (result.errored, parsed) {
        (false, Some(findings)) => {
            let findings_json = findings_json(&findings);
            if let Err(e) = ctx
                .product_repo()
                .set_agent_status(&agent_id, "done", Some(&findings_json), None, true)
                .await
            {
                warn!("product_run(retry): set_agent_status done {agent_id}: {e}");
            }
        }
        _ if result.reason == Some(SUPERSEDED) => {}
        _ => {
            let err = if result.errored {
                "retry: session produced no output (timeout/exit/start failure)".to_string()
            } else {
                format!(
                    "retry: could not parse Findings JSON from agent output (len={})",
                    result.raw.as_deref().map(|s| s.len()).unwrap_or(0)
                )
            };
            warn!("product_run(retry): agent {agent_id} failed: {err}");
            let _ = ctx
                .product_repo()
                .set_agent_status(&agent_id, "error", None, Some(&err), true)
                .await;
            if result.reason != Some("stopped") {
                let _ = ctx.events().send(otto_core::event::Event::Notice {
                    level: "warn".into(),
                    title: format!("Analysis agent failed: {}", agent.name),
                    body: err.clone(),
                });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn preserves_selected_model_and_legacy_empty_default() {
        assert_eq!(
            super::retry_model(" selected-model "),
            Some("selected-model")
        );
        assert_eq!(super::retry_model("  "), None);
    }
}
