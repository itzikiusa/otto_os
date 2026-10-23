//! The post-write review loop: reviewer slots per round, then the final
//! author's revision turn, until clean or out of iterations. Each reviewer and
//! revision runs through `otto_agent_run::fanout::attempts` (user retries,
//! wait-for-retry until cancel). Driven by [`super::run`].

use super::*;

pub(super) fn update_reviewer_state(
    run: &mut VaultDocsRun,
    iteration: u8,
    index: usize,
    update: impl Fn(&mut VaultDocsReviewer),
) {
    if let Some(reviewer) = run
        .review
        .rounds
        .iter_mut()
        .find(|round| round.iteration == iteration)
        .and_then(|round| round.reviewers.get_mut(index))
    {
        update(reviewer);
    }
}

pub(super) fn reset_reviewer_for_retry(run: &mut VaultDocsRun, iteration: u8, index: usize) {
    update_reviewer_state(run, iteration, index, |reviewer| {
        reviewer.state = "pending".into();
        reviewer.session_id = None;
        reviewer.error = None;
        reviewer.findings.clear();
    });
}

pub(super) fn reset_revision_for_retry(run: &mut VaultDocsRun, iteration: u8) {
    if let Some(round) = run
        .review
        .rounds
        .iter_mut()
        .find(|round| round.iteration == iteration)
    {
        round.revision.state = "pending".into();
        round.revision.error = None;
        round.revision.changed_paths.clear();
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn complete_review_or_finish(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    vault: &otto_vault::VaultRec,
    reg: &RunRegistry,
    repo: &otto_state::VaultDocsRunsRepo,
    run_id: &str,
    original_request: &str,
    target_dir: &str,
    author: &WriterSpec,
    author_session_id: Option<String>,
    skill_bundle: Option<crate::modules::StagedSkillPackages>,
    cancel: Arc<AtomicBool>,
    retries: otto_agent_run::fanout::Retries,
) {
    let configured = reg
        .lock()
        .unwrap()
        .get(run_id)
        .map(|entry| entry.run.review.clone())
        .unwrap_or_default();
    if configured.state == "skipped" {
        finish_run(reg, run_id, "done", None);
        finalize_run(reg, repo, run_id).await;
        return;
    }
    let Some(author_session_id) = author_session_id else {
        with_run(reg, run_id, |run| {
            run.review.state = "error".into();
            run.review.outcome = Some("error".into());
        });
        finish_run(
            reg,
            run_id,
            "error",
            Some("final author session is unavailable for review revisions".into()),
        );
        finalize_run(reg, repo, run_id).await;
        return;
    };

    let result = run_review_loop(
        ctx,
        ws,
        user,
        vault,
        reg,
        run_id,
        original_request,
        target_dir,
        author,
        &author_session_id,
        skill_bundle.as_ref(),
        cancel,
        retries,
    )
    .await;
    match result {
        ReviewLoopResult::Clean => finish_run(reg, run_id, "done", None),
        ReviewLoopResult::Exhausted => finish_run(reg, run_id, "done_with_findings", None),
        ReviewLoopResult::Cancelled => finish_run(reg, run_id, "cancelled", None),
        ReviewLoopResult::Error(error) => {
            with_run(reg, run_id, |run| {
                run.review.state = "error".into();
                run.review.outcome = Some("error".into());
            });
            finish_run(reg, run_id, "error", Some(error));
        }
    }
    finalize_run(reg, repo, run_id).await;
}

pub(super) enum ReviewLoopResult {
    Clean,
    Exhausted,
    Cancelled,
    Error(String),
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub(super) enum ReviewTurnKind {
    Reviewer { iteration: u8, index: usize },
    Revision { iteration: u8 },
}

/// Owned request at the agent-session seam. Tests can deterministically write
/// the requested result artifact without constructing the daemon's full
/// `ServerCtx`; production still delegates to the managed-session runner.
pub(super) struct ReviewTurnRequest {
    pub(super) kind: ReviewTurnKind,
    pub(super) existing: Option<Id>,
    pub(super) title: String,
    pub(super) cwd: String,
    pub(super) provider: String,
    pub(super) meta: Value,
    pub(super) prompt: String,
    pub(super) result_path: std::path::PathBuf,
    pub(super) done_path: std::path::PathBuf,
    pub(super) cancel: Arc<AtomicBool>,
}

#[async_trait::async_trait]
pub(super) trait ReviewTurnRunner: Send + Sync {
    async fn run_turn(
        &self,
        request: ReviewTurnRequest,
        on_ready: Box<dyn FnOnce(Id) + Send>,
    ) -> Result<(String, Id), String>;

    async fn scan(&self, vault_id: i64);
}

#[derive(Clone)]
pub(super) struct LiveReviewTurnRunner {
    pub(super) ctx: ServerCtx,
    pub(super) ws: Workspace,
    pub(super) user: User,
}

#[async_trait::async_trait]
impl ReviewTurnRunner for LiveReviewTurnRunner {
    async fn run_turn(
        &self,
        request: ReviewTurnRequest,
        on_ready: Box<dyn FnOnce(Id) + Send>,
    ) -> Result<(String, Id), String> {
        tracing::debug!(
            turn = ?request.kind,
            result_path = %request.result_path.display(),
            "vault docs review turn starting"
        );
        crate::agent_session::run_session_turn_with(
            &self.ctx,
            &self.ws,
            &self.user,
            request.existing.as_ref(),
            &request.title,
            &request.cwd,
            &request.provider,
            request.meta,
            &request.prompt,
            crate::agent_session::STUCK_IDLE,
            crate::agent_session::TurnOpts {
                cancel: Some(request.cancel),
                done_file: Some(request.done_path),
                done_file_validator: Some(valid_done_marker),
                kill_on_stall: true,
                ..Default::default()
            },
            move |sid| on_ready(sid.clone()),
        )
        .await
        .map_err(|error| error.0.to_string())
    }

    async fn scan(&self, vault_id: i64) {
        let _ = self.ctx.vault.scan(vault_id).await;
    }
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_review_loop(
    ctx: &ServerCtx,
    ws: &Workspace,
    user: &User,
    vault: &otto_vault::VaultRec,
    reg: &RunRegistry,
    run_id: &str,
    original_request: &str,
    target_dir: &str,
    author: &WriterSpec,
    author_session_id: &str,
    skill_bundle: Option<&crate::modules::StagedSkillPackages>,
    cancel: Arc<AtomicBool>,
    retries: otto_agent_run::fanout::Retries,
) -> ReviewLoopResult {
    let runner: Arc<dyn ReviewTurnRunner> = Arc::new(LiveReviewTurnRunner {
        ctx: ctx.clone(),
        ws: ws.clone(),
        user: user.clone(),
    });
    run_review_loop_with_runner(
        runner,
        vault.id,
        &vault.root_path,
        reg,
        run_id,
        original_request,
        target_dir,
        author,
        author_session_id,
        skill_bundle,
        cancel,
        retries,
    )
    .await
}

pub(super) struct ReviewerSlotArgs {
    pub(super) reg: RunRegistry,
    pub(super) run_id: String,
    pub(super) vault_id: i64,
    pub(super) root: String,
    pub(super) package_root: String,
    pub(super) original_request: String,
    pub(super) target_dir: String,
    pub(super) iteration: u8,
    pub(super) reviewer: VaultDocsReviewer,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) retries: otto_agent_run::fanout::Retries,
}

/// Run one independent reviewer including its targeted manual-retry state
/// machine. The round coordinator only joins these slots and decides the next
/// round action.
pub(super) async fn run_reviewer_slot(runner: Arc<dyn ReviewTurnRunner>, args: ReviewerSlotArgs) {
    let ReviewerSlotArgs {
        reg,
        run_id,
        vault_id,
        root,
        package_root,
        original_request,
        target_dir,
        iteration,
        reviewer,
        cancel,
        retries,
    } = args;
    let index = reviewer.index;
    let results_path = std::env::temp_dir().join(format!(
        "otto-vaultdocs-review-{run_id}-{iteration}-{index}.json"
    ));
    let results_str = results_path.to_string_lossy().to_string();
    let package_path = std::path::Path::new(&package_root)
        .join("skills")
        .join(&reviewer.skill)
        .join("SKILL.md")
        .to_string_lossy()
        .to_string();
    let prompt = build_reviewer_prompt(
        &original_request,
        vault_id,
        &target_dir,
        iteration,
        &reviewer.skill,
        reviewer.focus.as_deref(),
        &package_path,
        &results_str,
    );
    let mut meta = serde_json::json!({
        "source": "vault-docs-review",
        "vault_id": vault_id,
        "run_id": run_id.clone(),
        "iteration": iteration,
        "reviewer_index": index,
    });
    if let Some(model) = &reviewer.model {
        meta["model"] = Value::String(model.clone());
    }
    if let Some(dirs) = otto_review::session::review_skills_extra_dirs(
        &reviewer.provider,
        Some(package_root.as_str()),
    ) {
        meta["extra_dirs"] = dirs;
    }
    // The shared slot loop: a user retry re-runs this reviewer in a fresh
    // session; a failed reviewer keeps listening for one until the run is
    // cancelled (then it ends "cancelled").
    let state_reg = reg.clone();
    let state_run = run_id.clone();
    let mut ctl = otto_agent_run::fanout::SlotControl::new(
        retries,
        cancelled_fn(&cancel),
        MAX_USER_RETRIES,
        Arc::new(move |_slot: otto_agent_run::fanout::Slot, state| {
            reviewer_slot_state(&state_reg, &state_run, iteration, index, state)
        }),
    );
    ctl.wait = otto_agent_run::fanout::WaitForRetry::UntilCancelled;
    ctl.key = Arc::new(move |_: otto_agent_run::fanout::Slot| reviewer_retry_key(iteration, index));
    let _ = otto_agent_run::fanout::attempts(
        otto_agent_run::fanout::Slot::Reviewer(index),
        &ctl,
        || {
            let _ = std::fs::write(&results_path, "");
            let (done_path, done_line) = done_marker();
            let attempt_prompt = format!("{prompt}{done_line}");
            let ready_reg = reg.clone();
            let ready_run = run_id.clone();
            let on_ready = move |sid: Id| {
                let sid = sid.to_string();
                with_run(&ready_reg, &ready_run, |run| {
                    update_reviewer_state(run, iteration, index, |reviewer| {
                        reviewer.state = "running".into();
                        reviewer.session_id = Some(sid.clone());
                        reviewer.error = None;
                    });
                });
                persist(&ready_reg, &ready_run);
            };
            let request = ReviewTurnRequest {
                kind: ReviewTurnKind::Reviewer { iteration, index },
                existing: None,
                title: format!("Vault docs: review {iteration}.{}", index + 1),
                cwd: root.clone(),
                provider: reviewer.provider.clone(),
                meta: meta.clone(),
                prompt: attempt_prompt,
                result_path: results_path.clone(),
                done_path: done_path.clone(),
                cancel: Arc::clone(&cancel),
            };
            let (runner, reg, run_id, results_path) = (
                Arc::clone(&runner),
                reg.clone(),
                run_id.clone(),
                results_path.clone(),
            );
            async move {
                let turn = runner.run_turn(request, Box::new(on_ready)).await;
                let _ = std::fs::remove_file(&done_path);
                let (sid, findings) = turn.and_then(|(_reply, sid)| {
                    std::fs::read_to_string(&results_path)
                        .map_err(|error| format!("read reviewer output: {error}"))
                        .and_then(|content| parse_review_findings(&content))
                        .map(|findings| (sid, findings))
                })?;
                with_run(&reg, &run_id, |run| {
                    update_reviewer_state(run, iteration, index, |reviewer| {
                        reviewer.session_id = Some(sid.to_string());
                        reviewer.findings = findings.clone();
                        reviewer.error = None;
                    });
                });
                Ok(())
            }
        },
    )
    .await;
    let _ = std::fs::remove_file(results_path);
}

/// The run's cancel flag as the slot runner's predicate.
pub(super) fn cancelled_fn(cancel: &Arc<AtomicBool>) -> Arc<dyn Fn() -> bool + Send + Sync> {
    let cancel = Arc::clone(cancel);
    Arc::new(move || cancel.load(Ordering::Relaxed))
}

/// Land one reviewer slot state on its round row (and persist it). Success
/// is written by the attempt itself (session + findings), then `Done`.
pub(super) fn reviewer_slot_state(
    reg: &RunRegistry,
    run_id: &str,
    iteration: u8,
    index: usize,
    state: otto_agent_run::fanout::SlotState,
) {
    use otto_agent_run::fanout::SlotState;
    with_run(reg, run_id, |run| match state {
        SlotState::Running => {}
        SlotState::Retrying(_) => reset_reviewer_for_retry(run, iteration, index),
        SlotState::Done => update_reviewer_state(run, iteration, index, |reviewer| {
            reviewer.state = "done".into();
        }),
        SlotState::Failed(error) => update_reviewer_state(run, iteration, index, |reviewer| {
            reviewer.state = "error".into();
            reviewer.error = Some(error.clone());
            reviewer.findings.clear();
        }),
        SlotState::Cancelled => update_reviewer_state(run, iteration, index, |reviewer| {
            reviewer.state = "cancelled".into();
            reviewer.error = None;
        }),
    });
    persist(reg, run_id);
}

/// Land one revision slot state on the round's revision row. Success and
/// cancellation are the round coordinator's to record.
pub(super) fn revision_slot_state(
    reg: &RunRegistry,
    run_id: &str,
    iteration: u8,
    state: otto_agent_run::fanout::SlotState,
) {
    use otto_agent_run::fanout::SlotState;
    match state {
        SlotState::Retrying(_) => with_run(reg, run_id, |run| {
            reset_revision_for_retry(run, iteration);
        }),
        SlotState::Failed(error) => with_run(reg, run_id, |run| {
            if let Some(round) = run.review.rounds.last_mut() {
                round.revision.state = "error".into();
                round.revision.error = Some(error);
            }
        }),
        _ => return,
    }
    persist(reg, run_id);
}

pub(super) struct RevisionTurnArgs {
    pub(super) reg: RunRegistry,
    pub(super) run_id: String,
    pub(super) vault_id: i64,
    pub(super) root: String,
    pub(super) iteration: u8,
    pub(super) author: WriterSpec,
    pub(super) author_session_id: String,
    pub(super) prompt: String,
    pub(super) result_path: std::path::PathBuf,
    pub(super) cancel: Arc<AtomicBool>,
    pub(super) retries: otto_agent_run::fanout::Retries,
}

/// Resume the final author for one repair turn, including its independently
/// retryable result-artifact contract.
pub(super) async fn run_revision_turn(
    runner: Arc<dyn ReviewTurnRunner>,
    args: RevisionTurnArgs,
) -> Result<Vec<String>, String> {
    let RevisionTurnArgs {
        reg,
        run_id,
        vault_id,
        root,
        iteration,
        author,
        author_session_id,
        prompt,
        result_path,
        cancel,
        retries,
    } = args;
    // The shared slot loop (the revision is the final author's repair turn,
    // so it stands in the summarizer's slot): a user retry re-runs it; a
    // failed revision waits for one until the run is cancelled.
    let state_reg = reg.clone();
    let state_run = run_id.clone();
    let mut ctl = otto_agent_run::fanout::SlotControl::new(
        retries,
        cancelled_fn(&cancel),
        MAX_USER_RETRIES,
        Arc::new(move |_slot: otto_agent_run::fanout::Slot, state| {
            revision_slot_state(&state_reg, &state_run, iteration, state)
        }),
    );
    ctl.wait = otto_agent_run::fanout::WaitForRetry::UntilCancelled;
    ctl.key = Arc::new(move |_: otto_agent_run::fanout::Slot| revision_retry_key(iteration));
    let result =
        otto_agent_run::fanout::attempts(otto_agent_run::fanout::Slot::Summarizer, &ctl, || {
            let _ = std::fs::write(&result_path, "");
            let (done_path, done_line) = done_marker();
            let attempt_prompt = format!("{prompt}{done_line}");
            let ready_reg = reg.clone();
            let ready_run = run_id.clone();
            let on_ready = move |sid: Id| {
                with_run(&ready_reg, &ready_run, |run| {
                    if let Some(round) = run.review.rounds.last_mut() {
                        round.revision.state = "running".into();
                        round.revision.session_id = Some(sid.to_string());
                        round.revision.error = None;
                    }
                });
                persist(&ready_reg, &ready_run);
            };
            let mut meta = serde_json::json!({
                "source": "vault-docs",
                "vault_id": vault_id,
                "run_id": run_id,
                "iteration": iteration,
                "role": "revision",
            });
            if let Some(model) = &author.model {
                meta["model"] = Value::String(model.clone());
            }
            let request = ReviewTurnRequest {
                kind: ReviewTurnKind::Revision { iteration },
                existing: Some(Id::from(author_session_id.clone())),
                title: format!("Vault docs: revision {iteration}"),
                cwd: root.clone(),
                provider: author.provider.clone(),
                meta,
                prompt: attempt_prompt,
                result_path: result_path.clone(),
                done_path: done_path.clone(),
                cancel: Arc::clone(&cancel),
            };
            let (runner, result_path) = (Arc::clone(&runner), result_path.clone());
            async move {
                let turn = runner.run_turn(request, Box::new(on_ready)).await;
                let _ = std::fs::remove_file(done_path);
                turn.and_then(|_| {
                    std::fs::read_to_string(&result_path)
                        .map_err(|error| format!("read revision output: {error}"))
                        .and_then(|content| {
                            parse_results(&content).ok_or_else(|| {
                                "revision output must contain {\"written\": [...]}".into()
                            })
                        })
                })
            }
        })
        .await
        .map_err(|failure| {
            if failure.cancelled {
                "cancelled".to_string()
            } else {
                failure.error
            }
        });
    let _ = std::fs::remove_file(result_path);
    result
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn run_review_loop_with_runner(
    runner: Arc<dyn ReviewTurnRunner>,
    vault_id: i64,
    vault_root: &str,
    reg: &RunRegistry,
    run_id: &str,
    original_request: &str,
    target_dir: &str,
    author: &WriterSpec,
    author_session_id: &str,
    skill_bundle: Option<&crate::modules::StagedSkillPackages>,
    cancel: Arc<AtomicBool>,
    retries: otto_agent_run::fanout::Retries,
) -> ReviewLoopResult {
    let (configured_reviewers, max_iterations) = match reg.lock().unwrap().get(run_id) {
        Some(entry) => (
            entry.run.review.reviewers.clone(),
            entry.run.review.max_iterations,
        ),
        None => return ReviewLoopResult::Error("review run disappeared".into()),
    };
    let Some(packages) = skill_bundle else {
        return ReviewLoopResult::Error("review skill packages could not be staged".into());
    };
    for reviewer in &configured_reviewers {
        if !packages.files.contains_key(&reviewer.skill) {
            return ReviewLoopResult::Error(format!(
                "review skill package '{}' could not be staged",
                reviewer.skill
            ));
        }
    }

    for iteration in 1..=max_iterations {
        if cancel.load(Ordering::Relaxed) {
            return ReviewLoopResult::Cancelled;
        }
        let round = fresh_review_round(iteration, &configured_reviewers);
        with_run(reg, run_id, |run| {
            if is_terminal(&run.state) {
                return;
            }
            run.state = "reviewing".into();
            run.review.state = "reviewing".into();
            run.review.current_iteration = iteration;
            run.review.rounds.push(round);
        });
        persist(reg, run_id);

        let mut set = tokio::task::JoinSet::new();
        for reviewer in configured_reviewers.iter().cloned() {
            set.spawn(run_reviewer_slot(
                Arc::clone(&runner),
                ReviewerSlotArgs {
                    reg: reg.clone(),
                    run_id: run_id.to_string(),
                    vault_id,
                    root: vault_root.to_string(),
                    package_root: packages.root.clone(),
                    original_request: original_request.to_string(),
                    target_dir: target_dir.to_string(),
                    iteration,
                    reviewer,
                    cancel: Arc::clone(&cancel),
                    retries: Arc::clone(&retries),
                },
            ));
        }
        while set.join_next().await.is_some() {}

        if cancel.load(Ordering::Relaxed) {
            return ReviewLoopResult::Cancelled;
        }
        let reviewers = reg
            .lock()
            .unwrap()
            .get(run_id)
            .and_then(|entry| entry.run.review.rounds.last())
            .map(|round| round.reviewers.clone())
            .unwrap_or_default();
        match review_round_outcome(&reviewers, iteration, max_iterations) {
            ReviewRoundOutcome::Clean => {
                with_run(reg, run_id, |run| {
                    run.review.state = "clean".into();
                    run.review.outcome = Some("clean".into());
                    if let Some(round) = run.review.rounds.last_mut() {
                        round.state = "clean".into();
                    }
                });
                persist(reg, run_id);
                return ReviewLoopResult::Clean;
            }
            ReviewRoundOutcome::Exhausted => {
                with_run(reg, run_id, |run| {
                    run.review.state = "exhausted".into();
                    run.review.outcome = Some("findings_remain".into());
                    if let Some(round) = run.review.rounds.last_mut() {
                        round.state = "exhausted".into();
                    }
                });
                persist(reg, run_id);
                return ReviewLoopResult::Exhausted;
            }
            ReviewRoundOutcome::Error => {
                let error = reviewers
                    .iter()
                    .find_map(|reviewer| reviewer.error.clone())
                    .unwrap_or_else(|| "one or more reviewers failed".into());
                with_run(reg, run_id, |run| {
                    run.review.state = "error".into();
                    run.review.outcome = Some("error".into());
                    if let Some(round) = run.review.rounds.last_mut() {
                        round.state = "error".into();
                    }
                });
                persist(reg, run_id);
                return ReviewLoopResult::Error(format!("review round {iteration}: {error}"));
            }
            ReviewRoundOutcome::Revise => {}
        }

        let findings = reviewers
            .iter()
            .flat_map(|reviewer| reviewer.findings.clone())
            .collect::<Vec<_>>();
        let revision_path =
            std::env::temp_dir().join(format!("otto-vaultdocs-revision-{run_id}-{iteration}.json"));
        let revision_str = revision_path.to_string_lossy().to_string();
        let revision_prompt = build_revision_prompt(
            original_request,
            vault_id,
            target_dir,
            iteration,
            &findings,
            &revision_str,
        );
        with_run(reg, run_id, |run| {
            run.state = "revising".into();
            run.review.state = "revising".into();
            if let Some(round) = run.review.rounds.last_mut() {
                round.state = "revising".into();
                round.revision = VaultDocsRevision {
                    state: "pending".into(),
                    session_id: Some(author_session_id.to_string()),
                    changed_paths: Vec::new(),
                    error: None,
                };
            }
        });
        persist(reg, run_id);

        let revision_result = run_revision_turn(
            Arc::clone(&runner),
            RevisionTurnArgs {
                reg: reg.clone(),
                run_id: run_id.to_string(),
                vault_id,
                root: vault_root.to_string(),
                iteration,
                author: author.clone(),
                author_session_id: author_session_id.to_string(),
                prompt: revision_prompt,
                result_path: revision_path,
                cancel: Arc::clone(&cancel),
                retries: Arc::clone(&retries),
            },
        )
        .await;
        if cancel.load(Ordering::Relaxed) {
            return ReviewLoopResult::Cancelled;
        }
        match revision_result {
            Ok(changed_paths) => {
                runner.scan(vault_id).await;
                with_run(reg, run_id, |run| {
                    if let Some(round) = run.review.rounds.last_mut() {
                        round.state = "revised".into();
                        round.revision.state = "done".into();
                        round.revision.changed_paths = changed_paths.clone();
                        round.revision.error = None;
                    }
                    for path in &changed_paths {
                        if !run.written.contains(path) {
                            run.written.push(path.clone());
                        }
                    }
                });
                persist(reg, run_id);
            }
            Err(error) => {
                with_run(reg, run_id, |run| {
                    run.review.state = "error".into();
                    run.review.outcome = Some("error".into());
                    if let Some(round) = run.review.rounds.last_mut() {
                        round.state = "error".into();
                        round.revision.state = "error".into();
                        round.revision.error = Some(error.clone());
                    }
                });
                persist(reg, run_id);
                return ReviewLoopResult::Error(format!(
                    "review round {iteration} revision failed: {error}"
                ));
            }
        }
    }
    ReviewLoopResult::Exhausted
}
