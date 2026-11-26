//! The in-process scripted driver (test plan §11.2): every provider's thread
//! acts out the same `OTTO_FAKE:` scripts as the fake binaries
//! ([`crate::fake_script`]) with no agent CLI behind it, so an otto-server
//! suite can run a twelve-node workflow with twelve reviewers in about a
//! second. Its child is a plain `/bin/sh` used only as a timer (`sleep`,
//! `background`) and to die on `fail crash`; spawn, handshake and holders of
//! the real CLIs are proven against the fake binaries instead.
//!
//! What it cannot act out: `child` / `finish-child` (no MCP client) — they
//! answer "unsupported: <verb>". A prompt without a sentinel (and no persona)
//! answers `echo: <text>`.

use std::sync::Arc;

use otto_core::agent_runtime::{
    ApprovalDecision, ApprovalOption, BackgroundTask, ProviderFailure, QuestionKind, RequestKind,
    RuntimePromptReq, RuntimeProvider, RuntimeQuestion, TurnItemKind, TurnItemStatus,
    TurnTerminalStatus, TurnTokenUsage,
};

use crate::driver::{
    item, DriverError, DriverEvent, DriverFactory, DriverStep, LaunchContext, LaunchSpec,
    StructuredDriver,
};
use crate::fake_script::{Config, FailClass, Step, Turn};

/// The verbs only a process with an MCP client can act out.
const UNSUPPORTED: &[&str] = &["child", "finish-child"];

enum Pending {
    Wake(u64),
    Ask(String),
    Approve(String, String),
    Hang,
}

pub struct ScriptedDriver {
    provider: RuntimeProvider,
    cfg: Config,
    session: String,
    cwd: std::path::PathBuf,
    turn_id: Option<String>,
    run: Option<Turn>,
    pending: Option<Pending>,
    /// A `background` turn's rest, owed when its timer fires.
    owed: Option<(u64, Turn)>,
    timers: u64,
    turns: u32,
    items: u32,
}

impl ScriptedDriver {
    /// `cfg` is the binaries' environment as a value (time scale, scripts
    /// folder, personas, log file); its `provider` and `unsupported` are set here.
    pub fn new(provider: RuntimeProvider, cfg: &Config) -> Self {
        Self {
            provider,
            cfg: Config {
                provider: provider.as_str().to_string(),
                unsupported: UNSUPPORTED,
                ..cfg.clone()
            },
            session: String::new(),
            cwd: std::path::PathBuf::new(),
            turn_id: None,
            run: None,
            pending: None,
            owed: None,
            timers: 0,
            turns: 0,
            items: 0,
        }
    }

    /// A driver factory that scripts every provider.
    pub fn factory(cfg: Config) -> Arc<dyn DriverFactory> {
        Arc::new(
            move |p: RuntimeProvider| -> Option<Box<dyn StructuredDriver>> {
                Some(Box::new(ScriptedDriver::new(p, &cfg)))
            },
        )
    }

    fn timer(&mut self, step: &mut DriverStep, d: std::time::Duration, word: &str) -> u64 {
        self.timers += 1;
        let t = self.timers;
        step.write(format!("(sleep {:.3}; echo {word} {t}) &", d.as_secs_f64()));
        t
    }

    fn end(
        &mut self,
        step: &mut DriverStep,
        status: TurnTerminalStatus,
        failure: Option<ProviderFailure>,
    ) {
        if let Some(run) = self.run.take() {
            run.log();
        }
        self.pending = None;
        self.turn_id = None;
        step.push(DriverEvent::Terminal {
            status,
            failure,
            reason: None,
        });
    }

    /// Run the script until it waits on something or the turn ends.
    fn advance(&mut self, step: &mut DriverStep) {
        loop {
            let Some(run) = self.run.as_mut() else { return };
            match run.next_step() {
                Step::Say(text) => {
                    self.items += 1;
                    let id = format!("scripted-msg-{}", self.items);
                    step.push(DriverEvent::TurnItem(item(
                        id.clone(),
                        TurnItemStatus::Completed,
                        None,
                        TurnItemKind::AssistantMessage {
                            message_id: id,
                            text,
                            streaming: false,
                        },
                    )));
                }
                Step::Wait(d) => {
                    let t = self.timer(step, d, "wake");
                    self.pending = Some(Pending::Wake(t));
                    return;
                }
                Step::Ask(q) => {
                    self.items += 1;
                    let id = format!("scripted-ask-{}", self.items);
                    step.push(DriverEvent::Question {
                        id: id.clone(),
                        title: None,
                        questions: vec![RuntimeQuestion {
                            id: "q1".into(),
                            header: "Question".into(),
                            prompt: q,
                            kind: QuestionKind::Text,
                            required: true,
                            allow_empty: false,
                            allow_free_text: true,
                            options: Vec::new(),
                            open_url: None,
                        }],
                        tool_item_id: None,
                        for_person: false,
                    });
                    self.pending = Some(Pending::Ask(id));
                    return;
                }
                Step::Approve(tool) => {
                    self.items += 1;
                    let id = format!("scripted-approve-{}", self.items);
                    step.push(DriverEvent::Request {
                        id: id.clone(),
                        kind: RequestKind::Command,
                        title: Some(tool.clone()),
                        prompt: None,
                        reason: None,
                        options: vec![
                            ApprovalOption {
                                decision: ApprovalDecision::Accept,
                                label: "Allow".into(),
                            },
                            ApprovalOption {
                                decision: ApprovalDecision::Decline,
                                label: "Deny".into(),
                            },
                        ],
                        tool_item_id: None,
                    });
                    self.pending = Some(Pending::Approve(id, tool));
                    return;
                }
                Step::Usage { input, output } => step.push(DriverEvent::Usage(TurnTokenUsage {
                    input_tokens: input,
                    output_tokens: output,
                    cached_input_tokens: None,
                    reasoning_tokens: None,
                    cost_usd: None,
                })),
                Step::Fail(FailClass::Crash, _) => {
                    if let Some(run) = self.run.take() {
                        run.log();
                    }
                    step.write("exit 3".into());
                    return;
                }
                Step::Fail(FailClass::Hang, _) => {
                    self.pending = Some(Pending::Hang);
                    return;
                }
                Step::Fail(_, message) => {
                    let failure = ProviderFailure {
                        class: "provider_error".into(),
                        message,
                        code: None,
                        retryable: None,
                    };
                    return self.end(step, TurnTerminalStatus::Failed, Some(failure));
                }
                Step::Background(after) => {
                    let mut run = self.run.take().expect("running");
                    run.log();
                    let t = self.timer(step, after, "owed");
                    step.push(DriverEvent::BackgroundTasks(vec![BackgroundTask {
                        id: format!("scripted-bg-{t}"),
                        kind: "command".into(),
                        label: "background work".into(),
                        started_at: chrono::Utc::now(),
                        item_id: None,
                        due_at: None,
                        stopping: false,
                        stop_error: None,
                    }]));
                    self.turns += 1;
                    run.resumed(self.turns);
                    self.owed = Some((t, run));
                    self.pending = None;
                    self.turn_id = None;
                    step.push(DriverEvent::Terminal {
                        status: TurnTerminalStatus::Completed,
                        failure: None,
                        reason: None,
                    });
                    return;
                }
                Step::Mcp { .. } => run.mcp_answered(Err("no MCP client in process".into())),
                Step::Done => return self.end(step, TurnTerminalStatus::Completed, None),
            }
        }
    }
}

impl StructuredDriver for ScriptedDriver {
    fn provider(&self) -> RuntimeProvider {
        self.provider
    }

    /// A shell reading commands from stdin; the host's injected flags land in
    /// the outer `-c`'s positional parameters, unread.
    fn launch(&self, _ctx: &LaunchContext) -> LaunchSpec {
        LaunchSpec {
            program: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                "exec /bin/sh -s".into(),
                "otto-scripted".into(),
            ],
            ..LaunchSpec::default()
        }
    }

    fn start(&mut self, ctx: &LaunchContext) -> DriverStep {
        self.session = ctx.session_id.clone();
        self.cwd = ctx.cwd.clone().into();
        let mut step = DriverStep::default();
        step.push(DriverEvent::Ready {
            native_thread_id: Some(
                ctx.resume
                    .clone()
                    .unwrap_or_else(|| format!("scripted-{}", ctx.session_id)),
            ),
            model: ctx.model.clone(),
        });
        step
    }

    fn prompt(&mut self, turn_id: &str, req: &RuntimePromptReq) -> Result<DriverStep, DriverError> {
        if self.turn_id.is_some() {
            return Err(DriverError("a prompt is already running".into()));
        }
        self.turns += 1;
        self.turn_id = Some(turn_id.to_string());
        self.run = Some(
            Turn::start(&req.text, &self.cwd, &self.session, self.turns, &self.cfg).unwrap_or_else(
                || {
                    let echo = format!("reply echo: {}", req.text.replace('\n', " "));
                    Turn::start(
                        &format!("OTTO_FAKE: {echo}"),
                        &self.cwd,
                        &self.session,
                        self.turns,
                        &self.cfg,
                    )
                    .expect("a sentinel prompt")
                },
            ),
        );
        let mut step = DriverStep::default();
        self.advance(&mut step);
        Ok(step)
    }

    fn interrupt(&mut self) -> DriverStep {
        let mut step = DriverStep::default();
        if self.turn_id.is_some() {
            self.end(&mut step, TurnTerminalStatus::Interrupted, None);
        }
        step
    }

    fn respond(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<DriverStep, DriverError> {
        let Some(Pending::Approve(id, tool)) = self.pending.take() else {
            return Err(DriverError(format!("no pending request {request_id}")));
        };
        if id != request_id {
            self.pending = Some(Pending::Approve(id, tool));
            return Err(DriverError(format!("no pending request {request_id}")));
        }
        let mut step = DriverStep::default();
        if decision == ApprovalDecision::Cancel {
            self.end(&mut step, TurnTerminalStatus::Interrupted, None);
            return Ok(step);
        }
        let allowed = matches!(
            decision,
            ApprovalDecision::Accept
                | ApprovalDecision::AcceptForSession
                | ApprovalDecision::AcceptAlways
        );
        if let Some(run) = self.run.as_mut() {
            run.approved(&tool, allowed);
        }
        self.advance(&mut step);
        Ok(step)
    }

    fn respond_question(
        &mut self,
        request_id: &str,
        answers: &[otto_core::agent_runtime::QuestionAnswer],
    ) -> Result<DriverStep, DriverError> {
        match &self.pending {
            Some(Pending::Ask(id)) if id == request_id => self.pending = None,
            _ => return Err(DriverError(format!("no pending question {request_id}"))),
        }
        let text = answers
            .iter()
            .flat_map(|a| a.values())
            .collect::<Vec<_>>()
            .join(", ");
        if let Some(run) = self.run.as_mut() {
            run.answered(&text);
        }
        let mut step = DriverStep::default();
        self.advance(&mut step);
        Ok(step)
    }

    fn on_line(&mut self, line: &str) -> DriverStep {
        let mut step = DriverStep::default();
        let (word, t) = line.split_once(' ').unwrap_or((line, ""));
        let t: u64 = t.trim().parse().unwrap_or(0);
        match word {
            "wake" if matches!(self.pending, Some(Pending::Wake(w)) if w == t) => {
                self.pending = None;
                self.advance(&mut step);
            }
            // The agent's own follow-up turn, after its background work.
            "owed"
                if self.owed.as_ref().is_some_and(|(o, _)| *o == t) && self.turn_id.is_none() =>
            {
                let (_, run) = self.owed.take().expect("owed");
                let id = format!("scripted-own-{t}");
                step.push(DriverEvent::TurnStarted {
                    turn_id: id.clone(),
                });
                step.push(DriverEvent::BackgroundTasks(Vec::new()));
                self.turn_id = Some(id);
                self.run = Some(run);
                self.advance(&mut step);
            }
            _ => {}
        }
        step
    }
}
