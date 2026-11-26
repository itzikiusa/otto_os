//! The in-process fakes (test plan §11.2): the real `AgentRuntime` and the
//! real drivers, with the fake agents' own programs (`src/fakes/*.rs`, the
//! code the `otto-fake-*` binaries run) on in-memory pipes behind the
//! runtime's [`ChildTransport`] seam. No process is spawned, so an otto-server
//! suite can drive the real runtime through a twelve-node workflow with twelve
//! reviewers in about a second. Holders, restart survival, fds and the sandbox
//! are proven with the real binaries in this crate's own tests.
//!
//! ```ignore
//! let rt = AgentRuntime::new(repo, host, scripted::drivers(), RuntimeConfig {
//!     transport: Some(scripted::transport(vec![("OTTO_FAKE_SCRIPTS".into(), dir)])),
//!     ..RuntimeConfig::default()
//! });
//! ```
//!
//! Every fake reads its `OTTO_FAKE_*` settings from the spawn's env, then
//! from the transport's `env`, then from the test process's own. pi has no
//! fake: its provider has no driver here.

use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use otto_core::agent_runtime::RuntimeProvider;

use crate::acp::AcpDriver;
use crate::claude::ClaudeDriver;
use crate::codex::CodexDriver;
use crate::driver::{DriverFactory, StructuredDriver};
use crate::fake_io::{self, Io};
use crate::process::{ChildCommand, ChildControl, ChildHandle, ChildMsg, ChildTransport};

pub const CLAUDE: &str = "otto-fake-claude";
pub const CODEX: &str = "otto-fake-codex";
pub const ACP: &str = "otto-fake-agent";

/// The real drivers, launching the fakes by name (every ACP flavour runs
/// `otto-fake-agent`).
pub fn drivers() -> Arc<dyn DriverFactory> {
    Arc::new(|p: RuntimeProvider| -> Option<Box<dyn StructuredDriver>> {
        Some(match p {
            RuntimeProvider::Claude => Box::new(ClaudeDriver::with_program(CLAUDE)),
            RuntimeProvider::Codex => Box::new(CodexDriver::with_program(CODEX)),
            RuntimeProvider::Gemini => Box::new(AcpDriver::gemini(ACP)),
            RuntimeProvider::Agy => Box::new(AcpDriver::agy(ACP)),
            RuntimeProvider::Muse => Box::new(AcpDriver::muse(ACP)),
            RuntimeProvider::Grok | RuntimeProvider::Opencode => {
                Box::new(AcpDriver::new(p, ACP, Vec::new()))
            }
            RuntimeProvider::Pi => return None,
        })
    })
}

/// In-memory children running the fake programs; `env` is what every child
/// sees under the spawn's own env.
pub fn transport(env: Vec<(String, String)>) -> Arc<dyn ChildTransport> {
    Arc::new(InMemory {
        env,
        pids: AtomicU32::new(0),
    })
}

struct InMemory {
    env: Vec<(String, String)>,
    pids: AtomicU32,
}

impl ChildTransport for InMemory {
    fn spawn(&self, cmd: &ChildCommand) -> std::io::Result<ChildHandle> {
        let fake = |p: &str| -> Option<fn()> {
            match Path::new(p).file_name().and_then(|n| n.to_str()) {
                Some(CLAUDE) => Some(crate::fakes::claude::main),
                Some(CODEX) => Some(crate::fakes::codex::main),
                Some(ACP) => Some(crate::fakes::agent::main),
                _ => None,
            }
        };
        // A confined (read-only) thread runs under Seatbelt: the fake is the
        // program `sandbox-exec -p <profile> …` wraps.
        let (program, args) = match cmd.args.iter().position(|a| fake(a).is_some()) {
            Some(i) if cmd.program.ends_with("sandbox-exec") => (&cmd.args[i], &cmd.args[i + 1..]),
            _ => (&cmd.program, &cmd.args[..]),
        };
        let Some(main) = fake(program) else {
            return Err(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("no in-memory fake named {}", cmd.program),
            ));
        };
        let (msg_tx, messages) = tokio::sync::mpsc::channel::<ChildMsg>(256);
        let (stdin, mut stdin_rx) = tokio::sync::mpsc::unbounded_channel::<String>();
        let (control, mut control_rx) = tokio::sync::mpsc::unbounded_channel::<ChildControl>();
        let (feed_tx, feed_rx) = std::sync::mpsc::channel::<String>();
        // Dropping the feed is the child's stdin EOF.
        let feed = Arc::new(Mutex::new(Some(feed_tx)));
        // The exit is reported once: by the program, or by a signal first.
        let dead = Arc::new(AtomicBool::new(false));
        let ended = Arc::new(tokio::sync::Notify::new());

        tokio::spawn({
            let feed = Arc::clone(&feed);
            async move {
                while let Some(line) = stdin_rx.recv().await {
                    let sent = feed.lock().unwrap().as_ref().map(|f| f.send(line));
                    if !matches!(sent, Some(Ok(()))) {
                        break;
                    }
                }
                feed.lock().unwrap().take();
            }
        });

        let io = Arc::new(Io {
            args: std::iter::once(program.clone())
                .chain(args.iter().cloned())
                .collect(),
            env: self.env.iter().chain(&cmd.env).cloned().collect(),
            cwd: cmd.cwd.clone().into(),
            pid: 1_000_000 + self.pids.fetch_add(1, Ordering::Relaxed),
            stdin: Mutex::new(Some(feed_rx)),
            stdout: Box::new({
                let (tx, dead) = (msg_tx.clone(), Arc::clone(&dead));
                move |line| {
                    if !dead.load(Ordering::Acquire) {
                        let _ = tx.blocking_send(ChildMsg::Line(line));
                    }
                }
            }),
        });
        std::thread::Builder::new()
            .name(format!("in-memory {}", cmd.program))
            .spawn({
                let (tx, dead, ended) = (msg_tx.clone(), Arc::clone(&dead), Arc::clone(&ended));
                move || {
                    let code = fake_io::run_as(io, main);
                    if !dead.swap(true, Ordering::AcqRel) {
                        let _ = tx.blocking_send(ChildMsg::Exited {
                            code: Some(code),
                            stderr_tail: String::new(),
                        });
                    }
                    ended.notify_one();
                }
            })?;

        // A signal kills it at once; a stop closes stdin and gives it the grace.
        tokio::spawn(async move {
            let Some(c) = control_rx.recv().await else {
                return;
            };
            feed.lock().unwrap().take();
            if let ChildControl::Stop(grace) = c {
                let _ = tokio::time::timeout(grace, ended.notified()).await;
            }
            if !dead.swap(true, Ordering::AcqRel) {
                let _ = msg_tx
                    .send(ChildMsg::Exited {
                        code: None,
                        stderr_tail: String::new(),
                    })
                    .await;
            }
        });

        Ok(ChildHandle {
            pid: 0,
            stdin,
            control,
            messages,
        })
    }
}
