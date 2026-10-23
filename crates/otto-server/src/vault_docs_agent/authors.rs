//! The author turns of a docs run — the writers and the summarizer — as
//! slots of the shared fan-out runner (`otto_agent_run::fanout`). This file
//! owns only what is vault-docs specific: one managed-session turn
//! ([`AuthorTurn::run`]), the slot control (user/auto retry budgets, the
//! writers-stage window) and how each slot state lands on the run's rows.
//! `run_docs` drives them; the retry endpoints feed the same queue.

use super::*;
use otto_agent_run::fanout::{Slot, SlotControl, SlotState, WaitForRetry};

/// Which run row an author turn reports into.
#[derive(Clone, Copy, Debug)]
pub(super) enum AuthorRole {
    Writer(usize),
    Summarizer,
}

impl AuthorRole {
    /// Record the turn's session; `starting` also marks the row running (the
    /// session's `on_ready`), otherwise it is the finished turn's id.
    fn set_session(self, run: &mut VaultDocsRun, sid: String, starting: bool) {
        match self {
            AuthorRole::Writer(i) => {
                if let Some(a) = run.agents.get_mut(i) {
                    if starting {
                        a.state = "running".into();
                    }
                    a.session_id = Some(sid);
                }
            }
            AuthorRole::Summarizer => {
                if starting {
                    run.summarizer.state = "running".into();
                }
                run.summarizer.session_id = Some(sid);
            }
        }
    }
}

/// One attempt of a writer/summarizer: a FRESH managed session with a new
/// done-marker, and (for the final author) the results-file contract.
#[derive(Clone)]
pub(super) struct AuthorTurn {
    pub(super) ctx: ServerCtx,
    pub(super) ws: Workspace,
    pub(super) user: User,
    pub(super) reg: RunRegistry,
    pub(super) run_id: String,
    pub(super) root: String,
    pub(super) title: String,
    pub(super) provider: String,
    pub(super) meta: Value,
    pub(super) prompt: String,
    pub(super) cancel: Arc<AtomicBool>,
    /// The results file the FINAL author must fill (single writer or
    /// summarizer); `None` for a drafting writer.
    pub(super) results: Option<std::path::PathBuf>,
    pub(super) role: AuthorRole,
}

impl AuthorTurn {
    pub(super) async fn run(self) -> Result<(), String> {
        let (done_path, done_line) = done_marker();
        let attempt_prompt = format!("{}{done_line}", self.prompt);
        let (ready_reg, ready_run, role) = (self.reg.clone(), self.run_id.clone(), self.role);
        let on_ready = move |sid: &Id| {
            let sid = sid.to_string();
            with_run(&ready_reg, &ready_run, |r| role.set_session(r, sid, true));
            // Durable ASAP: a restart mid-turn must still know the session.
            persist(&ready_reg, &ready_run);
        };
        let res = crate::agent_session::run_session_turn_with(
            &self.ctx,
            &self.ws,
            &self.user,
            None,
            &self.title,
            &self.root,
            &self.provider,
            self.meta.clone(),
            &attempt_prompt,
            crate::agent_session::STUCK_IDLE,
            crate::agent_session::TurnOpts {
                cancel: Some(Arc::clone(&self.cancel)),
                done_file: Some(done_path.clone()),
                done_file_validator: Some(valid_done_marker),
                kill_on_stall: true,
                ..Default::default()
            },
            on_ready,
        )
        .await;
        let _ = std::fs::remove_file(&done_path);
        let (_reply, sid) = res
            .and_then(|value| {
                if let Some(results) = &self.results {
                    require_author_results(results)?;
                }
                Ok(value)
            })
            .map_err(|e| e.0.to_string())?;
        with_run(&self.reg, &self.run_id, |r| {
            self.role.set_session(r, sid.to_string(), false)
        });
        Ok(())
    }
}

/// The session meta every author turn carries (+ model, + the staged skill
/// packages' native view for providers that load it).
pub(super) fn author_meta(
    vault_id: i64,
    run_id: &str,
    spec: &WriterSpec,
    skill_bundle: Option<&crate::modules::StagedSkillPackages>,
) -> Value {
    let mut meta = serde_json::json!({
        "source": "vault-docs",
        "vault_id": vault_id,
        "run_id": run_id,
    });
    if let Some(model) = &spec.model {
        meta["model"] = Value::String(model.clone());
    }
    if let Some(dirs) = otto_review::session::review_skills_extra_dirs(
        &spec.provider,
        staged_package_root(skill_bundle),
    ) {
        meta["extra_dirs"] = dirs;
    }
    meta
}

/// A spawn-death flake: the CLI process exited/vanished underneath the turn
/// without a cancel or retry (codex has died seconds after its trust prompt).
fn spawn_death(error: &str) -> bool {
    error.contains("vanished") || error.contains("exited")
}

/// The writers' slot control: [`MAX_USER_RETRIES`] user retries, one
/// automatic respawn after a spawn death, and a failed writer keeps listening
/// for a user retry while any PEER is still moving (the writers barrier must
/// never hang on a slot nobody can un-stick). The summarizer reuses it with
/// no auto-respawn and no waiting.
pub(super) fn writer_control(
    reg: &RunRegistry,
    run_id: &str,
    cancel: &Arc<AtomicBool>,
    retries: &otto_agent_run::fanout::Retries,
) -> SlotControl {
    let (state_reg, state_run) = (reg.clone(), run_id.to_string());
    let cancel = Arc::clone(cancel);
    let mut ctl = SlotControl::new(
        Arc::clone(retries),
        Arc::new(move || cancel.load(Ordering::Relaxed)),
        MAX_USER_RETRIES,
        Arc::new(move |slot: Slot, state: SlotState| {
            author_state(&state_reg, &state_run, slot, state)
        }),
    );
    ctl.max_auto_retries = MAX_AUTO_RETRIES;
    ctl.transient = spawn_death;
    let (open_reg, open_run) = (reg.clone(), run_id.to_string());
    ctl.wait = WaitForRetry::WhileOpen(Arc::new(move |slot: Slot| match slot {
        Slot::Reviewer(i) => writers_stage_open(&open_reg, &open_run, i),
        Slot::Summarizer => false,
    }));
    ctl
}

/// Land one author slot state on the run's rows (and persist it).
pub(super) fn author_state(reg: &RunRegistry, run_id: &str, slot: Slot, state: SlotState) {
    match (slot, &state) {
        (Slot::Reviewer(i), SlotState::Retrying(Some(e))) => {
            warn!(
                "vault_docs: writer {} session died ({e}); auto-respawning",
                i + 1
            )
        }
        (Slot::Reviewer(i), SlotState::Failed(e)) => {
            warn!("vault_docs: writer {} failed: {e}", i + 1)
        }
        (Slot::Summarizer, SlotState::Failed(e)) => warn!("vault_docs: summarizer failed: {e}"),
        _ => {}
    }
    with_run(reg, run_id, |r| match slot {
        Slot::Reviewer(i) => {
            let Some(a) = r.agents.get_mut(i) else {
                return;
            };
            match state {
                // `on_ready` marks the row running once the session exists.
                SlotState::Running => {}
                SlotState::Done => a.state = "done".into(),
                SlotState::Retrying(_) => {
                    a.state = "pending".into();
                    a.error = None;
                }
                SlotState::Failed(e) => {
                    a.state = "error".into();
                    a.error = Some(e);
                }
                // A cancel KILLS the session — the resulting "vanished"/
                // "exited" error is expected, not a writer failure. A writer
                // that had already failed keeps its error.
                SlotState::Cancelled => {
                    if a.state != "error" {
                        a.state = "cancelled".into();
                    }
                }
            }
        }
        Slot::Summarizer => match state {
            SlotState::Running => {
                if !is_terminal(&r.state) {
                    r.state = "summarizing".into();
                }
                r.summarizer.state = "running".into();
            }
            SlotState::Done => r.summarizer.state = "done".into(),
            SlotState::Retrying(_) => {
                r.summarizer.state = "pending".into();
                r.summarizer.error = None;
            }
            SlotState::Failed(e) => {
                r.summarizer.state = "error".into();
                r.summarizer.error = Some(e);
            }
            SlotState::Cancelled => r.summarizer.state = "cancelled".into(),
        },
    });
    persist(reg, run_id);
}
