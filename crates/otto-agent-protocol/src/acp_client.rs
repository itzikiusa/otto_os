//! ACP client methods Otto serves to the agent (`fs/read_text_file`,
//! `fs/write_text_file`, `terminal/*`), advertised per flavour
//! (`AcpFlavor::client_fs` / `client_terminal`). The driver stays sans-IO:
//! it validates, asks the person where the access mode says so, and hands the
//! work to the runtime as a [`DriverEvent::ClientCall`] (`runtime/client_calls.rs`
//! runs it off the session lock, confined to the session folder, terminals
//! under the agent's own sandbox profile); the answer comes back through
//! `on_client_result` / `on_client_output`.

use super::*;
use crate::driver::ClientOp;

/// What a client call is waiting for.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) enum ClientWait {
    /// Answer the agent with the runtime's result as is.
    Plain { rpc_id: Value },
    /// A write: answer `{}` and show the change as a file diff.
    Write {
        rpc_id: Value,
        path: String,
        content: String,
    },
    /// A terminal start: answer `{terminalId}`.
    Create { rpc_id: Value, terminal_id: String },
}

/// A call held for the person's answer (approval-required).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct HeldCall {
    rpc_id: Value,
    op: ClientOpRepr,
}

/// `ClientOp` for the snapshot (the ops a person approves).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
enum ClientOpRepr {
    Write {
        root: String,
        path: String,
        content: String,
    },
    Create {
        terminal_id: String,
        program: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        cwd: String,
        byte_limit: Option<u64>,
    },
}

const NOT_SUPPORTED: i64 = -32601;
/// Output kept per client terminal for a tool row that embeds it late.
const TERMINAL_KEEP: usize = 256 << 10;

fn error_line(id: &Value, code: i64, message: &str) -> String {
    json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}).to_string()
}

impl AcpDriver {
    /// `fs/*` is served only for an agent that runs unconfined: under
    /// Seatbelt the daemon's own write would skip the profile's denials
    /// (`.git/hooks`, a read-only thread's `.claude/settings.json` …), so a
    /// confined agent keeps its own file tools, inside the profile.
    fn serves_fs(&self) -> bool {
        self.flavor.client_fs && self.state.confine.is_empty()
    }

    /// The client capabilities this flavour advertises in `initialize`.
    pub(super) fn client_capabilities(&self, init: &mut Value) {
        if self.serves_fs() {
            init["clientCapabilities"]["fs"] = json!({"readTextFile": true, "writeTextFile": true});
        }
        if self.flavor.client_terminal {
            init["clientCapabilities"]["terminal"] = json!(true);
        }
    }

    /// The prepared spec's sandbox wrapper (`sandbox-exec -p <profile>`) and
    /// env: a client terminal runs exactly as confined as the agent.
    pub(super) fn adopt_confinement(&mut self, spec: &LaunchSpec) {
        if !self.flavor.client_terminal && !self.flavor.client_fs {
            return;
        }
        let wrapped = spec.program.ends_with("sandbox-exec")
            && spec.args.first().map(String::as_str) == Some("-p")
            && spec.args.len() >= 2;
        self.state.confine = if wrapped {
            vec![
                spec.program.clone(),
                spec.args[0].clone(),
                spec.args[1].clone(),
            ]
        } else {
            Vec::new()
        };
        self.state.child_env = spec.env.clone();
    }

    /// An `fs/*` / `terminal/*` request from the agent.
    pub(super) fn on_client_request(
        &mut self,
        step: &mut DriverStep,
        method: &str,
        rpc_id: Value,
        params: &Value,
    ) {
        let enabled = if method.starts_with("fs/") {
            self.serves_fs()
        } else {
            self.flavor.client_terminal
        };
        if !enabled {
            step.write(error_line(
                &rpc_id,
                NOT_SUPPORTED,
                &format!("method not supported: {method}"),
            ));
            return;
        }
        if params.get("sessionId").and_then(Value::as_str) != self.state.session_id.as_deref() {
            step.write(error_line(&rpc_id, -32602, "not this session"));
            return;
        }
        let s = |k: &str| params.get(k).and_then(Value::as_str).map(str::to_string);
        let root = self.state.cwd.clone();
        let tid = || s("terminalId").unwrap_or_default();
        let supervised = self.state.access == RuntimeAccessMode::ApprovalRequired;
        let op = match method {
            "fs/read_text_file" => ClientOp::ReadFile {
                root,
                path: s("path").unwrap_or_default(),
                line: params.get("line").and_then(Value::as_u64),
                limit: params.get("limit").and_then(Value::as_u64),
            },
            "fs/write_text_file" => {
                let (path, content) = (
                    s("path").unwrap_or_default(),
                    s("content").unwrap_or_default(),
                );
                let op = ClientOpRepr::Write {
                    root,
                    path: path.clone(),
                    content,
                };
                if supervised {
                    self.hold(
                        step,
                        rpc_id,
                        op,
                        RequestKind::FileChange,
                        format!("Write {path}"),
                    );
                } else {
                    self.run_held(step, rpc_id, op);
                }
                return;
            }
            "terminal/create" => {
                let command = s("command").unwrap_or_default();
                let args: Vec<String> = params
                    .get("args")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|v| v.as_str().map(str::to_string))
                            .collect()
                    })
                    .unwrap_or_default();
                let line = std::iter::once(command.clone())
                    .chain(args.iter().cloned())
                    .collect::<Vec<_>>()
                    .join(" ");
                // A bare command line runs through the shell, as ACP agents send it.
                let (program, args) = if args.is_empty() {
                    ("/bin/sh".to_string(), vec!["-c".to_string(), command])
                } else {
                    (command, args)
                };
                let (program, args) = match self.state.confine.as_slice() {
                    [wrapper, rest @ ..] => (
                        wrapper.clone(),
                        rest.iter()
                            .cloned()
                            .chain(std::iter::once(program))
                            .chain(args)
                            .collect(),
                    ),
                    [] => (program, args),
                };
                let mut env = self.state.child_env.clone();
                env.extend(
                    params
                        .get("env")
                        .and_then(Value::as_array)
                        .into_iter()
                        .flatten()
                        .filter_map(|e| {
                            Some((
                                e.get("name")?.as_str()?.to_string(),
                                e.get("value")?.as_str()?.to_string(),
                            ))
                        }),
                );
                self.state.terminals += 1;
                let terminal_id = format!("term-{}", self.state.terminals);
                self.bind_terminal(&terminal_id, &line);
                let op = ClientOpRepr::Create {
                    terminal_id,
                    program,
                    args,
                    env,
                    cwd: s("cwd").unwrap_or(root),
                    byte_limit: params.get("outputByteLimit").and_then(Value::as_u64),
                };
                // Auto-accept edits covers edits only: a command asks unless
                // the thread is on Full access.
                if self.state.access != RuntimeAccessMode::FullAccess {
                    self.hold(step, rpc_id, op, RequestKind::Command, line);
                } else {
                    self.run_held(step, rpc_id, op);
                }
                return;
            }
            "terminal/output" => ClientOp::TerminalOutput { terminal_id: tid() },
            "terminal/wait_for_exit" => ClientOp::TerminalWait { terminal_id: tid() },
            "terminal/kill" => ClientOp::TerminalKill { terminal_id: tid() },
            "terminal/release" => {
                self.state.terminal_output.remove(&tid());
                ClientOp::TerminalRelease { terminal_id: tid() }
            }
            _ => {
                step.write(error_line(
                    &rpc_id,
                    NOT_SUPPORTED,
                    &format!("method not supported: {method}"),
                ));
                return;
            }
        };
        self.call(step, ClientWait::Plain { rpc_id }, op);
    }

    fn call(&mut self, step: &mut DriverStep, wait: ClientWait, op: ClientOp) {
        self.state.client_seq += 1;
        let id = format!("client-{}", self.state.client_seq);
        self.state.client_waits.insert(id.clone(), wait);
        step.push(DriverEvent::ClientCall { id, op });
    }

    /// Supervised: the person approves the write / command first.
    fn hold(
        &mut self,
        step: &mut DriverStep,
        rpc_id: Value,
        op: ClientOpRepr,
        kind: RequestKind,
        title: String,
    ) {
        self.ensure_autonomous_turn(step);
        self.close_segment(step);
        self.state.client_seq += 1;
        let request_id = format!("acp-client-{}", self.state.client_seq);
        self.state
            .client_held
            .insert(request_id.clone(), HeldCall { rpc_id, op });
        step.push(DriverEvent::Request {
            id: request_id,
            kind,
            title: Some(title.clone()),
            prompt: Some(title),
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
    }

    fn run_held(&mut self, step: &mut DriverStep, rpc_id: Value, op: ClientOpRepr) {
        match op {
            ClientOpRepr::Write {
                root,
                path,
                content,
            } => self.call(
                step,
                ClientWait::Write {
                    rpc_id,
                    path: path.clone(),
                    content: content.clone(),
                },
                ClientOp::WriteFile {
                    root,
                    path,
                    content,
                },
            ),
            ClientOpRepr::Create {
                terminal_id,
                program,
                args,
                env,
                cwd,
                byte_limit,
            } => self.call(
                step,
                ClientWait::Create {
                    rpc_id,
                    terminal_id: terminal_id.clone(),
                },
                ClientOp::TerminalCreate {
                    terminal_id,
                    program,
                    args,
                    env,
                    cwd,
                    byte_limit,
                },
            ),
        }
    }

    /// After a daemon restart (holder adoption): the calls in flight died
    /// with the old daemon, and its terminals were killed. Each restored wait
    /// is answered with an error on the next line, so the agent never waits
    /// for an answer nobody will send (review R5).
    pub(super) fn orphan_restored_waits(&mut self) {
        for (_, wait) in self.state.client_waits.drain() {
            let rpc_id = match wait {
                ClientWait::Plain { rpc_id }
                | ClientWait::Write { rpc_id, .. }
                | ClientWait::Create { rpc_id, .. } => rpc_id,
            };
            self.state.client_orphans.push(rpc_id);
        }
    }

    pub(super) fn answer_orphans(&mut self, step: &mut DriverStep) {
        for rpc_id in std::mem::take(&mut self.state.client_orphans) {
            step.write(error_line(
                &rpc_id,
                -32000,
                "Otto restarted; this call was not completed.",
            ));
        }
    }

    /// The person's answer to a held client call, if `request_id` is one.
    pub(super) fn respond_client(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Option<DriverStep> {
        let held = self.state.client_held.remove(request_id)?;
        let mut step = DriverStep::default();
        if matches!(
            decision,
            ApprovalDecision::Accept
                | ApprovalDecision::AcceptForSession
                | ApprovalDecision::AcceptAlways
        ) {
            self.run_held(&mut step, held.rpc_id, held.op);
        } else {
            step.write(error_line(
                &held.rpc_id,
                -32000,
                "The person declined this.",
            ));
        }
        Some(step)
    }

    /// Open held calls are refused when the turn is cancelled.
    pub(super) fn cancel_client_held(&mut self, step: &mut DriverStep) {
        for (_, held) in self.state.client_held.drain() {
            step.write(error_line(&held.rpc_id, -32000, "The turn was cancelled."));
        }
    }

    pub(super) fn client_result(&mut self, id: &str, result: Result<Value, String>) -> DriverStep {
        let mut step = DriverStep::default();
        let Some(wait) = self.state.client_waits.remove(id) else {
            return step;
        };
        let reply = |rpc_id: &Value, r: Result<Value, String>| match r {
            Ok(v) => json!({"jsonrpc":"2.0","id":rpc_id,"result":v}).to_string(),
            Err(e) => error_line(rpc_id, -32000, &e),
        };
        match wait {
            ClientWait::Plain { rpc_id } => step.write(reply(&rpc_id, result)),
            ClientWait::Create {
                rpc_id,
                terminal_id,
            } => step.write(reply(
                &rpc_id,
                result.map(|_| json!({"terminalId": terminal_id})),
            )),
            ClientWait::Write {
                rpc_id,
                path,
                content,
            } => {
                if let Ok(v) = &result {
                    // The change as a reviewable diff row.
                    let old = v.get("old").and_then(Value::as_str).map(str::to_string);
                    let (diff_str, old_str, new_str) =
                        crate::diff_compact::compact_file_change_fields(
                            &path,
                            None,
                            old,
                            Some(content),
                        );
                    self.state.client_seq += 1;
                    step.push(DriverEvent::TurnItem(item(
                        format!("{}:fs-write:{}", self.turn(), self.state.client_seq),
                        TurnItemStatus::Completed,
                        Some(format!("Write {path}")),
                        TurnItemKind::FileChange {
                            file_name: path,
                            diff_str,
                            old_str,
                            new_str,
                        },
                    )));
                }
                step.write(reply(&rpc_id, result.map(|_| json!({}))));
            }
        }
        step
    }

    /// The running command tool a new terminal belongs to: agents such as
    /// grok never embed `terminal` content, so the open `execute` call
    /// whose command the terminal runs (else the latest open one) gets it.
    fn bind_terminal(&mut self, terminal_id: &str, line: &str) {
        let open = |t: &&mut Tool| {
            t.kind == ToolKind::Execute
                && t.terminal.is_none()
                && matches!(
                    t.status,
                    ToolCallStatus::InProgress | ToolCallStatus::Pending
                )
        };
        let mut open: Vec<(&String, &mut Tool)> = self
            .state
            .tools
            .iter_mut()
            .filter(|(_, t)| open(t))
            .collect();
        open.sort_by(|a, b| a.0.cmp(b.0));
        let pick = open
            .iter()
            .position(|(_, t)| {
                command_text(t.raw_input.as_ref())
                    .is_some_and(|c| !c.is_empty() && line.contains(&c))
            })
            .or_else(|| open.len().checked_sub(1));
        if let Some(i) = pick {
            open[i].1.terminal = Some(terminal_id.to_string());
        }
    }

    /// Live output of a client terminal: into the tool row that embeds it
    /// (kept, bounded, for a row that embeds it later).
    pub(super) fn client_output(&mut self, terminal_id: &str, chunk: &str) -> DriverStep {
        let mut step = DriverStep::default();
        let turn = self.turn();
        let kept = self
            .state
            .terminal_output
            .entry(terminal_id.to_string())
            .or_default();
        kept.push_str(chunk);
        if kept.len() > TERMINAL_KEEP {
            let mut cut = kept.len() - TERMINAL_KEEP;
            while !kept.is_char_boundary(cut) {
                cut += 1;
            }
            kept.drain(..cut);
        }
        if let Some((id, t)) = self
            .state
            .tools
            .iter_mut()
            .find(|(_, t)| t.terminal.as_deref() == Some(terminal_id))
        {
            t.output.push_str(chunk);
            step.push(DriverEvent::CommandOutput {
                item_id: format!("{turn}:tool:{id}"),
                chunk: chunk.to_string(),
            });
        }
        step
    }
}
