//! Antigravity (`agy`) driver — agy's own print-mode stream-json.
//!
//! T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.) reaches Antigravity only
//! through an npm ACP-registry adapter (`antigravity-acp`,
//! `packages/client-runtime/src/providerClients.ts:39-43`). Otto installs
//! nothing: it drives the installed CLI's native NDJSON protocol (agy 1.4.12):
//!
//! * argv — `agy --print= --input-format stream-json --output-format
//!   stream-json [--model m] [--conversation <id>]`. `--print` takes a value;
//!   the empty `=` keeps it from eating the next flag.
//! * stdin — one `{"event":"user","message":{"content":"…"}}` per turn; agy
//!   ignores every other input event, so nothing can be answered over stdio.
//! * stdout — `init` (the `conversation_id`, the resume key), then
//!   `step_update {step_index, state: ACTIVE|DONE, step_type}`:
//!   `agent_response` grows by `text_delta` and reports that model call's
//!   `usage` on DONE; `tool` carries `tool_info {name, parameters, output?}`.
//!   The turn ends with `result {status: SUCCESS|ERROR, error?,
//!   finished_reason?, denied_actions?}`. `result.usage` and
//!   `denied_actions` are cumulative over the process, so the turn's usage is
//!   the sum of its steps' and only new denials are reported.
//! * permissions — headless agy cannot ask: a tool that needs approval is
//!   auto-DENIED and listed in `denied_actions` (only `settings.json`
//!   `permissions.allow` rules and `--mode` apply, as for a PTY agy). Otto
//!   never passes `--dangerously-skip-permissions`; the denial becomes a
//!   system notice.
//! * interrupt — SIGINT: agy ends the turn with `result` ERROR +
//!   `finished_reason: "interrupted"`, then exits; the next prompt respawns
//!   with `--conversation`.

use std::collections::HashMap;

use otto_core::agent_runtime::{
    ApprovalDecision, ProviderFailure, RuntimeProvider, TurnItemKind, TurnItemStatus,
    TurnTerminalStatus, TurnTokenUsage,
};
use serde_json::{json, Value};

use crate::driver::{
    item, ChildSignal, DriverError, DriverEvent, DriverStep, LaunchContext, LaunchSpec,
    StructuredDriver,
};

pub struct AgyDriver {
    program: String,
    turn_id: Option<String>,
    /// `agent_response` text so far, by step index.
    texts: HashMap<u64, String>,
    usage: Option<TurnTokenUsage>,
    /// `denied_actions` already reported by this process.
    denied_seen: usize,
}

impl AgyDriver {
    pub fn with_program(program: &str) -> Self {
        Self {
            program: program.into(),
            turn_id: None,
            texts: HashMap::new(),
            usage: None,
            denied_seen: 0,
        }
    }

    fn on_step(&mut self, step: &mut DriverStep, s: &Value) {
        let index = s.get("step_index").and_then(Value::as_u64).unwrap_or(0);
        let id = format!(
            "{}:step:{index}",
            self.turn_id.as_deref().unwrap_or_default()
        );
        let done = s.get("state").and_then(Value::as_str) == Some("DONE");
        match s.get("step_type").and_then(Value::as_str) {
            Some("agent_response") => {
                let text = self.texts.entry(index).or_default();
                text.push_str(s.get("text_delta").and_then(Value::as_str).unwrap_or(""));
                if !text.is_empty() {
                    let (status, kind) = (
                        if done {
                            TurnItemStatus::Completed
                        } else {
                            TurnItemStatus::Running
                        },
                        TurnItemKind::AssistantMessage {
                            message_id: id.clone(),
                            text: text.clone(),
                            streaming: !done,
                        },
                    );
                    step.push(DriverEvent::TurnItem(item(id, status, None, kind)));
                }
                if let Some(u) = s.get("usage").filter(|_| done) {
                    let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
                    let acc = self.usage.get_or_insert_with(TurnTokenUsage::default);
                    acc.input_tokens += n("input_tokens") + n("cache_read_tokens");
                    acc.output_tokens += n("output_tokens");
                    *acc.cached_input_tokens.get_or_insert(0) += n("cache_read_tokens");
                    *acc.reasoning_tokens.get_or_insert(0) += n("thinking_tokens");
                }
            }
            Some("tool") => {
                let info = s.get("tool_info").cloned().unwrap_or(Value::Null);
                let name = info.get("name").and_then(Value::as_str).unwrap_or("tool");
                let params = info.get("parameters").cloned().unwrap_or(Value::Null);
                let output = info
                    .get("output")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                let kind = match params.get("CommandLine").and_then(Value::as_str) {
                    Some(cmd) if name == "run_command" => TurnItemKind::CommandExecution {
                        input: cmd.to_string(),
                        output,
                        exit_code: None,
                    },
                    _ => TurnItemKind::DynamicTool {
                        tool_name: name.to_string(),
                        input: params,
                        output,
                    },
                };
                let status = if done {
                    TurnItemStatus::Completed
                } else {
                    TurnItemStatus::Running
                };
                step.push(DriverEvent::TurnItem(item(
                    id,
                    status,
                    Some(name.to_string()),
                    kind,
                )));
            }
            _ => {}
        }
    }

    fn on_result(&mut self, step: &mut DriverStep, r: &Value) {
        let turn = self.turn_id.take().unwrap_or_default();
        self.texts.clear();
        let all = r.get("denied_actions").and_then(Value::as_array);
        let fresh = all.map_or(&[][..], |a| a.get(self.denied_seen..).unwrap_or_default());
        self.denied_seen = all.map_or(self.denied_seen, Vec::len);
        let denied: Vec<&str> = fresh
            .iter()
            .filter_map(|a| a.get("display_name").or(a.get("action"))?.as_str())
            .collect();
        if !denied.is_empty() {
            let message = format!(
                "Antigravity refused {} — it cannot ask for approval here. Allow it with a \
                 permissions.allow rule in ~/.gemini/antigravity-cli/settings.json, or use a \
                 terminal session.",
                denied.join(", ")
            );
            step.push(DriverEvent::TurnItem(item(
                format!("{turn}:denied"),
                TurnItemStatus::Completed,
                None,
                TurnItemKind::SystemNotice { message },
            )));
        }
        if let Some(u) = self.usage.take() {
            step.push(DriverEvent::Usage(u));
        }
        let (status, failure) = match (
            r.get("status").and_then(Value::as_str),
            r.get("finished_reason").and_then(Value::as_str),
        ) {
            (Some("SUCCESS"), _) => (TurnTerminalStatus::Completed, None),
            (_, Some("interrupted")) => (TurnTerminalStatus::Interrupted, None),
            _ => (
                TurnTerminalStatus::Failed,
                Some(ProviderFailure {
                    class: "provider_error".into(),
                    message: r
                        .get("error")
                        .and_then(Value::as_str)
                        .unwrap_or("Antigravity reported an error")
                        .to_string(),
                    code: None,
                    retryable: None,
                }),
            ),
        };
        step.push(DriverEvent::Terminal {
            status,
            failure,
            reason: None,
        });
    }
}

impl StructuredDriver for AgyDriver {
    fn provider(&self) -> RuntimeProvider {
        RuntimeProvider::Agy
    }

    fn launch(&self, ctx: &LaunchContext) -> LaunchSpec {
        let mut args: Vec<String> = [
            "--print=",
            "--input-format",
            "stream-json",
            "--output-format",
            "stream-json",
        ]
        .map(String::from)
        .to_vec();
        if let Some(m) = &ctx.model {
            args.extend(["--model".into(), m.clone()]);
        }
        if let Some(id) = &ctx.resume {
            args.extend(["--conversation".into(), id.clone()]);
        }
        LaunchSpec {
            program: self.program.clone(),
            args,
            ..LaunchSpec::default()
        }
    }

    /// agy reads stdin at once; its `init` only follows the first prompt.
    fn start(&mut self, _ctx: &LaunchContext) -> DriverStep {
        let mut step = DriverStep::default();
        step.push(DriverEvent::Ready {
            native_thread_id: None,
            model: None,
        });
        step
    }

    fn prompt(&mut self, turn_id: &str, text: &str) -> Result<DriverStep, DriverError> {
        if self.turn_id.is_some() {
            return Err(DriverError("a prompt is already running".into()));
        }
        self.turn_id = Some(turn_id.to_string());
        let mut step = DriverStep::default();
        step.write(json!({"event": "user", "message": {"content": text}}).to_string());
        Ok(step)
    }

    fn interrupt(&mut self) -> DriverStep {
        DriverStep {
            signal: Some(ChildSignal::Interrupt),
            ..DriverStep::default()
        }
    }

    fn respond(
        &mut self,
        request_id: &str,
        _: ApprovalDecision,
    ) -> Result<DriverStep, DriverError> {
        Err(DriverError(format!("unknown request {request_id}")))
    }

    fn on_line(&mut self, line: &str) -> DriverStep {
        let mut step = DriverStep::default();
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            return step; // agy's own log lines
        };
        match v.get("event").and_then(Value::as_str) {
            Some("init") => step.push(DriverEvent::Ready {
                native_thread_id: v
                    .get("conversation_id")
                    .and_then(Value::as_str)
                    .map(str::to_string),
                model: None,
            }),
            Some("step_update") if self.turn_id.is_some() => {
                self.on_step(&mut step, &v["step_update"]);
            }
            Some("result") if self.turn_id.is_some() => self.on_result(&mut step, &v["result"]),
            _ => {}
        }
        step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(resume: Option<&str>) -> LaunchContext {
        LaunchContext {
            session_id: "s".into(),
            cwd: "/w".into(),
            model: None,
            resume: resume.map(str::to_string),
            sandboxed: false,
        }
    }

    fn feed(d: &mut AgyDriver, v: Value) -> Vec<DriverEvent> {
        d.on_line(&v.to_string()).events
    }

    fn step(index: u64, state: &str, ty: &str, extra: Value) -> Value {
        let mut s = json!({"conversation_id":"c1","step_index":index,"state":state,"step_type":ty});
        s.as_object_mut()
            .unwrap()
            .extend(extra.as_object().unwrap().clone());
        json!({"event":"step_update","step_update":s})
    }

    #[test]
    fn launch_is_print_mode_stream_json_and_resumes_the_conversation() {
        let d = AgyDriver::with_program("agy");
        let fresh = d.launch(&ctx(None));
        assert_eq!(
            fresh.args,
            [
                "--print=",
                "--input-format",
                "stream-json",
                "--output-format",
                "stream-json"
            ]
        );
        let resumed = d.launch(&ctx(Some("c1")));
        assert_eq!(resumed.args[5..], ["--conversation", "c1"]);
        assert!(!resumed.args.iter().any(|a| a.contains("dangerously")));
    }

    #[test]
    fn streams_text_and_tools_then_sums_step_usage() {
        let mut d = AgyDriver::with_program("agy");
        let w = d.prompt("t1", "hi").unwrap().writes;
        assert_eq!(w, [r#"{"event":"user","message":{"content":"hi"}}"#]);
        let ev = feed(
            &mut d,
            json!({"event":"init","conversation_id":"c1","init":{}}),
        );
        assert!(
            matches!(&ev[0], DriverEvent::Ready { native_thread_id: Some(n), .. } if n == "c1")
        );
        let usage = |i, o, c| json!({"usage":{"input_tokens":i,"output_tokens":o,"thinking_tokens":1,"cache_read_tokens":c}});
        assert!(feed(&mut d, step(0, "DONE", "user_input", json!({}))).is_empty());
        feed(&mut d, step(1, "DONE", "agent_response", usage(10, 2, 0)));
        let ev = feed(
            &mut d,
            step(
                2,
                "DONE",
                "tool",
                json!({"tool_info":{"name":"run_command","parameters":{"CommandLine":"ls"},"output":"a"}}),
            ),
        );
        assert!(matches!(&ev[0], DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::CommandExecution { input, output: Some(o), .. } if input == "ls" && o == "a")));
        let ev = feed(
            &mut d,
            step(3, "ACTIVE", "agent_response", json!({"text_delta":"o"})),
        );
        assert!(matches!(&ev[0], DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::AssistantMessage { text, streaming: true, .. } if text == "o")));
        let mut done = usage(3, 4, 7);
        done["text_delta"] = json!("k");
        let ev = feed(&mut d, step(3, "DONE", "agent_response", done));
        assert!(matches!(&ev[0], DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::AssistantMessage { text, streaming: false, .. } if text == "ok")));
        let ev = feed(
            &mut d,
            json!({"event":"result","result":{"status":"SUCCESS","response":"ok","usage":{"input_tokens":999}}}),
        );
        assert_eq!(
            ev[0],
            DriverEvent::Usage(TurnTokenUsage {
                input_tokens: 20,
                output_tokens: 6,
                cached_input_tokens: Some(7),
                reasoning_tokens: Some(2),
            })
        );
        assert!(matches!(
            ev[1],
            DriverEvent::Terminal {
                status: TurnTerminalStatus::Completed,
                ..
            }
        ));
    }

    #[test]
    fn error_interrupt_and_denied_results() {
        let mut d = AgyDriver::with_program("agy");
        d.prompt("t1", "x").unwrap();
        let ev = feed(
            &mut d,
            json!({"event":"result","result":{"status":"ERROR","error":"quota exhausted"}}),
        );
        assert!(
            matches!(&ev[0], DriverEvent::Terminal { status: TurnTerminalStatus::Failed, failure: Some(f), .. }
            if f.message == "quota exhausted")
        );

        d.prompt("t2", "x").unwrap();
        assert_eq!(d.interrupt().signal, Some(ChildSignal::Interrupt));
        let ev = feed(
            &mut d,
            json!({"event":"result","result":{"status":"ERROR","error":"interrupted","finished_reason":"interrupted"}}),
        );
        assert!(matches!(
            &ev[0],
            DriverEvent::Terminal {
                status: TurnTerminalStatus::Interrupted,
                failure: None,
                ..
            }
        ));

        d.prompt("t3", "x").unwrap();
        let ev = feed(
            &mut d,
            json!({"event":"result","result":{"status":"SUCCESS","denied_actions":[{"action":"command","display_name":"RunCommand"}]}}),
        );
        assert!(matches!(&ev[0], DriverEvent::TurnItem(i)
            if matches!(&i.kind, TurnItemKind::SystemNotice { message } if message.contains("RunCommand"))));
        d.prompt("t4", "x").unwrap();
        let ev = feed(
            &mut d,
            json!({"event":"result","result":{"status":"SUCCESS","denied_actions":[{"action":"command","display_name":"RunCommand"}]}}),
        );
        assert!(
            matches!(ev[0], DriverEvent::Terminal { .. }),
            "denials are cumulative: only new ones are reported"
        );
        assert!(d.respond("x", ApprovalDecision::Accept).is_err());
    }
}
