//! The story rewrite: ONE writer agent (jira-story-writer / rfc-writer)
//! suggests a new version. A single turn, so it does not fan out.

use std::time::Duration;

use tracing::warn;

use super::context::{append_answers_and_summary, skill_body, story_context_md, story_work};
use super::prompts::{augment_with_out_path, build_rewrite_prompt, extract_json_as};
use super::scratch::{session_cwd, trust_session_cwd};
use super::SessionAppearance;
use crate::host::ProductRunHost;

// ---------------------------------------------------------------------------
// run_rewrite — spawned as a background task by the rewrite handler
// ---------------------------------------------------------------------------

/// Parsed response from the writer agent.
#[derive(serde::Deserialize)]
struct RewriteFindings {
    title: String,
    body_markdown: String,
    change_notes: String,
}

/// Run the rewrite for a story.  Spawned as a background tokio task.
/// Returns `()` — all errors are logged; no panic propagates.
///
/// Provider-honoring: runs via `run_lens_session` (not `orchestrator.run_agent`),
/// so claude / codex / agy are all honored.  Context is written to a temp file.
#[allow(clippy::too_many_arguments)]
pub async fn run_rewrite<C: ProductRunHost>(
    ctx: C,
    ws: otto_core::domain::Workspace,
    user_id: otto_core::Id,
    story_id: otto_core::Id,
    provider: String,
    model: Option<String>,
    cwd: String,
    focus: Option<String>,
) {
    // Per-invocation session cwd: when there's no real story cwd we fell back to
    // the shared temp dir; give this rewrite session its own unique temp subdir so
    // the codex usage tailer attributes it 1:1 by cwd (re-running rewrite for the
    // same story never collides). A real story cwd passes through unchanged.
    let cwd = session_cwd(&cwd);

    // 1. Load story
    let story = match ctx.product_repo().get_story(&story_id).await {
        Ok(s) => s,
        Err(e) => {
            warn!("product_run(rewrite): get_story {story_id}: {e}");
            return;
        }
    };

    // 2. Pick writer skill: jira → jira-story-writer, else → rfc-writer
    let writer_skill_name = if story.source_kind == "jira" {
        "jira-story-writer"
    } else {
        "rfc-writer"
    };

    let skill_body = skill_body(&ctx, writer_skill_name);

    // 3. Build context file (enriched with Jira details, answered Q&A, learnings).
    //    The rewrite context also includes answered questions and analysis summary
    //    (build_agent_context provides story body + Jira + learnings; we enrich
    //    further here with the answered questions and analysis summary).
    let rewrite_id = otto_core::new_id();
    let context_path =
        std::env::temp_dir().join(format!("otto-product-rewrite-{rewrite_id}-context.md"));

    {
        let mut context_md = story_context_md(
            &ctx,
            &story_id,
            &story.title,
            focus.as_deref(),
            "product_run(rewrite)",
        )
        .await;
        append_answers_and_summary(&ctx, &story_id, &mut context_md).await;
        if let Err(e) = std::fs::write(&context_path, &context_md) {
            warn!("product_run(rewrite): write context file: {e}");
        }
    }

    // 4. Build prompt (references the context file) + out_path for JSON output.
    let out_path = std::env::temp_dir().join(format!("otto-product-rewrite-{rewrite_id}.json"));
    let base_prompt = build_rewrite_prompt(&skill_body, &context_path.to_string_lossy());
    let prompt = augment_with_out_path(&base_prompt, &out_path.to_string_lossy());

    // 5. Pre-trust provider.
    trust_session_cwd(&provider, &cwd);

    // 6. Run as a provider-honoring session.  The requested model (if any) is
    //    injected by run_agent_with_recovery → run_lens_session → SessionManager
    //    for providers that support `--model` (claude/codex). For agy/shell the
    //    model value is stored in meta for attribution only.
    //    Stamp the session with a WorkRef so cost is attributed to this story.
    let rewrite_work = story_work(&story_id);
    let result = ctx
        .run_agent_with_recovery(
            &ws,
            &user_id,
            &provider,
            model.as_deref(),
            rewrite_work,
            &cwd,
            &prompt,
            &out_path,
            Duration::from_secs(300),
            None,
            &SessionAppearance::analysis(&provider),
            None,
        )
        .await;

    // 7. Parse + persist the outcome.
    let output_opt = result
        .raw
        .as_deref()
        .and_then(extract_json_as::<RewriteFindings>);

    match output_opt {
        Some(findings) => {
            // 8. Persist suggested version
            if let Err(e) = ctx
                .product_repo()
                .add_version(otto_state::NewVersion {
                    story_id: story_id.clone(),
                    kind: "suggested".into(),
                    title: findings.title,
                    body_md: findings.body_markdown,
                    raw_json: None,
                    change_notes: Some(findings.change_notes.clone()),
                    created_by: story.created_by.clone(),
                })
                .await
            {
                warn!("product_run(rewrite): add_version: {e}");
            }

            // 9. Update story stage to "refined"
            if let Err(e) = ctx
                .product_repo()
                .update_story(
                    &story_id,
                    otto_state::StoryPatch {
                        stage: Some("refined".into()),
                        ..Default::default()
                    },
                )
                .await
            {
                warn!("product_run(rewrite): update_story stage: {e}");
            }

            // 10. Add event
            if let Err(e) = ctx
                .product_repo()
                .add_event(otto_state::NewEvent {
                    story_id: story_id.clone(),
                    section: "rewrite".into(),
                    kind: "rewrite_suggested".into(),
                    summary: format!(
                        "Rewrite suggested using skill '{writer_skill_name}'; change_notes: {}",
                        findings.change_notes
                    ),
                    actor_id: None,
                    meta_json: None,
                })
                .await
            {
                warn!("product_run(rewrite): add_event: {e}");
            }
            // Notify UI so the Rewrite tab can refresh without waiting for a poll.
            let _ = ctx.events().send(otto_core::event::Event::ProductChanged {
                workspace_id: ws.id.clone(),
                story_id: story_id.clone(),
                section: "rewrite".into(),
                status: "done".into(),
            });
        }
        None => {
            let reason = if result.errored {
                "rewrite agent session failed (timeout/exit/start failure)".to_string()
            } else {
                format!(
                    "rewrite agent returned unparseable output (len={})",
                    result.raw.as_deref().map(|s| s.len()).unwrap_or(0)
                )
            };
            warn!("product_run(rewrite): {reason}");
            let _ = ctx
                .product_repo()
                .add_event(otto_state::NewEvent {
                    story_id: story_id.clone(),
                    section: "rewrite".into(),
                    kind: "rewrite_error".into(),
                    summary: reason,
                    actor_id: None,
                    meta_json: None,
                })
                .await;
            let _ = ctx.events().send(otto_core::event::Event::ProductChanged {
                workspace_id: ws.id.clone(),
                story_id: story_id.clone(),
                section: "rewrite".into(),
                status: "error".into(),
            });
        }
    }
}
