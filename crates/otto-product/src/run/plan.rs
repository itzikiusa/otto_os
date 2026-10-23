//! Implementation-plan generation on the shared fan-out runner
//! (`otto_agent_run::fanout`, architecture-1-04): one VISIBLE planning
//! session per provider, and a plan summarizer only when there is more than
//! one candidate to consolidate.

use std::sync::{Arc, Mutex};
use std::time::Duration;

use otto_agent_run::fanout;
use otto_core::cancel::CancelRegistryExt;
use otto_core::Id;
use tracing::warn;

use super::context::{
    append_answers_and_summary, skill_body, story_context_md, story_work, trust_providers,
};
use super::prompts::{
    augment_with_out_path, build_plan_prompt, build_plan_summarizer_prompt, extract_json_as,
    PLAN_AUTONOMY_DIRECTIVE,
};
use super::scratch::session_cwd;
use super::{story_run_cancel_key, SessionAppearance};
use crate::host::ProductRunHost;

/// Every planning (and plan-summarizer) turn's budget.
const PLAN_TIMEOUT: Duration = Duration::from_secs(300);

/// Parsed response from the task-breakdown agent.
#[derive(serde::Deserialize)]
pub(super) struct PlanFindings {
    pub(super) plan_markdown: String,
}

/// The non-empty `plan_markdown` of an agent reply, if any.
fn parse_plan(raw: Option<&str>) -> Option<String> {
    raw.and_then(extract_json_as::<PlanFindings>)
        .map(|f| f.plan_markdown)
        .filter(|m| !m.trim().is_empty())
}

/// Run the implementation-plan generation for a story. Spawned as a background
/// tokio task by the generate-plan handler. Returns `()` — all errors are
/// logged; no panic propagates.
///
/// MULTI-AGENT, on the same runner as `run_analysis`: one planning agent runs per
/// provider in `providers` (each as its own real, openable, VISIBLE session so the
/// user can watch them), and — when >1 planner produced a plan — a summarizer
/// consolidates the candidate plans into ONE; a single candidate is the plan,
/// and a failed summarizer falls back to the first candidate. A `plan_run` event
/// surfaces the live session ids so the UI can tile them side-by-side. When
/// `interactive` is false (the default) every planning prompt is prefixed with an
/// autonomy directive so agents work unattended and never block on questions.
/// Deleting the story stops the run without writing a result.
///
/// The shared context file is enriched with answered questions, the latest
/// analysis summary, AND the latest run's approved test cases.
#[allow(clippy::too_many_arguments)]
pub async fn run_generate_plan<C: ProductRunHost>(
    ctx: C,
    ws: otto_core::domain::Workspace,
    user_id: otto_core::Id,
    story_id: otto_core::Id,
    providers: Vec<String>,
    summarizer_provider: String,
    interactive: bool,
    // Optional model override, threaded into each planning (and summarizer)
    // session so SessionManager injects `--model <name>` for providers that
    // support it (claude/codex); empty is filtered inside run_agent_with_recovery.
    model: Option<String>,
    cwd: String,
    focus: Option<String>,
) {
    // Normalize the provider list: drop blanks; if somehow empty, the handler
    // should have guaranteed at least one — but guard anyway.
    let providers: Vec<String> = providers
        .into_iter()
        .filter(|p| !p.trim().is_empty())
        .collect();
    if providers.is_empty() {
        warn!("product_run(plan): no providers resolved; nothing to do");
        return;
    }

    // 1. Load story
    let story = match ctx.product_repo().get_story(&story_id).await {
        Ok(s) => s,
        Err(e) => {
            warn!("product_run(plan): get_story {story_id}: {e}");
            return;
        }
    };

    // 2. Resolve skill body + build the context file (Jira details + answered
    //    Q&A + analysis summary + approved test cases + learnings).
    let skill_body = skill_body(&ctx, "story-task-breakdown");
    let plan_id = otto_core::new_id();
    let context_path = std::env::temp_dir().join(format!("otto-product-plan-{plan_id}-context.md"));
    {
        let mut context_md = story_context_md(
            &ctx,
            &story_id,
            &story.title,
            focus.as_deref(),
            "product_run(plan)",
        )
        .await;
        append_answers_and_summary(&ctx, &story_id, &mut context_md).await;
        append_approved_tests(&ctx, &story_id, &mut context_md).await;
        if let Err(e) = std::fs::write(&context_path, &context_md) {
            warn!("product_run(plan): write context file: {e}");
        }
    }
    let context_path = context_path.to_string_lossy().to_string();

    // 3. Pre-trust every distinct provider on the base cwd (mirror analysis).
    trust_providers(
        providers
            .iter()
            .map(String::as_str)
            .chain([summarizer_provider.as_str()]),
        &cwd,
    );

    // Deleting the story trips this (`signal_story_cancels`): the planners'
    // own sessions stop through their story-keyed flags, and the run ends
    // without a summarizer or a result write.
    let run_cancel = ctx
        .agent_cancels()
        .register(&story_run_cancel_key(&story_id));
    let sessions = PlanSessions {
        ctx: ctx.clone(),
        ws_id: ws.id.clone(),
        story_id: story_id.clone(),
        interactive,
        ids: Arc::default(),
    };
    let turn = PlanTurn {
        ctx,
        ws,
        user_id,
        model,
        work: story_work(&story_id),
        interactive,
        sessions,
    };

    // 4. Planners as the runner's reviewers; the summarizer only from two
    //    candidates up (below that, the fallback: the one candidate, or none).
    let planners: Vec<_> = providers
        .into_iter()
        .enumerate()
        .map(|(i, provider)| {
            let turn = turn.clone();
            let out_path =
                std::env::temp_dir().join(format!("otto-product-plan-{plan_id}-{i}.json"));
            let prompt = augment_with_out_path(
                &build_plan_prompt(&skill_body, &context_path),
                &out_path.to_string_lossy(),
            );
            let cwd = session_cwd(&cwd);
            async move {
                let title = format!("Plan: {provider}");
                parse_plan(
                    turn.run(&provider, &title, &cwd, prompt, &out_path)
                        .await
                        .as_deref(),
                )
                .map(|plan| (provider, plan))
                .ok_or_else(|| "the planner produced no usable plan".to_string())
            }
        })
        .collect();
    let summarize = |done: &[(usize, (String, String))]| {
        let candidates: Vec<(String, String)> = done.iter().map(|(_, c)| c.clone()).collect();
        let out_path =
            std::env::temp_dir().join(format!("otto-product-plan-{plan_id}-summary.json"));
        let base =
            build_plan_summarizer_prompt(&skill_body, &story.title, &context_path, &candidates);
        let prompt = augment_with_out_path(&base, &out_path.to_string_lossy());
        let (turn, provider, cwd) = (&turn, &summarizer_provider, session_cwd(&cwd));
        async move {
            let title = format!("Plan summarizer: {provider}");
            parse_plan(
                turn.run(provider, &title, &cwd, prompt, &out_path)
                    .await
                    .as_deref(),
            )
            .map(Some)
            .ok_or_else(|| "the plan summarizer produced no usable plan".to_string())
        }
    };
    // Fall back to the first candidate plan — better a single plan than none.
    let first_candidate =
        |done: &[(usize, (String, String))]| done.first().map(|(_, (_, plan))| plan.clone());
    let Some(outcome) = fanout::run_with_quorum(
        planners,
        || run_cancel.is_cancelled(),
        2,
        summarize,
        first_candidate,
        |_, _| std::future::ready(()),
    )
    .await
    else {
        return;
    };
    if let Some(e) = &outcome.summary.error {
        warn!("product_run(plan): {e}; keeping the first candidate plan");
    }
    persist_plan(&turn.ctx, &turn.ws.id, &story, outcome.summary.value).await;
}

/// The live session ids of one plan run. Each planner (and the summarizer)
/// reports its session the moment it exists; every new one re-emits a
/// `plan_run` event so the Plan tab can tile the sessions live.
#[derive(Clone)]
struct PlanSessions<C> {
    ctx: C,
    ws_id: Id,
    story_id: Id,
    interactive: bool,
    ids: Arc<Mutex<Vec<Id>>>,
}

impl<C: ProductRunHost> PlanSessions<C> {
    fn record(&self, sid: &Id) {
        let ids = {
            let mut guard = self.ids.lock().unwrap_or_else(|e| e.into_inner());
            if !guard.contains(sid) {
                guard.push(sid.clone());
            }
            guard.clone()
        };
        let _ = self.ctx.events().send(otto_core::event::Event::PlanRun {
            workspace_id: self.ws_id.clone(),
            story_id: self.story_id.clone(),
            session_ids: ids,
            interactive: self.interactive,
        });
    }
}

/// What every plan session shares: who runs it, where it is attributed, and
/// the live-tiling hook.
#[derive(Clone)]
struct PlanTurn<C> {
    ctx: C,
    ws: otto_core::domain::Workspace,
    user_id: Id,
    model: Option<String>,
    work: Option<serde_json::Value>,
    interactive: bool,
    sessions: PlanSessions<C>,
}

impl<C: ProductRunHost> PlanTurn<C> {
    /// Run one VISIBLE plan session (`product-plan` sessions tile in the
    /// Agents view) and return its raw reply. Non-interactive runs (the
    /// default) prepend the autonomy directive so it never blocks on input.
    async fn run(
        &self,
        provider: &str,
        title: &str,
        cwd: &str,
        prompt: String,
        out_path: &std::path::Path,
    ) -> Option<String> {
        let prompt = if self.interactive {
            prompt
        } else {
            format!("{PLAN_AUTONOMY_DIRECTIVE}{prompt}")
        };
        let on_session = |sid: &Id| self.sessions.record(sid);
        self.ctx
            .run_agent_with_recovery(
                &self.ws,
                &self.user_id,
                provider,
                self.model.as_deref(),
                self.work.clone(),
                cwd,
                &prompt,
                out_path,
                PLAN_TIMEOUT,
                None,
                &SessionAppearance {
                    title: title.to_string(),
                    source: "product-plan",
                },
                Some(&on_session),
            )
            .await
            .raw
    }
}

/// Append the latest test run's APPROVED cases to the plan context (mirrors
/// inject bundle section 4).
async fn append_approved_tests<C: ProductRunHost>(ctx: &C, story_id: &Id, context_md: &mut String) {
    // Append approved test cases from the latest run (mirrors inject bundle
    // section 4: list_testcase_runs → first → list_testcases → approved).
    let approved: Vec<otto_state::ProductTestcase> = {
        let runs = ctx
            .product_repo()
            .list_testcase_runs(story_id)
            .await
            .unwrap_or_default();
        match runs.first() {
            Some(run) => ctx
                .product_repo()
                .list_testcases(&run.id)
                .await
                .unwrap_or_default()
                .into_iter()
                .filter(|c| c.status == "approved")
                .collect(),
            None => Vec::new(),
        }
    };
    if !approved.is_empty() {
        context_md.push_str("\n## Approved Test Cases\n\n");
        for c in &approved {
            context_md.push_str(&format!(
                "### {} ({}, {})\n",
                c.title, c.category, c.priority
            ));
            if let Ok(steps) = serde_json::from_str::<serde_json::Value>(&c.steps_json) {
                if let Some(pre) = steps.get("preconditions").and_then(|v| v.as_array()) {
                    if !pre.is_empty() {
                        context_md.push_str("**Preconditions:**\n");
                        for p in pre {
                            if let Some(s) = p.as_str() {
                                context_md.push_str(&format!("- {s}\n"));
                            }
                        }
                    }
                }
                if let Some(st) = steps.get("steps").and_then(|v| v.as_array()) {
                    if !st.is_empty() {
                        context_md.push_str("**Steps:**\n");
                        for (i, s) in st.iter().enumerate() {
                            if let Some(s) = s.as_str() {
                                context_md.push_str(&format!("{}. {s}\n", i + 1));
                            }
                        }
                    }
                }
                if let Some(exp) = steps.get("expected").and_then(|v| v.as_str()) {
                    if !exp.trim().is_empty() {
                        context_md.push_str(&format!("**Expected:** {exp}\n"));
                    }
                }
            }
            context_md.push('\n');
        }
    }
}

/// Persist the final (consolidated, or single) plan as a new kind="plan"
/// version, or record why there is none.
async fn persist_plan<C: ProductRunHost>(
    ctx: &C,
    ws_id: &Id,
    story: &otto_state::ProductStory,
    final_plan: Option<String>,
) {
    match final_plan {
        Some(plan_markdown) => {
            if let Err(e) = ctx
                .product_repo()
                .add_version(otto_state::NewVersion {
                    story_id: story.id.clone(),
                    kind: "plan".into(),
                    title: "Implementation Plan".into(),
                    body_md: plan_markdown,
                    raw_json: None,
                    change_notes: None,
                    created_by: story.created_by.clone(),
                })
                .await
            {
                warn!("product_run(plan): add_version: {e}");
            }

            // Update story stage to "planned".
            if let Err(e) = ctx
                .product_repo()
                .update_story(
                    &story.id,
                    otto_state::StoryPatch {
                        stage: Some("planned".into()),
                        ..Default::default()
                    },
                )
                .await
            {
                warn!("product_run(plan): update_story stage: {e}");
            }

            // Add event.
            if let Err(e) = ctx
                .product_repo()
                .add_event(otto_state::NewEvent {
                    story_id: story.id.clone(),
                    section: "plan".into(),
                    kind: "plan_generated".into(),
                    summary: "Implementation plan generated".into(),
                    actor_id: None,
                    meta_json: None,
                })
                .await
            {
                warn!("product_run(plan): add_event: {e}");
            }
            let _ = ctx.events().send(otto_core::event::Event::ProductChanged {
                workspace_id: ws_id.clone(),
                story_id: story.id.clone(),
                section: "plan".into(),
                status: "done".into(),
            });
        }
        None => {
            let reason = "plan generation produced no usable plan (all planning agents \
                          failed or returned unparseable/empty output)"
                .to_string();
            warn!("product_run(plan): {reason}");
            let _ = ctx
                .product_repo()
                .add_event(otto_state::NewEvent {
                    story_id: story.id.clone(),
                    section: "plan".into(),
                    kind: "plan_error".into(),
                    summary: reason,
                    actor_id: None,
                    meta_json: None,
                })
                .await;
            let _ = ctx.events().send(otto_core::event::Event::ProductChanged {
                workspace_id: ws_id.clone(),
                story_id: story.id.clone(),
                section: "plan".into(),
                status: "error".into(),
            });
        }
    }
}
