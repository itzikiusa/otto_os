//! A person stops one piece of an agent's background work, or all of it
//! (`POST /runtime-sessions/{id}/tasks/{task_id}/stop`, `…/tasks/stop`).
//!
//! The driver stops it with the provider's own call where it has one
//! ([`TaskStop::Native`]: Claude Code's `stop_task` control request, Codex
//! `turn/interrupt` on a spawned agent's thread). Otherwise — or when the
//! provider refuses ("callback not registered") — Otto sends the agent a
//! message asking it to stop the work (`asked`: not a guarantee). Otto
//! never kills a process it cannot prove is the task's own, so `signal` is
//! reserved for a driver that knows the task's pid; none does today.
//!
//! The task stays listed, marked `stopping`, until the provider settles it
//! (the task leaves the driver's [`DriverEvent::BackgroundTasks`]). A stop
//! that is not confirmed in time clears `stopping` and sets `stop_error`,
//! so the row never hangs on "Stopping…".

use super::*;
use crate::driver::TaskStop;
use otto_core::agent_runtime::BackgroundTask;

/// How long a route waits for the provider to answer a native stop call.
const ANSWER_WAIT: Duration = Duration::from_secs(5);
/// How long an accepted native stop may stay unsettled.
const NATIVE_CONFIRM: Duration = Duration::from_secs(30);
/// How long an asked agent gets before the row says it has not stopped.
const ASKED_CONFIRM: Duration = Duration::from_secs(180);
/// The longest task label quoted in the message to the agent.
const LABEL_MAX: usize = 200;

/// How a stop was carried out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TaskStopOutcome {
    /// The provider's own stop call was accepted.
    Native,
    /// Otto killed a process it knows for certain is the task's.
    Signal,
    /// Otto asked the agent to stop it (not a guarantee).
    Asked,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TaskStopResp {
    pub task_id: String,
    pub outcome: TaskStopOutcome,
}

/// One row of a stop-all answer: the outcome, or why that task's stop failed.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(untagged)]
pub enum TaskStopEntry {
    Done(TaskStopResp),
    Failed { task_id: String, error: String },
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct TaskStopAllResp {
    pub results: Vec<TaskStopEntry>,
}

type Waiters = std::sync::Mutex<HashMap<(Id, String), oneshot::Sender<Option<String>>>>;

/// Routes waiting on a provider's answer, by (session, task).
fn waiters() -> &'static Waiters {
    static W: std::sync::OnceLock<Waiters> = std::sync::OnceLock::new();
    W.get_or_init(Default::default)
}

fn take_waiter(session: &Id, task_id: &str) -> Option<oneshot::Sender<Option<String>>> {
    waiters()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .remove(&(session.clone(), task_id.to_string()))
}

enum Begin {
    Native(oneshot::Receiver<Option<String>>, BackgroundTask),
    Ask(BackgroundTask),
}

/// Set (or clear) a listed task's stop marks and publish the list. `false`
/// when the task is not listed any more.
fn mark(
    inner: &Inner,
    live: &Live,
    st: &mut State,
    task_id: &str,
    stopping: bool,
    error: Option<String>,
) -> bool {
    let Some(t) = st
        .session
        .background_tasks
        .iter_mut()
        .find(|t| t.id == task_id)
    else {
        return false;
    };
    t.stopping = stopping;
    t.stop_error = error;
    let (status, last_error) = current_status(st);
    set_session_status(inner, live, st, status, last_error);
    true
}

/// Whether the timeline shows `task_id` settled (finished, failed, stopped).
fn has_ended(st: &State, task_id: &str) -> bool {
    st.proj.items.values().any(|i| {
        matches!(
            &i.kind,
            TurnItemKind::SystemNotice { task_settlement: Some(s), .. } if s.task_id == task_id
        )
    })
}

/// Under the session lock: find the task, mark it stopping and hand the
/// driver's native stop to the child (or say the agent must be asked).
fn begin(inner: &Inner, live: &Live, task_id: &str) -> Result<Begin> {
    let mut st = live.lock();
    if st.session.status == RuntimeSessionStatus::Closed {
        return Err(Error::Conflict("the thread is closed".into()));
    }
    let Some(task) = st
        .session
        .background_tasks
        .iter()
        .find(|t| t.id == task_id)
        .cloned()
    else {
        return Err(if has_ended(&st, task_id) {
            Error::Conflict("that background task already ended".into())
        } else {
            Error::NotFound("no such background task in this thread".into())
        });
    };
    let plan = match st.proc.as_mut() {
        Some(p) => p.driver.stop_task(task_id),
        None => TaskStop::Unsupported,
    };
    mark(inner, live, &mut st, task_id, true, None);
    match plan {
        TaskStop::Native(step) => {
            let (tx, rx) = oneshot::channel();
            waiters()
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .insert((st.session.id.clone(), task_id.to_string()), tx);
            apply_step_opts(inner, live, &mut st, step, true, true);
            Ok(Begin::Native(rx, task))
        }
        TaskStop::Unsupported => Ok(Begin::Ask(task)),
    }
}

/// The driver relayed the provider's answer to a native stop (the
/// [`DriverEvent::TaskStopAnswered`] arm of `apply_step`). A route still
/// waiting gets it; a late refusal shows on the row.
pub(super) fn on_answer(
    inner: &Inner,
    live: &Live,
    st: &mut State,
    task_id: String,
    error: Option<String>,
) {
    if let Some(tx) = take_waiter(&st.session.id, &task_id) {
        if tx.send(error.clone()).is_ok() {
            return;
        }
    }
    if let Some(e) = error {
        mark(inner, live, st, &task_id, false, Some(e));
    }
}

/// A driver's whole-list update keeps the runtime's stop marks on the
/// tasks still listed (a driver never sets them).
pub(super) fn carry_marks(
    old: &[BackgroundTask],
    mut new: Vec<BackgroundTask>,
) -> Vec<BackgroundTask> {
    for t in new
        .iter_mut()
        .filter(|t| !t.stopping && t.stop_error.is_none())
    {
        if let Some(o) = old.iter().find(|o| o.id == t.id) {
            t.stopping = o.stopping;
            t.stop_error = o.stop_error.clone();
        }
    }
    new
}

/// The message that asks the agent to stop `tasks`.
fn ask_text(tasks: &[BackgroundTask]) -> String {
    let mut text = String::from(
        "Otto: the person asked to stop this background work now. Stop it with your \
         own task-stop tool (or end the process you started), do not restart it, and \
         reply in one line with what you stopped:",
    );
    for t in tasks {
        let mut label = t.label.trim().to_string();
        if label.len() > LABEL_MAX {
            let mut end = LABEL_MAX;
            while !label.is_char_boundary(end) {
                end -= 1;
            }
            label.truncate(end);
            label.push('…');
        }
        text.push_str(&format!("\n- `{}` ({}): {label}", t.id, t.kind));
    }
    text
}

impl AgentRuntime {
    /// Stop one background task of thread `id` (see the module docs).
    pub async fn stop_task(&self, id: &Id, task_id: &str) -> Result<TaskStopResp> {
        let live = self.live(id).await?;
        let outcome = match begin(&self.inner, &live, task_id)? {
            Begin::Native(rx, task) => {
                let answer = tokio::time::timeout(ANSWER_WAIT, rx).await;
                take_waiter(id, task_id);
                let refusal = match answer {
                    Ok(Ok(refusal)) => refusal,
                    _ => None,
                };
                self.after_native(id, &live, refusal, &task).await?
            }
            Begin::Ask(task) => {
                self.ask(id, &live, std::slice::from_ref(&task)).await?;
                TaskStopOutcome::Asked
            }
        };
        Ok(TaskStopResp {
            task_id: task_id.to_string(),
            outcome,
        })
    }

    /// Stop every background task of thread `id`: native stops first, then
    /// one message asking the agent for the rest.
    pub async fn stop_all_tasks(&self, id: &Id) -> Result<TaskStopAllResp> {
        let live = self.live(id).await?;
        let ids: Vec<String> = live
            .lock()
            .session
            .background_tasks
            .iter()
            .map(|t| t.id.clone())
            .collect();
        let mut results = Vec::new();
        let mut native = Vec::new();
        let mut to_ask = Vec::new();
        for task_id in ids {
            match begin(&self.inner, &live, &task_id) {
                Ok(Begin::Native(rx, task)) => native.push((rx, task)),
                Ok(Begin::Ask(task)) => to_ask.push(task),
                Err(e) => results.push(TaskStopEntry::Failed {
                    task_id,
                    error: e.to_string(),
                }),
            }
        }
        let deadline = tokio::time::Instant::now() + ANSWER_WAIT;
        for (rx, task) in native {
            let answer = tokio::time::timeout_at(deadline, rx).await;
            take_waiter(id, &task.id);
            match answer {
                Ok(Ok(Some(refusal))) => {
                    tracing::info!(task = %task.id, %refusal, "native task stop refused; asking the agent");
                    to_ask.push(task);
                }
                _ => {
                    self.confirm_later(&live, &task.id, NATIVE_CONFIRM);
                    results.push(TaskStopEntry::Done(TaskStopResp {
                        task_id: task.id,
                        outcome: TaskStopOutcome::Native,
                    }));
                }
            }
        }
        if !to_ask.is_empty() {
            match self.ask(id, &live, &to_ask).await {
                Ok(()) => results.extend(to_ask.into_iter().map(|t| {
                    TaskStopEntry::Done(TaskStopResp {
                        task_id: t.id,
                        outcome: TaskStopOutcome::Asked,
                    })
                })),
                Err(e) => results.extend(to_ask.into_iter().map(|t| TaskStopEntry::Failed {
                    task_id: t.id,
                    error: e.to_string(),
                })),
            }
        }
        Ok(TaskStopAllResp { results })
    }

    /// How many background tasks thread `id` runs (the interrupt answer).
    pub async fn background_count(&self, id: &Id) -> Result<usize> {
        let live = self.live(id).await?;
        let n = live.lock().session.background_tasks.len();
        Ok(n)
    }

    /// A native stop's answer (or none in time): accepted / unanswered is
    /// `native`; a refusal falls back to asking the agent.
    async fn after_native(
        &self,
        id: &Id,
        live: &Arc<Live>,
        refusal: Option<String>,
        task: &BackgroundTask,
    ) -> Result<TaskStopOutcome> {
        if let Some(refusal) = refusal {
            tracing::info!(task = %task.id, %refusal, "native task stop refused; asking the agent");
            self.ask(id, live, std::slice::from_ref(task)).await?;
            return Ok(TaskStopOutcome::Asked);
        }
        self.confirm_later(live, &task.id, NATIVE_CONFIRM);
        Ok(TaskStopOutcome::Native)
    }

    /// Ask the agent (a steered message into a running turn, else a turn of
    /// its own). A failed send clears the marks and says why on each row.
    async fn ask(&self, id: &Id, live: &Arc<Live>, tasks: &[BackgroundTask]) -> Result<()> {
        let req = RuntimePromptReq {
            delivery: Some(PromptDelivery::Steer),
            ..RuntimePromptReq::of(&ask_text(tasks))
        };
        if let Err(e) = self.prompt_with(id, &req).await {
            let mut st = live.lock();
            for t in tasks {
                let why = format!("could not ask the agent to stop it: {e}");
                mark(&self.inner, live, &mut st, &t.id, false, Some(why));
            }
            return Err(e);
        }
        for t in tasks {
            self.confirm_later(live, &t.id, ASKED_CONFIRM);
        }
        Ok(())
    }

    /// Past `grace`, a task still listed as stopping gets `stop_error`.
    fn confirm_later(&self, live: &Arc<Live>, task_id: &str, grace: Duration) {
        let inner = Arc::clone(&self.inner);
        let live = Arc::clone(live);
        let task_id = task_id.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(grace).await;
            let mut st = live.lock();
            let still = st
                .session
                .background_tasks
                .iter()
                .any(|t| t.id == task_id && t.stopping);
            if still {
                let why = format!(
                    "still running {}s after the stop — it may be ignoring it; try again or close the thread",
                    grace.as_secs()
                );
                mark(&inner, &live, &mut st, &task_id, false, Some(why));
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn task(id: &str) -> BackgroundTask {
        BackgroundTask {
            id: id.into(),
            kind: "command".into(),
            label: "npm run dev".into(),
            started_at: Utc::now(),
            item_id: None,
            due_at: None,
            stopping: false,
            stop_error: None,
        }
    }

    #[test]
    fn a_drivers_list_update_keeps_the_runtimes_stop_marks() {
        let mut old = task("a");
        old.stopping = true;
        let mut failed = task("b");
        failed.stop_error = Some("refused".into());
        let new = carry_marks(&[old, failed], vec![task("a"), task("b"), task("c")]);
        assert!(new[0].stopping);
        assert_eq!(new[1].stop_error.as_deref(), Some("refused"));
        assert!(!new[2].stopping && new[2].stop_error.is_none());
    }

    #[test]
    fn the_ask_names_every_task_and_bounds_long_labels() {
        let mut long = task("b2");
        long.label = "x".repeat(500);
        let text = ask_text(&[task("a1"), long]);
        assert!(text.contains("`a1` (command): npm run dev"));
        assert!(text.contains("`b2` (command): "));
        assert!(text.contains(&format!("{}…", "x".repeat(LABEL_MAX))));
        assert!(!text.contains(&"x".repeat(LABEL_MAX + 1)));
        assert!(text.contains("do not restart it"));
    }

    #[test]
    fn outcomes_serialize_as_the_contract_spells_them() {
        let ok = TaskStopEntry::Done(TaskStopResp {
            task_id: "t".into(),
            outcome: TaskStopOutcome::Asked,
        });
        let bad = TaskStopEntry::Failed {
            task_id: "u".into(),
            error: "gone".into(),
        };
        assert_eq!(
            serde_json::to_value(TaskStopAllResp {
                results: vec![ok, bad]
            })
            .unwrap(),
            serde_json::json!({"results": [
                {"task_id": "t", "outcome": "asked"},
                {"task_id": "u", "error": "gone"},
            ]})
        );
    }
}
