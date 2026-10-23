//! The shared assist job against a fake document and a fake turn runner.

use super::*;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use otto_core::domain::{User, Workspace};

fn slots() -> &'static AssistSlots {
    Box::leak(Box::default())
}

fn fixture_ws_user() -> (Workspace, User) {
    let now = chrono::Utc::now();
    (
        Workspace {
            id: "ws".into(),
            name: "ws".into(),
            root_path: "/tmp".into(),
            settings: serde_json::Value::Null,
            archived: false,
            created_at: now,
        },
        User {
            id: "alice".into(),
            username: "alice".into(),
            display_name: "Alice".into(),
            is_root: false,
            disabled: false,
            created_at: now,
        },
    )
}

/// The live previews the job pushed, plus a wake-up for whoever waits on
/// one: the fake turn waits for the preview instead of sleeping and hoping
/// the 5 ms poll won a race against it (test-quality-6w2-03).
#[derive(Default)]
struct Previews {
    seen: Mutex<Vec<String>>,
    arrived: tokio::sync::Notify,
}

impl Previews {
    fn push(&self, text: String) {
        self.seen.lock().unwrap().push(text);
        self.arrived.notify_waiters();
    }

    fn get(&self) -> Vec<String> {
        self.seen.lock().unwrap().clone()
    }

    /// Resolve once at least one preview landed. Registers interest before
    /// checking, so a preview pushed in between still wakes it.
    async fn first(&self) {
        loop {
            let arrived = self.arrived.notified();
            tokio::pin!(arrived);
            arrived.as_mut().enable();
            if !self.get().is_empty() {
                return;
            }
            arrived.await;
        }
    }

    /// [`Self::first`] with a generous cap: a missing preview fails the test
    /// with a message instead of hanging it.
    async fn wait_first(&self) {
        if tokio::time::timeout(Duration::from_secs(5), self.first())
            .await
            .is_err()
        {
            panic!("the live preview never arrived");
        }
    }
}

/// A document in a temp file; records what the job did to it.
struct FakeDoc {
    key: String,
    file: PathBuf,
    parse_fails: bool,
    cap: Option<Duration>,
    ws: Workspace,
    user: User,
    live: Arc<Previews>,
    parsed: AtomicUsize,
    applied: AtomicUsize,
    states: Mutex<Vec<String>>,
    _dir: tempfile::TempDir,
}

impl FakeDoc {
    fn new(key: &str) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let (ws, user) = fixture_ws_user();
        Self {
            key: key.into(),
            file: dir.path().join("doc.txt"),
            parse_fails: false,
            cap: None,
            ws,
            user,
            live: Arc::default(),
            parsed: AtomicUsize::new(0),
            applied: AtomicUsize::new(0),
            states: Mutex::default(),
            _dir: dir,
        }
    }

    fn live(&self) -> Vec<String> {
        self.live.get()
    }

    fn states(&self) -> Vec<String> {
        self.states.lock().unwrap().clone()
    }
}

impl AssistTarget for FakeDoc {
    type Proposal = String;
    type Applied = String;

    fn busy_key(&self) -> &str {
        &self.key
    }
    async fn load(&self) -> Result<()> {
        tokio::fs::write(&self.file, "base")
            .await
            .map_err(|e| Error::Internal(e.to_string()))
    }
    fn prompt(&self) -> String {
        "draw it".into()
    }
    fn turn<'a>(&'a self, prompt: &'a str) -> AgentTurn<'a> {
        AgentTurn {
            ws: &self.ws,
            user: &self.user,
            existing: None,
            title: "fake",
            cwd: "/tmp",
            provider: "fake",
            meta: serde_json::Value::Null,
            prompt,
            stuck_after: Duration::from_secs(60),
            done_file: None,
            quiet_done: None,
        }
    }
    fn turn_cap(&self) -> Option<Duration> {
        self.cap
    }
    fn live_path(&self) -> Option<PathBuf> {
        Some(self.file.clone())
    }
    fn live_cap(&self) -> usize {
        1024
    }
    fn live_base(&self) -> Vec<u8> {
        b"base".to_vec()
    }
    fn live_every(&self) -> Duration {
        Duration::from_millis(5)
    }
    fn on_live(&self, bytes: &[u8]) {
        self.live.push(String::from_utf8_lossy(bytes).into());
    }
    async fn parse(&self, reply: &str) -> Result<String> {
        self.parsed.fetch_add(1, Ordering::SeqCst);
        if self.parse_fails {
            return Err(Error::Invalid("the agent produced no diagram".into()));
        }
        Ok(reply.to_uppercase())
    }
    async fn apply(&self, proposal: String, _session: &Id) -> Result<String> {
        self.applied.fetch_add(1, Ordering::SeqCst);
        Ok(proposal)
    }
    fn report(&self, state: AssistState<'_>) {
        let name = match state {
            AssistState::Running { .. } => "running".to_string(),
            AssistState::Applying => "applying".into(),
            AssistState::Applied => "applied".into(),
            AssistState::Stopped => "stopped".into(),
            AssistState::Failed { stage, error } => format!("failed:{stage:?}:{error}"),
        };
        self.states.lock().unwrap().push(name);
    }
}

/// What the fake agent does during its turn.
enum Script {
    /// Edit the file once, then reply.
    Edit,
    /// Edit, get stopped by the owner, keep editing, then reply.
    StoppedMidTurn(&'static AssistSlots),
    /// Never finish.
    Hang,
}

struct FakeRunner {
    file: PathBuf,
    /// The document's previews: the turn waits on the first one.
    previews: Arc<Previews>,
    script: Script,
    calls: AtomicUsize,
    killed: Mutex<Vec<Id>>,
}

impl FakeRunner {
    fn new(doc: &FakeDoc, script: Script) -> Self {
        Self {
            file: doc.file.clone(),
            previews: Arc::clone(&doc.live),
            script,
            calls: AtomicUsize::new(0),
            killed: Mutex::default(),
        }
    }
}

/// How long a NEGATIVE check waits for something that must not happen (an
/// edit after the stop or after the job reaching the preview). Positive
/// checks wait on [`Previews::wait_first`] instead.
const SETTLE: Duration = Duration::from_millis(60);

impl TurnRunner for FakeRunner {
    async fn run_turn<F: FnOnce(&Id) + Send>(
        &self,
        _turn: AgentTurn<'_>,
        on_ready: F,
    ) -> Result<(String, Id)> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        let sid: Id = "sid-1".into();
        on_ready(&sid);
        tokio::fs::write(&self.file, "draft one").await.unwrap();
        // The turn ends only after the poll previewed the edit: a starved
        // runtime used to let the turn's sleep win the job's `select!` first.
        self.previews.wait_first().await;
        match &self.script {
            Script::Edit => {}
            Script::StoppedMidTurn(slots) => {
                assert!(slots.stop("doc"), "the job holds the claim");
                tokio::fs::write(&self.file, "a longer draft after the stop")
                    .await
                    .unwrap();
                tokio::time::sleep(SETTLE).await;
            }
            Script::Hang => std::future::pending::<()>().await,
        }
        Ok(("reply".into(), sid))
    }

    fn kill_session(&self, sid: &Id) -> impl Future<Output = ()> + Send {
        self.killed.lock().unwrap().push(sid.clone());
        async {}
    }
}

/// After the job returned, another edit must not reach the live preview.
async fn assert_poll_stopped(doc: &FakeDoc) {
    let before = doc.live();
    tokio::fs::write(&doc.file, "an edit after the job settled")
        .await
        .unwrap();
    tokio::time::sleep(SETTLE).await;
    assert_eq!(doc.live(), before, "the live poll outlived the job");
}

#[tokio::test]
async fn success_applies_once_and_stops_the_live_poll() {
    let slots = slots();
    let doc = FakeDoc::new("doc");
    let runner = FakeRunner::new(&doc, Script::Edit);
    let claim = slots.claim("doc").unwrap();
    let outcome = run_assist_job(&runner, &doc, &claim).await;
    match outcome {
        AssistOutcome::Applied { applied, session } => {
            assert_eq!(applied, "REPLY");
            assert_eq!(session, "sid-1");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(doc.applied.load(Ordering::SeqCst), 1);
    assert_eq!(
        doc.live(),
        ["draft one"],
        "the live edit was previewed once"
    );
    assert_eq!(doc.states(), ["running", "applying", "applied"]);
    assert_poll_stopped(&doc).await;
}

#[tokio::test]
async fn stop_mid_turn_applies_nothing_and_stops_the_poll() {
    let slots = slots();
    let doc = FakeDoc::new("doc");
    let runner = FakeRunner::new(&doc, Script::StoppedMidTurn(slots));
    let claim = slots.claim("doc").unwrap();
    let outcome = run_assist_job(&runner, &doc, &claim).await;
    assert!(
        matches!(&outcome, AssistOutcome::Stopped { session: Some(s) } if s == "sid-1"),
        "{outcome:?}"
    );
    assert_eq!(
        doc.parsed.load(Ordering::SeqCst),
        0,
        "a stopped reply is never parsed"
    );
    assert_eq!(doc.applied.load(Ordering::SeqCst), 0);
    // The edit after the stop never reached the preview.
    assert_eq!(doc.live(), ["draft one"]);
    assert_eq!(doc.states(), ["running", "stopped"]);
    assert_poll_stopped(&doc).await;
}

#[tokio::test]
async fn parse_failure_applies_nothing_and_reports_the_error() {
    let slots = slots();
    let mut doc = FakeDoc::new("doc");
    doc.parse_fails = true;
    let runner = FakeRunner::new(&doc, Script::Edit);
    let claim = slots.claim("doc").unwrap();
    let outcome = run_assist_job(&runner, &doc, &claim).await;
    match outcome {
        AssistOutcome::Failed {
            stage: Stage::Parse,
            error: Error::Invalid(msg),
            session: Some(s),
        } => {
            assert_eq!(msg, "the agent produced no diagram");
            assert_eq!(s, "sid-1");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(doc.applied.load(Ordering::SeqCst), 0);
    assert_eq!(
        doc.states(),
        [
            "running",
            "failed:Parse:invalid: the agent produced no diagram"
        ]
    );
}

#[tokio::test]
async fn a_second_claim_while_busy_is_refused_and_stop_flags_only_the_holder() {
    let slots = slots();
    assert!(!slots.stop("doc"), "nothing running");
    let first = slots.claim("doc").expect("first claim");
    assert!(
        slots.claim("doc").is_none(),
        "second concurrent claim refused"
    );
    assert!(
        slots.claim("other").is_some(),
        "other documents are independent"
    );
    assert!(slots.stop("doc"));
    assert!(first.stopped());
    drop(first);
    let next = slots.claim("doc").expect("released on drop");
    assert!(!next.stopped(), "a new job starts unflagged");
}

#[tokio::test]
async fn a_job_without_its_documents_claim_runs_nothing() {
    let slots = slots();
    let doc = FakeDoc::new("doc");
    let runner = FakeRunner::new(&doc, Script::Edit);
    let wrong = slots.claim("another-doc").unwrap();
    let outcome = run_assist_job(&runner, &doc, &wrong).await;
    assert!(
        matches!(
            outcome,
            AssistOutcome::Failed {
                stage: Stage::Load,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
    assert!(!doc.file.exists(), "load never ran");
}

#[tokio::test]
async fn an_overrun_turn_is_killed_and_applies_nothing() {
    let slots = slots();
    let mut doc = FakeDoc::new("doc");
    doc.cap = Some(Duration::from_millis(150));
    let runner = FakeRunner::new(&doc, Script::Hang);
    let claim = slots.claim("doc").unwrap();
    let outcome = run_assist_job(&runner, &doc, &claim).await;
    match outcome {
        AssistOutcome::Failed {
            stage: Stage::Deadline,
            error: Error::Upstream(msg),
            session: Some(s),
        } => {
            assert_eq!(msg, "the agent turn exceeded 0 minutes and was stopped");
            assert_eq!(s, "sid-1");
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(*runner.killed.lock().unwrap(), ["sid-1"]);
    assert_eq!(doc.applied.load(Ordering::SeqCst), 0);
}

/// S4-12: a dropped request (client disconnect / reload) drops the job, and
/// the poll lives inside it — it used to be a spawned task that leaked,
/// re-broadcasting the pre-turn document on every later change.
#[tokio::test]
async fn a_dropped_job_stops_its_live_poll() {
    let slots = slots();
    let doc = FakeDoc::new("doc");
    let runner = FakeRunner::new(&doc, Script::Hang);
    let claim = slots.claim("doc").unwrap();
    let job = run_assist_job(&runner, &doc, &claim);
    // Drop the (hanging) job once it previewed, not after a fixed window.
    tokio::select! {
        outcome = job => panic!("a hanging turn settled: {outcome:?}"),
        () = doc.live.wait_first() => {}
    }
    assert_eq!(doc.live(), ["draft one"], "it previewed while running");
    assert_poll_stopped(&doc).await;
    assert_eq!(doc.applied.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_failed_load_never_starts_the_turn() {
    let slots = slots();
    let mut doc = FakeDoc::new("doc");
    doc.file = doc.file.join("missing-dir").join("doc.txt");
    let runner = FakeRunner::new(&doc, Script::Edit);
    let claim = slots.claim("doc").unwrap();
    let outcome = run_assist_job(&runner, &doc, &claim).await;
    assert!(
        matches!(
            outcome,
            AssistOutcome::Failed {
                stage: Stage::Load,
                session: None,
                ..
            }
        ),
        "{outcome:?}"
    );
    assert_eq!(runner.calls.load(Ordering::SeqCst), 0);
    assert_eq!(doc.applied.load(Ordering::SeqCst), 0);
}
