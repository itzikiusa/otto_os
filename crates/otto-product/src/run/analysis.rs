//! The analysis fan-out on the shared runner (`otto_agent_run::fanout`,
//! architecture-1-04): every (lens × provider) is a reviewer slot, ONE
//! summarizer session consolidates their findings, and the Rust-side merge of
//! the lens findings stands in when the summarizer fails. This module keeps
//! only what is product-specific: the lens turn and its row, the summarizer
//! row, and what a finished analysis persists.

use std::collections::HashSet;
use std::sync::Mutex;
use std::time::Duration;

use otto_agent_run::fanout::{self, Slot, SlotState};
use otto_core::cancel::CancelRegistryExt;
use otto_core::Id;
use otto_state::{LearningPatch, NewAnalysisAgent, NewEvent, NewLearning, NewQuestion, StoryPatch};
use tokio::sync::OwnedMutexGuard;
use tracing::warn;

use super::context::{skill_body, story_context_md, story_work, trust_providers};
use super::prompts::{
    augment_with_out_path, build_analysis_prompt, build_summarizer_prompt, extract_json_as,
    parse_summary_findings, SummaryFindings,
};
use super::reap::SUMMARIZER_NAME_PREFIX;
use super::scratch::session_cwd;
use super::{
    story_run_cancel_key, AgentSpec, Findings, SessionAppearance, MAX_AGENT_ATTEMPTS, SUPERSEDED,
};
use crate::host::ProductRunHost;

/// One lens turn's budget (also a retried lens's).
pub(super) const LENS_TIMEOUT: Duration = Duration::from_secs(600);
const SUMMARIZER_TIMEOUT: Duration = Duration::from_secs(600);
/// The summarizer skill.
const SUMMARIZER_SKILL: &str = "po-story-overview";
/// The summarizer "failure" a Stop of its row produces: the Stop route already
/// wrote the row, so the runner's failure edge must not overwrite it.
const SUMMARIZER_STOPPED: &str = "the summarizer was stopped";

/// One lens that produced findings (a successful reviewer slot).
struct LensDone {
    name: String,
    provider: String,
    /// The findings JSON persisted on the lens's agent row (canonicalised).
    findings_json: String,
    findings: Findings,
}

/// The canonical findings JSON stored on a lens's agent row.
pub(super) fn findings_json(findings: &Findings) -> String {
    serde_json::to_string(&serde_json::json!({
        "summary": findings.summary,
        "related_repos": findings.related_repos,
        "functionalities": findings.functionalities,
        "integration_points": findings.integration_points,
        "risks": findings.risks,
        "open_questions": findings.open_questions.iter().map(|q| serde_json::json!({"text": q.text, "rationale": q.rationale, "category": q.category})).collect::<Vec<_>>(),
        "suggested_learnings": findings.suggested_learnings.iter().map(|l| serde_json::json!({"kind": l.kind, "title": l.title, "body": l.body})).collect::<Vec<_>>(),
    }))
    .unwrap_or_default()
}

/// Run the full analysis fan-out for a story.  Spawned as a background tokio
/// task by the analyze handler.  Returns `()` — all errors are logged and
/// isolated; no panic propagates.
///
/// Every entry in `specs` is one (lens × provider) and runs as its own real,
/// openable session (claude / codex / agy all honored). A final summarizer
/// session consolidates, dedupes, and resolves conflicts across all lenses;
/// when it fails (or is stopped), the lenses' own findings are merged instead.
/// Deleting the story stops the run before the summarizer and writes nothing.
#[allow(clippy::too_many_arguments)]
pub async fn run_analysis<C: ProductRunHost>(
    ctx: C,
    ws: otto_core::domain::Workspace,
    user_id: otto_core::Id,
    story_id: otto_core::Id,
    analysis_id: otto_core::Id,
    specs: Vec<AgentSpec>,
    summarizer_provider: String,
    cwd: String,
    focus: Option<String>,
) {
    // 1. Load the story + write the context file every lens agent reads.
    let story = match ctx.product_repo().get_story(&story_id).await {
        Ok(s) => s,
        Err(e) => {
            warn!("product_run: get_story {story_id}: {e}");
            let _ = ctx
                .product_repo()
                .set_analysis_status(&analysis_id, "error", Some("failed to load story"), true)
                .await;
            return;
        }
    };
    let context_path = std::env::temp_dir().join(format!("otto-product-{analysis_id}-context.md"));
    let context_md = story_context_md(
        &ctx,
        &story_id,
        &story.title,
        focus.as_deref(),
        "product_run",
    )
    .await;
    if let Err(e) = std::fs::write(&context_path, &context_md) {
        warn!("product_run: write context file: {e}");
    }
    let context_path = context_path.to_string_lossy().to_string();

    // 2. Pre-trust every distinct provider on cwd.
    trust_providers(
        specs
            .iter()
            .map(|s| s.provider.as_str())
            .chain([summarizer_provider.as_str()]),
        &cwd,
    );

    // Deleting the story trips this (`signal_story_cancels`) as well as every
    // lens's own flag: the run then ends without a summarizer or any write.
    let run_cancel = ctx
        .agent_cancels()
        .register(&story_run_cancel_key(&story_id));

    // 3. One reviewer slot per (lens × provider); each owns its agent row.
    let lens_count = specs.len();
    let lens = LensTurn {
        ctx: ctx.clone(),
        ws: ws.clone(),
        user_id: user_id.clone(),
        story_id: story_id.clone(),
        analysis_id: analysis_id.clone(),
        context_path,
    };
    let lenses: Vec<_> = specs
        .into_iter()
        .enumerate()
        // Per-lens session cwd: a story without a real cwd gets a unique temp
        // child per session (codex usage attribution); a real one is kept.
        .map(|(i, spec)| lens.clone().run(i, spec, session_cwd(&cwd)))
        .collect();

    // 4. The summarizer slot: its row opens on `Running`, its failure edge
    //    closes the row (no row is left spinning in "running").
    let summarizer = SummarizerTurn {
        lens,
        provider: summarizer_provider,
        story_title: story.title.clone(),
        cwd,
        row: Mutex::default(),
        reply: Mutex::default(),
    };
    let Some(outcome) = fanout::run(
        lenses,
        || run_cancel.is_cancelled(),
        |done: &[(usize, LensDone)]| {
            let inputs = done
                .iter()
                .map(|(_, l)| (l.name.clone(), l.provider.clone(), l.findings_json.clone()))
                .collect();
            summarizer.run(inputs)
        },
        merge_lens_findings,
        |slot, state| summarizer.on_state(slot, state),
    )
    .await
    else {
        return;
    };

    // 5-8. Questions, learnings, the analysis row, the story stage, the event.
    let partial = outcome.reviewers.len() < lens_count;
    persist_analysis(
        &ctx,
        &ws.id,
        &story,
        &analysis_id,
        lens_count,
        partial,
        outcome,
    )
    .await;
}

/// What every lens (and the summarizer) session of one analysis shares.
#[derive(Clone)]
struct LensTurn<C> {
    ctx: C,
    ws: otto_core::domain::Workspace,
    user_id: Id,
    story_id: Id,
    analysis_id: Id,
    context_path: String,
}

impl<C: ProductRunHost> LensTurn<C> {
    /// One lens as a reviewer slot: create its agent row (status=running),
    /// run the lens prompt as a real session honoring the spec's provider and
    /// model, and persist its findings — or its failure, with a notice for a
    /// genuine one (not a user Stop) so an unattended pipeline notices.
    async fn run(self, i: usize, spec: AgentSpec, cwd: String) -> Result<LensDone, String> {
        let ctx = &self.ctx;
        // The display name disambiguates the lens across providers, like review.
        let agent = ctx
            .product_repo()
            .add_analysis_agent(NewAnalysisAgent {
                analysis_id: self.analysis_id.clone(),
                name: format!("{} \u{00b7} {}", spec.name, spec.provider),
                skill: spec.skill.clone(),
                provider: spec.provider.clone(),
                model: spec.model.clone().unwrap_or_default(),
                status: "running".into(),
                session_id: None,
            })
            .await
            .map_err(|e| {
                warn!("product_run: add_analysis_agent '{}': {e}", spec.name);
                format!("could not create the agent row: {e}")
            })?;
        let agent_id = agent.id;
        let _owner = crate::agent_attempt::drain(&agent_id).await;
        if !ctx
            .product_repo()
            .get_analysis_agent(&agent_id)
            .await
            .is_ok_and(|row| row.status == "running")
        {
            return Err("stopped before it started".into());
        }

        // The lens prompt (context lives in the context file) + write-to-file.
        let out_path =
            std::env::temp_dir().join(format!("otto-product-{}-{i}.json", self.analysis_id));
        let base_prompt =
            build_analysis_prompt(&skill_body(ctx, &spec.skill), &self.context_path, None);
        let prompt = augment_with_out_path(&base_prompt, &out_path.to_string_lossy());
        // Each lens carries its own model (persisted on the row above), so
        // SessionManager injects `--model <name>` where the provider supports it.
        let result = ctx
            .run_agent_with_recovery(
                &self.ws,
                &self.user_id,
                &spec.provider,
                spec.model.as_deref(),
                story_work(&self.story_id),
                &cwd,
                &prompt,
                &out_path,
                LENS_TIMEOUT,
                Some(&agent_id),
                &SessionAppearance::analysis(&spec.provider),
                None,
            )
            .await;
        // Record the session id so the UI can Open the live terminal.
        if let Some(ref sid) = result.session_id {
            if let Err(e) = ctx.product_repo().set_agent_session(&agent_id, sid).await {
                warn!("product_run: set_agent_session {agent_id}: {e}");
            }
        }

        let parsed = result.raw.as_deref().and_then(extract_json_as::<Findings>);
        match (result.errored, parsed) {
            (false, Some(findings)) => {
                let findings_json = findings_json(&findings);
                if let Err(e) = ctx
                    .product_repo()
                    .set_agent_status(&agent_id, "done", Some(&findings_json), None, true)
                    .await
                {
                    warn!("product_run: set_agent_status done {agent_id}: {e}");
                }
                Ok(LensDone {
                    name: spec.name,
                    provider: spec.provider,
                    findings_json,
                    findings,
                })
            }
            // A newer run (Retry after Stop) owns the row now: no status write.
            _ if result.reason == Some(SUPERSEDED) => Err(SUPERSEDED.into()),
            _ => {
                let stopped = result.reason == Some("stopped");
                let err = if result.errored {
                    match result.reason {
                        Some("stopped") => "stopped by user".to_string(),
                        Some(r) => {
                            format!("agent failed after {MAX_AGENT_ATTEMPTS} attempts ({r})")
                        }
                        None => {
                            "session produced no output (timeout/exit/start failure)".to_string()
                        }
                    }
                } else {
                    format!(
                        "could not parse Findings JSON from agent output (len={})",
                        result.raw.as_deref().map(|s| s.len()).unwrap_or(0)
                    )
                };
                warn!(
                    "product_run: lens '{}' ({}) failed: {err}",
                    spec.name, spec.provider
                );
                let _ = ctx
                    .product_repo()
                    .set_agent_status(&agent_id, "error", None, Some(&err), true)
                    .await;
                if !stopped {
                    let _ = ctx.events().send(otto_core::event::Event::Notice {
                        level: "warn".into(),
                        title: format!("Analysis agent failed: {} · {}", spec.name, spec.provider),
                        body: err.clone(),
                    });
                }
                Err(err)
            }
        }
    }
}

/// The summarizer slot: its agent row (opened when the runner starts the
/// slot, held through the final writes like a lens's) and its session.
struct SummarizerTurn<C> {
    lens: LensTurn<C>,
    provider: String,
    story_title: String,
    cwd: String,
    /// The summarizer's agent row and its ownership guard, once opened.
    row: Mutex<Option<(Id, OwnedMutexGuard<()>)>>,
    /// The raw reply the row stores when the turn succeeded.
    reply: Mutex<Option<String>>,
}

impl<C: ProductRunHost> SummarizerTurn<C> {
    fn row_id(&self) -> Option<Id> {
        let row = self.row.lock().unwrap_or_else(|e| e.into_inner());
        row.as_ref().map(|(id, _)| id.clone())
    }

    /// The runner's state edges for the summarizer row: `Running` opens it,
    /// `Done` stores the reply, `Failed` records why (a Stop already did).
    /// Lens rows are written by the lens turns, which hold their findings.
    async fn on_state(&self, slot: Slot, state: SlotState) {
        if slot != Slot::Summarizer {
            return;
        }
        let repo = self.lens.ctx.product_repo();
        match state {
            SlotState::Running => {
                let added = repo
                    .add_analysis_agent(NewAnalysisAgent {
                        analysis_id: self.lens.analysis_id.clone(),
                        name: format!("{SUMMARIZER_NAME_PREFIX}{}", self.provider),
                        skill: SUMMARIZER_SKILL.into(),
                        provider: self.provider.clone(),
                        model: String::new(),
                        status: "running".into(),
                        session_id: None,
                    })
                    .await;
                if let Ok(agent) = added {
                    let owner = crate::agent_attempt::drain(&agent.id).await;
                    *self.row.lock().unwrap_or_else(|e| e.into_inner()) = Some((agent.id, owner));
                }
            }
            SlotState::Done => {
                let reply = self.reply.lock().unwrap_or_else(|e| e.into_inner()).take();
                if let Some(id) = self.row_id() {
                    let _ = repo
                        .set_agent_status(&id, "done", reply.as_deref(), None, true)
                        .await;
                }
            }
            SlotState::Failed(e) if e != SUMMARIZER_STOPPED => {
                if let Some(id) = self.row_id() {
                    let _ = repo
                        .set_agent_status(&id, "error", None, Some(&e), true)
                        .await;
                }
            }
            _ => {}
        }
    }

    /// ONE session consolidates / dedupes / resolves the successful lenses'
    /// findings (`(name, provider, findings_json)` each). No lens output, a
    /// Stop of the row, or an unparseable reply is a failure: the runner
    /// then merges the lens findings in Rust instead.
    async fn run(&self, lenses: Vec<(String, String, String)>) -> Result<SummaryFindings, String> {
        let lens = &self.lens;
        let row = self.row_id();
        if let Some(id) = &row {
            let running = lens
                .ctx
                .product_repo()
                .get_analysis_agent(id)
                .await
                .is_ok_and(|r| r.status == "running");
            if !running {
                return Err(SUMMARIZER_STOPPED.into());
            }
        }
        if lenses.is_empty() {
            return Err("no successful lens outputs to consolidate".into());
        }
        let out_path =
            std::env::temp_dir().join(format!("otto-product-{}-summary.json", lens.analysis_id));
        let base = build_summarizer_prompt(
            &skill_body(&lens.ctx, SUMMARIZER_SKILL),
            &self.story_title,
            &lenses,
        );
        let prompt = augment_with_out_path(&base, &out_path.to_string_lossy());
        let result = lens
            .ctx
            .run_agent_with_recovery(
                &lens.ws,
                &lens.user_id,
                &self.provider,
                None, // no model override for the summarizer; uses provider default
                story_work(&lens.story_id),
                // Own unique session cwd, like the lenses.
                &session_cwd(&self.cwd),
                &prompt,
                &out_path,
                SUMMARIZER_TIMEOUT,
                row.as_ref(),
                &SessionAppearance::analysis(&self.provider),
                None,
            )
            .await;
        if let (Some(id), Some(sid)) = (&row, &result.session_id) {
            let _ = lens.ctx.product_repo().set_agent_session(id, sid).await;
        }
        let parsed = result.raw.as_deref().and_then(parse_summary_findings);
        let parsed = parsed.ok_or_else(|| "summarizer produced no parseable JSON".to_string())?;
        *self.reply.lock().unwrap_or_else(|e| e.into_inner()) = result.raw;
        Ok(parsed)
    }
}

/// The lens summaries joined, or a placeholder when none has one.
fn lens_summaries(done: &[(usize, LensDone)]) -> String {
    let joined = done
        .iter()
        .map(|(_, l)| l.findings.summary.clone())
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join(" | ");
    if joined.is_empty() {
        "(no agent summaries available)".to_string()
    } else {
        joined
    }
}

/// The deterministic fallback: every lens's questions and suggested
/// learnings (deduped when they are persisted) and their joined summaries.
fn merge_lens_findings(done: &[(usize, LensDone)]) -> SummaryFindings {
    let findings = || done.iter().map(|(_, l)| &l.findings);
    SummaryFindings {
        summary: lens_summaries(done),
        questions: findings()
            .flat_map(|f| f.open_questions.iter().cloned())
            .collect(),
        suggested_learnings: findings()
            .flat_map(|f| f.suggested_learnings.iter().cloned())
            .collect(),
        ..Default::default()
    }
}

/// Persist a finished analysis: new questions (deduped against the story's
/// existing ones), suggested learnings (inactive until reviewed), the final
/// status + summary, the story stage, and the timeline event.
async fn persist_analysis<C: ProductRunHost>(
    ctx: &C,
    ws_id: &Id,
    story: &otto_state::ProductStory,
    analysis_id: &Id,
    lens_count: usize,
    partial: bool,
    outcome: fanout::Outcome<LensDone, SummaryFindings>,
) {
    let summary = &outcome.summary.value;
    let question_count = create_questions(ctx, story, analysis_id, summary).await;
    create_suggested_learnings(ctx, story, summary).await;

    // The summarizer's summary, else (an empty one) the lenses' own.
    let final_summary = if summary.summary.trim().is_empty() {
        lens_summaries(&outcome.reviewers)
    } else {
        summary.summary.clone()
    };
    let final_status = if partial { "partial" } else { "done" };
    if let Err(e) = ctx
        .product_repo()
        .set_analysis_status(analysis_id, final_status, Some(&final_summary), true)
        .await
    {
        warn!("product_run: set_analysis_status: {e}");
    }
    // Notify subscribed UI tabs so they can refresh without waiting for the next poll.
    let _ = ctx.events().send(otto_core::event::Event::ProductChanged {
        workspace_id: ws_id.clone(),
        story_id: story.id.clone(),
        section: "analysis".into(),
        status: final_status.into(),
    });

    if let Err(e) = ctx
        .product_repo()
        .update_story(
            &story.id,
            StoryPatch {
                stage: Some("analyzed".into()),
                ..Default::default()
            },
        )
        .await
    {
        warn!("product_run: update_story stage: {e}");
    }

    let summary_event =
        format!("analysis completed: {lens_count} lens agent(s), {question_count} new question(s)");
    if let Err(e) = ctx
        .product_repo()
        .add_event(NewEvent {
            story_id: story.id.clone(),
            section: "analysis".into(),
            kind: "analyzed".into(),
            summary: summary_event,
            actor_id: None,
            meta_json: None,
        })
        .await
    {
        warn!("product_run: add_event: {e}");
    }
}

/// Create the consolidated questions the story does not already have (case-
/// and whitespace-insensitive); returns how many were created.
async fn create_questions<C: ProductRunHost>(
    ctx: &C,
    story: &otto_state::ProductStory,
    analysis_id: &Id,
    summary: &SummaryFindings,
) -> usize {
    let existing = ctx
        .product_repo()
        .list_questions(&story.id)
        .await
        .unwrap_or_default();
    let mut seen: HashSet<String> = existing
        .iter()
        .map(|q| q.text.trim().to_lowercase())
        .collect();
    let mut created = 0usize;
    for q in &summary.questions {
        let norm = q.text.trim().to_lowercase();
        if norm.is_empty() || !seen.insert(norm) {
            continue;
        }
        let new = NewQuestion {
            story_id: story.id.clone(),
            analysis_id: Some(analysis_id.clone()),
            text: q.text.clone(),
            rationale: q.rationale.clone(),
            category: q.category.clone(),
            created_by: story.created_by.clone(),
        };
        match ctx.product_repo().create_question(new).await {
            Ok(_) => created += 1,
            Err(e) => warn!("product_run: create_question: {e}"),
        }
    }
    created
}

/// Suggested learnings become INACTIVE product learnings: a human reviews
/// them before any agent uses them.
async fn create_suggested_learnings<C: ProductRunHost>(
    ctx: &C,
    story: &otto_state::ProductStory,
    summary: &SummaryFindings,
) {
    for l in &summary.suggested_learnings {
        if l.title.trim().is_empty() {
            continue;
        }
        let created = ctx
            .product_repo()
            .create_learning(NewLearning {
                workspace_id: story.workspace_id.clone(),
                kind: l.kind.clone(),
                title: l.title.clone(),
                body: l.body.clone(),
                tags: String::new(),
                refs_json: "[]".into(),
                source_story_id: Some(story.id.clone()),
                created_by: story.created_by.clone(),
            })
            .await;
        match created {
            Ok(learning) => {
                // create_learning hardcodes active=1; flip it off.
                let inactive = LearningPatch {
                    kind: None,
                    title: None,
                    body: None,
                    tags: None,
                    refs_json: None,
                    active: Some(false),
                };
                if let Err(e) = ctx
                    .product_repo()
                    .update_learning(&learning.id, inactive)
                    .await
                {
                    warn!("product_run: deactivate learning {}: {e}", learning.id);
                }
            }
            Err(e) => warn!("product_run: create_learning '{}': {e}", l.title),
        }
    }
}
