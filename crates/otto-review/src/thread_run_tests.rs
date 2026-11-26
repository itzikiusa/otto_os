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
}

#[async_trait]
impl StepRunner for Fake {
    async fn run(
        &self,
        t: StepTurn,
        ev: tokio::sync::mpsc::UnboundedSender<StepEvent>,
        cancel: CancelSignal,
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
        let thread_id: Id = format!("thread-{n}").into();
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
    async fn resume(
        &self,
        _: &Id,
        _: u32,
        t: &StepTurn,
        ev: tokio::sync::mpsc::UnboundedSender<StepEvent>,
        c: CancelSignal,
    ) -> otto_core::Result<StepOutcome> {
        self.run(t.clone(), ev, c).await
    }
    async fn close(&self, id: &Id, _: chrono::DateTime<Utc>) {
        self.closed.lock().unwrap().push(id.clone());
    }
    async fn set_retain_until(&self, _: &[Id], _: chrono::DateTime<Utc>) {}
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
            .get_review(&self.review_id.clone().into())
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
        "timed out (grace period elapsed and the agent went quiet)"
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
