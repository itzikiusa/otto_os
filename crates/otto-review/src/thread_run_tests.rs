//! Review agents on the thread runner, against an in-memory [`StepRunner`]
//! that acts out each attempt (the findings file it writes, its reply, its
//! failure class, its events).
use super::*;
use crate::session::{findings_path, run_agent_session_with_recovery};
use async_trait::async_trait;
use otto_agent_run::step::StepError;
use otto_core::agent_runtime::{Attention, AttentionKind, WaitReason};
use otto_core::domain::{ReviewAgentState, User, Workspace};
use otto_core::Id;
use std::sync::Mutex as StdMutex;

/// One attempt as the fake agent plays it.
#[derive(Clone, Default)]
struct Act {
    write: Option<String>,
    reply: String,
    fail: Option<StepErrorClass>,
    events: Vec<StepEvent>,
    /// Wait for the cancel signal, then end `canceled`.
    hang: bool,
}

#[derive(Default)]
struct Fake {
    acts: StdMutex<Vec<Act>>,
    turns: StdMutex<Vec<StepTurn>>,
    closed: StdMutex<Vec<Id>>,
    kept: StdMutex<Vec<(Vec<Id>, chrono::DateTime<Utc>)>>,
    resumed: StdMutex<Vec<Id>>,
}

impl Fake {
    async fn play(
        &self,
        t: StepTurn,
        ev: tokio::sync::mpsc::UnboundedSender<StepEvent>,
        cancel: CancelSignal,
        resume: Option<Id>,
    ) -> otto_core::Result<StepOutcome> {
        let n = {
            let mut turns = self.turns.lock().unwrap();
            turns.push(t.clone());
            turns.len()
        };
        let act = {
            let acts = self.acts.lock().unwrap();
            acts.get(n - 1).or(acts.last()).cloned().unwrap_or_default()
        };
        let thread_id: Id = resume.unwrap_or_else(|| format!("thread-{n}"));
        let _ = ev.send(StepEvent::Started {
            thread_id: thread_id.clone(),
            start_ordinal: 0,
        });
        for e in act.events {
            let _ = ev.send(e);
        }
        let mut fail = act.fail;
        if act.hang {
            cancel.cancelled().await;
            fail = Some(StepErrorClass::Canceled);
        }
        if let (Some(text), Some(path)) = (&act.write, &t.handoff) {
            std::fs::write(path, text).unwrap();
        }
        Ok(StepOutcome {
            thread_id,
            reply: act.reply,
            handoff_written: act.write.is_some(),
            turn_status: if fail.is_some() {
                "failed"
            } else {
                "completed"
            }
            .into(),
            error: fail.map(|c| StepError::new(c, c.as_str())),
            context_usage: None,
            turns: 1,
            usage: Default::default(),
            model: t.model.clone(),
            effort: t.effort.clone(),
            sub_threads: vec![],
            digest: None,
        })
    }
}

#[async_trait]
impl StepRunner for Fake {
    async fn run(
        &self,
        t: StepTurn,
        ev: tokio::sync::mpsc::UnboundedSender<StepEvent>,
        cancel: CancelSignal,
    ) -> otto_core::Result<StepOutcome> {
        self.play(t, ev, cancel, None).await
    }
    async fn resume(
        &self,
        id: &Id,
        _: u32,
        t: &StepTurn,
        ev: tokio::sync::mpsc::UnboundedSender<StepEvent>,
        c: CancelSignal,
    ) -> otto_core::Result<StepOutcome> {
        self.resumed.lock().unwrap().push(id.clone());
        self.play(t.clone(), ev, c, Some(id.clone())).await
    }

    async fn close(&self, id: &Id, _: chrono::DateTime<Utc>) {
        self.closed.lock().unwrap().push(id.clone());
    }
    async fn set_retain_until(&self, ids: &[Id], at: chrono::DateTime<Utc>) {
        self.kept.lock().unwrap().push((ids.to_vec(), at));
    }
    async fn release_process(&self, _: &Id) {}
    async fn delete(&self, _: &Id) {}
}

struct Rig {
    _dir: tempfile::TempDir,
    manager: Arc<otto_sessions::SessionManager>,
    reviews: ReviewsRepo,
    ws: Workspace,
    user: User,
    review_id: String,
    states: SharedStates,
    fake: Arc<Fake>,
    threads: Threads,
}

async fn rig(acts: Vec<Act>) -> Rig {
    let dir = tempfile::tempdir().unwrap();
    let pool = otto_state::open(&dir.path().join("state.db"))
        .await
        .unwrap();
    let user = otto_state::UsersRepo::new(pool.clone())
        .create("reviewer", "x", "R", false)
        .await
        .unwrap();
    let ws = otto_state::WorkspacesRepo::new(pool.clone())
        .create("w", dir.path().to_str().unwrap(), &user.id)
        .await
        .unwrap();
    let (events, _rx) = tokio::sync::broadcast::channel(16);
    let manager = Arc::new(otto_sessions::SessionManager::new(
        otto_state::SessionsRepo::new(pool.clone()),
        events,
        otto_sessions::ProviderRegistry::new(None),
    ));
    let reviews = ReviewsRepo::new(pool);
    let review = reviews.create_review(&"repo".into(), 0).await.unwrap();
    let rows = vec![
        ReviewAgentState {
            name: "Correctness".into(),
            provider: "claude".into(),
            model: "opus".into(),
            effort: "high".into(),
            status: "pending".into(),
            lens: "Correctness".into(),
            ..Default::default()
        },
        ReviewAgentState {
            name: "Summarizer".into(),
            provider: "claude".into(),
            status: "pending".into(),
            ..Default::default()
        },
    ];
    reviews.set_agents(&review.id, &rows).await.unwrap();
    let fake = Arc::new(Fake::default());
    *fake.acts.lock().unwrap() = acts;
    let threads = Threads {
        runner: fake.clone(),
        has_driver: Arc::new(|_| true),
        meta: serde_json::json!({"run_id": "run-1", "node_id": "review",
            "work": {"kind": "workflow_run", "id": "run-1", "workflow_id": "wf-1"}}),
        keep_failed: chrono::Duration::hours(168),
        keep_success: chrono::Duration::hours(24),
    };
    Rig {
        _dir: dir,
        manager,
        reviews,
        ws,
        user,
        review_id: review.id.to_string(),
        states: Arc::new(tokio::sync::Mutex::new(rows)),
        fake,
        threads,
    }
}

impl Rig {
    async fn review(
        &self,
        attempts: u32,
        cancel: Option<&Arc<AtomicBool>>,
    ) -> crate::session::AgentRunResult {
        run_agent_session_with_recovery(
            &self.manager,
            &self.reviews,
            &self.states,
            &self.ws,
            &self.user,
            "claude",
            "opus",
            self._dir.path().to_str().unwrap(),
            &self.review_id,
            0,
            "Review the diff.",
            Duration::from_secs(600),
            Some(attempts),
            cancel,
            Some("/tmp/otto-review-skills"),
            &[],
            Some(&self.threads),
        )
        .await
    }
    async fn row(&self) -> ReviewAgentState {
        self.reviews
            .get_review(&self.review_id)
            .await
            .unwrap()
            .agents[0]
            .clone()
    }
}

const FINDINGS: &str = r#"[{"path":"src/a.rs","line":3,"severity":"bug","body":"off by one"},
{"path":"src/b.rs","line":9,"severity":"warn","body":"unwrapped error"},
{"lens_score":6,"score_reason":"one real bug"}]"#;

#[tokio::test]
async fn a_reviewer_thread_runs_the_same_prompt_and_files_and_fills_its_row() {
    let r = rig(vec![Act {
        write: Some(FINDINGS.into()),
        ..Default::default()
    }])
    .await;
    let res = r.review(3, None).await;
    assert!(!res.errored);
    assert_eq!(res.findings.len(), 2);
    let row = r.row().await;
    assert_eq!(row.status, "done");
    assert_eq!(row.note, "2 findings");
    assert_eq!(row.comment_count, 2);
    assert_eq!(
        (row.score, row.score_reason.as_str()),
        (Some(6), "one real bug")
    );
    assert!(row.thread, "the row says its session is a thread");
    assert_eq!(row.session_id.as_deref(), Some("thread-1"));
    assert!(row.started_at.is_some() && row.finished_at.is_some());
    let turns = r.fake.turns.lock().unwrap().clone();
    let t = &turns[0];
    assert_eq!(
        (t.provider.as_str(), t.model.as_deref(), t.effort.as_deref()),
        ("claude", Some("opus"), Some("high"))
    );
    assert!(t.read_only && t.continue_thread.is_none() && t.timeout.is_none());
    assert_eq!(
        t.no_progress,
        Some(Duration::from_secs(600)),
        "the reviewer grace"
    );
    let path = findings_path(&r.review_id, 0);
    assert_eq!(t.handoff.as_deref(), Some(path.as_path()));
    // The prompt is the terminal path's, byte for byte.
    assert_eq!(
        t.prompt,
        crate::session::typed_reviewer_prompt("Review the diff.", &path.to_string_lossy(), None)
    );
    assert_eq!(t.title, "Correctness");
    let m = &t.meta;
    assert_eq!(m["source"], "review");
    assert_eq!(m["review_id"], r.review_id.as_str());
    assert_eq!(m["agent_index"], 0);
    assert_eq!(
        (m["read_only"].as_bool(), m["project_settings"].as_bool()),
        (Some(true), Some(false))
    );
    assert_eq!(
        m["extra_dirs"],
        serde_json::json!(["/tmp/otto-review-skills"]),
        "claude loads the skills bundle"
    );
    assert_eq!(m["run_id"], "run-1");
    assert_eq!(m["work"]["kind"], "workflow_run");
    assert_eq!(*r.fake.closed.lock().unwrap(), vec![Id::from("thread-1")]);
}

#[tokio::test]
async fn an_unusable_attempt_is_retried_on_a_fresh_thread() {
    let r = rig(vec![
        Act {
            write: Some("not json".into()),
            ..Default::default()
        },
        Act {
            reply: FINDINGS.into(),
            ..Default::default()
        },
    ])
    .await;
    let res = r.review(2, None).await;
    assert!(!res.errored, "the answer text is the fallback result");
    assert_eq!(r.row().await.note, "2 findings");
    assert_eq!(r.fake.turns.lock().unwrap().len(), 2);
    assert_eq!(r.row().await.session_id.as_deref(), Some("thread-2"));
    assert_eq!(
        r.fake.closed.lock().unwrap().len(),
        2,
        "every attempt's thread is closed"
    );
}

#[tokio::test]
async fn a_reviewer_that_times_out_every_attempt_ends_in_error() {
    let r = rig(vec![Act {
        fail: Some(StepErrorClass::Timeout),
        ..Default::default()
    }])
    .await;
    let res = r.review(1, None).await;
    assert!(res.errored);
    let row = r.row().await;
    assert_eq!(row.status, "error");
    assert_eq!(
        row.note,
        "timed out (grace period elapsed and the agent went quiet): timeout"
    );
    assert!(row.findings.is_empty());
}

#[tokio::test]
async fn a_review_cancel_reaches_the_thread_and_is_not_retried() {
    let r = rig(vec![Act {
        hang: true,
        ..Default::default()
    }])
    .await;
    let flag = Arc::new(AtomicBool::new(false));
    let stop = {
        let flag = flag.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(300)).await;
            flag.store(true, Ordering::SeqCst);
        })
    };
    let res = tokio::time::timeout(Duration::from_secs(5), r.review(3, Some(&flag)))
        .await
        .expect("cancel ends the attempt");
    stop.await.unwrap();
    assert!(res.errored);
    assert_eq!(r.row().await.note, "stopped by user");
    assert_eq!(
        r.fake.turns.lock().unwrap().len(),
        1,
        "a stop is never retried"
    );
}

#[test]
fn runner_events_drive_the_row_in_todays_words() {
    let mut row = ReviewAgentState::default();
    let wait = WaitReason {
        code: "memory".into(),
        text: "Waiting for memory — 2 agents running".into(),
        since: Utc::now(),
        detail: None,
    };
    assert!(apply_event(&mut row, &StepEvent::Waiting(wait)));
    assert_eq!(
        (row.status.as_str(), row.note.as_str()),
        ("pending", "Waiting for memory — 2 agents running")
    );
    assert!(apply_event(
        &mut row,
        &StepEvent::Started {
            thread_id: "t1".into(),
            start_ordinal: 0
        }
    ));
    assert_eq!(
        (row.status.as_str(), row.session_id.as_deref(), row.thread),
        ("running", Some("t1"), true)
    );
    let ask = Attention::new(AttentionKind::Question, "Which base branch?", Utc::now());
    assert!(apply_event(&mut row, &StepEvent::NeedsYou(ask)));
    assert_eq!(
        (row.status.as_str(), row.note.as_str()),
        ("waiting", "needs you: Which base branch?")
    );
    assert!(apply_event(
        &mut row,
        &StepEvent::Resumed {
            request_id: None,
            answer: None,
            by: None
        }
    ));
    assert_eq!(row.status, "running");
    assert!(!apply_event(&mut row, &StepEvent::Working));
}

#[tokio::test]
async fn the_summarizer_turn_on_a_thread_returns_its_validated_result_file() {
    let draft = r#"[{"path":"src/a.rs","line":3,"severity":"bug","body":"off by one"}]"#;
    let r = rig(vec![
        Act {
            write: Some(draft.into()),
            events: vec![StepEvent::Working],
            ..Default::default()
        },
        Act {
            write: Some("{ not an array".into()),
            fail: Some(StepErrorClass::Provider),
            ..Default::default()
        },
    ])
    .await;
    let mut cfg = crate::engine::default_review_config("claude").summarizer;
    (cfg.model, cfg.effort) = ("sonnet".into(), "low".into());
    let attempt = crate::summarizer::Attempt::new("claude", "sonnet").unwrap();
    let (tx, rx) = tokio::sync::oneshot::channel();
    let cwd = r._dir.path().to_str().unwrap();
    let text = r
        .threads
        .summarizer_turn(&attempt, ("brief", &cfg, &r.ws, &r.user, cwd), tx, None)
        .await
        .unwrap();
    assert_eq!(text, draft);
    assert_eq!(
        rx.await.unwrap(),
        "thread-1",
        "ready hears the thread once it exists"
    );
    let t = r.fake.turns.lock().unwrap()[0].clone();
    assert_eq!(
        t.prompt,
        attempt.prompt("brief"),
        "the terminal turn's brief and hand-off"
    );
    assert_eq!(t.handoff, Some(attempt.result_path()));
    assert_eq!(
        (t.model.as_deref(), t.effort.as_deref()),
        (Some("sonnet"), Some("low"))
    );
    assert_eq!(t.meta["source"], "review_summarizer");
    assert_eq!(t.meta["run_id"], "run-1");
    // An unusable result is an error the caller retries or falls back on.
    let second = crate::summarizer::Attempt::new("claude", "").unwrap();
    let (tx, _rx) = tokio::sync::oneshot::channel();
    let err = r
        .threads
        .summarizer_turn(&second, ("brief", &cfg, &r.ws, &r.user, cwd), tx, None)
        .await
        .unwrap_err();
    assert_eq!(err, "provider");
}

#[test]
fn review_rows_become_the_steps_children_and_merge_per_review() {
    let rows = vec![
        ReviewAgentState {
            name: "Correctness · codex".into(),
            provider: "codex".into(),
            model: "gpt-5.5".into(),
            effort: "high".into(),
            status: "waiting".into(),
            note: "needs you: which base?".into(),
            session_id: Some("t-7".into()),
            thread: true,
            started_at: Some(Utc::now()),
            ..Default::default()
        },
        ReviewAgentState {
            name: "Security".into(),
            provider: "claude".into(),
            status: "done".into(),
            note: "3 findings".into(),
            session_id: Some("pty-1".into()),
            ..Default::default()
        },
        ReviewAgentState {
            name: "Summarizer".into(),
            provider: "claude".into(),
            status: "pending".into(),
            ..Default::default()
        },
    ];
    let kids = child_rows("r1", &rows);
    assert_eq!(
        kids.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(),
        ["r1.0", "r1.1", "r1.2"]
    );
    assert_eq!(
        kids.iter().map(|k| k.status.as_str()).collect::<Vec<_>>(),
        ["needs_you", "success", "pending"]
    );
    assert_eq!(kids[0].thread_id.as_deref(), Some("t-7"));
    assert_eq!(
        kids[1].thread_id, None,
        "a terminal session is not a thread"
    );
    assert_eq!(
        (kids[0].model.as_deref(), kids[0].effort.as_deref()),
        (Some("gpt-5.5"), Some("high"))
    );
    assert!(kids[0].started_at.is_some() && kids[2].model.is_none() && kids[2].note.is_none());
    let mut children = child_rows("r0", &rows[..1]);
    assert!(merge_children(&mut children, "r1", kids.clone()));
    assert!(
        !merge_children(&mut children, "r1", kids.clone()),
        "unchanged rows are no change"
    );
    assert!(merge_children(&mut children, "r1", kids[..1].to_vec()));
    assert_eq!(
        children.iter().map(|k| k.id.as_str()).collect::<Vec<_>>(),
        ["r0.0", "r1.0"]
    );
}

#[tokio::test]
async fn a_finished_review_keeps_its_threads_for_the_success_period() {
    let r = rig(vec![]).await;
    let rows = vec![
        ReviewAgentState {
            session_id: Some("t-1".into()),
            thread: true,
            ..Default::default()
        },
        ReviewAgentState {
            session_id: Some("pty-1".into()),
            ..Default::default()
        },
        ReviewAgentState::default(),
    ];
    r.threads.finished(&rows).await;
    let kept = r.fake.kept.lock().unwrap().clone();
    assert_eq!(kept.len(), 1);
    assert_eq!(kept[0].0, vec![Id::from("t-1")], "only thread rows");
    let hours = (kept[0].1 - Utc::now()).num_minutes();
    assert!((23 * 60..=24 * 60).contains(&hours), "{hours} min");
}

#[tokio::test]
async fn after_a_restart_a_held_reviewer_is_awaited_and_only_the_lost_one_re_runs() {
    let r = rig(vec![Act {
        write: Some(FINDINGS.into()),
        ..Default::default()
    }])
    .await;
    let id: Id = r.review_id.clone();
    let row = |name: &str, status: &str, thread: Option<&str>| ReviewAgentState {
        name: name.into(),
        provider: "claude".into(),
        status: status.into(),
        lens: name.into(),
        session_id: thread.map(Into::into),
        thread: thread.is_some(),
        ..Default::default()
    };
    let mut done = row("Done", "done", Some("t-old"));
    done.findings = crate::session::parse_findings(FINDINGS);
    let rows = vec![
        done,
        row("Held", "running", Some("t-held")),
        row("Lost", "running", None),
        row("Summarizer", "running", Some("t-sum")),
    ];
    r.reviews.set_agents(&id, &rows).await.unwrap();
    r.reviews
        .set_agent_prompt(&id, 2, "Review the diff.")
        .await
        .unwrap();
    let findings = crate::thread_resume::resume_reviewers(crate::thread_resume::ResumeCtx {
        manager: r.manager.clone(),
        threads: r.threads.clone(),
        reviews: r.reviews.clone(),
        ws: r.ws.clone(),
        user: r.user.clone(),
        cwd: r._dir.path().to_string_lossy().into_owned(),
        review_id: r.review_id.clone(),
        grace: Duration::from_secs(600),
        skills_dir: None,
        cancel: None,
        agent_cancels: Default::default(),
    })
    .await
    .unwrap();
    assert_eq!(
        *r.fake.resumed.lock().unwrap(),
        vec![Id::from("t-held")],
        "the held thread is awaited, not re-run"
    );
    let turns = r.fake.turns.lock().unwrap().clone();
    assert_eq!(
        turns.len(),
        2,
        "one resume + one fresh run for the lost row"
    );
    assert_eq!(turns[1].title, "Lost");
    assert!(
        turns[1].prompt.contains("Review the diff."),
        "re-run from its persisted prompt"
    );
    let after = r.reviews.get_review(&id).await.unwrap().agents;
    assert_eq!(
        after.iter().map(|a| a.status.as_str()).collect::<Vec<_>>(),
        ["done", "done", "done", "running"]
    );
    assert_eq!(after[1].session_id.as_deref(), Some("t-held"));
    assert_eq!(
        findings.iter().map(Vec::len).collect::<Vec<_>>(),
        [2, 2, 2],
        "every reviewer's findings for the summarizer"
    );
    assert!(
        r.fake.closed.lock().unwrap().contains(&Id::from("t-sum")),
        "the old summarizer thread is closed"
    );
}

/// CR1-029: a reviewer resumed after a restart registers its per-agent cancel
/// flag just like a fresh one, so pressing Stop on it stops only that row,
/// keeps its status "error" / "stopped by user", and never falls through to
/// re-run it — while sibling reviewers finish normally.
#[tokio::test]
async fn stopping_a_resumed_reviewer_keeps_it_stopped_and_never_re_runs_it() {
    use otto_core::cancel::{CancelRegistry, CancelRegistryExt};
    let r = rig(vec![
        Act {
            hang: true,
            ..Default::default()
        },
        Act {
            write: Some(FINDINGS.into()),
            ..Default::default()
        },
    ])
    .await;
    let id: Id = r.review_id.clone();
    let row = |name: &str, thread: Option<&str>| ReviewAgentState {
        name: name.into(),
        provider: "claude".into(),
        status: "running".into(),
        lens: name.into(),
        session_id: thread.map(Into::into),
        thread: thread.is_some(),
        ..Default::default()
    };
    let rows = vec![
        row("Stopped", Some("t-stop")),
        row("Sibling", Some("t-sib")),
        row("Summarizer", None),
    ];
    r.reviews.set_agents(&id, &rows).await.unwrap();
    r.reviews
        .set_agent_prompt(&id, 0, "Review the diff.")
        .await
        .unwrap();
    let agent_cancels: CancelRegistry = Default::default();
    let review_cancel = Arc::new(AtomicBool::new(false));
    let key0 = crate::engine::review_agent_cancel_key(&id, 0);
    let key1 = crate::engine::review_agent_cancel_key(&id, 1);

    let stopper = {
        let cancels = agent_cancels.clone();
        let reviews = r.reviews.clone();
        let id = id.clone();
        let key0 = key0.clone();
        tokio::spawn(async move {
            let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
            while !cancels.is_live(&key0) {
                if tokio::time::Instant::now() >= deadline {
                    return false;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            let signalled = cancels.signal(&key0);
            let mut stopped = reviews.get_review(&id).await.unwrap().agents[0].clone();
            stopped.status = "error".into();
            stopped.note = "stopped by user".into();
            reviews.set_agent_at(&id, 0, &stopped).await.unwrap();
            signalled
        })
    };

    let findings = tokio::time::timeout(
        Duration::from_secs(5),
        crate::thread_resume::resume_reviewers(crate::thread_resume::ResumeCtx {
            manager: r.manager.clone(),
            threads: r.threads.clone(),
            reviews: r.reviews.clone(),
            ws: r.ws.clone(),
            user: r.user.clone(),
            cwd: r._dir.path().to_string_lossy().into_owned(),
            review_id: r.review_id.clone(),
            grace: Duration::from_secs(600),
            skills_dir: None,
            cancel: Some(review_cancel),
            agent_cancels: agent_cancels.clone(),
        }),
    )
    .await
    .expect("stopping a resumed reviewer must end it promptly")
    .unwrap();

    assert!(
        stopper.await.unwrap(),
        "per-agent cancel flag must be registered while the resumed reviewer runs"
    );
    let after = r.reviews.get_review(&id).await.unwrap().agents;
    assert_eq!(after[0].status, "error");
    assert_eq!(after[0].note, "stopped by user");
    assert!(after[0].findings.is_empty());
    assert_eq!(after[1].status, "done");
    assert_eq!(after[1].findings.len(), 2);
    assert_eq!(
        findings.iter().map(Vec::len).collect::<Vec<_>>(),
        [0, 2],
        "stopped row contributes no findings; sibling keeps its findings"
    );
    assert_eq!(
        r.fake.turns.lock().unwrap().len(),
        2,
        "stopped row must not fall through and re-run on a fresh thread"
    );
    assert!(
        !agent_cancels.is_live(&key0) && !agent_cancels.is_live(&key1),
        "per-agent cancel guards unregister once resume_reviewers finishes"
    );
}

/// CR1-029 race: when Stop closes the held thread before the 200 ms cancel poll
/// fires (so `drive` settles with a non-Canceled error), the tripped per-agent
/// flag must still keep the row "stopped by user" instead of falling through to
/// re-run the reviewer and overwriting the row with "done".
#[tokio::test]
async fn stopping_a_resumed_reviewer_when_its_thread_closes_first_never_overwrites_stopped() {
    use otto_core::cancel::{CancelRegistry, CancelRegistryExt};
    // Turn 1 (resumed held thread): fails as non-Canceled (closed by Stop).
    // Turn 2 (would-be fallback re-run): writes valid findings that would
    // overwrite "stopped by user" with "done" if resume_row fell through.
    let r = rig(vec![
        Act {
            fail: Some(StepErrorClass::Provider),
            ..Default::default()
        },
        Act {
            write: Some(FINDINGS.into()),
            ..Default::default()
        },
    ])
    .await;
    let id: Id = r.review_id.clone();
    let rows = vec![
        ReviewAgentState {
            name: "Stopped".into(),
            provider: "claude".into(),
            status: "running".into(),
            lens: "Stopped".into(),
            session_id: Some("t-stop".into()),
            thread: true,
            ..Default::default()
        },
        ReviewAgentState {
            name: "Summarizer".into(),
            provider: "claude".into(),
            status: "pending".into(),
            ..Default::default()
        },
    ];
    r.reviews.set_agents(&id, &rows).await.unwrap();
    r.reviews
        .set_agent_prompt(&id, 0, "Review the diff.")
        .await
        .unwrap();
    // Pre-trip review cancel? No — simulate per-agent Stop by tripping the
    // review-wide flag=false while the per-agent flag is tripped. Wait until
    // drive returns? Even tighter: wrap the fake runner so the first resume
    // call signals the per-agent cancel flag and writes "stopped by user"
    // right as the thread closes with Provider error.
    struct StopOnClose {
        inner: Arc<Fake>,
        cancels: CancelRegistry,
        reviews: ReviewsRepo,
        review_id: Id,
    }
    #[async_trait]
    impl StepRunner for StopOnClose {
        async fn run(
            &self,
            t: StepTurn,
            ev: tokio::sync::mpsc::UnboundedSender<StepEvent>,
            c: CancelSignal,
        ) -> otto_core::Result<StepOutcome> {
            self.inner.run(t, ev, c).await
        }
        async fn resume(
            &self,
            id: &Id,
            ord: u32,
            t: &StepTurn,
            ev: tokio::sync::mpsc::UnboundedSender<StepEvent>,
            c: CancelSignal,
        ) -> otto_core::Result<StepOutcome> {
            let key = crate::engine::review_agent_cancel_key(&self.review_id, 0);
            self.cancels.signal(&key);
            let mut stopped = self
                .reviews
                .get_review(&self.review_id)
                .await
                .unwrap()
                .agents[0]
                .clone();
            stopped.status = "error".into();
            stopped.note = "stopped by user".into();
            self.reviews
                .set_agent_at(&self.review_id, 0, &stopped)
                .await
                .unwrap();
            self.inner.resume(id, ord, t, ev, c).await
        }
        async fn close(&self, id: &Id, until: chrono::DateTime<Utc>) {
            self.inner.close(id, until).await;
        }
        async fn set_retain_until(&self, ids: &[Id], at: chrono::DateTime<Utc>) {
            self.inner.set_retain_until(ids, at).await;
        }
        async fn release_process(&self, id: &Id) {
            self.inner.release_process(id).await;
        }
        async fn delete(&self, id: &Id) {
            self.inner.delete(id).await;
        }
    }

    let agent_cancels: CancelRegistry = Default::default();
    let threads = Threads {
        runner: Arc::new(StopOnClose {
            inner: r.fake.clone(),
            cancels: agent_cancels.clone(),
            reviews: r.reviews.clone(),
            review_id: id.clone(),
        }),
        ..r.threads.clone()
    };
    let findings = crate::thread_resume::resume_reviewers(crate::thread_resume::ResumeCtx {
        manager: r.manager.clone(),
        threads,
        reviews: r.reviews.clone(),
        ws: r.ws.clone(),
        user: r.user.clone(),
        cwd: r._dir.path().to_string_lossy().into_owned(),
        review_id: r.review_id.clone(),
        grace: Duration::from_secs(600),
        skills_dir: None,
        cancel: Some(Arc::new(AtomicBool::new(false))),
        agent_cancels: agent_cancels.clone(),
    })
    .await
    .unwrap();

    let after = r.reviews.get_review(&id).await.unwrap().agents;
    assert_eq!(after[0].status, "error");
    assert_eq!(after[0].note, "stopped by user");
    assert!(findings[0].is_empty());
    assert_eq!(
        r.fake.turns.lock().unwrap().len(),
        1,
        "must not fall through to re-run after per-agent Stop"
    );
}

#[tokio::test]
async fn after_a_restart_with_all_reviewers_done_a_held_summarizer_thread_is_not_closed() {
    let r = rig(vec![]).await;
    let id: Id = r.review_id.clone();
    let mut done = ReviewAgentState {
        name: "Done".into(),
        provider: "claude".into(),
        status: "done".into(),
        lens: "Done".into(),
        session_id: Some("t-rev".into()),
        thread: true,
        ..Default::default()
    };
    done.findings = crate::session::parse_findings(FINDINGS);
    let summarizer = ReviewAgentState {
        name: "Summarizer".into(),
        provider: "claude".into(),
        status: "running".into(),
        session_id: Some("t-sum".into()),
        thread: true,
        ..Default::default()
    };
    r.reviews
        .set_agents(&id, &[done, summarizer])
        .await
        .unwrap();
    let findings = crate::thread_resume::resume_reviewers(crate::thread_resume::ResumeCtx {
        manager: r.manager.clone(),
        threads: r.threads.clone(),
        reviews: r.reviews.clone(),
        ws: r.ws.clone(),
        user: r.user.clone(),
        cwd: r._dir.path().to_string_lossy().into_owned(),
        review_id: r.review_id.clone(),
        grace: Duration::from_secs(600),
        skills_dir: None,
        cancel: None,
        agent_cancels: Default::default(),
    })
    .await
    .unwrap();
    assert_eq!(findings.len(), 1);
    assert_eq!(findings[0].len(), 2);
    assert!(
        !r.fake.closed.lock().unwrap().contains(&Id::from("t-sum")),
        "when all reviewers already finished, the held summarizer thread must stay open to be awaited"
    );
}

#[tokio::test]
async fn after_a_restart_a_resumed_reviewer_carries_its_persisted_prompt_for_zero_turn_threads() {
    let r = rig(vec![Act {
        write: Some(FINDINGS.into()),
        ..Default::default()
    }])
    .await;
    let id: Id = r.review_id.clone();
    let rows = vec![
        ReviewAgentState {
            name: "Held".into(),
            provider: "claude".into(),
            status: "running".into(),
            lens: "Held".into(),
            session_id: Some("t-held".into()),
            thread: true,
            ..Default::default()
        },
        ReviewAgentState {
            name: "Summarizer".into(),
            provider: "claude".into(),
            status: "pending".into(),
            ..Default::default()
        },
    ];
    r.reviews.set_agents(&id, &rows).await.unwrap();
    r.reviews
        .set_agent_prompt(&id, 0, "Review the diff carefully.")
        .await
        .unwrap();
    let _ = crate::thread_resume::resume_reviewers(crate::thread_resume::ResumeCtx {
        manager: r.manager.clone(),
        threads: r.threads.clone(),
        reviews: r.reviews.clone(),
        ws: r.ws.clone(),
        user: r.user.clone(),
        cwd: r._dir.path().to_string_lossy().into_owned(),
        review_id: r.review_id.clone(),
        grace: Duration::from_secs(600),
        skills_dir: None,
        cancel: None,
        agent_cancels: Default::default(),
    })
    .await
    .unwrap();
    let turns = r.fake.turns.lock().unwrap().clone();
    assert_eq!(turns.len(), 1);
    assert!(
        turns[0].prompt.contains("Review the diff carefully."),
        "StepRunner::resume needs the real prompt if restart landed before turn 1 went out; got: {:?}",
        turns[0].prompt
    );
}
