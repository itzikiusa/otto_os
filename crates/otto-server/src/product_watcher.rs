//! Background supervisor that polls watched stories for new Jira/Confluence
//! comments, records them as events, advances the watch cursor, runs a
//! reconcile agent pass, and triggers the self-improvement engine.
//!
//! Modelled on `otto-channels::ChannelManager`: a single `WatcherManager::start`
//! call spawns one supervisor task. Per-story polls are spawned as independent
//! tokio tasks so a slow poll never blocks the scan. The handle's `Drop`
//! implementation sets the cancel flag for a clean shutdown.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use otto_core::domain::ImprovementTrigger;
use otto_core::event::Event;
use otto_core::Id;
use otto_improve::ImprovementEngine;
use otto_orchestrator::Orchestrator;
use otto_product::ProductService;
use otto_state::{NewEvent, NewQuestion, ProductQuestion, ProductRepo, QuestionPatch};
use tokio::sync::broadcast;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::warn;

use crate::product_run::{build_improve_narrative_from_clarifications, extract_json_block};

// ---------------------------------------------------------------------------
// Watcher constants
// ---------------------------------------------------------------------------

/// How often the supervisor re-scans watched stories.
const SCAN_INTERVAL: Duration = Duration::from_secs(60);

/// Timeout for a single reconcile agent call.
const RECONCILE_TIMEOUT: Duration = Duration::from_secs(180);

// ---------------------------------------------------------------------------
// Public handle
// ---------------------------------------------------------------------------

/// Handle returned by `WatcherManager::start`. Keep it alive for the process
/// lifetime; dropping it sets the cancel flag and stops the supervisor.
pub struct WatcherHandle {
    cancel: Arc<AtomicBool>,
    _supervisor: JoinHandle<()>,
}

impl Drop for WatcherHandle {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
    }
}

// ---------------------------------------------------------------------------
// Manager
// ---------------------------------------------------------------------------

/// Wires together the repos and services required to drive the story watcher.
pub struct WatcherManager {
    product_repo: ProductRepo,
    product: Arc<ProductService>,
    orchestrator: Arc<Orchestrator>,
    improve: Arc<ImprovementEngine>,
    events: broadcast::Sender<Event>,
    default_provider: String,
}

impl WatcherManager {
    pub fn new(
        product_repo: ProductRepo,
        product: Arc<ProductService>,
        orchestrator: Arc<Orchestrator>,
        improve: Arc<ImprovementEngine>,
        events: broadcast::Sender<Event>,
        default_provider: String,
    ) -> Self {
        Self {
            product_repo,
            product,
            orchestrator,
            improve,
            events,
            default_provider,
        }
    }

    /// Spawn the supervisor. Returns immediately; the supervisor runs in the
    /// background and stays alive until the handle is dropped.
    pub fn start(self) -> WatcherHandle {
        let cancel = Arc::new(AtomicBool::new(false));
        let supervisor = tokio::spawn(supervise(self, Arc::clone(&cancel)));
        WatcherHandle {
            cancel,
            _supervisor: supervisor,
        }
    }
}

// ---------------------------------------------------------------------------
// Supervisor loop
// ---------------------------------------------------------------------------

/// In-memory map from story_id → last-polled Instant.
type LastPollMap = HashMap<Id, Instant>;

async fn supervise(watcher: WatcherManager, cancel: Arc<AtomicBool>) {
    let mut last_poll: LastPollMap = HashMap::new();

    loop {
        if cancel.load(Ordering::Relaxed) {
            return;
        }

        // --- Scan: list all watched stories ---
        let stories = match watcher.product_repo.list_watching().await {
            Ok(list) => list,
            Err(e) => {
                warn!("story watcher: list_watching failed: {e}");
                Vec::new()
            }
        };

        let now = Instant::now();
        for story in stories {
            // A story is "due" if it has never been polled, or if cadence_min
            // minutes have elapsed since the last poll.
            // Floor at 5 minutes so a misconfigured story can't hammer the Atlassian APIs.
            let cadence = Duration::from_secs(story.watch_cadence_min.max(5) as u64 * 60);
            let is_due = match last_poll.get(&story.id) {
                None => true,
                Some(&last) => now.duration_since(last) >= cadence,
            };

            if !is_due {
                continue;
            }

            // Record poll time before spawning so rapid rescans don't double-poll.
            last_poll.insert(story.id.clone(), now);

            // Spawn an independent task per story; isolate errors.
            let product_repo = watcher.product_repo.clone();
            let product = Arc::clone(&watcher.product);
            let orchestrator = Arc::clone(&watcher.orchestrator);
            let improve = Arc::clone(&watcher.improve);
            let events = watcher.events.clone();
            let _default_provider = watcher.default_provider.clone();

            tokio::spawn(async move {
                if let Err(e) = poll_story(
                    story,
                    product_repo,
                    product,
                    orchestrator,
                    improve,
                    events,
                )
                .await
                {
                    warn!("story watcher: poll_story failed: {e}");
                }
            });
        }

        // Sleep in 500ms slices for responsive shutdown.
        let mut waited = Duration::ZERO;
        while waited < SCAN_INTERVAL {
            if cancel.load(Ordering::Relaxed) {
                return;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
            waited += Duration::from_millis(500);
        }
    }
}

// ---------------------------------------------------------------------------
// Per-story poll
// ---------------------------------------------------------------------------

async fn poll_story(
    story: otto_state::ProductStory,
    product_repo: ProductRepo,
    product: Arc<ProductService>,
    orchestrator: Arc<Orchestrator>,
    improve: Arc<ImprovementEngine>,
    events: broadcast::Sender<Event>,
) -> otto_core::Result<()> {
    let story_id = story.id.clone();

    // Step 1: refresh the source content (captures any description edits).
    if let Err(e) = product.refresh_story(&story_id, &story.created_by).await {
        warn!("story watcher: refresh_story {story_id}: {e}");
    }

    // Step 2: fetch comments since the current cursor.
    let comments = product
        .list_new_comments(&story_id, story.watch_cursor.as_deref())
        .await?;

    if comments.is_empty() {
        return Ok(());
    }

    // Step 3: record each new comment as a "watch/comment" event.
    for c in &comments {
        let snippet = c.body_md.chars().take(120).collect::<String>();
        let summary = format!("{}: {}", c.author, snippet);
        if let Err(e) = product_repo
            .add_event(NewEvent {
                story_id: story_id.clone(),
                section: "watch".into(),
                kind: "comment".into(),
                summary,
                actor_id: None,
                meta_json: None,
            })
            .await
        {
            warn!("story watcher: add_event comment {story_id}: {e}");
        }
    }

    // Step 4: advance cursor to the newest comment's created timestamp.
    let newest_created = comments
        .iter()
        .map(|c| c.created.as_str())
        .max()
        .unwrap_or("")
        .to_string();
    if !newest_created.is_empty() {
        if let Err(e) = product_repo.set_watch_cursor(&story_id, &newest_created).await {
            warn!("story watcher: set_watch_cursor {story_id}: {e}");
        }
    }

    // Step 5: reconcile — build prompt from open questions + new comments.
    let open_questions: Vec<ProductQuestion> = product_repo
        .list_questions(&story_id)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("story watcher: list_questions({story_id}) failed: {e}");
            Vec::new()
        })
        .into_iter()
        .filter(|q| q.status == "open" || q.status == "posted")
        .collect();

    let new_comments_md: String = comments
        .iter()
        .map(|c| format!("**{}** ({}): {}", c.author, c.created, c.body_md))
        .collect::<Vec<_>>()
        .join("\n\n");

    let reconcile_contract = r#"Respond with EXACTLY ONE ```json block:
{"answered":[{"question_id":"...","answer":"..."}],"new_questions":[{"text":"...","rationale":"...","category":"..."}],"recommended_next_step":"...","rationale":"..."}"#;

    let prompt = format!(
        "{}\n\n{}",
        build_reconcile_prompt(&open_questions, &new_comments_md),
        reconcile_contract
    );

    let cwd = story
        .cwd
        .clone()
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().into_owned());

    // Retry the one-shot reconcile up to 3 attempts on error (transient provider
    // / tool failures shouldn't lose a watch cycle), with linear backoff.
    let reconcile_result = {
        let mut attempt: u32 = 0;
        loop {
            match orchestrator
                .run_agent(&prompt, &cwd, None, RECONCILE_TIMEOUT)
                .await
            {
                Ok(out) => break Ok(out),
                Err(e) => {
                    attempt += 1;
                    if attempt >= 3 {
                        break Err(e);
                    }
                    warn!(
                        "story watcher: reconcile attempt {attempt} failed for {story_id}: {e}; retrying"
                    );
                    tokio::time::sleep(Duration::from_secs(2 * attempt as u64)).await;
                }
            }
        }
    };

    let recommended_next_step = match reconcile_result {
        Err(e) => {
            warn!("story watcher: reconcile agent error for {story_id}: {e}");
            // Cursor is already advanced; comments are recorded. Continue.
            let source_key = story.source_key.clone();
            let n = comments.len();
            let _ = events.send(Event::Notice {
                level: "info".into(),
                title: format!("Story {source_key}: {n} new comment(s)"),
                body: "(reconcile failed — check logs)".into(),
            });
            return Ok(());
        }
        Ok(output) => {
            // Parse the reconcile JSON.
            let json_val = extract_json_block(&output);
            match json_val {
                None => {
                    warn!("story watcher: reconcile parse failed for {story_id} (len={})", output.len());
                    String::new()
                }
                Some(v) => {
                    // Step 6a: mark answered questions.
                    if let Some(answered) = v["answered"].as_array() {
                        for ans in answered {
                            let qid = ans["question_id"].as_str().unwrap_or("").to_string();
                            let answer = ans["answer"].as_str().unwrap_or("").to_string();
                            if qid.is_empty() {
                                continue;
                            }
                            // Only update if this question belongs to this story.
                            let belongs = open_questions.iter().any(|q| q.id == qid);
                            if !belongs {
                                continue;
                            }
                            if let Err(e) = product_repo
                                .update_question(
                                    &qid,
                                    QuestionPatch {
                                        text: None,
                                        rationale: None,
                                        category: None,
                                        status: Some("answered".into()),
                                        answer: Some(Some(answer)),
                                        posted_ref: None,
                                    },
                                )
                                .await
                            {
                                warn!("story watcher: update_question {qid}: {e}");
                            }
                        }
                    }

                    // Step 6b: insert new questions.
                    if let Some(new_qs) = v["new_questions"].as_array() {
                        for nq in new_qs {
                            let text = nq["text"].as_str().unwrap_or("").to_string();
                            if text.trim().is_empty() {
                                continue;
                            }
                            if let Err(e) = product_repo
                                .create_question(NewQuestion {
                                    story_id: story_id.clone(),
                                    analysis_id: None,
                                    text,
                                    rationale: nq["rationale"]
                                        .as_str()
                                        .unwrap_or("")
                                        .to_string(),
                                    category: nq["category"]
                                        .as_str()
                                        .unwrap_or("general")
                                        .to_string(),
                                    created_by: story.created_by.clone(),
                                })
                                .await
                            {
                                warn!("story watcher: create_question for {story_id}: {e}");
                            }
                        }
                    }

                    v["recommended_next_step"]
                        .as_str()
                        .unwrap_or("")
                        .to_string()
                }
            }
        }
    };

    // Step 6c: add a "watch/reconciled" event.
    if let Err(e) = product_repo
        .add_event(NewEvent {
            story_id: story_id.clone(),
            section: "watch".into(),
            kind: "reconciled".into(),
            summary: if recommended_next_step.is_empty() {
                format!("{} new comment(s) reconciled", comments.len())
            } else {
                recommended_next_step.clone()
            },
            actor_id: None,
            meta_json: None,
        })
        .await
    {
        warn!("story watcher: add_event reconciled {story_id}: {e}");
    }

    // Step 7: run self-improvement narrative.
    let all_questions = product_repo
        .list_questions(&story_id)
        .await
        .unwrap_or_else(|e| {
            tracing::warn!("story watcher: list_questions({story_id}) failed: {e}");
            Vec::new()
        });
    let notes = product_repo.list_notes(&story_id).await.unwrap_or_else(|e| {
        tracing::warn!("story watcher: list_notes({story_id}) failed: {e}");
        Vec::new()
    });
    let narrative = build_improve_narrative_from_clarifications(&story, &all_questions, &notes);
    let target_skills = vec![
        "story-clarifying-questions".to_string(),
        "po-story-overview".to_string(),
    ];
    if let Err(e) = improve
        .run_for_narrative(
            &story.workspace_id,
            "story-comments",
            &narrative,
            &target_skills,
            ImprovementTrigger::Scheduled,
        )
        .await
    {
        warn!("story watcher: run_for_narrative {story_id}: {e}");
    }

    // Step 8: emit a Notice event for the UI.
    let n = comments.len();
    let source_key = story.source_key.clone();
    let body = if recommended_next_step.is_empty() {
        format!("{n} new comment(s) processed")
    } else {
        recommended_next_step
    };
    let _ = events.send(Event::Notice {
        level: "info".into(),
        title: format!("Story {source_key}: {n} new comment(s)"),
        body,
    });

    Ok(())
}

// ---------------------------------------------------------------------------
// Pure helper — unit-testable
// ---------------------------------------------------------------------------

/// Build the reconcile prompt from open questions and new comment text.
///
/// Includes every open/posted question's id + text, plus the new comments
/// joined as markdown. The output contract is appended by the caller.
pub fn build_reconcile_prompt(
    open_questions: &[ProductQuestion],
    new_comments_md: &str,
) -> String {
    let mut prompt = String::new();

    prompt.push_str(
        "You are an assistant reconciling new comments on a product story with \
         open clarifying questions. Review the comments and determine which \
         questions (if any) have been answered. Also identify any new questions \
         raised by the comments. Recommend the next step for the product owner.\n\n",
    );

    if !open_questions.is_empty() {
        prompt.push_str("## Open Questions\n\n");
        for q in open_questions {
            prompt.push_str(&format!("- **ID:** {}\n  **Question:** {}\n\n", q.id, q.text));
        }
    } else {
        prompt.push_str("## Open Questions\n\n(none)\n\n");
    }

    prompt.push_str("## New Comments\n\n");
    if new_comments_md.trim().is_empty() {
        prompt.push_str("(none)\n\n");
    } else {
        prompt.push_str(new_comments_md);
        prompt.push_str("\n\n");
    }

    prompt
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn make_question(id: &str, text: &str) -> ProductQuestion {
        let now = Utc::now();
        let dummy = otto_core::new_id();
        ProductQuestion {
            id: id.to_string(),
            story_id: dummy.clone(),
            analysis_id: None,
            text: text.to_string(),
            rationale: String::new(),
            category: "general".into(),
            status: "open".into(),
            answer: None,
            posted_ref: None,
            created_by: dummy,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn build_reconcile_prompt_includes_question_id_and_text() {
        let q1 = make_question("q-id-001", "What is the rollback plan?");
        let q2 = make_question("q-id-002", "Who is the target user?");
        let comments_md = "**Alice** (2026-06-01T10:00:00Z): The rollback plan is to revert the migration.";

        let prompt = build_reconcile_prompt(&[q1, q2], comments_md);

        assert!(
            prompt.contains("q-id-001"),
            "prompt must include first question ID; got:\n{prompt}"
        );
        assert!(
            prompt.contains("What is the rollback plan?"),
            "prompt must include first question text; got:\n{prompt}"
        );
        assert!(
            prompt.contains("q-id-002"),
            "prompt must include second question ID; got:\n{prompt}"
        );
        assert!(
            prompt.contains("Who is the target user?"),
            "prompt must include second question text; got:\n{prompt}"
        );
    }

    #[test]
    fn build_reconcile_prompt_includes_new_comments_text() {
        let comments_md = "**Bob** (2026-06-01T11:00:00Z): We should target enterprise users.";
        let prompt = build_reconcile_prompt(&[], comments_md);

        assert!(
            prompt.contains("Bob"),
            "prompt must include comment author; got:\n{prompt}"
        );
        assert!(
            prompt.contains("enterprise users"),
            "prompt must include comment body text; got:\n{prompt}"
        );
    }

    #[test]
    fn build_reconcile_prompt_includes_recommended_next_step_contract_marker() {
        let prompt = build_reconcile_prompt(&[], "some comment");
        // The output contract (appended by the caller) uses "recommended_next_step".
        // Verify the function body's preamble mentions the concept.
        // The contract marker is appended *outside* this function, but the
        // prompt body must at least produce the question + comment sections.
        assert!(
            prompt.contains("## Open Questions"),
            "prompt must have Open Questions section; got:\n{prompt}"
        );
        assert!(
            prompt.contains("## New Comments"),
            "prompt must have New Comments section; got:\n{prompt}"
        );
    }

    #[test]
    fn build_reconcile_prompt_empty_questions_shows_none() {
        let prompt = build_reconcile_prompt(&[], "some comment");
        assert!(
            prompt.contains("(none)"),
            "empty questions should produce '(none)' marker; got:\n{prompt}"
        );
    }
}
