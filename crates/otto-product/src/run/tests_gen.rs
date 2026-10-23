//! Test-case generation: ONE agent (story-test-cases) drafts a test run. A
//! single turn, so it does not fan out.

use std::time::Duration;

use tracing::warn;

use super::context::{append_answers_and_summary, skill_body, story_context_md, story_work};
use super::prompts::{augment_with_out_path, build_tests_prompt, extract_json_as};
use super::scratch::{session_cwd, trust_session_cwd};
use super::SessionAppearance;
use crate::host::ProductRunHost;

// ---------------------------------------------------------------------------
// Local deserialization structs for the test-generation response.
// ---------------------------------------------------------------------------

#[derive(serde::Deserialize)]
struct GenTestcases {
    testcases: Vec<GenTestcase>,
}

#[derive(serde::Deserialize)]
struct GenTestcase {
    title: String,
    category: String,
    #[serde(default = "default_priority")]
    priority: String,
    #[serde(default)]
    preconditions: Vec<String>,
    #[serde(default)]
    steps: Vec<String>,
    #[serde(default)]
    expected: String,
}

fn default_priority() -> String {
    "medium".to_string()
}

// ---------------------------------------------------------------------------
// run_generate_tests — spawned as a background task by the generate handler
// ---------------------------------------------------------------------------

/// Run the test-case generation for a story.  Spawned as a background tokio
/// task by the generate handler.  Returns `()` — all errors are logged; no
/// panic propagates.
///
/// Provider-honoring: runs via `run_lens_session` (not `orchestrator.run_agent`).
/// Context is written to a temp file.
#[allow(clippy::too_many_arguments)]
pub async fn run_generate_tests<C: ProductRunHost>(
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
    // the shared temp dir; give this test-gen session its own unique temp subdir so
    // the codex usage tailer attributes it 1:1 by cwd. A real story cwd passes
    // through unchanged.
    let cwd = session_cwd(&cwd);

    // 1. Load story
    let story = match ctx.product_repo().get_story(&story_id).await {
        Ok(s) => s,
        Err(e) => {
            warn!("product_run(generate_tests): get_story {story_id}: {e}");
            return;
        }
    };

    // 2. Resolve skill body: story-test-cases
    let skill_name = "story-test-cases";
    let skill_body = skill_body(&ctx, skill_name);

    // 3. Build context file (enriched with Jira details, answered Q&A, learnings).
    let gen_id = otto_core::new_id();
    let context_path = std::env::temp_dir().join(format!("otto-product-tests-{gen_id}-context.md"));

    {
        let mut context_md = story_context_md(
            &ctx,
            &story_id,
            &story.title,
            focus.as_deref(),
            "product_run(generate_tests)",
        )
        .await;
        append_answers_and_summary(&ctx, &story_id, &mut context_md).await;
        if let Err(e) = std::fs::write(&context_path, &context_md) {
            warn!("product_run(generate_tests): write context file: {e}");
        }
    }

    // 4. Build prompt (references the context file) + out_path for JSON output.
    let out_path = std::env::temp_dir().join(format!("otto-product-tests-{gen_id}.json"));
    let base_prompt = build_tests_prompt(&skill_body, &context_path.to_string_lossy());
    let prompt = augment_with_out_path(&base_prompt, &out_path.to_string_lossy());

    // 5. Pre-trust provider.
    trust_session_cwd(&provider, &cwd);

    // 6. Run as a provider-honoring session.  The requested model (if any) is
    //    threaded through SessionManager's spawn path for claude/codex.
    //    Stamp the session with a WorkRef so cost is attributed to this story.
    let tests_work = story_work(&story_id);
    let result = ctx
        .run_agent_with_recovery(
            &ws,
            &user_id,
            &provider,
            model.as_deref(),
            tests_work,
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
    let parsed_opt = result
        .raw
        .as_deref()
        .and_then(extract_json_as::<GenTestcases>);

    match parsed_opt {
        Some(parsed) => {
            // 8. Create testcase run
            let run = match ctx
                .product_repo()
                .create_testcase_run(&story_id, &story.created_by)
                .await
            {
                Ok(r) => r,
                Err(e) => {
                    warn!("product_run(generate_tests): create_testcase_run: {e}");
                    let _ = ctx
                        .product_repo()
                        .add_event(otto_state::NewEvent {
                            story_id: story_id.clone(),
                            section: "tests".into(),
                            kind: "tests_error".into(),
                            summary: format!("test generation failed (db): {e}"),
                            actor_id: None,
                            meta_json: None,
                        })
                        .await;
                    return;
                }
            };

            // 9. Insert each test case
            let tc_count = parsed.testcases.len();
            for (i, tc) in parsed.testcases.into_iter().enumerate() {
                let steps_json = serde_json::to_string(&serde_json::json!({
                    "preconditions": tc.preconditions,
                    "steps": tc.steps,
                    "expected": tc.expected,
                }))
                .unwrap_or_else(|_| "{}".into());

                if let Err(e) = ctx
                    .product_repo()
                    .add_testcase(otto_state::NewTestcase {
                        run_id: run.id.clone(),
                        story_id: story_id.clone(),
                        title: tc.title,
                        category: tc.category,
                        priority: tc.priority,
                        steps_json,
                        order_idx: i as i64,
                    })
                    .await
                {
                    warn!("product_run(generate_tests): add_testcase[{i}]: {e}");
                }
            }

            // 10. Update story stage
            if let Err(e) = ctx
                .product_repo()
                .update_story(
                    &story_id,
                    otto_state::StoryPatch {
                        stage: Some("tests_drafted".into()),
                        ..Default::default()
                    },
                )
                .await
            {
                warn!("product_run(generate_tests): update_story stage: {e}");
            }

            // 11. Add event
            if let Err(e) = ctx
                .product_repo()
                .add_event(otto_state::NewEvent {
                    story_id: story_id.clone(),
                    section: "tests".into(),
                    kind: "tests_drafted".into(),
                    summary: format!("test cases generated: {tc_count} case(s) drafted"),
                    actor_id: None,
                    meta_json: None,
                })
                .await
            {
                warn!("product_run(generate_tests): add_event: {e}");
            }
            let _ = ctx.events().send(otto_core::event::Event::ProductChanged {
                workspace_id: ws.id.clone(),
                story_id: story_id.clone(),
                section: "testcases".into(),
                status: "done".into(),
            });
        }
        None => {
            let reason = if result.errored {
                "test generation session failed (timeout/exit/start failure)".to_string()
            } else {
                format!(
                    "test generation agent returned unparseable output (len={})",
                    result.raw.as_deref().map(|s| s.len()).unwrap_or(0)
                )
            };
            warn!("product_run(generate_tests): {reason}");
            let _ = ctx
                .product_repo()
                .add_event(otto_state::NewEvent {
                    story_id: story_id.clone(),
                    section: "tests".into(),
                    kind: "tests_error".into(),
                    summary: reason,
                    actor_id: None,
                    meta_json: None,
                })
                .await;
            let _ = ctx.events().send(otto_core::event::Event::ProductChanged {
                workspace_id: ws.id.clone(),
                story_id: story_id.clone(),
                section: "testcases".into(),
                status: "error".into(),
            });
        }
    }
}
