//! Unified live sideband session tailer + replayed-burst deduplicator across
//! all ACP agents (`agy`, `grok`, `gemini`, `opencode`, and custom ACP servers
//! via `OTTO_ACP_SIDEBAND_FILE`).
//!
//! When an ACP server buffers `session/update` notifications between
//! `session/prompt` RPCs while executing background tasks, timers, or subagent
//! wakeups, [`AcpSideband`] mirrors its real-time on-disk session log into
//! Otto's timeline and arms [`BurstFilter`] on the next `session/prompt` so any
//! buffered stdout replay burst is absorbed without dropping new-turn output.

mod agy;
pub(crate) mod burst;
mod gemini;
pub(crate) mod grok;
pub(crate) mod io;
mod opencode;
#[cfg(test)]
mod tests;

use std::time::Duration;

use agent_client_protocol::v1::schema::{ToolCallStatus, ToolKind};
use otto_core::agent_runtime::{PlanStep, RuntimeProvider, TurnTokenUsage};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use self::agy::{default_agy_brain_dir, AgyState};
use self::burst::{consume_budget, now_ms, BurstFilter, MirroredCounts};
use self::gemini::{default_gemini_home, GeminiState};
use self::grok::{default_grok_home, GrokState};
use self::opencode::{default_opencode_data_dir, OpencodeState};
use crate::driver::{SidebandPayload, SidebandPoll, SidebandResumeSeed, SidebandSource};

const POLL_INTERVAL: Duration = Duration::from_millis(400);

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum SidebandAction {
    TurnStarted,
    Thought {
        text: String,
        streaming: bool,
    },
    Message {
        text: String,
        message_id: Option<String>,
        streaming: bool,
    },
    ToolStart {
        id: String,
        title: String,
        kind: ToolKind,
        raw_input: Option<Value>,
        file: Option<String>,
        diff: Option<(Option<String>, String)>,
    },
    ToolUpdate {
        id: String,
        status: ToolCallStatus,
        title: Option<String>,
        kind: Option<ToolKind>,
        raw_input: Option<Value>,
        output: Option<String>,
        file: Option<String>,
        diff: Option<(Option<String>, String)>,
    },
    Plan(Vec<PlanStep>),
    Usage(TurnTokenUsage),
    TurnCompleted {
        reason: Option<String>,
    },
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct AcpSideband {
    #[serde(default, alias = "brain_dir")]
    root_dir: Option<String>,
    #[serde(default)]
    custom_file: Option<String>,
    #[serde(default)]
    idle_since_ms: u64,
    #[serde(default)]
    native_autonomous_seen: bool,
    #[serde(default)]
    mirrored: MirroredCounts,
    #[serde(default)]
    burst: BurstFilter,
    #[serde(default)]
    agy: AgyState,
    #[serde(default)]
    grok: GrokState,
    #[serde(default)]
    gemini: GeminiState,
    #[serde(default)]
    opencode: OpencodeState,
}

impl AcpSideband {
    pub fn for_provider(provider: RuntimeProvider) -> Self {
        Self {
            root_dir: default_root_for(provider),
            custom_file: std::env::var("OTTO_ACP_SIDEBAND_FILE")
                .ok()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty()),
            ..Self::default()
        }
    }

    pub fn set_root_dir(&mut self, dir: Option<String>) {
        self.root_dir = dir.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    }

    pub fn set_custom_file(&mut self, file: Option<String>) {
        self.custom_file = file.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
    }

    pub fn adopt_env(&mut self, provider: RuntimeProvider, env: &mut Vec<(String, String)>) {
        if let Some(val) = take_env(env, "OTTO_ACP_SIDEBAND_FILE") {
            self.custom_file = Some(val);
        }
        let provider_vars: &[&str] = match provider {
            RuntimeProvider::Agy => &["OTTO_AGY_BRAIN_DIR"],
            RuntimeProvider::Grok => &["OTTO_GROK_HOME", "GROK_HOME"],
            RuntimeProvider::Gemini => &["OTTO_GEMINI_HOME", "GEMINI_CLI_HOME"],
            RuntimeProvider::Opencode => &["OTTO_OPENCODE_DATA_DIR"],
            _ => &[],
        };
        for var in provider_vars {
            let strip = var.starts_with("OTTO_");
            let found = if strip {
                take_env(env, var)
            } else {
                peek_env(env, var)
            };
            if let Some(val) = found {
                self.root_dir = Some(val);
                return;
            }
        }
        if self.root_dir.is_none() {
            self.root_dir = default_root_for(provider);
        }
    }

    pub fn ensure_restored(&mut self, provider: RuntimeProvider) {
        let mut empty = Vec::new();
        self.adopt_env(provider, &mut empty);
        if self.idle_since_ms == 0 {
            self.idle_since_ms = now_ms();
        }
    }

    pub fn ensure_idle_floor(&mut self) {
        if self.idle_since_ms == 0 {
            self.idle_since_ms = now_ms();
        }
    }

    pub fn sideband_poll(
        &self,
        provider: RuntimeProvider,
        ready: bool,
        idle_between_prompts: bool,
        cwd: &str,
        session_id: Option<&str>,
    ) -> Option<SidebandPoll> {
        if !ready || !idle_between_prompts || self.native_autonomous_seen {
            return None;
        }
        let source = self.source_for(provider, cwd, session_id)?;
        Some(SidebandPoll {
            source,
            interval: POLL_INTERVAL,
        })
    }

    pub fn resume_source(
        &self,
        provider: RuntimeProvider,
        cwd: &str,
        resume_id: &str,
    ) -> Option<SidebandSource> {
        self.source_for(provider, cwd, Some(resume_id))
    }

    fn source_for(
        &self,
        provider: RuntimeProvider,
        cwd: &str,
        session_id: Option<&str>,
    ) -> Option<SidebandSource> {
        if let Some(file) = self.custom_file.as_deref() {
            return self.grok.source_for_custom_file(file);
        }
        let root = self.root_dir.as_deref().filter(|s| !s.is_empty())?;
        let sid = session_id?;
        match provider {
            RuntimeProvider::Agy => self.agy.source(root, sid),
            RuntimeProvider::Grok => self.grok.source_for_grok(root, cwd, sid),
            RuntimeProvider::Gemini => self.gemini.source(root, cwd, sid),
            RuntimeProvider::Opencode => self.opencode.source(root, sid, self.idle_since_ms),
            _ => None,
        }
    }

    pub fn init_resume(&mut self, provider: RuntimeProvider, seed: SidebandResumeSeed) {
        self.reset_for_prompt();
        match seed {
            SidebandResumeSeed::AppendOffset {
                resolved_path,
                using_fallback,
                offset,
            } => {
                if self.custom_file.is_some() || provider == RuntimeProvider::Grok {
                    self.grok.resolved_path = resolved_path;
                    self.grok.offset = offset;
                } else if provider == RuntimeProvider::Agy {
                    self.agy.using_fallback = using_fallback;
                    self.agy.offset = offset;
                }
                self.idle_since_ms = now_ms();
            }
            SidebandResumeSeed::IdleSinceMs(ms) => {
                self.idle_since_ms = ms;
            }
        }
    }

    pub fn has_mirrored(&self) -> bool {
        self.mirrored.has_any()
    }

    pub fn note_native_autonomous(&mut self) {
        self.native_autonomous_seen = true;
    }

    pub fn is_awaiting_followup(&self) -> bool {
        self.agy.is_awaiting()
            || self.grok.is_awaiting()
            || self.gemini.is_awaiting()
            || self.opencode.is_awaiting()
    }

    /// Settle one autonomous (`auto-N`) turn while remaining in the between-prompt
    /// idle gap. Must NOT wipe `in_autonomous_gap` or cross-wakeup state when a
    /// subsequent wakeup (`auto-(N+1)`) has already landed in the sideband log.
    pub fn on_turn_settled(&mut self) {
        self.agy.on_turn_settled();
        self.grok.on_turn_settled();
        self.gemini.on_turn_settled();
        self.opencode.on_turn_settled();
    }

    pub fn arm_burst_filter(&mut self) {
        self.mirrored.transfer_into(&mut self.burst);
        self.reset_for_prompt();
    }

    pub fn finish_prompted_turn(&mut self) {
        self.burst = BurstFilter::default();
        self.idle_since_ms = now_ms();
        self.reset_for_prompt();
    }

    fn reset_for_prompt(&mut self) {
        self.agy.reset_for_prompt();
        self.grok.reset_for_prompt();
        self.gemini.reset_for_prompt();
        self.opencode.reset_for_prompt();
    }

    pub fn filter_thought(&mut self, text: &str) -> Option<String> {
        consume_budget(&mut self.burst.remaining_thought_chars, text)
    }

    pub fn filter_message(&mut self, text: &str) -> Option<String> {
        if self.burst.is_draining_pre_prompt() && text.trim_start().starts_with("⚠ ") {
            return None;
        }
        consume_budget(&mut self.burst.remaining_msg_chars, text)
    }

    pub fn filter_tool_call(&mut self, tool_call_id: &str) -> bool {
        if self.burst.suppressed_tool_ids.contains(tool_call_id) {
            return true;
        }
        if self.burst.preseeded_tool_ids.remove(tool_call_id) {
            self.burst.remaining_tool_calls = self.burst.remaining_tool_calls.saturating_sub(1);
            self.burst
                .suppressed_tool_ids
                .insert(tool_call_id.to_string());
            return true;
        }
        if self.burst.remaining_tool_calls > 0 {
            self.burst.remaining_tool_calls -= 1;
            self.burst
                .suppressed_tool_ids
                .insert(tool_call_id.to_string());
            return true;
        }
        false
    }

    pub fn filter_tool_update(&mut self, tool_call_id: &str, known_in_turn: bool) -> bool {
        if self.burst.suppressed_tool_ids.contains(tool_call_id)
            || self.burst.preseeded_tool_ids.contains(tool_call_id)
        {
            return true;
        }
        // Suppress orphan `tool_call_update` frames replayed for tools started in
        // a prior turn (e.g. `schedule` timers or background commands completing
        // during the autonomous gap without a fresh `tool_call`).
        if !known_in_turn && self.burst.armed_from_mirrored {
            self.burst
                .suppressed_tool_ids
                .insert(tool_call_id.to_string());
            return true;
        }
        false
    }

    pub fn on_payload(
        &mut self,
        provider: RuntimeProvider,
        session_id: Option<&str>,
        payload: SidebandPayload,
    ) -> Vec<SidebandAction> {
        match payload {
            SidebandPayload::AppendLines {
                resolved_path,
                using_fallback,
                next_offset,
                bytes,
            } => {
                if self.custom_file.is_some() || provider == RuntimeProvider::Grok {
                    self.grok.parse_bytes(
                        resolved_path,
                        next_offset,
                        &bytes,
                        self.idle_since_ms,
                        &mut self.mirrored,
                    )
                } else {
                    self.agy.parse_bytes(
                        using_fallback,
                        next_offset,
                        &bytes,
                        self.idle_since_ms,
                        &mut self.mirrored,
                    )
                }
            }
            SidebandPayload::GeminiJson {
                resolved_path,
                stamp,
                bytes,
            } => self.gemini.parse_snapshot(
                resolved_path,
                stamp,
                &bytes,
                session_id,
                self.idle_since_ms,
                &mut self.mirrored,
            ),
            SidebandPayload::OpencodeSqlite { stamp, rows_jsonl } => {
                self.opencode
                    .parse_rows(stamp, &rows_jsonl, self.idle_since_ms, &mut self.mirrored)
            }
        }
    }
}

fn default_root_for(provider: RuntimeProvider) -> Option<String> {
    match provider {
        RuntimeProvider::Agy => default_agy_brain_dir(),
        RuntimeProvider::Grok => default_grok_home(),
        RuntimeProvider::Gemini => default_gemini_home(),
        RuntimeProvider::Opencode => default_opencode_data_dir(),
        _ => None,
    }
}

fn take_env(env: &mut Vec<(String, String)>, key: &str) -> Option<String> {
    let pos = env.iter().position(|(k, _)| k == key)?;
    let (_, val) = env.remove(pos);
    let trimmed = val.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

fn peek_env(env: &[(String, String)], key: &str) -> Option<String> {
    env.iter()
        .find(|(k, _)| k == key)
        .map(|(_, v)| v.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}
