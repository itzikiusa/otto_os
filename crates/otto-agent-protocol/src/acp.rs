//! ACP (Agent Client Protocol, v1) client driver — used for Gemini
//! (`gemini --acp`), Grok (`grok … agent stdio`), OpenCode, Antigravity (`agy`),
//! and the fake ACP test agent. Per-agent differences live in [`AcpFlavor`].
//!
//! Wire types come from `agent-client-protocol` 5.0.0; this module handles
//! JSON-RPC framing + request correlation while keeping the driver sans-IO so
//! every spawn passes Otto's safety pipeline. Lifecycle and entity mapping are
//! ported from T3 Code (MIT, Copyright (c) 2026 T3 Tools Inc.):
//! * `AcpSessionRuntime.ts` — `initialize`, `session/load` or `session/new`,
//!   lazy `authenticate` on `-32000`, and `session/cancel` on interrupt.
//! * `runtimeModel.ts` + `adapter.ts` — message/thought chunk accumulation,
//!   `tool_call` / `tool_call_update` merging, `plan` → `todo_list`,
//!   `session/request_permission` → `approval_request`, and `session/set_mode`
//!   or respawn for live access-mode transitions.

use std::collections::{HashMap, VecDeque};

use agent_client_protocol::schema::{MaybeUndefined, ProtocolVersion};
use agent_client_protocol::v1::schema::{
    AuthenticateRequest, AvailableCommandInput, BlobResourceContents, CancelNotification,
    CompactionStatus, ContentBlock, EmbeddedResource, EmbeddedResourceResource, EnvVariable,
    ImageContent, InitializeRequest, InitializeResponse, LoadSessionRequest, McpServer,
    McpServerStdio, NewSessionRequest, NewSessionResponse, PermissionOption, PermissionOptionKind,
    Plan, PlanEntryStatus, PromptRequest, PromptResponse, RequestPermissionOutcome,
    RequestPermissionRequest, RequestPermissionResponse, SelectedPermissionOutcome,
    SessionNotification, SessionUpdate, SetSessionModeRequest, StopReason, TextContent, ToolCall,
    ToolCallContent, ToolCallStatus, ToolCallUpdate, ToolKind,
};
use otto_core::agent_runtime::{
    ApprovalDecision, ApprovalOption, BackgroundTask, PlanStep, PlanStepStatus, ProviderCommand,
    ProviderFailure, RequestKind, RuntimeAccessMode, RuntimeCapabilities, RuntimePromptReq,
    RuntimeProvider, ThreadTokenUsage, TurnItem, TurnItemKind, TurnItemStatus, TurnTerminalStatus,
    TurnTokenUsage, EFFORT_RANK,
};
use serde_json::{json, Value};

use crate::driver::{
    compaction_notice, item, notice, provider_name, rpc_failure, rpc_key, tool_error, DriverError,
    DriverEvent, DriverStep, LaunchContext, LaunchSpec, McpDelivery, McpServerDef, SidebandPayload,
    SidebandPoll, SidebandResumeSeed, SidebandSource, StructuredDriver,
};

#[path = "acp_bg.rs"]
mod bg;
#[path = "acp_client.rs"]
pub(crate) mod client;
#[path = "acp_raw.rs"]
mod raw;
#[path = "acp_subagent.rs"]
pub(crate) mod subagent;

const M_INITIALIZE: &str = "initialize";
const M_AUTHENTICATE: &str = "authenticate";
/// JSON-RPC error ACP agents return from `session/new|load` when the user
/// must authenticate first.
const AUTH_REQUIRED: i64 = -32000;
const M_SESSION_NEW: &str = "session/new";
const M_SET_CONFIG_OPTION: &str = "session/set_config_option";
const M_SESSION_LOAD: &str = "session/load";
/// ACP `session/resume` (`sessionCapabilities.resume`): the session back
/// without replaying its history (live: agy, grok, opencode, muse).
const M_SESSION_RESUME: &str = "session/resume";
const M_SESSION_PROMPT: &str = "session/prompt";
const M_SESSION_CANCEL: &str = "session/cancel";
const M_SESSION_UPDATE: &str = "session/update";
const M_REQUEST_PERMISSION: &str = "session/request_permission";
const M_SESSION_SET_MODE: &str = "session/set_mode";
const M_SESSION_SET_MODEL: &str = "session/set_model";

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum Phase {
    #[default]
    Initializing,
    Creating {
        loading: bool,
    },
    Authenticating {
        load: bool,
    },
    Ready,
}

/// An access mode's extra `(head, tail)` argv.
pub type AccessArgs = fn(RuntimeAccessMode) -> (Vec<String>, Vec<String>);

/// An access mode's extra environment.
pub type AccessEnv = fn(RuntimeAccessMode) -> Vec<(String, String)>;

/// What differs between ACP agents. The launch is
/// `program head_args… [model_flag <model>] [context flags] tail_args…`.
#[derive(Debug, Clone, Default)]
pub struct AcpFlavor {
    pub program: String,
    pub head_args: Vec<String>,
    /// Args after the context hook's injected flags (a subcommand).
    pub tail_args: Vec<String>,
    /// The flag that selects a model (`--model`), placed in the head.
    pub model_flag: String,
    /// The `authenticate` method to prefer when the agent advertises it.
    pub auth_preference: Option<String>,
    /// `_meta` for `initialize` / `session/cancel`.
    pub initialize_meta: Option<Value>,
    pub cancel_meta: Option<Value>,
    /// Extra argv per access mode.
    pub access_args: Option<AccessArgs>,
    /// The `session/set_mode` id of each access mode (live switching); `None`
    /// for a mode only a launch flag gives (agy's full access), which the
    /// runtime enters and leaves by respawning.
    pub mode_id: Option<fn(RuntimeAccessMode) -> Option<&'static str>>,
    /// Environment per access mode (opencode's `OPENCODE_CONFIG_CONTENT`; Muse's
    /// launcher switch, the same in every mode).
    pub access_env: Option<AccessEnv>,
    /// When the agent takes its model as a session config option instead of
    /// a launch flag (opencode: `session/set_config_option {configId:
    /// "model"}`), that option's id; `model_flag` is then unused.
    pub model_config_id: Option<&'static str>,
    /// The model goes as `session/set_model {modelId}` once ready (agy:
    /// `gemini-4-argon-high` — the config option would drop the effort).
    pub set_model: bool,
    /// When the agent exposes reasoning effort as a session config option
    /// (agy: `session/set_config_option {configId: "thought_level"}`), that
    /// option's id.
    pub effort_config_id: Option<&'static str>,
    /// A native method that adds a message to the running prompt (Grok's
    /// `_x.ai/interject`, T3 Code #37335): steering, `capabilities().steer`.
    pub interject: Option<&'static str>,
    /// The env var naming an Otto context bundle (`AGENTS.md` +
    /// `.agents/skills`) to send as `_meta.antigravity.systemPrompt.append`
    /// instead of a workspace root (otto-context `AGY_BUNDLE_ENV`).
    pub bundle_env: Option<&'static str>,
    /// The flag that turns the agent's own OS sandbox off when Otto's
    /// Seatbelt already confines it (macOS refuses a nested sandbox).
    pub own_sandbox_off: Option<&'static str>,
    /// Never answer `auth_required` with `authenticate`: the agent's only
    /// method is an interactive browser login the person starts themselves
    /// (Muse's `muse-login`), so the handshake fails with its own "log in"
    /// message instead of a daemon opening a browser.
    pub no_authenticate: bool,
    /// Serve `fs/read_text_file` / `fs/write_text_file` (advertised in
    /// `initialize`): only for an agent the live tests showed uses them.
    pub client_fs: bool,
    /// Serve `terminal/*` (Otto runs the command under the agent's sandbox
    /// and streams its output into the tool row).
    pub client_terminal: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) enum SegmentKind {
    Message,
    Thought,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Segment {
    kind: SegmentKind,
    item_id: String,
    message_id: String,
    /// The agent named it (`messageId`): later chunks of that id append.
    named: bool,
    text: String,
    started_at: chrono::DateTime<chrono::Utc>,
}

#[derive(Debug, Default, Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Tool {
    pub(super) title: String,
    pub(super) kind: ToolKind,
    pub(super) status: ToolCallStatus,
    pub(super) raw_input: Option<Value>,
    pub(super) output: String,
    pub(super) images: Vec<otto_core::agent_runtime::ToolOutputImage>,
    pub(super) file: Option<String>,
    pub(super) diff: Option<(Option<String>, String)>,
    /// Every `(path, old, new)` diff of the call — a multi-file edit renders
    /// as one patch (T3 Code `acpToolCallDiffPatch`, ddd36a53a5).
    pub(super) diffs: Vec<(String, Option<String>, String)>,
    /// The client terminal this call embeds (`terminal` content).
    #[serde(default)]
    pub(super) terminal: Option<String>,
    /// Every `locations[].path` the agent named.
    #[serde(default)]
    pub(super) locations: Vec<String>,
}

const MAX_RETIRED_TOOL_IDS: usize = 512;

impl Tool {
    pub(super) fn compact_terminal(&mut self) {
        if matches!(
            self.status,
            ToolCallStatus::Completed | ToolCallStatus::Failed
        ) {
            self.output = String::new();
            self.diff = None;
            self.diffs = Vec::new();
            self.images = Vec::new();
            if matches!(
                self.kind,
                ToolKind::Edit | ToolKind::Delete | ToolKind::Move
            ) {
                self.raw_input = None;
            }
        }
    }
}

/// A bounded contextual unified patch over `diffs`, one `diff --git` section each.
pub(super) fn multi_file_patch(diffs: &[(String, Option<String>, String)]) -> String {
    crate::diff_compact::multi_file_unified_patch(diffs)
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(super) struct Pending {
    rpc_id: Value,
    options: Vec<PermissionOption>,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct AcpDriverState {
    pub(super) cwd: String,
    pub(super) resume: Option<String>,
    /// There was a conversation to resume but the agent could not load it
    /// (no `loadSession`, or the load failed): the new session's `Ready`
    /// says so in the thread instead of silently starting fresh.
    #[serde(default)]
    pub(super) lost_resume: bool,
    /// Otto runtime session ID (for scoping `/tmp/otto-acp-bg/<sid>/` tails).
    #[serde(default)]
    pub(super) otto_session_id: Option<String>,
    /// `agy` conversation IDs observed for this session and its spawned
    /// subagents (caps brain-dir transcript/task-log reads to this session).
    #[serde(default)]
    pub(super) observed_agy_conv_ids: VecDeque<String>,
    /// Handed over in `session/new|load` (Otto's `otto` tools + the user's
    /// enabled servers — the set a terminal session gets). Never in a
    /// snapshot: their env carries the session's credential, and the
    /// handshake that sends them is over by the time a state is restored.
    #[serde(skip)]
    pub(super) mcp_servers: Vec<McpServerDef>,
    /// The model to apply through `flavor.model_config_id` / `set_model`
    /// once ready.
    pub(super) model: Option<String>,
    pub(super) config_req: Option<u64>,
    /// The model write failed for a reason other than a refusal: written
    /// again before the next prompt (T3 Code #34782).
    pub(super) config_retry: bool,
    pub(super) next_id: u64,
    pub(super) phase: Phase,
    pub(super) init_id: u64,
    pub(super) create_id: u64,
    pub(super) auth_id: u64,
    /// `authMethods` from `initialize`: `(id, type)`.
    pub(super) auth_methods: Vec<(String, Option<String>)>,
    pub(super) authenticated: bool,
    pub(super) session_id: Option<String>,
    /// `promptCapabilities.image` / `.embeddedContext` from `initialize`.
    pub(super) images: bool,
    pub(super) embedded: bool,
    /// `modes.availableModes[].id` from `session/new|load` — or, for an
    /// agent with no session modes, the values of its `mode`-category
    /// config option (opencode's `build` / `plan`).
    pub(super) modes: Vec<String>,
    /// That config option's id: modes then go as `session/set_config_option`.
    pub(super) mode_config_id: Option<String>,
    /// The agent advertised `sessionCapabilities.resume` and a resume is
    /// in flight as `session/resume` (a refusal retries as `session/load`).
    pub(super) resuming: bool,
    /// `models.availableModels[].modelId` from `session/new|load`, when advertised.
    #[serde(default)]
    pub(super) available_models: Vec<String>,
    /// Offered `thought_level` values from `configOptions`, when advertised.
    pub(super) effort_options: Vec<String>,
    pub(super) access: RuntimeAccessMode,
    pub(super) mode_req: Option<u64>,
    pub(super) effort_req: Option<u64>,
    /// In-flight interjections (`AcpFlavor::interject`).
    #[serde(default)]
    pub(super) interject_reqs: Vec<u64>,
    /// The bundle text for `_meta.antigravity.systemPrompt.append`.
    pub(super) system_append: Option<String>,
    pub(super) queued: Option<(String, RuntimePromptReq)>,
    pub(super) prompt_id: Option<u64>,
    pub(super) turn_id: Option<String>,
    /// Armed when `session/cancel` goes out for an active `session/prompt` so
    /// buffered pre-cancel notifications are dropped until `PromptResponse`
    /// settles or the next prompt begins.
    #[serde(default)]
    pub(super) cancelling: bool,
    pub(super) autonomous: bool,
    pub(super) auto_turns: u64,
    pub(super) segment: Option<Segment>,
    pub(super) segments: u32,
    /// This turn's closed named segments by `messageId`: a message that
    /// resumes after a thought (or another message) keeps its text and its
    /// place (T3 Code #33604).
    pub(super) closed_named: HashMap<String, Segment>,
    pub(super) tools: HashMap<String, Tool>,
    #[serde(default)]
    pub(super) retired_tool_ids: VecDeque<String>,
    pub(super) bg_tasks: Vec<BackgroundTask>,
    #[serde(default)]
    pub(super) bg_meta: HashMap<String, subagent::BgTaskMeta>,
    #[serde(default)]
    pub(super) subagents: HashMap<String, subagent::AcpSubagent>,
    #[serde(default)]
    pub(super) child_tools: HashMap<String, subagent::ChildToolEntry>,
    #[serde(default)]
    pub(super) finished_bg_ids: VecDeque<String>,
    pub(super) pending: HashMap<String, Pending>,
    pub(super) questions: HashMap<String, (Value, Vec<otto_core::agent_runtime::RuntimeQuestion>)>,
    #[serde(default, alias = "agy_tail")]
    pub(super) sideband: crate::acp_sideband::AcpSideband,
    /// agy swarm members this session added, by member name.
    pub(super) swarm: HashMap<String, TurnItem>,
    /// Every swarm member that has a card (open or settled): a `session/load`
    /// replay cards only members it never saw (added before an agent restart
    /// or before cards existed), never one twice.
    #[serde(default)]
    pub(super) swarm_seen: std::collections::HashSet<String>,
    /// The mode the session opened in (`modes.currentModeId`): where leaving
    /// plan mode returns when the access mode has no mode of its own.
    pub(super) base_mode: Option<String>,
    /// The session is in the agent's `plan` mode.
    pub(super) plan_active: bool,
    /// Grok's silent "denied by prompt policy" was explained once.
    pub(super) denied_noticed: bool,
    /// The agent's sandbox wrapper (`sandbox-exec -p <profile>`), reused for
    /// client terminals; empty when the agent runs unconfined.
    pub(super) confine: Vec<String>,
    /// The agent's prepared env (a client terminal gets it too). Not in a
    /// snapshot.
    #[serde(skip)]
    pub(super) child_env: Vec<(String, String)>,
    pub(super) terminals: u64,
    /// Each open client terminal's output so far (bounded), by terminal id.
    #[serde(skip)]
    pub(super) terminal_output: HashMap<String, String>,
    pub(super) client_seq: u64,
    pub(super) client_waits: HashMap<String, client::ClientWait>,
    /// Restored waits nobody can answer any more (answered with an error).
    #[serde(skip)]
    pub(super) client_orphans: Vec<Value>,
    /// Supervised writes / commands waiting for the person.
    pub(super) client_held: HashMap<String, client::HeldCall>,
    /// The session's running cost (`usage_update.cost`, USD) and its value
    /// when the running turn started.
    pub(super) cost_total: Option<f64>,
    pub(super) cost_at_turn: Option<f64>,
}

impl AcpDriverState {
    pub(super) fn note_agy_conv_id(&mut self, id: &str) {
        let trimmed = id.trim();
        if trimmed.is_empty() || self.observed_agy_conv_ids.iter().any(|x| x == trimmed) {
            return;
        }
        while self.observed_agy_conv_ids.len() >= 64 {
            self.observed_agy_conv_ids.pop_front();
        }
        self.observed_agy_conv_ids.push_back(trimmed.to_string());
    }
}

pub struct AcpDriver {
    provider: RuntimeProvider,
    flavor: AcpFlavor,
    pub(super) state: AcpDriverState,
}

impl AcpDriver {
    /// `program args…` is the ACP agent command (e.g. `gemini --acp`).
    pub fn new(provider: RuntimeProvider, program: impl Into<String>, args: Vec<String>) -> Self {
        Self::with_flavor(
            provider,
            AcpFlavor {
                program: program.into(),
                head_args: args,
                model_flag: "--model".into(),
                ..AcpFlavor::default()
            },
        )
    }

    pub fn with_flavor(provider: RuntimeProvider, flavor: AcpFlavor) -> Self {
        Self {
            provider,
            flavor,
            state: AcpDriverState {
                cwd: String::new(),
                resume: None,
                otto_session_id: None,
                observed_agy_conv_ids: VecDeque::new(),
                mcp_servers: Vec::new(),
                model: None,
                config_req: None,
                config_retry: false,
                next_id: 0,
                phase: Phase::Initializing,
                init_id: 0,
                create_id: 0,
                auth_id: 0,
                auth_methods: Vec::new(),
                authenticated: false,
                session_id: None,
                images: false,
                embedded: false,
                modes: Vec::new(),
                mode_config_id: None,
                resuming: false,
                available_models: Vec::new(),
                effort_options: Vec::new(),
                access: RuntimeAccessMode::default(),
                mode_req: None,
                effort_req: None,
                interject_reqs: Vec::new(),
                system_append: None,
                queued: None,
                prompt_id: None,
                turn_id: None,
                cancelling: false,
                autonomous: false,
                auto_turns: 0,
                segment: None,
                segments: 0,
                closed_named: HashMap::new(),
                tools: HashMap::new(),
                retired_tool_ids: VecDeque::new(),
                bg_tasks: Vec::new(),
                bg_meta: HashMap::new(),
                subagents: HashMap::new(),
                child_tools: HashMap::new(),
                finished_bg_ids: VecDeque::new(),
                pending: HashMap::new(),
                questions: HashMap::new(),
                sideband: crate::acp_sideband::AcpSideband::for_provider(provider),
                swarm: HashMap::new(),
                swarm_seen: Default::default(),
                base_mode: None,
                plan_active: false,
                denied_noticed: false,
                confine: Vec::new(),
                child_env: Vec::new(),
                terminals: 0,
                terminal_output: HashMap::new(),
                client_seq: 0,
                client_waits: HashMap::new(),
                client_orphans: Vec::new(),
                client_held: HashMap::new(),
                cost_total: None,
                cost_at_turn: None,
                lost_resume: false,
            },
        }
    }

    pub(super) fn retire_tool_id(&mut self, id: String) {
        if id.is_empty() || self.state.retired_tool_ids.iter().any(|r| r == &id) {
            return;
        }
        if self.state.retired_tool_ids.len() >= MAX_RETIRED_TOOL_IDS {
            self.state.retired_tool_ids.pop_front();
        }
        self.state.retired_tool_ids.push_back(id);
    }

    pub(super) fn retire_tools(&mut self) {
        let ids: Vec<String> = self.state.tools.drain().map(|(id, _)| id).collect();
        for id in ids {
            if !self.state.subagents.contains_key(&id) {
                self.retire_tool_id(id);
            }
        }
    }

    pub(super) fn settle_foreground_subagents(
        &mut self,
        step: &mut DriverStep,
        status: TurnItemStatus,
    ) {
        let fg_ids: Vec<String> = self
            .state
            .subagents
            .iter()
            .filter(|(_, a)| !a.background && a.status.is_open())
            .map(|(k, _)| k.clone())
            .collect();
        for cid in fg_ids {
            self.finish_subagent_children(step, &cid, status);
            if let Some(mut card) = self.state.subagents.remove(&cid) {
                card.status = status;
                step.push(DriverEvent::TurnItem(self.subagent_turn_item(&card)));
                self.retire_tool_id(cid);
            }
        }
    }

    /// Override the provider's on-disk sideband home/data directory (for
    /// isolated unit/integration tests so real `$HOME` is never consulted).
    pub fn set_sideband_root_dir(&mut self, dir: impl Into<String>) {
        self.state.sideband.set_root_dir(Some(dir.into()));
    }

    /// Override an explicit JSONL sideband file path (`OTTO_ACP_SIDEBAND_FILE`).
    pub fn set_custom_sideband_file(&mut self, path: impl Into<String>) {
        self.state.sideband.set_custom_file(Some(path.into()));
    }

    /// Override the `agy` brain directory (for unit/integration tests so real
    /// `$HOME` is never consulted).
    pub fn set_agy_brain_dir(&mut self, dir: impl Into<String>) {
        self.set_sideband_root_dir(dir);
    }

    /// The Gemini CLI in ACP mode (T3 Code runs it from the ACP registry's
    /// `@google/gemini-cli` entry with the registry's argv; the installed
    /// CLI's own flag is `--acp`).
    pub fn gemini(program: &str) -> Self {
        let mut d = Self::new(RuntimeProvider::Gemini, program, vec!["--acp".into()]);
        d.flavor.access_args = Some(|m| {
            let mode = match m {
                RuntimeAccessMode::ApprovalRequired => "default",
                RuntimeAccessMode::AutoAcceptEdits => "auto_edit",
                RuntimeAccessMode::FullAccess => "yolo",
            };
            (vec!["--approval-mode".into(), mode.into()], Vec::new())
        });
        d.flavor.mode_id = Some(|m| {
            Some(match m {
                RuntimeAccessMode::ApprovalRequired => "default",
                RuntimeAccessMode::AutoAcceptEdits => "autoEdit",
                RuntimeAccessMode::FullAccess => "yolo",
            })
        });
        d
    }

    /// Antigravity over its own ACP server (`agy acp`; verified live against
    /// 1.5.8). `session/new|load` advertise the modes `default`,
    /// `accept-edits`, `auto` and `plan`: approval-required and
    /// auto-accept-edits are `session/set_mode` (also right after the session
    /// opens, `agy acp` has no mode flag); full access is the launch flag
    /// `--dangerously-skip-permissions` (`auto` still asks before commands
    /// that leave its sandbox), so entering or leaving it respawns. The model
    /// is `session/set_model` with a `modelId` such as
    /// `gemini-4-argon-high` (`agy acp` takes no `--model`). Otto's context
    /// bundle goes as the session's system-prompt append, not `--add-dir`:
    /// agy runs commands in the alphabetically-first workspace root.
    pub fn agy(program: &str) -> Self {
        Self::with_flavor(
            RuntimeProvider::Agy,
            AcpFlavor {
                program: program.into(),
                head_args: vec!["acp".into()],
                access_args: Some(|m| match m {
                    RuntimeAccessMode::FullAccess => {
                        (vec!["--dangerously-skip-permissions".into()], Vec::new())
                    }
                    _ => (Vec::new(), Vec::new()),
                }),
                mode_id: Some(|m| match m {
                    RuntimeAccessMode::ApprovalRequired => Some("default"),
                    RuntimeAccessMode::AutoAcceptEdits => Some("accept-edits"),
                    RuntimeAccessMode::FullAccess => None,
                }),
                set_model: true,
                effort_config_id: Some("thought_level"),
                bundle_env: Some("OTTO_AGY_CONTEXT_DIR"),
                ..AcpFlavor::default()
            },
        )
    }

    /// Grok over ACP, as T3 Code launches it (`grokAcpSpawnArgs`,
    /// `acpSupport.ts:47-58`): approval-required is `--permission-mode
    /// default agent stdio` (every tool call asks), auto-accept-edits
    /// `--permission-mode acceptEdits` (1.0.92), full-access `agent
    /// --always-approve stdio`. `-m` is grok's top-level model flag; context flags
    /// (`--rules=`) must precede `agent`, hence the tail. Auth: `xai.api_key` when
    /// `XAI_API_KEY` is set, else `cached_token` (`acpSupport.ts:21-25,
    /// 77-81`); `_meta` on initialize / cancel (`:98-101`). T3 Code also sets
    /// `GROK_OAUTH2_REFERRER=t3code` — a T3-specific attribution Otto omits.
    pub fn grok() -> Self {
        let auth = if std::env::var_os("XAI_API_KEY").is_some() {
            "xai.api_key"
        } else {
            "cached_token"
        };
        Self::with_flavor(
            RuntimeProvider::Grok,
            AcpFlavor {
                program: "grok".into(),
                access_args: Some(|m| {
                    let v = |a: &[&str]| a.iter().map(|s| s.to_string()).collect();
                    match m {
                        RuntimeAccessMode::FullAccess => {
                            (Vec::new(), v(&["agent", "--always-approve", "stdio"]))
                        }
                        RuntimeAccessMode::AutoAcceptEdits => (
                            v(&["--permission-mode", "acceptEdits"]),
                            v(&["agent", "stdio"]),
                        ),
                        _ => (v(&["--permission-mode", "default"]), v(&["agent", "stdio"])),
                    }
                }),
                model_flag: "-m".into(),
                effort_config_id: Some("reasoning_effort"),
                auth_preference: Some(auth.into()),
                initialize_meta: Some(json!({"clientType": "extension"})),
                cancel_meta: Some(json!({"cancelTrigger": "ctrl_c"})),
                interject: Some("_x.ai/interject"),
                // Grok 1.0.92 reads files and runs commands through the client
                // when it may (live 2026-11-23); agy, opencode and Muse keep
                // their own tools whatever is advertised.
                client_fs: true,
                client_terminal: true,
                ..AcpFlavor::default()
            },
        )
    }

    /// opencode over ACP (`opencode acp`; verified live against 2.3.0): no
    /// model flag — the model is the `model` session config option (value
    /// `provider/model`), effort the `effort` one, plan the `plan` value of
    /// its `mode` option (`build` / `plan`; it has no session modes); it
    /// takes stdio `mcpServers` with their env in `session/new|load`. The
    /// access mode is a `permission` object in `OPENCODE_CONFIG_CONTENT`
    /// (inline config merged over the user's; 2.x dropped
    /// `OPENCODE_PERMISSION`, which 2.3.0 ignores): full access allows
    /// everything; the others ask for everything but reads (plus edits for
    /// auto-accept-edits) through `session/request_permission`. Without it
    /// opencode's own default is to allow, so every mode sets it.
    pub fn opencode() -> Self {
        Self::with_flavor(
            RuntimeProvider::Opencode,
            AcpFlavor {
                program: "opencode".into(),
                tail_args: vec!["acp".into()],
                access_env: Some(|m| {
                    let rules = match m {
                        RuntimeAccessMode::FullAccess => r#"{"*":"allow"}"#,
                        RuntimeAccessMode::AutoAcceptEdits => {
                            r#"{"*":"ask","read":"allow","glob":"allow","grep":"allow","list":"allow","edit":"allow"}"#
                        }
                        RuntimeAccessMode::ApprovalRequired => {
                            r#"{"*":"ask","read":"allow","glob":"allow","grep":"allow","list":"allow"}"#
                        }
                    };
                    vec![(
                        "OPENCODE_CONFIG_CONTENT".into(),
                        format!(r#"{{"permission":{rules}}}"#),
                    )]
                }),
                model_config_id: Some("model"),
                effort_config_id: Some("effort"),
                ..AcpFlavor::default()
            },
        )
    }

    /// Meta's Muse Code over ACP (`muse acp`; verified live against 1.4.25).
    /// Its session modes are approval policies, switched live with
    /// `session/set_mode`: approval-required asks for anything no permission
    /// rule covers (`promptUnmatched`), auto-accept-edits is Muse's own
    /// default (`onRequest`: asks only when an action needs it), full access
    /// runs everything (`allowAll`). No plan mode. The model is the `model`
    /// session config option (switched live; 1.4.25 advertises it); effort
    /// rides the `thought_level` option only where the session advertises it
    /// (1.4.25 offers none: "this session offers no option"). `--trust-workspace` loads the
    /// folder's skills and rules (Otto pre-trusts every session cwd); Muse's
    /// own sandbox is off under Otto's Seatbelt. `MUSE_NO_AUTO_UPDATE=1` stops the launcher from checking for
    /// a new binary on every spawn (Otto's update pass does that), as T3 Code
    /// launches it. A signed-out Muse refuses `session/new` until the person
    /// runs `muse login`.
    pub fn muse(program: &str) -> Self {
        Self::with_flavor(
            RuntimeProvider::Muse,
            AcpFlavor {
                program: program.into(),
                head_args: vec!["acp".into(), "--trust-workspace".into()],
                model_flag: "--model".into(),
                mode_id: Some(|m| {
                    Some(match m {
                        RuntimeAccessMode::ApprovalRequired => "promptUnmatched",
                        RuntimeAccessMode::AutoAcceptEdits => "onRequest",
                        RuntimeAccessMode::FullAccess => "allowAll",
                    })
                }),
                access_env: Some(|_| {
                    let (k, v) = otto_core::agent_runtime::MUSE_NO_AUTO_UPDATE;
                    vec![(k.into(), v.into())]
                }),
                model_config_id: Some("model"),
                effort_config_id: Some("thought_level"),
                own_sandbox_off: Some("--disable-sandbox"),
                no_authenticate: true,
                ..AcpFlavor::default()
            },
        )
    }

    fn acp_mcp_servers(&self) -> Vec<McpServer> {
        self.state
            .mcp_servers
            .iter()
            .map(|s| {
                McpServer::Stdio(
                    McpServerStdio::new(s.name.clone(), s.command.clone())
                        .args(s.args.clone())
                        .env(
                            s.env
                                .iter()
                                .map(|(k, v)| EnvVariable::new(k.clone(), v.clone()))
                                .collect(),
                        ),
                )
            })
            .collect()
    }

    fn request(&mut self, method: &str, params: impl serde::Serialize) -> (u64, String) {
        self.state.next_id += 1;
        let id = self.state.next_id;
        let line = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string();
        (id, line)
    }

    fn create_session(&mut self, step: &mut DriverStep, load: bool) {
        let line = match (&self.state.resume, load) {
            (Some(sid), true) => {
                let mut req = LoadSessionRequest::new(sid.clone(), self.state.cwd.clone())
                    .mcp_servers(self.acp_mcp_servers());
                req.meta = self.session_meta();
                // Same params as `session/load`.
                let method = if self.state.resuming {
                    M_SESSION_RESUME
                } else {
                    M_SESSION_LOAD
                };
                let (id, l) = self.request(method, req);
                self.state.create_id = id;
                l
            }
            _ => {
                let mut req = NewSessionRequest::new(self.state.cwd.clone())
                    .mcp_servers(self.acp_mcp_servers());
                req.meta = self.session_meta();
                let (id, l) = self.request(M_SESSION_NEW, req);
                self.state.create_id = id;
                l
            }
        };
        self.state.phase = Phase::Creating { loading: load };
        step.write(line);
    }

    /// The agent could not reload the earlier conversation (a model or
    /// access switch restarts it, as does a daemon restart): a session
    /// boundary plus a notice, never a silent new session.
    fn push_fresh_boundary(&self, step: &mut DriverStep) {
        let id = otto_core::new_id();
        step.push(DriverEvent::TurnItem(item(
            format!("acp-reset:{id}:session-boundary"),
            TurnItemStatus::Completed,
            None,
            TurnItemKind::SessionBoundary { handoff: None },
        )));
        step.push(DriverEvent::TurnItem(notice(
            format!("acp-reset:{id}"),
            format!(
                "{} could not reload this conversation \u{2014} it starts fresh from here and does not remember the messages above.",
                provider_name(self.provider)
            ),
        )));
    }

    fn session_meta(&self) -> Option<serde_json::Map<String, Value>> {
        let text = self.state.system_append.as_ref()?;
        let meta = json!({"antigravity": {"systemPrompt": {"append": text}}});
        meta.as_object().cloned()
    }

    /// The model as a session config option (`flavor.model_config_id`) or
    /// `session/set_model` (`flavor.set_model`).
    fn write_model_config(&mut self, step: &mut DriverStep) {
        if let (true, Some(model), Some(sid)) = (
            self.flavor.set_model,
            self.state.model.clone(),
            self.state.session_id.clone(),
        ) {
            let (id, line) = self.request(
                M_SESSION_SET_MODEL,
                json!({"sessionId": sid, "modelId": model}),
            );
            self.state.config_req = Some(id);
            step.write(line);
        } else if let (Some(config_id), Some(model), Some(sid)) = (
            self.flavor.model_config_id,
            self.state.model.clone(),
            self.state.session_id.clone(),
        ) {
            let (id, line) = self.request(
                M_SET_CONFIG_OPTION,
                json!({"sessionId": sid, "configId": config_id, "value": model}),
            );
            self.state.config_req = Some(id);
            step.write(line);
        }
    }

    fn update_effort_options(&mut self, root: &Value) {
        let Some(config_id) = self.flavor.effort_config_id else {
            return;
        };
        let Some(opts) = root
            .get("configOptions")
            .and_then(Value::as_array)
            .and_then(|arr| {
                arr.iter()
                    .find(|c| c.get("id").and_then(Value::as_str) == Some(config_id))
            })
            .and_then(|c| c.get("options"))
            .and_then(Value::as_array)
        else {
            return;
        };
        let vals: Vec<String> = opts
            .iter()
            .filter_map(|o| o.get("value").and_then(Value::as_str).map(str::to_string))
            // opencode's `default` is "no pick", not a tier.
            .filter(|v| v != "default")
            .collect();
        if !vals.is_empty() {
            self.state.effort_options = vals;
        }
    }

    fn efforts_for_model(&self, model: &str) -> Option<Vec<String>> {
        if self.state.available_models.is_empty() {
            return None;
        }
        const SUFFIXES: [(&str, &str); 7] = [
            ("-minimal", "low"),
            ("-low", "low"),
            ("-medium", "medium"),
            ("-high", "high"),
            ("-xhigh", "xhigh"),
            ("-max", "max"),
            ("-ultra", "max"),
        ];
        let base = SUFFIXES
            .iter()
            .find_map(|(suf, _)| model.strip_suffix(suf))
            .unwrap_or(model);
        let mut opts: Vec<String> = Vec::new();
        for (_, canonical) in SUFFIXES {
            if opts.iter().any(|o| o == canonical) {
                continue;
            }
            let has_variant = SUFFIXES.iter().any(|(suf, c)| {
                *c == canonical && {
                    let candidate = format!("{base}{suf}");
                    self.state.available_models.iter().any(|m| m == &candidate)
                }
            });
            if has_variant {
                opts.push(canonical.to_string());
            }
        }
        if !opts.is_empty() {
            return Some(opts);
        }
        if self.state.available_models.iter().any(|m| m == base) {
            return Some(Vec::new());
        }
        None
    }

    fn model_is_untiered(&self) -> bool {
        self.state
            .model
            .as_deref()
            .and_then(|m| self.efforts_for_model(m))
            .is_some_and(|opts| opts.is_empty())
    }

    fn resolve_effort(&self, raw: &str) -> Result<String, DriverError> {
        self.resolve_effort_with(&self.state.effort_options, raw)
    }

    fn resolve_effort_with(&self, options: &[String], raw: &str) -> Result<String, DriverError> {
        let name = self.provider.as_str();
        if self.flavor.effort_config_id.is_none() {
            return Err(DriverError(format!(
                "{name} does not take a reasoning effort"
            )));
        }
        // A session that advertised neither an effort option nor tiered
        // models takes none (Muse 1.4.25: "this session offers no option").
        if self.state.phase == Phase::Ready
            && options.is_empty()
            && self.state.available_models.is_empty()
        {
            return Err(DriverError(format!(
                "{name} does not take a reasoning effort"
            )));
        }
        let raw = raw.trim().to_ascii_lowercase();
        // A tier the agent lists by that name goes as is (Muse's `none`,
        // `minimal`, `ultra`).
        if options.contains(&raw) {
            return Ok(raw);
        }
        let rank = match raw.as_str() {
            "minimal" | "low" => 0,
            "medium" => 1,
            "high" => 2,
            "xhigh" => 3,
            "max" | "ultra" => 4,
            other => {
                return Err(DriverError(format!(
                    "{name} has no '{other}' reasoning effort"
                )))
            }
        };
        const LEVELS: [&str; 5] = ["low", "medium", "high", "xhigh", "max"];
        let wanted = LEVELS[rank];
        if options.is_empty() || options.iter().any(|o| o == wanted) {
            return Ok(wanted.to_string());
        }
        for candidate in LEVELS[..=rank].iter().rev() {
            if options.iter().any(|o| o == candidate) {
                return Ok((*candidate).to_string());
            }
        }
        Ok(options[0].clone())
    }

    fn send_prompt(&mut self, step: &mut DriverStep, turn_id: String, req: &RuntimePromptReq) {
        let model_changed = req
            .model
            .as_ref()
            .is_some_and(|m| self.state.model.as_ref() != Some(m));
        if let Some(m) = req.model.as_ref().filter(|_| model_changed) {
            self.state.model = Some(m.clone());
            if let Some(opts) = self.efforts_for_model(m) {
                self.state.effort_options = opts;
            }
        }
        if std::mem::take(&mut self.state.config_retry) || model_changed {
            self.write_model_config(step);
        }
        let sid = self.state.session_id.clone().unwrap_or_default();
        if !self.model_is_untiered() {
            if let Some(raw) = req.effort.as_deref().filter(|e| !e.trim().is_empty()) {
                if let (Some(config_id), Ok(val)) =
                    (self.flavor.effort_config_id, self.resolve_effort(raw))
                {
                    let (id, line) = self.request(
                        M_SET_CONFIG_OPTION,
                        json!({"sessionId": sid, "configId": config_id, "value": val}),
                    );
                    self.state.effort_req = Some(id);
                    step.write(line);
                }
            }
        }
        let mut blocks: Vec<ContentBlock> = req
            .images
            .iter()
            .map(|i| ContentBlock::Image(ImageContent::new(&i.data_base64, &i.mime)))
            .collect();
        blocks.extend(req.files.iter().map(|f| {
            let blob = BlobResourceContents::new(&f.data_base64, format!("attachment:{}", f.name))
                .mime_type(f.mime.clone());
            ContentBlock::Resource(EmbeddedResource::new(
                EmbeddedResourceResource::BlobResourceContents(blob),
            ))
        }));
        if !req.text.is_empty() {
            blocks.push(ContentBlock::Text(TextContent::new(&req.text)));
        }
        let access_changed = req.access_mode.is_some_and(|m| m != self.state.access);
        self.state.access = req.access_mode.unwrap_or(self.state.access);
        if req.plan && self.state.modes.iter().any(|m| m == "plan") {
            // Plan mode is one of the agent's session modes (agy `plan`).
            if !self.state.plan_active {
                self.set_mode(step, &sid, "plan");
                self.state.plan_active = true;
            }
        } else if self.state.plan_active {
            // Leaving plan mode: back to the access mode's own mode, else
            // the one the session opened in.
            self.state.plan_active = false;
            let back = self
                .flavor
                .mode_id
                .and_then(|f| f(self.state.access))
                .map(str::to_string)
                .or_else(|| self.state.base_mode.clone());
            if let Some(mode) = back {
                self.set_mode(step, &sid, &mode);
            }
        } else if access_changed {
            self.write_mode(step, &sid, self.state.access);
        }
        self.state.cost_at_turn = self.state.cost_total;
        let (id, line) = self.request(M_SESSION_PROMPT, PromptRequest::new(sid, blocks));
        self.state.sideband.arm_burst_filter();
        self.retire_tools();
        self.state.cancelling = false;
        self.state.prompt_id = Some(id);
        self.state.turn_id = Some(turn_id);
        self.state.autonomous = false;
        self.state.segments = 0;
        self.state.closed_named.clear();
        self.settle_bg_on_user_prompt(step);
        step.write(line);
    }

    fn write_mode(&mut self, step: &mut DriverStep, sid: &str, mode: RuntimeAccessMode) {
        if let Some(mode_id) = self.flavor.mode_id.and_then(|f| f(mode)) {
            self.set_mode(step, sid, mode_id);
        }
    }

    /// The access mode whose `mode_id` is `mode`, when the agent moved to
    /// it on its own: the thread's access chip follows (gap 5).
    fn follow_mode(&mut self, step: &mut DriverStep, mode: &str) {
        let Some(f) = self.flavor.mode_id else { return };
        let to = [
            RuntimeAccessMode::ApprovalRequired,
            RuntimeAccessMode::AutoAcceptEdits,
            RuntimeAccessMode::FullAccess,
        ]
        .into_iter()
        .find(|m| f(*m) == Some(mode));
        if let Some(to) = to.filter(|to| *to != self.state.access) {
            self.state.access = to;
            step.push(DriverEvent::AgentSettings {
                model: None,
                access: Some(to),
            });
        }
    }

    /// The model the agent reports in a `config_option_update` (its model
    /// config option), when it differs from the one the person picked.
    fn follow_model(&mut self, step: &mut DriverStep, update: &Value) {
        let (Some(config_id), Some(current)) =
            (self.flavor.model_config_id, self.state.model.clone())
        else {
            return;
        };
        let reported = update
            .get("configOptions")
            .and_then(Value::as_array)
            .and_then(|a| {
                a.iter()
                    .find(|c| c.get("id").and_then(Value::as_str) == Some(config_id))
            })
            .and_then(|c| c.get("currentValue"))
            .and_then(Value::as_str);
        if let Some(m) = reported.filter(|m| *m != current) {
            self.state.model = Some(m.to_string());
            step.push(DriverEvent::AgentSettings {
                model: Some(m.to_string()),
                access: None,
            });
        }
    }

    /// `session/set_mode`, or the agent's mode config option.
    fn set_mode(&mut self, step: &mut DriverStep, sid: &str, mode: &str) {
        let (id, line) = match self.state.mode_config_id.clone() {
            Some(config_id) => self.request(
                M_SET_CONFIG_OPTION,
                json!({"sessionId": sid, "configId": config_id, "value": mode}),
            ),
            None => self.request(
                M_SESSION_SET_MODE,
                SetSessionModeRequest::new(sid.to_string(), mode.to_string()),
            ),
        };
        self.state.mode_req = Some(id);
        step.write(line);
    }

    /// T3 Code's choice (`AcpSessionRuntime.ts:1065-1074`): a preferred
    /// method the agent advertised, else the first `agent`-type (or untyped)
    /// one. `terminal` / `env_var` methods are never sent to `authenticate`.
    fn auth_method(&self) -> Option<String> {
        if self.flavor.no_authenticate {
            return None;
        }
        let advertised = |id: &str| self.state.auth_methods.iter().any(|(m, _)| m == id);
        self.flavor
            .auth_preference
            .iter()
            .find(|p| advertised(p))
            .cloned()
            .or_else(|| {
                self.state
                    .auth_methods
                    .iter()
                    .find(|(_, ty)| ty.as_deref().is_none_or(|t| t == "agent"))
                    .map(|(id, _)| id.clone())
            })
    }

    fn turn(&self) -> String {
        self.state.turn_id.clone().unwrap_or_default()
    }

    /// Open a fresh agent-initiated (`origin: agent`) turn when substantive
    /// ACP notifications/requests arrive outside an active `session/prompt`
    /// RPC (e.g. `agy` waking from `schedule`, background `run_command`,
    /// subagents, or `swarm_wait`).
    fn ensure_autonomous_turn(&mut self, step: &mut DriverStep) {
        self.ensure_autonomous_turn_inner(step, true);
    }

    pub(super) fn ensure_autonomous_turn_for_sideband_wakeup(&mut self, step: &mut DriverStep) {
        self.ensure_autonomous_turn_inner(step, false);
    }

    fn ensure_autonomous_turn_inner(&mut self, step: &mut DriverStep, native_wakeup: bool) {
        if self.state.phase != Phase::Ready
            || self.state.turn_id.is_some()
            || self.state.prompt_id.is_some()
            || self.state.queued.is_some()
        {
            return;
        }
        self.retire_tools();
        self.state.auto_turns += 1;
        let id = format!("auto-{}", self.state.auto_turns);
        self.state.turn_id = Some(id.clone());
        self.state.autonomous = true;
        self.state.segments = 0;
        self.state.closed_named.clear();
        step.push(DriverEvent::TurnStarted { turn_id: id });
        if native_wakeup {
            self.settle_bg_on_native_wakeup(step);
        }
    }

    fn segment_item(&self, seg: &Segment, streaming: bool) -> TurnItem {
        let status = if streaming {
            TurnItemStatus::Running
        } else {
            TurnItemStatus::Completed
        };
        let kind = match seg.kind {
            SegmentKind::Message => TurnItemKind::AssistantMessage {
                message_id: seg.message_id.clone(),
                text: seg.text.clone(),
                streaming,
            },
            SegmentKind::Thought => TurnItemKind::Reasoning {
                text: seg.text.clone(),
                streaming,
            },
        };
        let mut it = item(seg.item_id.clone(), status, None, kind);
        it.started_at = Some(seg.started_at);
        it
    }

    fn close_segment(&mut self, step: &mut DriverStep) {
        if let Some(seg) = self.state.segment.take() {
            step.push(DriverEvent::TurnItem(self.segment_item(&seg, false)));
            if seg.named {
                self.state.closed_named.insert(seg.message_id.clone(), seg);
            }
        }
    }

    fn append_text(
        &mut self,
        step: &mut DriverStep,
        kind: SegmentKind,
        text: &str,
        mid: Option<String>,
    ) {
        let reuse = self
            .state
            .segment
            .as_ref()
            .is_some_and(|s| s.kind == kind && mid.as_ref().is_none_or(|m| *m == s.message_id));
        if !reuse {
            self.close_segment(step);
            let resumed = mid
                .as_ref()
                .and_then(|m| self.state.closed_named.remove(m))
                .filter(|seg| seg.kind == kind);
            self.state.segment = Some(match resumed {
                Some(seg) => seg,
                None => {
                    self.state.segments += 1;
                    let item_id = format!("{}:seg{}", self.turn(), self.state.segments);
                    Segment {
                        kind,
                        named: mid.is_some(),
                        message_id: mid.unwrap_or_else(|| item_id.clone()),
                        item_id,
                        text: String::new(),
                        started_at: chrono::Utc::now(),
                    }
                }
            });
        }
        let seg = self.state.segment.as_mut().expect("set above");
        seg.text.push_str(text);
        let item = {
            let seg = self.state.segment.as_ref().expect("set above");
            self.segment_item(seg, true)
        };
        step.push(DriverEvent::TurnItem(item));
    }

    fn tool_item(&self, id: &str, t: &Tool) -> TurnItem {
        let status = match t.status {
            ToolCallStatus::InProgress => TurnItemStatus::Running,
            ToolCallStatus::Completed => TurnItemStatus::Completed,
            ToolCallStatus::Failed => TurnItemStatus::Failed,
            _ => TurnItemStatus::Pending,
        };
        let output = (!t.output.is_empty()).then(|| t.output.clone());
        let error = (status == TurnItemStatus::Failed)
            .then(|| tool_error(&t.output))
            .flatten();
        let kind = match t.kind {
            ToolKind::Execute => TurnItemKind::CommandExecution {
                input: command_text(t.raw_input.as_ref()).unwrap_or_else(|| t.title.clone()),
                output,
                exit_code: None,
            },
            ToolKind::Edit | ToolKind::Delete | ToolKind::Move => {
                // Every file it names when no diff lists them (a move, a delete).
                let file_name = if t.diffs.is_empty() && t.locations.len() > 1 {
                    t.locations.join(", ")
                } else {
                    t.file.clone().unwrap_or_else(|| t.title.clone())
                };
                let raw_diff = (t.diffs.len() > 1).then(|| multi_file_patch(&t.diffs));
                let raw_old = if t.diffs.len() > 1 {
                    None
                } else {
                    t.diff.as_ref().and_then(|d| d.0.clone())
                };
                let raw_new = if t.diffs.len() > 1 {
                    None
                } else {
                    t.diff.as_ref().map(|d| d.1.clone())
                };
                let (diff_str, old_str, new_str) = crate::diff_compact::compact_file_change_fields(
                    &file_name, raw_diff, raw_old, raw_new,
                );
                TurnItemKind::FileChange {
                    file_name,
                    diff_str,
                    old_str,
                    new_str,
                }
            }
            _ => TurnItemKind::DynamicTool {
                image_files: Vec::new(),
                tool_name: t.title.clone(),
                input: t.raw_input.clone().unwrap_or(Value::Null),
                // Every location, not only the first (a search across files).
                output: match (output, t.locations.len() > 1) {
                    (out, true) => Some(format!(
                        "{}Files: {}",
                        out.map(|o| format!("{o}\n")).unwrap_or_default(),
                        t.locations.join(", ")
                    )),
                    (out, false) => out,
                },
                images: t.images.clone(),
            },
        };
        let mut it = item(
            format!("{}:tool:{id}", self.turn()),
            status,
            Some(t.title.clone()),
            kind,
        );
        it.error = error;
        it
    }

    fn merge_content(t: &mut Tool, content: &[ToolCallContent]) {
        let mut media = crate::tool_images::ToolOutput::default();
        for c in content {
            match c {
                ToolCallContent::Content(c) => {
                    if let ContentBlock::Text(tx) = &c.content {
                        t.output.push_str(&tx.text);
                    } else if let ContentBlock::Image(image) = &c.content {
                        media.image(&image.mime_type, &image.data);
                    } else if let Ok(v) = serde_json::to_value(&c.content) {
                        // Resource links, embedded resources, audio.
                        media.block(&v);
                    }
                }
                ToolCallContent::Terminal(term) => {
                    t.terminal = Some(term.terminal_id.0.to_string());
                }
                ToolCallContent::Diff(d) => {
                    let path = d.path.display().to_string();
                    t.file = Some(path.clone());
                    t.diff = Some((d.old_text.clone(), d.new_text.clone()));
                    // An update re-sends the same file: replace, don't repeat.
                    t.diffs.retain(|(p, _, _)| *p != path);
                    t.diffs.push((path, d.old_text.clone(), d.new_text.clone()));
                }
                _ => {}
            }
        }
        if !media.text.is_empty() {
            if !t.output.is_empty() {
                t.output.push('\n');
            }
            t.output.push_str(&media.text);
        }
        t.images = media.images;
    }

    /// OpenCode may deliver inline file attachments in rawOutput. Content images
    /// take precedence so the same screenshot is never retained twice.
    fn merge_raw_images(t: &mut Tool, raw: Option<&Value>) {
        if !t.images.is_empty() {
            return;
        }
        let Some(raw) =
            raw.filter(|v| v.get("attachments").is_some() || v.get("content").is_some())
        else {
            return;
        };
        let media = crate::tool_images::extract(Some(raw));
        if !media.text.is_empty() && media.text != t.output {
            if !t.output.is_empty() {
                t.output.push('\n');
            }
            t.output.push_str(&media.text);
        }
        t.images = media.images;
    }

    fn on_tool_call(
        &mut self,
        step: &mut DriverStep,
        call: ToolCall,
        has_raw_status: bool,
        parent_call_id: Option<&str>,
    ) {
        if parent_call_id.is_none() {
            self.close_segment(step);
        }
        let id = call.tool_call_id.0.to_string();
        self.state.retired_tool_ids.retain(|r| r != &id);
        let (title, kind, classified_file, classified_diff) = if call.kind == ToolKind::Other {
            let c = crate::acp_sideband::burst::classify_tool(&call.title, call.raw_input.as_ref());
            if c.kind != ToolKind::Other {
                (c.title, c.kind, c.file, c.diff)
            } else {
                (call.title, call.kind, None, None)
            }
        } else {
            (call.title, call.kind, None, None)
        };
        let status = if !has_raw_status && call.status == ToolCallStatus::Pending {
            ToolCallStatus::InProgress
        } else {
            call.status
        };
        let file = call
            .locations
            .first()
            .map(|l| l.path.display().to_string())
            .or(classified_file);
        let diffs = match (&file, &classified_diff) {
            (Some(f), Some((old, new))) => vec![(f.clone(), old.clone(), new.clone())],
            _ => Vec::new(),
        };
        let mut t = Tool {
            title,
            kind,
            status,
            raw_input: call.raw_input,
            file,
            diff: classified_diff,
            diffs,
            locations: call
                .locations
                .iter()
                .map(|l| l.path.display().to_string())
                .collect(),
            ..Tool::default()
        };
        Self::merge_content(&mut t, &call.content);
        if t.output.is_empty() {
            if let Some(txt) = raw_output_text(call.raw_output.as_ref()) {
                t.output = txt;
            }
        }
        Self::merge_raw_images(&mut t, call.raw_output.as_ref());
        let meta_val = call.meta.map(Value::Object);
        self.emit_tool_or_subagent(
            step,
            &id,
            &t,
            meta_val.as_ref(),
            call.raw_output.as_ref(),
            parent_call_id,
        );
        if parent_call_id.is_none() {
            self.swarm_members(step, &t);
            if !self
                .state
                .subagents
                .get(&id)
                .is_some_and(|a| a.background && a.status.is_open())
            {
                t.compact_terminal();
            }
            self.state.tools.insert(id, t);
        }
    }

    fn on_tool_update(
        &mut self,
        step: &mut DriverStep,
        up: ToolCallUpdate,
        parent_call_id: Option<&str>,
    ) {
        if parent_call_id.is_none() {
            self.close_segment(step);
        }
        let id = up.tool_call_id.0.to_string();
        let mut t = if parent_call_id.is_some() {
            self.state
                .child_tools
                .get(&id)
                .map(|e| e.tool.clone())
                .unwrap_or_default()
        } else {
            self.state.tools.remove(&id).unwrap_or_default()
        };
        let meta_val = up.meta.map(Value::Object);
        let f = up.fields;
        if let Some(k) = f.kind {
            t.kind = k;
        }
        if let Some(s) = f.status {
            t.status = s;
        }
        if let Some(title) = f.title {
            t.title = title;
        }
        if let Some(i) = f.raw_input {
            t.raw_input = Some(i);
        }
        if let Some(locs) = f.locations.as_ref().filter(|l| !l.is_empty()) {
            t.locations = locs.iter().map(|l| l.path.display().to_string()).collect();
            t.file = t.locations.first().cloned();
        }
        if let Some(content) = &f.content {
            // An update's content replaces the previous content (ACP).
            t.output.clear();
            Self::merge_content(&mut t, content);
            // A client terminal it embeds: what it printed so far.
            if let Some(kept) = t
                .terminal
                .as_ref()
                .and_then(|id| self.state.terminal_output.get(id))
            {
                if t.output.is_empty() {
                    t.output = kept.clone();
                }
            }
        } else if let Some(txt) = raw_output_text(f.raw_output.as_ref()) {
            t.output = txt;
        }
        if f.content.is_none()
            && f.raw_output
                .as_ref()
                .is_some_and(|raw| raw.get("attachments").is_some() || raw.get("content").is_some())
        {
            t.images.clear();
        }
        Self::merge_raw_images(&mut t, f.raw_output.as_ref());
        // Grok 1.0.92 in agent mode denies an unapproved tool without a
        // `session/request_permission` when its own settings say `dontAsk`
        // (it also reads Claude Code's `permissions.defaultMode`; live).
        if self.provider == RuntimeProvider::Grok
            && !self.state.denied_noticed
            && t.output.contains("denied by prompt policy")
        {
            self.state.denied_noticed = true;
            step.push(DriverEvent::TurnItem(notice(
                format!("{}:grok-denied", self.turn()),
                "Grok denies tools without asking when ~/.claude/settings.json sets \
                 permissions.defaultMode to dontAsk \u{b7} Use Full access for this thread"
                    .into(),
            )));
        }
        self.emit_tool_or_subagent(
            step,
            &id,
            &t,
            meta_val.as_ref(),
            f.raw_output.as_ref(),
            parent_call_id,
        );
        if parent_call_id.is_none() {
            self.swarm_members(step, &t);
            if !self
                .state
                .subagents
                .get(&id)
                .is_some_and(|a| a.background && a.status.is_open())
            {
                t.compact_terminal();
            }
            self.state.tools.insert(id, t);
        }
    }

    /// agy's native swarm (`swarm{action: add|kill, name, brief, n?,
    /// tasks?}`; verified live against 1.5.8): agy reports each call as a
    /// plain `kind: other` tool titled "Run swarm" with its args as
    /// `rawInput` and NO result, sends nothing while the workers run (they
    /// run on their own in agy's backend), and keeps the swarm on disk under
    /// `brain/<session>/.system_generated/swarm` (read by `native_work.rs`).
    /// An `add` is one background subagent card per member (`name`, or
    /// `name-1..k` for `n` / `tasks`), open until a `kill` names it or the
    /// process ends.
    fn swarm_members(&mut self, step: &mut DriverStep, t: &Tool) {
        if t.title != "Run swarm" || t.status != ToolCallStatus::Completed {
            return;
        }
        if let Some(input) = t.raw_input.as_ref() {
            self.swarm_from(step, input, false);
        }
    }

    /// A `session/load` replay's completed "Run swarm" call ([`Self::swarm_from`]).
    fn replayed_swarm(&mut self, step: &mut DriverStep, update: &SessionUpdate) {
        let (title, status, input) = match update {
            SessionUpdate::ToolCall(c) => {
                (Some(c.title.as_str()), Some(c.status), c.raw_input.as_ref())
            }
            SessionUpdate::ToolCallUpdate(u) => (
                u.fields.title.as_deref(),
                u.fields.status,
                u.fields.raw_input.as_ref(),
            ),
            _ => return,
        };
        if title == Some("Run swarm") && status == Some(ToolCallStatus::Completed) {
            if let Some(input) = input {
                self.swarm_from(step, input, true);
            }
        }
    }

    /// [`Self::swarm_members`] on a call's `rawInput`; `replayed`: from a
    /// `session/load` replay — only members without a card get one, outside
    /// any turn (`replay:swarm:<member>`).
    fn swarm_from(&mut self, step: &mut DriverStep, input: &Value, replayed: bool) {
        if self.provider != RuntimeProvider::Agy {
            return;
        }
        let s = |k: &str| input.get(k).and_then(Value::as_str);
        let Some(name) = s("name").filter(|n| !n.is_empty()) else {
            return;
        };
        match s("action") {
            Some("add") => {
                let brief = s("brief").unwrap_or_default();
                let tasks: Vec<&str> = input
                    .get("tasks")
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).collect())
                    .unwrap_or_default();
                let n = input.get("n").and_then(Value::as_u64).unwrap_or(0) as usize;
                let members: Vec<(String, String)> = if !tasks.is_empty() {
                    tasks
                        .iter()
                        .enumerate()
                        .map(|(i, task)| {
                            let prompt = format!("{brief}\n\n{task}").trim().to_string();
                            (format!("{name}-{}", i + 1), prompt)
                        })
                        .collect()
                } else if n > 1 {
                    (1..=n.min(32))
                        .map(|i| (format!("{name}-{i}"), brief.to_string()))
                        .collect()
                } else {
                    vec![(name.to_string(), brief.to_string())]
                };
                for (member, prompt) in members {
                    if self.state.swarm.contains_key(&member)
                        || (replayed && self.state.swarm_seen.contains(&member))
                    {
                        continue;
                    }
                    let id = if replayed {
                        format!("replay:swarm:{member}")
                    } else {
                        format!("{}:swarm:{member}", self.turn())
                    };
                    let mut it = item(
                        id,
                        TurnItemStatus::Running,
                        Some(member.clone()),
                        TurnItemKind::Subagent {
                            tool_name: "swarm".into(),
                            description: Some(member.clone()),
                            subagent_type: Some("Swarm worker".into()),
                            prompt: (!prompt.is_empty()).then_some(prompt),
                            output: None,
                            background: true,
                            last_tool: None,
                            tool_uses: 0,
                        },
                    );
                    // Its card stays in the turn that added it.
                    it.turn_id = if replayed {
                        None
                    } else {
                        self.state.turn_id.clone()
                    };
                    step.push(DriverEvent::TurnItem(it.clone()));
                    self.state.swarm_seen.insert(member.clone());
                    self.state.swarm.insert(member, it);
                }
            }
            // `kill{name}` also takes a whole `name-1..k` group.
            Some("kill") => {
                let gone: Vec<String> = self
                    .state
                    .swarm
                    .keys()
                    .filter(|m| {
                        *m == name
                            || m.strip_prefix(name)
                                .and_then(|r| r.strip_prefix('-'))
                                .is_some_and(|r| {
                                    !r.is_empty() && r.bytes().all(|b| b.is_ascii_digit())
                                })
                    })
                    .cloned()
                    .collect();
                for m in gone {
                    if let Some(mut it) = self.state.swarm.remove(&m) {
                        let now = chrono::Utc::now();
                        it.status = TurnItemStatus::Interrupted;
                        it.completed_at = Some(now);
                        it.updated_at = now;
                        step.push(DriverEvent::TurnItem(it));
                    }
                }
            }
            _ => {}
        }
    }

    fn on_plan(&mut self, step: &mut DriverStep, plan: Plan) {
        self.close_segment(step);
        let steps: Vec<PlanStep> = plan
            .entries
            .into_iter()
            .enumerate()
            .map(|(i, e)| PlanStep {
                id: (i + 1).to_string(),
                text: e.content,
                status: match e.status {
                    PlanEntryStatus::InProgress => PlanStepStatus::Running,
                    PlanEntryStatus::Completed => PlanStepStatus::Completed,
                    _ => PlanStepStatus::Pending,
                },
            })
            .collect();
        step.push(DriverEvent::Plan {
            steps,
            explanation: None,
        });
    }

    fn on_update(
        &mut self,
        step: &mut DriverStep,
        n: SessionNotification,
        has_raw_status: bool,
        parent_call_id: Option<&str>,
    ) {
        // Session-level news counts during a `session/load` replay too.
        match &n.update {
            SessionUpdate::AvailableCommandsUpdate(u) => {
                step.push(DriverEvent::Commands(
                    u.available_commands
                        .iter()
                        .map(|c| {
                            let mut cmd =
                                ProviderCommand::new(c.name.clone(), c.description.clone());
                            cmd.hint = match &c.input {
                                Some(AvailableCommandInput::Unstructured(i)) => {
                                    Some(i.hint.clone())
                                }
                                _ => None,
                            }
                            .filter(|h| !h.trim().is_empty());
                            cmd
                        })
                        .collect(),
                ));
                return;
            }
            SessionUpdate::UsageUpdate(u) => {
                step.push(DriverEvent::ContextUsage(ThreadTokenUsage {
                    used_tokens: u.used,
                    max_tokens: (u.size > 0).then_some(u.size),
                    ..ThreadTokenUsage::default()
                }));
                if let Some(c) = u
                    .cost
                    .as_ref()
                    .filter(|c| c.currency.eq_ignore_ascii_case("USD"))
                {
                    self.state.cost_total = Some(c.amount);
                    let at_turn = self.state.cost_at_turn.unwrap_or(c.amount);
                    if self.state.turn_id.is_some() && c.amount >= at_turn {
                        step.push(DriverEvent::TurnCost(c.amount - at_turn));
                    }
                }
                return;
            }
            SessionUpdate::SessionInfoUpdate(u) => {
                if let MaybeUndefined::Value(t) = &u.title {
                    step.push(DriverEvent::Title(t.clone()));
                }
                return;
            }
            SessionUpdate::CurrentModeUpdate(u) => {
                let id = u.current_mode_id.0.as_ref();
                self.state.plan_active = id == "plan";
                self.follow_mode(step, id);
                return;
            }
            _ => {}
        }
        if matches!(self.state.phase, Phase::Creating { loading: true }) {
            // History replayed by session/load is dropped — except a swarm
            // member with no card yet: it gets one, like a new member.
            self.replayed_swarm(step, &n.update);
            return;
        }
        match n.update {
            SessionUpdate::AgentMessageChunk(c) => {
                let t = match c.content {
                    ContentBlock::Text(t) => t,
                    ContentBlock::Image(img) => {
                        self.note_stdout_activity();
                        self.ensure_autonomous_turn(step);
                        self.answer_image(step, &img);
                        return;
                    }
                    other => match block_text(&other) {
                        Some(text) => TextContent::new(text),
                        None => return,
                    },
                };
                {
                    self.note_stdout_activity();
                    if let Some(text) = self
                        .state
                        .sideband
                        .filter_message(&t.text)
                        .filter(|s| !s.is_empty())
                    {
                        self.ensure_autonomous_turn(step);
                        let mid = c.message_id.map(|m| m.0.to_string());
                        if let Some(pid) = parent_call_id {
                            self.append_child_text(step, pid, SegmentKind::Message, &text, mid);
                        } else {
                            self.append_text(step, SegmentKind::Message, &text, mid);
                        }
                    }
                }
            }
            SessionUpdate::AgentThoughtChunk(c) => {
                if let ContentBlock::Text(t) = c.content {
                    self.note_stdout_activity();
                    if let Some(text) = self
                        .state
                        .sideband
                        .filter_thought(&t.text)
                        .filter(|s| !s.is_empty())
                    {
                        self.ensure_autonomous_turn(step);
                        if let Some(pid) = parent_call_id {
                            self.append_child_text(step, pid, SegmentKind::Thought, &text, None);
                        } else {
                            self.append_text(step, SegmentKind::Thought, &text, None);
                        }
                    }
                }
            }
            SessionUpdate::ToolCall(call) => {
                self.note_stdout_activity();
                if self
                    .state
                    .sideband
                    .filter_tool_call(call.tool_call_id.0.as_ref())
                {
                    return;
                }
                self.ensure_autonomous_turn(step);
                self.on_tool_call(step, call, has_raw_status, parent_call_id);
            }
            SessionUpdate::ToolCallUpdate(up) => {
                let id = up.tool_call_id.0.as_ref();
                let known = self.state.tools.contains_key(id)
                    || self.state.subagents.contains_key(id)
                    || self.state.child_tools.contains_key(id);
                if !known {
                    let has_title = up
                        .fields
                        .title
                        .as_deref()
                        .is_some_and(|s| !s.trim().is_empty());
                    let is_retired = self.state.retired_tool_ids.iter().any(|r| r == id);
                    if is_retired || (self.state.prompt_id.is_none() && !has_title) {
                        return;
                    }
                }
                self.note_stdout_activity();
                if self
                    .state
                    .sideband
                    .filter_tool_update(id, known, self.state.prompt_id.is_some())
                {
                    return;
                }
                self.ensure_autonomous_turn(step);
                self.on_tool_update(step, up, parent_call_id);
            }
            SessionUpdate::Plan(plan) => {
                self.ensure_autonomous_turn(step);
                self.on_plan(step, plan);
            }
            // One quiet timeline line each (gap 6).
            SessionUpdate::Notice(n) => {
                self.ensure_autonomous_turn(step);
                self.close_segment(step);
                let message = match n.description.filter(|d| !d.trim().is_empty()) {
                    Some(d) => format!("{} \u{2014} {d}", n.title),
                    None => n.title,
                };
                let id = format!("{}:notice:{}", self.turn(), otto_core::new_id());
                step.push(DriverEvent::TurnItem(notice(id, message)));
            }
            SessionUpdate::CompactionUpdate(c) => {
                self.ensure_autonomous_turn(step);
                let turn = self.turn();
                step.push(DriverEvent::TurnItem(match c.status {
                    CompactionStatus::InProgress => compaction_notice(&turn, false),
                    CompactionStatus::Completed => compaction_notice(&turn, true),
                    _ => {
                        let why = match c.error {
                            MaybeUndefined::Value(e) if !e.trim().is_empty() => format!(": {e}"),
                            _ => String::new(),
                        };
                        notice(
                            format!("{turn}:compaction"),
                            format!("Context compaction did not finish{why}"),
                        )
                    }
                }));
            }
            _ => {}
        }
    }

    /// An image in the agent's answer: its own row, rendered like a tool's.
    fn answer_image(&mut self, step: &mut DriverStep, img: &ImageContent) {
        self.close_segment(step);
        let mut media = crate::tool_images::ToolOutput::default();
        media.image(&img.mime_type, &img.data);
        let id = format!("{}:image:{}", self.turn(), otto_core::new_id());
        step.push(DriverEvent::TurnItem(item(
            id,
            TurnItemStatus::Completed,
            Some("Image".into()),
            TurnItemKind::DynamicTool {
                image_files: Vec::new(),
                tool_name: "Image".into(),
                input: Value::Null,
                output: (!media.text.is_empty()).then_some(media.text),
                images: media.images,
            },
        )));
    }

    fn on_permission(
        &mut self,
        step: &mut DriverStep,
        rpc_id: Value,
        req: RequestPermissionRequest,
    ) {
        self.ensure_autonomous_turn(step);
        self.close_segment(step);
        let request_id = format!("acp-{}", rpc_key(&rpc_id));
        let kind = match req.tool_call.fields.kind {
            Some(ToolKind::Execute) => RequestKind::Command,
            Some(ToolKind::Edit | ToolKind::Delete | ToolKind::Move) => RequestKind::FileChange,
            Some(ToolKind::Read | ToolKind::Search) => RequestKind::FileRead,
            _ => RequestKind::Permission,
        };
        let mut options: Vec<ApprovalOption> = Vec::new();
        for o in &req.options {
            let decision = match o.kind {
                PermissionOptionKind::AllowOnce => ApprovalDecision::Accept,
                PermissionOptionKind::AllowAlways => ApprovalDecision::AcceptForSession,
                PermissionOptionKind::RejectOnce => ApprovalDecision::Decline,
                PermissionOptionKind::RejectAlways => ApprovalDecision::DeclineAlways,
                // Unknown kinds have no decision of their own; `cancel`
                // (always allowed) rejects without remembering.
                _ => continue,
            };
            if !options.iter().any(|x| x.decision == decision) {
                options.push(ApprovalOption {
                    decision,
                    label: o.name.clone(),
                });
            }
        }
        let title = req.tool_call.fields.title.clone();
        let tool_item_id = format!("{}:tool:{}", self.turn(), req.tool_call.tool_call_id.0);
        self.state.pending.insert(
            request_id.clone(),
            Pending {
                rpc_id,
                options: req.options,
            },
        );
        step.push(DriverEvent::Request {
            id: request_id,
            kind,
            title: title.clone(),
            prompt: title,
            reason: None,
            options,
            tool_item_id: Some(tool_item_id),
        });
    }

    fn on_response(
        &mut self,
        step: &mut DriverStep,
        id: u64,
        result: Option<Value>,
        error: Option<Value>,
    ) {
        if let Phase::Authenticating { load } = self.state.phase {
            if id == self.state.auth_id {
                if let Some(e) = error {
                    step.push(DriverEvent::Fatal(rpc_failure("auth_required", &e)));
                    return;
                }
                self.create_session(step, load);
                return;
            }
        }
        if id == self.state.init_id && self.state.phase == Phase::Initializing {
            let result_resume = result
                .as_ref()
                .and_then(|r| r.pointer("/agentCapabilities/sessionCapabilities/resume"))
                .is_some();
            self.state.auth_methods = result
                .as_ref()
                .and_then(|r| r.get("authMethods"))
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|m| {
                            let id = m.get("id")?.as_str()?.to_string();
                            let ty = m.get("type").and_then(Value::as_str).map(str::to_string);
                            Some((id, ty))
                        })
                        .collect()
                })
                .unwrap_or_default();
            let caps = result
                .and_then(|r| serde_json::from_value::<InitializeResponse>(r).ok())
                .map(|r| r.agent_capabilities);
            let load = caps.as_ref().is_some_and(|c| c.load_session);
            self.state.resuming = result_resume;
            self.state.images = caps.as_ref().is_some_and(|c| c.prompt_capabilities.image);
            self.state.embedded = caps.is_some_and(|c| c.prompt_capabilities.embedded_context);
            if let Some(e) = error {
                step.push(DriverEvent::Fatal(rpc_failure("handshake_error", &e)));
                return;
            }
            self.state.lost_resume = self.state.resume.is_some() && !load;
            self.create_session(step, load && self.state.resume.is_some());
        } else if id == self.state.create_id && matches!(self.state.phase, Phase::Creating { .. }) {
            let loading = matches!(self.state.phase, Phase::Creating { loading: true });
            if let Some(e) = error {
                if e.get("code").and_then(Value::as_i64) == Some(AUTH_REQUIRED)
                    && !self.state.authenticated
                {
                    if let Some(method) = self.auth_method() {
                        let (id, line) =
                            self.request(M_AUTHENTICATE, AuthenticateRequest::new(method));
                        self.state.auth_id = id;
                        self.state.authenticated = true;
                        self.state.phase = Phase::Authenticating { load: loading };
                        step.write(line);
                        return;
                    }
                }
                if loading && std::mem::take(&mut self.state.resuming) {
                    // A refused `session/resume`: the replaying `session/load`.
                    self.create_session(step, true);
                    return;
                }
                if loading {
                    // T3 Code: a failed load / resume starts a fresh session
                    // (and the thread says so at `Ready`).
                    self.state.resume = None;
                    self.state.lost_resume = true;
                    self.create_session(step, false);
                    return;
                }
                step.push(DriverEvent::Fatal(rpc_failure("handshake_error", &e)));
                return;
            }
            if let Some(r) = &result {
                self.update_effort_options(r);
            }
            self.state.available_models = result
                .as_ref()
                .and_then(|r| r.pointer("/models/availableModels"))
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|m| Some(m.get("modelId")?.as_str()?.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            if let Some(opts) = self
                .state
                .model
                .as_deref()
                .and_then(|m| self.efforts_for_model(m))
            {
                self.state.effort_options = opts;
            }
            self.state.modes = result
                .as_ref()
                .and_then(|r| r.pointer("/modes/availableModes"))
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|m| Some(m.get("id")?.as_str()?.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            let mut current_mode = result
                .as_ref()
                .and_then(|r| r.pointer("/modes/currentModeId"))
                .and_then(Value::as_str)
                .map(str::to_string);
            let mode_option = result
                .as_ref()
                .and_then(|r| r.get("configOptions"))
                .and_then(Value::as_array)
                .and_then(|a| {
                    a.iter()
                        .find(|c| c.get("category").and_then(Value::as_str) == Some("mode"))
                })
                .filter(|_| self.state.modes.is_empty());
            if let Some(c) = mode_option {
                self.state.mode_config_id = c.get("id").and_then(Value::as_str).map(str::to_string);
                self.state.modes = c
                    .get("options")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|o| Some(o.get("value")?.as_str()?.to_string()))
                            .collect()
                    })
                    .unwrap_or_default();
                current_mode = c
                    .get("currentValue")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
            self.state.plan_active = current_mode.as_deref() == Some("plan");
            // A new session has spent nothing; a loaded one's running cost is
            // unknown until its first `usage_update` (that turn counts none).
            self.state.cost_total = (!loading).then_some(0.0);
            self.state.base_mode = current_mode.clone().filter(|m| m != "plan");
            self.state.session_id = if loading {
                self.state.resume.clone()
            } else {
                result
                    .and_then(|r| serde_json::from_value::<NewSessionResponse>(r).ok())
                    .map(|r| r.session_id.0.to_string())
            };
            if self.provider == RuntimeProvider::Agy {
                if let Some(sid) = self.state.session_id.clone() {
                    self.state.note_agy_conv_id(&sid);
                }
            }
            self.state.phase = Phase::Ready;
            step.push(DriverEvent::Ready {
                native_thread_id: self.state.session_id.clone(),
                model: None,
            });
            if !loading && std::mem::take(&mut self.state.lost_resume) {
                self.push_fresh_boundary(step);
            }
            self.write_model_config(step);
            // A mode no launch flag gave (agy has none): set it now.
            if let Some(want) = self.flavor.mode_id.and_then(|f| f(self.state.access)) {
                if current_mode.as_deref() != Some(want)
                    && self.state.modes.iter().any(|m| m == want)
                {
                    let sid = self.state.session_id.clone().unwrap_or_default();
                    self.write_mode(step, &sid, self.state.access);
                }
            }
            // A prompt queued before `initialize` answered goes as is; the
            // agent refuses images it cannot take.
            if let Some((turn, req)) = self.state.queued.take() {
                self.send_prompt(step, turn, &req);
            } else {
                self.state.sideband.ensure_idle_floor();
            }
        } else if Some(id) == self.state.config_req {
            self.state.config_req = None;
            if let Some(r) = &result {
                self.update_effort_options(r);
            }
            if let Some(e) = error {
                // -32602: the agent refused the value; anything else (an
                // internal error, a dropped request) was not applied.
                let refused = e.get("code").and_then(Value::as_i64) == Some(-32602);
                self.state.config_retry = !refused;
                let mut failure = rpc_failure("provider_error", &e);
                if refused {
                    // The agent still runs its earlier model; the runtime puts
                    // the thread back on it, and the next pick is sent again.
                    self.state.model = None;
                    failure.code = Some(otto_core::agent_runtime::MODEL_SWITCH_REFUSED.into());
                }
                step.push(DriverEvent::Error(failure));
            }
        } else if let Some(pos) = self.state.interject_reqs.iter().position(|r| *r == id) {
            self.state.interject_reqs.swap_remove(pos);
            // Only `{status: "queued"}` reaches the running turn.
            let queued = result
                .as_ref()
                .and_then(|r| r.get("status"))
                .and_then(Value::as_str)
                == Some("queued");
            if !queued {
                step.push(DriverEvent::Error(ProviderFailure {
                    class: "provider_error".into(),
                    message: format!(
                        "{} did not take the message mid-turn; send it again once the turn ends.",
                        self.provider.as_str()
                    ),
                    code: error
                        .as_ref()
                        .and_then(|e| e.get("code"))
                        .map(|c| c.to_string()),
                    retryable: Some(true),
                }));
            }
        } else if Some(id) == self.state.effort_req {
            self.state.effort_req = None;
            if let Some(r) = &result {
                self.update_effort_options(r);
            }
            if let Some(e) = error {
                step.push(DriverEvent::Error(rpc_failure("provider_error", &e)));
            }
        } else if Some(id) == self.state.mode_req {
            self.state.mode_req = None;
            if let Some(e) = error {
                step.push(DriverEvent::Error(rpc_failure("provider_error", &e)));
            }
        } else if Some(id) == self.state.prompt_id {
            let was_cancelling = std::mem::replace(&mut self.state.cancelling, false);
            self.state.prompt_id = None;
            self.state.sideband.finish_prompted_turn();
            self.close_segment(step);
            self.settle_foreground_subagents(
                step,
                if was_cancelling {
                    TurnItemStatus::Interrupted
                } else if error.is_some() {
                    TurnItemStatus::Failed
                } else {
                    TurnItemStatus::Completed
                },
            );
            self.retire_tools();
            let ev = match (result, error) {
                (_, Some(e)) => DriverEvent::Terminal {
                    status: if was_cancelling {
                        TurnTerminalStatus::Interrupted
                    } else {
                        TurnTerminalStatus::Failed
                    },
                    failure: (!was_cancelling).then(|| rpc_failure("provider_error", &e)),
                    reason: None,
                },
                (Some(r), None) => {
                    // The draft `usage` of a prompt response (agy, gemini):
                    // input excludes the cache reads.
                    if let Some(u) = r.get("usage").filter(|u| u.is_object()) {
                        let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0);
                        step.push(DriverEvent::Usage(TurnTokenUsage {
                            input_tokens: n("inputTokens") + n("cachedReadTokens"),
                            output_tokens: n("outputTokens"),
                            cached_input_tokens: Some(n("cachedReadTokens")),
                            reasoning_tokens: Some(n("thoughtTokens")),
                            cost_usd: None,
                        }));
                    }
                    let stop = serde_json::from_value::<PromptResponse>(r)
                        .map(|p| p.stop_reason)
                        .unwrap_or(StopReason::EndTurn);
                    // T3 Code #28797: a refusal or a limit stop is not a
                    // finished answer, so it fails the turn with a reason.
                    let stopped = |code: &str, message: &str| DriverEvent::Terminal {
                        status: TurnTerminalStatus::Failed,
                        failure: Some(ProviderFailure {
                            class: "provider_error".into(),
                            message: message.into(),
                            code: Some(code.into()),
                            retryable: None,
                        }),
                        reason: Some(code.into()),
                    };
                    let done = |status| DriverEvent::Terminal {
                        status,
                        failure: None,
                        reason: None,
                    };
                    if was_cancelling {
                        done(TurnTerminalStatus::Interrupted)
                    } else {
                        match stop {
                            StopReason::Cancelled => done(TurnTerminalStatus::Interrupted),
                            StopReason::MaxTokens => stopped(
                                "max_tokens",
                                "The agent stopped: the reply hit the model's output token limit.",
                            ),
                            StopReason::MaxTurnRequests => stopped(
                                "max_turn_requests",
                                "The agent stopped: the turn hit its limit of model requests.",
                            ),
                            StopReason::Refusal => stopped(
                                "refusal",
                                "The agent refused this request. It won't remember this message.",
                            ),
                            _ => done(TurnTerminalStatus::Completed),
                        }
                    }
                }
                (None, None) => DriverEvent::Terminal {
                    status: if was_cancelling {
                        TurnTerminalStatus::Interrupted
                    } else {
                        TurnTerminalStatus::Completed
                    },
                    failure: None,
                    reason: None,
                },
            };
            step.push(ev);
            self.state.autonomous = false;
            self.state.turn_id = None;
        }
    }
}

/// The agent's levels low → high (what the effort picker lists); a tier Otto
/// doesn't know sorts as `medium`.
fn effort_levels(levels: &[String]) -> Vec<String> {
    let rank = |l: &str| EFFORT_RANK.iter().position(|r| *r == l);
    let mut out = levels.to_vec();
    out.sort_by_key(|l| rank(l).or(rank("medium")));
    out
}

/// An Otto context bundle as system-prompt text: its `AGENTS.md`, then an
/// index of its `.agents/skills` (name, description, absolute `SKILL.md`)
/// for the agent to read on demand. `None` when both are empty. Blocking:
/// [`StructuredDriver::adopt_spec`] runs off the async runtime.
#[allow(clippy::disallowed_methods)]
fn bundle_prompt(dir: &std::path::Path) -> Option<String> {
    let mut out = std::fs::read_to_string(dir.join("AGENTS.md"))
        .unwrap_or_default()
        .trim()
        .to_string();
    let mut skills: Vec<String> = std::fs::read_dir(dir.join(".agents/skills"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let md = e.path().join("SKILL.md");
            let body = std::fs::read_to_string(&md).ok()?;
            let mut lines = body.lines().map(str::trim);
            let mut desc = lines
                .find_map(|l| l.strip_prefix("description:"))
                .unwrap_or_default()
                .trim();
            // A folded / literal YAML block: its first line.
            if matches!(desc, ">" | "|" | ">-" | "|-") {
                desc = lines.next().unwrap_or_default();
            }
            let name = e.file_name().to_string_lossy().into_owned();
            Some(format!(
                "- {name}: {} ({})",
                desc.trim_matches('"'),
                md.display()
            ))
        })
        .collect();
    if !skills.is_empty() {
        skills.sort();
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(
            "## Skills\n\nRead a skill's SKILL.md before you use it; its folder may hold \
             more files it refers to.\n\n",
        );
        out.push_str(&skills.join("\n"));
    }
    (!out.is_empty()).then_some(out)
}

/// `raw_input.command` as a display string (a string or an argv array).
pub(super) fn command_text(raw: Option<&Value>) -> Option<String> {
    let raw = raw?;
    match raw.get("command").or_else(|| raw.get("CommandLine"))? {
        Value::String(s) => Some(s.clone()),
        Value::Array(a) => Some(
            a.iter()
                .filter_map(Value::as_str)
                .collect::<Vec<_>>()
                .join(" "),
        ),
        _ => None,
    }
}

/// A non-text, non-image content block as a line of text: a resource link
/// as a markdown link, an embedded text resource as its text, a blob or an
/// audio clip named (gap 7).
fn block_text(block: &ContentBlock) -> Option<String> {
    Some(match block {
        ContentBlock::ResourceLink(l) => format!("[{}]({})", l.name, l.uri),
        ContentBlock::Resource(r) => match &r.resource {
            EmbeddedResourceResource::TextResourceContents(t) => t.text.clone(),
            EmbeddedResourceResource::BlobResourceContents(b) => format!("(attachment {})", b.uri),
            _ => return None,
        },
        ContentBlock::Audio(_) => "(audio clip — not played here)".into(),
        _ => return None,
    })
}

fn raw_output_text(raw: Option<&Value>) -> Option<String> {
    let raw = raw?;
    if let Some(s) = raw.as_str() {
        return (!s.is_empty()).then(|| s.to_string());
    }
    if let Some(s) = raw
        .get("output")
        .or_else(|| raw.get("stdout"))
        .or_else(|| raw.get("error"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
    {
        return Some(s.to_string());
    }
    // An MCP tool's own result (`{content: [...], isError}`) as its rawOutput.
    let text = crate::tool_images::extract(raw.get("content").map(|_| raw)).text;
    (!text.is_empty()).then_some(text)
}

impl StructuredDriver for AcpDriver {
    fn provider(&self) -> RuntimeProvider {
        self.provider
    }

    /// The provider baseline, then what `initialize` / `session/new|load`
    /// advertised: `promptCapabilities`, the effort option, a `plan` mode.
    fn capabilities(&self) -> RuntimeCapabilities {
        let mut c = RuntimeCapabilities::for_provider(self.provider);
        if self.state.phase == Phase::Ready {
            c.images = self.state.images;
            c.files = self.state.embedded;
            if self.flavor.effort_config_id.is_none() {
                c.efforts.clear();
            } else if !self.state.effort_options.is_empty() {
                c.efforts = effort_levels(&self.state.effort_options);
            } else if self.state.available_models.is_empty() {
                c.efforts.clear();
            }
            c.plan = self.state.modes.iter().any(|m| m == "plan");
        }
        c.steer = self.flavor.interject.is_some();
        c
    }

    /// Steering through the flavor's `interject` method: text and images
    /// join the running prompt; the running tool keeps going.
    fn steer(&mut self, req: &RuntimePromptReq) -> Result<DriverStep, DriverError> {
        let Some(method) = self.flavor.interject else {
            return Err(DriverError(
                "this agent cannot take a message mid-turn".into(),
            ));
        };
        let Some(sid) = self
            .state
            .session_id
            .clone()
            .filter(|_| self.state.prompt_id.is_some())
        else {
            return Err(DriverError("no prompt is running".into()));
        };
        if req.text.trim().is_empty() || !req.files.is_empty() {
            return Err(DriverError(
                "only text and images join a running turn".into(),
            ));
        }
        let mut params = json!({"sessionId": sid, "text": req.text});
        if !req.images.is_empty() {
            params["content"] = req
                .images
                .iter()
                .map(|i| json!({"type": "image", "data": i.data_base64, "mimeType": i.mime}))
                .collect();
        }
        let (id, line) = self.request(method, params);
        self.state.interject_reqs.push(id);
        let mut step = DriverStep::default();
        // What streams after the steer answers it: a new segment, after it.
        self.close_segment(&mut step);
        step.write(line);
        Ok(step)
    }

    fn launch(&self, ctx: &LaunchContext) -> LaunchSpec {
        let mut args = self.flavor.head_args.clone();
        let mut tail_args = self.flavor.tail_args.clone();
        if let Some(f) = self.flavor.access_args {
            let (head, tail) = f(ctx.access_mode);
            args.extend(head);
            tail_args.extend(tail);
        }
        if let Some(m) = ctx
            .model
            .as_ref()
            .filter(|_| self.flavor.model_config_id.is_none() && !self.flavor.set_model)
        {
            args.extend([self.flavor.model_flag.clone(), m.clone()]);
        }
        if let Some(flag) = self.flavor.own_sandbox_off.filter(|_| ctx.sandboxed) {
            args.push(flag.into());
        }
        LaunchSpec {
            program: self.flavor.program.clone(),
            args,
            tail_args,
            env: self
                .flavor
                .access_env
                .map(|f| f(ctx.access_mode))
                .unwrap_or_default(),
        }
    }

    fn mcp_delivery(&self) -> McpDelivery {
        McpDelivery::Protocol
    }

    fn start(&mut self, ctx: &LaunchContext) -> DriverStep {
        self.state.cwd = ctx.cwd.clone();
        self.state.resume = ctx.resume.clone();
        self.state.otto_session_id =
            (!ctx.session_id.trim().is_empty()).then(|| ctx.session_id.clone());
        if self.provider == RuntimeProvider::Agy {
            if let Some(r) = ctx.resume.as_deref() {
                self.state.note_agy_conv_id(r);
            }
        }
        self.state.mcp_servers = ctx.mcp_servers.clone();
        self.state.model = ctx.model.clone();
        self.state.access = ctx.access_mode;
        let mut step = DriverStep::default();
        let mut init = InitializeRequest::new(ProtocolVersion::V1);
        if let Some(Value::Object(meta)) = &self.flavor.initialize_meta {
            init.meta = Some(meta.clone());
        }
        // Elicitation is a draft ACP capability not yet exposed by our pinned SDK.
        let mut init = serde_json::to_value(init).expect("initialize serializes");
        init["clientCapabilities"]["elicitation"] = json!({"form":{}});
        self.client_capabilities(&mut init);
        let (id, line) = self.request(M_INITIALIZE, init);
        self.state.init_id = id;
        step.write(line);
        step
    }

    fn prompt(&mut self, turn_id: &str, req: &RuntimePromptReq) -> Result<DriverStep, DriverError> {
        if self.state.prompt_id.is_some()
            || self.state.queued.is_some()
            || self.state.turn_id.is_some()
        {
            return Err(DriverError("a prompt is already running".into()));
        }
        let name = self.provider.as_str();
        let mut step = DriverStep::default();
        // An effort the agent cannot take is dropped with a notice, never a
        // refused prompt (an "Ask another agent" create carries the
        // previous provider's level).
        let mut owned = None;
        if let Some(raw) = req.effort.as_deref().filter(|e| !e.trim().is_empty()) {
            let next_opts = req.model.as_deref().and_then(|m| self.efforts_for_model(m));
            let opts = next_opts.as_deref().unwrap_or(&self.state.effort_options);
            if let Err(DriverError(why)) = self.resolve_effort_with(opts, raw) {
                step.push(DriverEvent::TurnItem(notice(
                    format!("{turn_id}:effort-dropped"),
                    format!(
                        "{why} \u{2014} this turn runs at {}'s default effort.",
                        provider_name(self.provider)
                    ),
                )));
                owned = Some(RuntimePromptReq {
                    effort: None,
                    ..req.clone()
                });
            }
        }
        let req = owned.as_ref().unwrap_or(req);
        // Capabilities are only known once `initialize` answered; before that a
        // queued attachment goes out and the agent itself refuses it if it must.
        let ready = self.state.phase == Phase::Ready;
        if ready && req.plan && !self.state.modes.iter().any(|m| m == "plan") {
            return Err(DriverError(format!("{name} has no plan mode")));
        }
        if ready && !req.images.is_empty() && !self.state.images {
            return Err(DriverError(format!("{name} does not accept images")));
        }
        if ready && !req.files.is_empty() && !self.state.embedded {
            return Err(DriverError(format!(
                "{name} does not accept file attachments"
            )));
        }
        if ready {
            self.send_prompt(&mut step, turn_id.to_string(), req);
        } else {
            if self.switches_model_live() {
                if let Some(m) = &req.model {
                    self.state.model = Some(m.clone());
                }
            }
            self.state.turn_id = Some(turn_id.to_string());
            self.state.queued = Some((turn_id.to_string(), req.clone()));
        }
        Ok(step)
    }

    fn switches_model_live(&self) -> bool {
        self.flavor.set_model || self.flavor.model_config_id.is_some()
    }

    /// Live only through an advertised `session/set_mode`, between modes
    /// that both have one.
    fn switches_access_live(&self, to: RuntimeAccessMode) -> bool {
        self.flavor.mode_id.is_some_and(|f| {
            f(self.state.access).is_some()
                && f(to).is_some_and(|id| self.state.modes.iter().any(|m| m == id))
        })
    }

    fn adopt_spec(&mut self, spec: &mut LaunchSpec) {
        self.adopt_confinement(spec);
        self.state.sideband.adopt_env(self.provider, &mut spec.env);
        let Some(var) = self.flavor.bundle_env else {
            return;
        };
        if let Some(i) = spec.env.iter().position(|(k, _)| k == var) {
            let (_, dir) = spec.env.remove(i);
            self.state.system_append = bundle_prompt(std::path::Path::new(&dir));
        }
    }

    fn interrupt(&mut self) -> DriverStep {
        let mut step = DriverStep::default();
        self.cancel_client_held(&mut step);
        self.state.sideband.on_turn_settled();
        // ACP: the client answers every open permission request `cancelled`.
        for (_, (rpc_id, _)) in self.state.questions.drain() {
            step.write(
                json!({"jsonrpc":"2.0","id":rpc_id,"result":{"action":"cancel"}}).to_string(),
            );
        }
        let mut open: Vec<(String, Pending)> = self.state.pending.drain().collect();
        open.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, p) in open {
            let resp = RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled);
            step.write(json!({"jsonrpc":"2.0","id":p.rpc_id,"result":resp}).to_string());
        }
        if self.state.queued.take().is_some() {
            self.state.cancelling = false;
            self.state.autonomous = false;
            self.state.turn_id = None;
            step.push(DriverEvent::Terminal {
                status: TurnTerminalStatus::Interrupted,
                failure: None,
                reason: None,
            });
        } else if let Some(sid) = self.state.session_id.clone() {
            self.state.cancelling = self.state.prompt_id.is_some();
            let mut n = CancelNotification::new(sid);
            if let Some(Value::Object(meta)) = &self.flavor.cancel_meta {
                n.meta = Some(meta.clone());
            }
            step.write(json!({"jsonrpc":"2.0","method":M_SESSION_CANCEL,"params":n}).to_string());
            if self.state.autonomous
                && self.state.prompt_id.is_none()
                && self.state.turn_id.is_some()
            {
                self.close_segment(&mut step);
                self.settle_foreground_subagents(&mut step, TurnItemStatus::Interrupted);
                self.retire_tools();
                self.state.cancelling = false;
                self.state.autonomous = false;
                self.state.turn_id = None;
                step.push(DriverEvent::Terminal {
                    status: TurnTerminalStatus::Interrupted,
                    failure: None,
                    reason: None,
                });
            }
        }
        step
    }

    fn respond(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<DriverStep, DriverError> {
        if let Some(step) = self.respond_client(request_id, decision) {
            return Ok(step);
        }
        let p = self
            .state
            .pending
            .remove(request_id)
            .ok_or_else(|| DriverError(format!("unknown request {request_id}")))?;
        // T3 Code's `selectPermissionOptionId` (adapter.ts:1255-1266, plus
        // 21e4f980ca): one option kind per decision, never a WIDER grant than
        // asked for (accept never falls back to allow_always) — but a session
        // grant the agent doesn't offer narrows to allow_once instead of
        // cancelling the turn; none → cancelled.
        let kinds: &[PermissionOptionKind] = match decision {
            ApprovalDecision::Accept => &[PermissionOptionKind::AllowOnce],
            ApprovalDecision::AcceptForSession | ApprovalDecision::AcceptAlways => &[
                PermissionOptionKind::AllowAlways,
                PermissionOptionKind::AllowOnce,
            ],
            ApprovalDecision::Decline => &[PermissionOptionKind::RejectOnce],
            ApprovalDecision::DeclineAlways => &[
                PermissionOptionKind::RejectAlways,
                PermissionOptionKind::RejectOnce,
            ],
            ApprovalDecision::Cancel => &[],
        };
        let option = kinds.iter().find_map(|k| {
            p.options
                .iter()
                .find(|o| o.kind == *k)
                .map(|o| o.option_id.clone())
        });
        let outcome = match option {
            Some(id) => RequestPermissionOutcome::Selected(SelectedPermissionOutcome::new(id)),
            None => RequestPermissionOutcome::Cancelled,
        };
        let resp = RequestPermissionResponse::new(outcome);
        let mut step = DriverStep::default();
        step.write(json!({"jsonrpc":"2.0","id":p.rpc_id,"result":resp}).to_string());
        Ok(step)
    }

    fn respond_question(
        &mut self,
        request_id: &str,
        answers: &[otto_core::agent_runtime::QuestionAnswer],
    ) -> Result<DriverStep, DriverError> {
        let (rpc_id, questions) = self
            .state
            .questions
            .get(request_id)
            .ok_or_else(|| DriverError("no such pending question".into()))?;
        otto_core::agent_runtime::validate_question_answers(questions, answers)
            .map_err(DriverError)?;
        // One mapping for every agent: a url answer is a bare accept, a form
        // answer its fields (`questions::elicitation_answer`).
        let result = crate::questions::elicitation_answer(questions, answers);
        let mut step = DriverStep::default();
        step.write(json!({"jsonrpc":"2.0","id":rpc_id,"result":result}).to_string());
        self.state.questions.remove(request_id);
        Ok(step)
    }

    fn dismiss_question(
        &mut self,
        request_id: &str,
        action: otto_core::agent_runtime::QuestionDismissAction,
    ) -> Result<DriverStep, DriverError> {
        let (rpc_id, _) = self
            .state
            .questions
            .get(request_id)
            .ok_or_else(|| DriverError("no such pending question".into()))?;
        let mut step = DriverStep::default();
        step.write(json!({"jsonrpc":"2.0","id":rpc_id,"result":{"action":action}}).to_string());
        self.state.questions.remove(request_id);
        Ok(step)
    }

    fn on_line(&mut self, line: &str) -> DriverStep {
        let mut step = DriverStep::default();
        self.answer_orphans(&mut step);
        let Ok(msg) = serde_json::from_str::<Value>(line) else {
            step.push(DriverEvent::Error(ProviderFailure {
                class: "protocol_error".into(),
                message: format!(
                    "unparseable agent output: {}",
                    line.chars().take(200).collect::<String>()
                ),
                code: None,
                retryable: None,
            }));
            return step;
        };
        let method = msg
            .get("method")
            .and_then(Value::as_str)
            .map(str::to_string);
        let id = msg.get("id").cloned().filter(|v| !v.is_null());
        let params = msg.get("params").cloned().unwrap_or(Value::Null);
        match (method, id) {
            (Some(m), None) if m == M_SESSION_UPDATE => {
                if let Some(up) = params.get("update") {
                    self.update_effort_options(up);
                    if up.get("sessionUpdate").and_then(Value::as_str)
                        == Some("config_option_update")
                        && self.state.config_req.is_none()
                    {
                        self.follow_model(&mut step, up);
                    }
                }
                if self.state.cancelling {
                    return step;
                }
                let parent_cid = self.resolve_parent_subagent_call_id(&params);
                let has_raw_status = params.pointer("/update/status").is_some();
                match serde_json::from_value::<SessionNotification>(params.clone()) {
                    Ok(n) => self.on_update(&mut step, n, has_raw_status, parent_cid.as_deref()),
                    Err(_) => self.on_raw_update(&mut step, &params, parent_cid.as_deref()),
                }
            }
            (Some(_), None) => {} // unknown notification
            (Some(m), Some(id)) if m == "elicitation/create" => {
                if self.state.cancelling {
                    step.write(
                        json!({"jsonrpc":"2.0","id":id,"result":{"action":"cancel"}}).to_string(),
                    );
                    return step;
                }
                let parsed = if params.get("sessionId").and_then(Value::as_str)
                    != self.state.session_id.as_deref()
                    || self.state.session_id.is_none()
                    || params.get("requestId").is_some()
                {
                    Err("elicitation must belong to this live session".to_string())
                } else if params.get("mode").and_then(Value::as_str) == Some("url") {
                    crate::questions::acp_url(&params)
                } else {
                    crate::questions::acp_form(&params)
                };
                match parsed {
                    Ok(questions) => {
                        self.ensure_autonomous_turn(&mut step);
                        self.close_segment(&mut step);
                        let request_id = format!("acp-question-{}", rpc_key(&id));
                        self.state
                            .questions
                            .insert(request_id.clone(), (id, questions.clone()));
                        step.push(DriverEvent::Question {
                            id: request_id,
                            title: params
                                .get("message")
                                .and_then(Value::as_str)
                                .map(str::to_string),
                            questions,
                            tool_item_id: None,
                        });
                    }
                    Err(message) => step.write(
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":message}})
                            .to_string(),
                    ),
                }
            }
            (Some(m), Some(id)) if m == M_REQUEST_PERMISSION => {
                if self.state.cancelling {
                    let resp = RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled);
                    step.write(json!({"jsonrpc":"2.0","id":id,"result":resp}).to_string());
                    return step;
                }
                match serde_json::from_value::<RequestPermissionRequest>(params) {
                    Ok(req) => self.on_permission(&mut step, id, req),
                    Err(e) => step.write(
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":e.to_string()}})
                            .to_string(),
                    ),
                }
            }
            (Some(m), Some(id)) if m.starts_with("fs/") || m.starts_with("terminal/") => {
                self.on_client_request(&mut step, &m, id, &params);
            }
            (Some(m), Some(id)) => {
                // Anything else: not offered in our client capabilities.
                step.write(
                    json!({"jsonrpc":"2.0","id":id,"error":{"code":-32601,"message":format!("method not supported: {m}")}})
                        .to_string(),
                );
            }
            (None, Some(id)) => {
                if let Some(n) = id.as_u64() {
                    let result = msg.get("result").cloned();
                    let error = msg.get("error").cloned();
                    self.on_response(&mut step, n, result, error);
                }
            }
            (None, None) => {}
        }
        step
    }

    fn on_exit(&mut self, _code: Option<i32>) -> DriverStep {
        self.state.pending.clear();
        self.state.questions.clear();
        self.state.tools.clear();
        self.state.retired_tool_ids.clear();
        self.state.bg_tasks.clear();
        self.state.bg_meta.clear();
        self.state.subagents.clear();
        self.state.child_tools.clear();
        self.state.finished_bg_ids.clear();
        self.state.cancelling = false;
        self.state.autonomous = false;
        self.state.segment = None;
        self.state.closed_named.clear();
        self.state.sideband.on_turn_settled();
        DriverStep::default()
    }

    fn snapshot_state(&self) -> Value {
        serde_json::to_value(&self.state).unwrap_or(Value::Null)
    }

    fn restore_state(&mut self, state: &Value) -> Result<(), DriverError> {
        if state.is_null() {
            return Ok(());
        }
        let mut s: AcpDriverState = serde_json::from_value(state.clone())
            .map_err(|e| DriverError(format!("invalid acp driver state: {e}")))?;
        s.sideband.ensure_restored(self.provider);
        s.mcp_servers = std::mem::take(&mut self.state.mcp_servers);
        self.state = s;
        self.orphan_restored_waits();
        Ok(())
    }

    fn needs_idle_turn_settle(&self) -> bool {
        self.state.autonomous
            && self.state.turn_id.is_some()
            && self.state.prompt_id.is_none()
            && self.state.pending.is_empty()
            && self.state.questions.is_empty()
            && !self.state.sideband.is_awaiting_followup()
            && !self.state.tools.values().any(|t| {
                matches!(
                    t.status,
                    ToolCallStatus::Pending | ToolCallStatus::InProgress
                )
            })
    }

    fn on_idle_settle(&mut self) -> DriverStep {
        let mut step = DriverStep::default();
        self.settle_expired_timers(&mut step, chrono::Utc::now());
        if !self.needs_idle_turn_settle() {
            return step;
        }
        self.settle_autonomous_now(&mut step, TurnTerminalStatus::Completed, None);
        step
    }

    fn sideband_poll(&self) -> Option<SidebandPoll> {
        self.build_sideband_poll()
    }

    fn sideband_resume_poll(&self, resume_id: &str, cwd: &str) -> Option<SidebandSource> {
        self.state
            .sideband
            .resume_source(self.provider, cwd, resume_id)
    }

    fn init_sideband_resume(&mut self, seed: SidebandResumeSeed) {
        self.state.sideband.init_resume(self.provider, seed);
    }

    fn on_client_result(&mut self, id: &str, result: Result<Value, String>) -> DriverStep {
        self.client_result(id, result)
    }

    fn on_client_output(&mut self, terminal_id: &str, chunk: &str) -> DriverStep {
        self.client_output(terminal_id, chunk)
    }

    fn on_sideband(&mut self, payload: SidebandPayload) -> DriverStep {
        let mut step = DriverStep::default();
        let ready_and_live = self.state.phase == Phase::Ready && !self.state.cancelling;

        let primary_payload = match payload {
            SidebandPayload::Batch {
                primary,
                tails,
                timer_tick: _,
            } => {
                if ready_and_live {
                    self.apply_active_tail_chunks(&mut step, tails);
                    self.settle_expired_timers(&mut step, chrono::Utc::now());
                }
                primary.map(|p| *p)
            }
            other => {
                if ready_and_live {
                    self.settle_expired_timers(&mut step, chrono::Utc::now());
                }
                Some(other)
            }
        };

        let Some(prim) = primary_payload else {
            return step;
        };
        let has_open_subagent = self.state.subagents.values().any(|a| a.status.is_open());
        let actions = self.state.sideband.on_payload(
            self.provider,
            self.state.session_id.as_deref(),
            has_open_subagent,
            prim,
        );
        if !ready_and_live {
            return step;
        }
        if self.state.prompt_id.is_some() || self.state.queued.is_some() {
            let child_actions: Vec<_> = actions
                .into_iter()
                .filter(|a| matches!(a, crate::acp_sideband::SidebandAction::ChildScope { .. }))
                .collect();
            if !child_actions.is_empty() {
                self.apply_sideband_actions(&mut step, child_actions);
            }
            return step;
        }
        self.apply_sideband_actions(&mut step, actions);
        step
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(resume: Option<&str>) -> LaunchContext {
        LaunchContext {
            mcp_servers: Vec::new(),
            session_id: "s".into(),
            cwd: "/w".into(),
            model: None,
            resume: resume.map(str::to_string),
            sandboxed: false,
            access_mode: RuntimeAccessMode::default(),
        }
    }

    fn sent(step: &DriverStep, i: usize) -> Value {
        serde_json::from_str(&step.writes[i]).unwrap()
    }

    /// agy: full access is a launch flag, so only the asking modes switch
    /// live; the session opens in the mode no flag gave and carries the
    /// bundle's system-prompt append (the env var is consumed).
    #[test]
    fn agy_full_access_respawns_and_the_bundle_rides_session_new() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("AGENTS.md"), "Rules.").unwrap();
        let mut d = AcpDriver::agy("agy");
        let full = LaunchContext {
            access_mode: RuntimeAccessMode::FullAccess,
            ..ctx(None)
        };
        let mut spec = d.launch(&full);
        assert_eq!(spec.args, ["acp", "--dangerously-skip-permissions"]);
        spec.env.push((
            "OTTO_AGY_CONTEXT_DIR".into(),
            dir.path().display().to_string(),
        ));
        d.adopt_spec(&mut spec);
        assert!(spec.env.is_empty());
        d.start(&full);
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        assert_eq!(
            sent(&s, 0)["params"]["_meta"]["antigravity"]["systemPrompt"]["append"],
            "Rules."
        );
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1","modes":{"currentModeId":"default","availableModes":[{"id":"default"},{"id":"accept-edits"},{"id":"auto"}]}}}"#);
        assert!(s.writes.is_empty(), "full access has no mode to set");
        assert!(!d.switches_access_live(RuntimeAccessMode::ApprovalRequired));

        let mut d = AcpDriver::agy("agy");
        let edits = LaunchContext {
            access_mode: RuntimeAccessMode::AutoAcceptEdits,
            model: Some("gemini-4-argon-high".into()),
            ..ctx(None)
        };
        assert_eq!(d.launch(&edits).args, ["acp"]);
        d.start(&edits);
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1","modes":{"currentModeId":"default","availableModes":[{"id":"default"},{"id":"accept-edits"}]}}}"#);
        assert_eq!(sent(&s, 0)["method"], "session/set_model");
        assert_eq!(sent(&s, 0)["params"]["modelId"], "gemini-4-argon-high");
        assert_eq!(sent(&s, 1)["method"], "session/set_mode");
        assert_eq!(sent(&s, 1)["params"]["modeId"], "accept-edits");
        assert!(d.switches_access_live(RuntimeAccessMode::ApprovalRequired));
        assert!(!d.switches_access_live(RuntimeAccessMode::FullAccess));
    }

    /// agy 1.5.8 / grok advertise effort as a `select` config option; a
    /// prompt's effort writes it (clamped) before the prompt, once.
    #[test]
    fn effort_rides_the_agents_thought_level_option() {
        let mut d = AcpDriver::agy("agy");
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"promptCapabilities":{"image":true}}}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1","configOptions":[{"id":"model","type":"select","currentValue":"gemini-4-carbon","options":[{"value":"gemini-4-carbon"}]},{"id":"thought_level","category":"thought_level","type":"select","currentValue":"medium","options":[{"value":"low"},{"value":"medium"},{"value":"high"}]}]}}"#);
        let caps = d.capabilities();
        assert_eq!(caps.efforts, ["low", "medium", "high"]);
        assert!(caps.images && !caps.files && !caps.steer);
        let mut req = RuntimePromptReq::of("go");
        req.effort = Some("xhigh".into());
        let s = d.prompt("t1", &req).unwrap();
        let w: Vec<Value> = (0..s.writes.len()).map(|i| sent(&s, i)).collect();
        assert_eq!(w[0]["method"], "session/set_config_option");
        assert_eq!(
            (
                w[0]["params"]["configId"].as_str(),
                w[0]["params"]["value"].as_str()
            ),
            (Some("thought_level"), Some("high"))
        );
        assert_eq!(w[1]["method"], "session/prompt");
        // grok lists high → low; `max` is its top level, the picker sorts low → high.
        assert_eq!(
            AcpDriver::grok()
                .resolve_effort_with(&["xhigh".into(), "high".into(), "low".into()], "max")
                .unwrap(),
            "xhigh"
        );
        assert_eq!(
            effort_levels(&["xhigh".into(), "high".into(), "low".into()]),
            ["low", "high", "xhigh"]
        );
        // The current value is not written again; an unknown level is refused.
        let mut d2 = AcpDriver::gemini("gemini");
        d2.start(&ctx(None));
        d2.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d2.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#);
        // No effort option: the prompt still goes, at the agent's default,
        // and the timeline says so (never a refused prompt).
        let sent = d2
            .prompt("t1", &req)
            .expect("effort is dropped, not refused");
        assert!(sent.events.iter().any(|e| matches!(
            e,
            DriverEvent::TurnItem(i) if matches!(
                &i.kind,
                TurnItemKind::SystemNotice { message, .. }
                    if message.contains("does not take a reasoning effort")
                        && message.contains("default effort")
            )
        )));
        assert!(sent.writes.iter().all(|w| !w.contains("thought_level")));
        assert!(d2.capabilities().efforts.is_empty());
    }

    /// Muse 1.4.20's real handshake frames: `muse acp --trust-workspace`
    /// with `--model`, its sandbox off under Otto's Seatbelt and the
    /// launcher's update check off; the approval policy is a live
    /// `session/set_mode`; effort is the `thought_level` option, sent by the
    /// tier name Muse lists.
    #[test]
    fn muse_launch_modes_and_effort() {
        let mut d = AcpDriver::muse("muse");
        let c = LaunchContext {
            model: Some("muse-spark-1.3".into()),
            access_mode: RuntimeAccessMode::ApprovalRequired,
            sandboxed: true,
            ..ctx(None)
        };
        let spec = d.launch(&c);
        assert_eq!(spec.program, "muse");
        assert_eq!(
            spec.args,
            ["acp", "--trust-workspace", "--disable-sandbox"],
            "the model is a config option, not a flag"
        );
        assert!(spec.tail_args.is_empty());
        assert_eq!(
            spec.env,
            [("MUSE_NO_AUTO_UPDATE".to_string(), "1".to_string())]
        );
        let unconfined = d.launch(&LaunchContext {
            sandboxed: false,
            ..c.clone()
        });
        assert!(!unconfined.args.iter().any(|a| a == "--disable-sandbox"));
        assert!(d.switches_model_live());

        d.start(&c);
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"promptCapabilities":{"image":true,"audio":false,"embeddedContext":true}},"authMethods":[{"id":"muse-login","name":"Log in to Muse"}],"agentInfo":{"name":"muse","version":"1.4.20"}}}"#);
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"01a1e323","modes":{"currentModeId":"onRequest","availableModes":[{"id":"allowAll","name":"Allow all"},{"id":"promptUnmatched","name":"Ask before unmatched actions"},{"id":"onRequest","name":"Ask on request"},{"id":"denyUnmatched","name":"Deny unmatched actions"}]},"configOptions":[{"id":"mode","category":"mode","type":"select","currentValue":"onRequest","options":[]},{"id":"thought_level","category":"thought_level","type":"select","currentValue":"high","options":[{"value":"minimal"},{"value":"low"},{"value":"medium"},{"value":"high"},{"value":"xhigh"},{"value":"max"},{"value":"ultra"}]}]}}"#);
        let w: Vec<Value> = (0..s.writes.len()).map(|i| sent(&s, i)).collect();
        assert_eq!(
            (
                w[0]["params"]["configId"].as_str(),
                w[0]["params"]["value"].as_str()
            ),
            (Some("model"), Some("muse-spark-1.3"))
        );
        assert_eq!(w[1]["method"], "session/set_mode");
        assert_eq!(w[1]["params"]["modeId"], "promptUnmatched");
        let caps = d.capabilities();
        assert!(caps.images && caps.files && !caps.plan && !caps.steer);
        assert_eq!(
            caps.efforts,
            ["minimal", "low", "medium", "high", "xhigh", "max", "ultra"]
        );
        for (m, id) in [
            (RuntimeAccessMode::AutoAcceptEdits, "onRequest"),
            (RuntimeAccessMode::FullAccess, "allowAll"),
        ] {
            assert!(d.switches_access_live(m), "{m:?}");
            let mut step = DriverStep::default();
            d.write_mode(&mut step, "01a1e323", m);
            assert_eq!(sent(&step, 0)["params"]["modeId"], id, "{m:?}");
        }
        let mut req = RuntimePromptReq::of("go");
        req.effort = Some("ultra".into());
        let s = d.prompt("t1", &req).unwrap();
        let w: Vec<Value> = (0..s.writes.len()).map(|i| sent(&s, i)).collect();
        assert_eq!(
            (
                w[0]["params"]["configId"].as_str(),
                w[0]["params"]["value"].as_str()
            ),
            (Some("thought_level"), Some("ultra"))
        );
        assert_eq!(w[1]["method"], "session/prompt");
        assert!(d.prompt("t2", &RuntimePromptReq::of("again")).is_err());
    }

    /// Muse 1.4.25 (live): `mode` + `model` config options, no
    /// `thought_level` — no effort is offered, and one asked for is dropped
    /// with a notice instead of a `-32602` from the agent.
    #[test]
    fn muse_without_an_effort_option_offers_none_and_drops_one_asked_for() {
        let mut d = AcpDriver::muse("muse");
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s","modes":{"currentModeId":"onRequest","availableModes":[{"id":"onRequest","name":"Ask"}]},"configOptions":[{"id":"mode","category":"mode","type":"select","currentValue":"onRequest","options":[{"value":"onRequest"}]},{"id":"model","category":"model","type":"select","currentValue":"muse-wonder-full-1.0","options":[{"value":"muse-spark-1.3"}]}]}}"#);
        assert!(d.capabilities().efforts.is_empty());
        let mut req = RuntimePromptReq::of("go");
        req.effort = Some("high".into());
        let s = d.prompt("t1", &req).unwrap();
        let w: Vec<Value> = (0..s.writes.len()).map(|i| sent(&s, i)).collect();
        assert_eq!(w.len(), 1, "{w:?}");
        assert_eq!(w[0]["method"], "session/prompt");
        assert!(s.events.iter().any(|e| matches!(e,
            DriverEvent::TurnItem(i) if i.id.ends_with(":effort-dropped"))));
    }

    /// A signed-out Muse refuses `session/new` (-32000). Its only auth
    /// method opens a browser device login, so Otto never calls
    /// `authenticate`: the thread fails with Muse's own message.
    #[test]
    fn muse_signed_out_fails_without_starting_a_browser_login() {
        let mut d = AcpDriver::muse("muse");
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"authMethods":[{"id":"muse-login","name":"Log in to Muse","description":"Sign in with a device code shown in your browser."}]}}"#);
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"log in to Muse before opening a session"}}"#);
        assert!(s.writes.is_empty(), "no authenticate: {:?}", s.writes);
        assert!(
            matches!(&s.events[0], DriverEvent::Fatal(f) if f.class == "handshake_error" && f.message.contains("log in to Muse")),
            "{:?}",
            s.events
        );
    }

    /// agy's swarm (live 1.5.8 frames): each `swarm add` is a background
    /// subagent card that a `swarm kill` stops; other actions stay tool rows.
    #[test]
    fn a_load_replay_cards_only_swarm_members_without_a_card() {
        let mut d = AcpDriver::agy("agy");
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#);
        // Carded before a restart (kept in the driver's state).
        d.state.swarm_seen.insert("echoer".into());
        // A refused resume replays the conversation through session/load.
        d.state.phase = Phase::Creating { loading: true };
        let upd = |id: &str, input: &str| {
            format!(
                r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s1","update":{{"sessionUpdate":"tool_call_update","toolCallId":"{id}","name":"swarm","status":"completed","kind":"other","title":"Run swarm","rawInput":{input}}}}}}}"#
            )
        };
        let cards = |s: &DriverStep| -> Vec<TurnItem> {
            s.events
                .iter()
                .filter_map(|e| match e {
                    DriverEvent::TurnItem(i) if matches!(i.kind, TurnItemKind::Subagent { .. }) => {
                        Some(i.clone())
                    }
                    _ => None,
                })
                .collect()
        };
        let s = d.on_line(&upd(
            "c1",
            r#"{"action":"add","name":"echoer","brief":"b"}"#,
        ));
        assert!(cards(&s).is_empty(), "already carded");
        let s = d.on_line(&upd(
            "c2",
            r#"{"action":"add","name":"lost","brief":"find it"}"#,
        ));
        let made = cards(&s);
        assert_eq!(made.len(), 1);
        assert_eq!(made[0].id, "replay:swarm:lost");
        assert!(made[0].turn_id.is_none());
        assert!(made[0].is_background_open());
        // Replayed again: no second card. Other replayed history stays dropped.
        assert!(
            cards(&d.on_line(&upd("c3", r#"{"action":"add","name":"lost","brief":"x"}"#)))
                .is_empty()
        );
        let chunk = r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"old"}}}}"#;
        assert!(d.on_line(chunk).events.is_empty());
    }

    #[test]
    fn agy_swarm_members_are_background_subagents() {
        let mut d = AcpDriver::agy("agy");
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#);
        d.prompt("t1", &RuntimePromptReq::of("swarm it")).unwrap();
        let upd = |id: &str, input: &str, status: &str| {
            format!(
                r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s1","update":{{"sessionUpdate":"tool_call_update","toolCallId":"{id}","name":"swarm","status":"{status}","kind":"other","title":"Run swarm","rawInput":{input}}}}}}}"#
            )
        };
        let s = d.on_line(&upd(
            "c1",
            r#"{"action":"add","name":"echoer","brief":"echo hi"}"#,
            "completed",
        ));
        let card = s.events.iter().find_map(|e| match e {
            DriverEvent::TurnItem(i) if matches!(i.kind, TurnItemKind::Subagent { .. }) => {
                Some(i.clone())
            }
            _ => None,
        });
        let card = card.expect("a subagent card");
        assert!(card.is_background_open());
        assert_eq!(card.turn_id.as_deref(), Some("t1"));
        assert!(
            matches!(&card.kind, TurnItemKind::Subagent { description: Some(n), prompt: Some(p), .. } if n == "echoer" && p == "echo hi")
        );
        let s = d.on_line(&upd(
            "c2",
            r#"{"action":"add","name":"shard","brief":"b","n":2}"#,
            "completed",
        ));
        let names: Vec<String> = s
            .events
            .iter()
            .filter_map(|e| match e {
                DriverEvent::TurnItem(TurnItem {
                    kind:
                        TurnItemKind::Subagent {
                            description: Some(n),
                            ..
                        },
                    ..
                }) => Some(n.clone()),
                _ => None,
            })
            .collect();
        assert_eq!(names, ["shard-1", "shard-2"]);
        let s = d.on_line(&upd("c3", r#"{"action":"status"}"#, "completed"));
        assert_eq!(s.events.len(), 1, "status is just its tool row");
        let s = d.on_line(&upd(
            "c4",
            r#"{"action":"kill","name":"shard"}"#,
            "completed",
        ));
        let stopped: Vec<&TurnItem> = s
            .events
            .iter()
            .filter_map(|e| match e {
                DriverEvent::TurnItem(i) if i.status == TurnItemStatus::Interrupted => Some(i),
                _ => None,
            })
            .collect();
        assert_eq!(stopped.len(), 2);
        // Only agy: the same frames from another ACP agent stay tool rows.
        let mut g = AcpDriver::gemini("gemini");
        g.start(&ctx(None));
        g.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        g.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#);
        g.prompt("t1", &RuntimePromptReq::of("x")).unwrap();
        assert_eq!(
            g.on_line(&upd(
                "c1",
                r#"{"action":"add","name":"e","brief":"b"}"#,
                "completed"
            ))
            .events
            .len(),
            1
        );
    }

    /// The host's MCP servers (Otto's `otto` tools + the user's) ride
    /// `session/new` as stdio entries, each with its own env.
    #[test]
    fn session_new_carries_the_mcp_servers() {
        let mut d = AcpDriver::gemini("gemini");
        assert_eq!(d.mcp_delivery(), McpDelivery::Protocol);
        let c = LaunchContext {
            mcp_servers: vec![McpServerDef {
                name: "otto".into(),
                command: "/bin/ottod".into(),
                args: vec!["mcp-tools".into()],
                env: vec![("OTTO_MCP_TOKEN".into(), "t".into())],
            }],
            ..ctx(None)
        };
        d.start(&c);
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        let req = sent(&s, 0);
        assert_eq!(req["method"], "session/new");
        let m = &req["params"]["mcpServers"][0];
        assert_eq!(m["name"], "otto");
        assert_eq!(m["command"], "/bin/ottod");
        assert_eq!(m["args"][0], "mcp-tools");
        assert_eq!(m["env"][0]["name"], "OTTO_MCP_TOKEN");
        assert_eq!(m["env"][0]["value"], "t");
    }

    #[test]
    fn opencode_takes_its_model_as_a_config_option_and_access_as_env() {
        let mut d = AcpDriver::opencode();
        let c = LaunchContext {
            model: Some("anthropic/claude-sonnet".into()),
            access_mode: RuntimeAccessMode::FullAccess,
            ..ctx(None)
        };
        let spec = d.launch(&c);
        assert_eq!(
            (spec.program.as_str(), spec.tail_args.as_slice()),
            ("opencode", &["acp".to_string()][..])
        );
        assert!(spec.args.is_empty(), "no model flag: {:?}", spec.args);
        assert_eq!(
            spec.env,
            [(
                "OPENCODE_CONFIG_CONTENT".to_string(),
                r#"{"permission":{"*":"allow"}}"#.to_string()
            )]
        );
        // Every mode is explicit and valid JSON (opencode's default allows).
        for (mode, bash, edit) in [
            (RuntimeAccessMode::ApprovalRequired, "ask", "ask"),
            (RuntimeAccessMode::AutoAcceptEdits, "ask", "allow"),
        ] {
            let spec = d.launch(&LaunchContext {
                access_mode: mode,
                ..c.clone()
            });
            let config: Value = serde_json::from_str(&spec.env[0].1).unwrap();
            let rules = &config["permission"];
            assert_eq!(rules["*"], bash);
            assert_eq!(
                rules.get("edit").map_or("ask", |v| v.as_str().unwrap()),
                edit
            );
            assert_eq!(rules["read"], "allow");
        }
        d.start(&c);
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"ses_1"}}"#);
        let set = sent(&s, 0);
        assert_eq!(set["method"], "session/set_config_option");
        assert_eq!(set["params"]["configId"], "model");
        assert_eq!(set["params"]["value"], "anthropic/claude-sonnet");
    }

    /// opencode 2.3.0 (live dump): no session modes; `effort` and `mode`
    /// (`build` / `plan`) config options. Effort and plan mode go as
    /// `session/set_config_option`; leaving plan returns to `build`.
    #[test]
    fn opencode_effort_and_plan_ride_its_config_options() {
        let mut d = AcpDriver::opencode();
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"ses_1","configOptions":[{"id":"model","category":"model","type":"select","currentValue":"a/b","options":[{"value":"a/b"}]},{"id":"effort","category":"thought_level","type":"select","currentValue":"default","options":[{"value":"minimal"},{"value":"low"},{"value":"medium"},{"value":"high"},{"value":"xhigh"},{"value":"max"},{"value":"default"}]},{"id":"mode","category":"mode","type":"select","currentValue":"build","options":[{"value":"build"},{"value":"plan"}]}]}}"#);
        let caps = d.capabilities();
        assert!(caps.plan);
        assert!(
            caps.efforts.iter().any(|e| e == "high"),
            "{:?}",
            caps.efforts
        );
        let mut req = RuntimePromptReq::of("go");
        req.effort = Some("high".into());
        req.plan = true;
        let s = d.prompt("t1", &req).unwrap();
        let w: Vec<Value> = (0..s.writes.len()).map(|i| sent(&s, i)).collect();
        let cfg = |v: &Value| {
            (
                v["params"]["configId"].clone(),
                v["params"]["value"].clone(),
            )
        };
        assert_eq!(cfg(&w[0]), (json!("effort"), json!("high")));
        assert_eq!(cfg(&w[1]), (json!("mode"), json!("plan")));
        assert_eq!(w[2]["method"], "session/prompt");
        d.on_line(r#"{"jsonrpc":"2.0","id":5,"result":{"stopReason":"end_turn"}}"#);
        let s = d.prompt("t2", &RuntimePromptReq::of("now build")).unwrap();
        assert_eq!(cfg(&sent(&s, 0)), (json!("mode"), json!("build")));
    }

    /// Gap 11: an agent advertising `sessionCapabilities.resume` gets
    /// `session/resume` (no history replay); a refusal retries as load.
    #[test]
    fn resume_is_preferred_and_falls_back_to_load() {
        let mut d = AcpDriver::opencode();
        d.start(&ctx(Some("ses_old")));
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true,"sessionCapabilities":{"resume":{}}}}}"#);
        assert_eq!(sent(&s, 0)["method"], "session/resume");
        assert_eq!(sent(&s, 0)["params"]["sessionId"], "ses_old");
        let s = d.on_line(
            r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32603,"message":"not resident"}}"#,
        );
        assert_eq!(sent(&s, 0)["method"], "session/load");
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":3,"result":{}}"#);
        assert!(s.events.iter().any(|e| matches!(e,
            DriverEvent::Ready { native_thread_id: Some(n), .. } if n == "ses_old")));
    }

    #[test]
    fn handshake_loads_when_resuming_and_supported_and_queues_the_prompt() {
        let mut d = AcpDriver::gemini("gemini");
        let s = d.start(&ctx(Some("old")));
        assert_eq!(sent(&s, 0)["method"], "initialize");
        assert_eq!(
            sent(&s, 0)["params"]["clientCapabilities"]["terminal"],
            false
        );
        let q = d.prompt("t1", &RuntimePromptReq::of("hi")).unwrap();
        assert!(q.writes.is_empty(), "queued until the session exists");
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}}"#);
        assert_eq!(sent(&s, 0)["method"], "session/load");
        assert_eq!(sent(&s, 0)["params"]["sessionId"], "old");
        // History replayed during load is dropped.
        let h = d.on_line(r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"old","update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"old"}}}}"#);
        assert!(h.events.is_empty());
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{}}"#);
        assert!(
            matches!(&s.events[0], DriverEvent::Ready { native_thread_id: Some(n), .. } if n == "old")
        );
        assert_eq!(sent(&s, 0)["method"], "session/prompt");
        assert_eq!(sent(&s, 0)["params"]["prompt"][0]["text"], "hi");
    }

    #[test]
    fn images_go_as_image_blocks_only_where_advertised() {
        let image = RuntimePromptReq {
            images: vec![otto_core::agent_runtime::PromptImage {
                mime: "image/png".into(),
                data_base64: "AAAA".into(),
                name: None,
            }],
            ..RuntimePromptReq::of("look")
        };
        for advertised in [false, true] {
            let mut d = AcpDriver::gemini("gemini");
            d.start(&ctx(None));
            let caps = json!({"protocolVersion":1,"agentCapabilities":{"promptCapabilities":{"image":advertised}}});
            d.on_line(&json!({"jsonrpc":"2.0","id":1,"result":caps}).to_string());
            d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#);
            match d.prompt("t1", &image) {
                Err(e) => assert!(!advertised && e.0 == "gemini does not accept images"),
                Ok(s) => {
                    assert!(advertised);
                    let p = &sent(&s, 0)["params"]["prompt"];
                    assert_eq!(
                        p[0],
                        json!({"type":"image","data":"AAAA","mimeType":"image/png"})
                    );
                    assert_eq!(p[1]["text"], "look");
                }
            }
        }
    }

    #[test]
    fn grok_launch_order_and_lazy_authenticate_then_retry() {
        let mut d = AcpDriver::grok();
        let spec = d.launch(&LaunchContext {
            mcp_servers: Vec::new(),
            model: Some("grok-4.6".into()),
            ..ctx(None)
        });
        assert_eq!(spec.program, "grok");
        assert_eq!(
            spec.args,
            ["--permission-mode", "default", "-m", "grok-4.6"]
        );
        assert_eq!(spec.tail_args, ["agent", "stdio"]);
        let s = d.start(&ctx(None));
        assert_eq!(sent(&s, 0)["params"]["_meta"]["clientType"], "extension");
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"authMethods":[{"id":"grok.com","name":"Grok"},{"id":"cached_token","name":"cached"}]}}"#);
        // session/new refused: auth required → authenticate with the
        // preferred advertised method, then session/new again.
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32000,"message":"Authentication required"}}"#);
        assert_eq!(sent(&s, 0)["method"], "authenticate");
        let want = if std::env::var_os("XAI_API_KEY").is_some() {
            "grok.com" // xai.api_key not advertised → first agent method
        } else {
            "cached_token"
        };
        assert_eq!(sent(&s, 0)["params"]["methodId"], want);
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":3,"result":{}}"#);
        assert_eq!(sent(&s, 0)["method"], "session/new");
        // A second auth failure is reported, not looped.
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":4,"error":{"code":-32000,"message":"Authentication required"}}"#);
        assert!(s.writes.is_empty());
        assert!(matches!(&s.events[0], DriverEvent::Fatal(f) if f.class == "handshake_error"));
        // Cancel carries Grok's _meta.
        let mut d = AcpDriver::grok();
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"g1"}}"#);
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let s = d.interrupt();
        assert_eq!(sent(&s, 0)["params"]["_meta"]["cancelTrigger"], "ctrl_c");
    }

    #[test]
    fn failed_load_falls_back_to_a_new_session() {
        let mut d = AcpDriver::gemini("gemini");
        d.start(&ctx(Some("gone")));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"loadSession":true}}}"#);
        let s = d.on_line(
            r#"{"jsonrpc":"2.0","id":2,"error":{"code":-32002,"message":"no such session"}}"#,
        );
        assert_eq!(sent(&s, 0)["method"], "session/new");
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":3,"result":{"sessionId":"fresh"}}"#);
        assert!(
            matches!(&s.events[0], DriverEvent::Ready { native_thread_id: Some(n), .. } if n == "fresh")
        );
    }

    /// Live wire (grok 1.0.92, approval-required, Claude `defaultMode:
    /// dontAsk` on the machine): the tool fails with no permission request.
    /// The thread says why, once.
    #[test]
    fn grok_denied_without_asking_says_so_once() {
        let mut d = AcpDriver::grok();
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"g1"}}"#);
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let denied = |id: &str| {
            format!(
                r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"g1","update":{{"sessionUpdate":"tool_call_update","toolCallId":"{id}","title":"echo","kind":"execute","status":"failed","content":[{{"type":"content","content":{{"type":"text","text":"Tool `run_terminal_command` was not executed: denied by prompt policy (tool not pre-approved)"}}}}]}}}}}}"#
            )
        };
        let notices = |s: &DriverStep| {
            s.events
                .iter()
                .filter(|e| matches!(e, DriverEvent::TurnItem(i) if i.id.ends_with(":grok-denied")))
                .count()
        };
        assert_eq!(notices(&d.on_line(&denied("c1"))), 1);
        assert_eq!(notices(&d.on_line(&denied("c2"))), 0, "once per session");
    }

    /// Gaps 6 + 7: notices and compaction are quiet timeline lines; an
    /// image answer gets its own row; links and embedded text resources
    /// read as text, in answers and in tool output.
    #[test]
    fn notices_compaction_and_non_text_content_are_shown() {
        let mut d = AcpDriver::gemini("gemini");
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s"}}"#);
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let up = |u: &str| {
            format!(
                r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s","update":{u}}}}}"#
            )
        };
        let items = |s: DriverStep| -> Vec<TurnItem> {
            s.events
                .into_iter()
                .filter_map(|e| match e {
                    DriverEvent::TurnItem(i) => Some(i),
                    _ => None,
                })
                .collect()
        };
        let msg = |i: &TurnItem| match &i.kind {
            TurnItemKind::SystemNotice { message, .. } => message.clone(),
            TurnItemKind::AssistantMessage { text, .. } => text.clone(),
            TurnItemKind::DynamicTool { output, images, .. } => {
                format!(
                    "{} images:{}",
                    output.clone().unwrap_or_default(),
                    images.len()
                )
            }
            _ => String::new(),
        };
        let n = items(d.on_line(&up(
            r#"{"sessionUpdate":"notice","severity":"warning","title":"Rate limited","description":"retrying in 5s"}"#,
        )));
        assert_eq!(msg(&n[0]), "Rate limited \u{2014} retrying in 5s");
        let c = items(d.on_line(&up(
            r#"{"sessionUpdate":"compaction_update","compactionId":"k1","status":"in_progress"}"#,
        )));
        assert_eq!(msg(&c[0]), "Compacting context\u{2026}");
        let c = items(d.on_line(&up(
            r#"{"sessionUpdate":"compaction_update","compactionId":"k1","status":"completed"}"#,
        )));
        assert_eq!(
            (msg(&c[0]).as_str(), c[0].id.as_str()),
            ("Context compacted", "t1:compaction")
        );
        let l = items(d.on_line(&up(
            r#"{"sessionUpdate":"agent_message_chunk","content":{"type":"resource_link","name":"spec","uri":"file:///a/spec.md"}}"#,
        )));
        assert_eq!(msg(l.last().unwrap()), "[spec](file:///a/spec.md)");
        let png = "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
        let i = items(d.on_line(&up(&format!(
            r#"{{"sessionUpdate":"agent_message_chunk","content":{{"type":"image","mimeType":"image/png","data":"{png}"}}}}"#
        ))));
        assert!(
            msg(i.last().unwrap()).ends_with("images:1"),
            "{:?}",
            i.last()
        );
        d.on_line(&up(r#"{"sessionUpdate":"tool_call","toolCallId":"c1","title":"fetch","kind":"fetch","status":"in_progress"}"#));
        let t = items(d.on_line(&up(
            r#"{"sessionUpdate":"tool_call_update","toolCallId":"c1","status":"completed","content":[{"type":"content","content":{"type":"resource","resource":{"uri":"file:///n.txt","text":"note body"}}}]}"#,
        )));
        assert!(
            msg(t.last().unwrap()).starts_with("note body"),
            "{:?}",
            t.last()
        );
    }

    /// Gap 10: url-mode elicitation is one choice naming the URL (accept
    /// when chosen); a non-http(s) url is refused `-32602`.
    #[test]
    fn url_elicitation_names_the_url_and_accepts_when_chosen() {
        let mut d = AcpDriver::gemini("gemini");
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s"}}"#);
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":9,"method":"elicitation/create","params":{"sessionId":"s","mode":"url","elicitationId":"e1","url":"https://example.com/login","message":"Sign in to continue"}}"#);
        let (id, qs) = s
            .events
            .iter()
            .find_map(|e| match e {
                DriverEvent::Question { id, questions, .. } => {
                    Some((id.clone(), questions.clone()))
                }
                _ => None,
            })
            .expect("a question");
        assert_eq!(
            qs[0].options[0].description.as_deref(),
            Some("https://example.com/login")
        );
        assert!(qs[0].prompt.contains("Sign in to continue"));
        let answer = otto_core::agent_runtime::QuestionAnswer::Single {
            question_id: "acp_url".into(),
            value: "open".into(),
        };
        let s = d.respond_question(&id, &[answer]).unwrap();
        assert_eq!(sent(&s, 0)["result"], json!({"action": "accept"}));
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":10,"method":"elicitation/create","params":{"sessionId":"s","mode":"url","url":"file:///etc/passwd","message":"x"}}"#);
        assert_eq!(sent(&s, 0)["error"]["code"], -32602);
    }

    /// Gap 10: `reject_always` is offered as `decline_always` and answers
    /// with that option.
    #[test]
    fn reject_always_is_offered_and_answered() {
        let mut d = AcpDriver::gemini("gemini");
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s"}}"#);
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":"p1","method":"session/request_permission","params":{"sessionId":"s","toolCall":{"toolCallId":"c1","kind":"execute","title":"rm"},"options":[{"optionId":"a","name":"Allow","kind":"allow_once"},{"optionId":"n","name":"Never","kind":"reject_always"},{"optionId":"r","name":"Deny","kind":"reject_once"}]}}"#);
        let (id, options) = s
            .events
            .iter()
            .find_map(|e| match e {
                DriverEvent::Request { id, options, .. } => Some((id.clone(), options.clone())),
                _ => None,
            })
            .unwrap();
        assert!(options
            .iter()
            .any(|o| o.decision == ApprovalDecision::DeclineAlways && o.label == "Never"));
        let s = d.respond(&id, ApprovalDecision::DeclineAlways).unwrap();
        assert_eq!(sent(&s, 0)["result"]["outcome"]["optionId"], "n");
    }

    /// Every location a tool names is shown, not only the first.
    /// grok runs a command through `terminal/create` without embedding the
    /// terminal in the call: the open execute call it runs still streams.
    #[test]
    fn a_client_terminal_streams_into_the_execute_call_it_runs() {
        let mut d = AcpDriver::grok();
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"g"}}"#);
        d.state.access = RuntimeAccessMode::FullAccess;
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        d.on_line(r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"g","update":{"sessionUpdate":"tool_call","toolCallId":"c1","title":"run","kind":"execute","status":"in_progress","rawInput":{"command":"echo x"}}}}"#);
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":3,"method":"terminal/create","params":{"sessionId":"g","command":"/bin/bash -lc 'echo x'"}}"#);
        let (id, op) = s
            .events
            .iter()
            .find_map(|e| match e {
                DriverEvent::ClientCall { id, op } => Some((id.clone(), op.clone())),
                _ => None,
            })
            .expect("a client call");
        let crate::driver::ClientOp::TerminalCreate { program, args, .. } = op else {
            panic!("{op:?}")
        };
        assert_eq!((program.as_str(), args[0].as_str()), ("/bin/sh", "-c"));
        let s = d.on_client_result(&id, Ok(json!({})));
        assert_eq!(sent(&s, 0)["result"]["terminalId"], "term-1");
        let s = d.on_client_output("term-1", "x\n");
        assert!(s.events.iter().any(|e| matches!(e,
            DriverEvent::CommandOutput { item_id, chunk } if item_id == "t1:tool:c1" && chunk == "x\n")));
    }

    #[test]
    fn every_location_is_shown() {
        let mut d = AcpDriver::gemini("gemini");
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s"}}"#);
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let s = d.on_line(r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"tool_call","toolCallId":"c1","title":"grep","kind":"search","status":"completed","locations":[{"path":"/w/a.rs"},{"path":"/w/b.rs"}]}}}"#);
        let out = s.events.iter().find_map(|e| match e {
            DriverEvent::TurnItem(TurnItem {
                kind: TurnItemKind::DynamicTool { output, .. },
                ..
            }) => output.clone(),
            _ => None,
        });
        assert_eq!(out.as_deref(), Some("Files: /w/a.rs, /w/b.rs"));
        let s = d.on_line(r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s","update":{"sessionUpdate":"tool_call","toolCallId":"c2","title":"mv","kind":"move","status":"completed","locations":[{"path":"/w/old.rs"},{"path":"/w/new.rs"}]}}}"#);
        assert!(s.events.iter().any(|e| matches!(e,
            DriverEvent::TurnItem(TurnItem { kind: TurnItemKind::FileChange { file_name, .. }, .. })
                if file_name == "/w/old.rs, /w/new.rs")));
    }

    /// Security 6: a chatty command's row keeps only the tail.
    #[test]
    fn a_client_terminals_row_keeps_a_bounded_tail() {
        let mut d = AcpDriver::grok();
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"g"}}"#);
        d.state.access = RuntimeAccessMode::FullAccess;
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        d.on_line(r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"g","update":{"sessionUpdate":"tool_call","toolCallId":"c1","title":"run","kind":"execute","status":"in_progress","rawInput":{"command":"yes"}}}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":3,"method":"terminal/create","params":{"sessionId":"g","command":"yes"}}"#);
        let chunk = "y\n".repeat(32 * 1024);
        for _ in 0..16 {
            d.on_client_output("term-1", &chunk);
        }
        assert!(d.state.tools["c1"].output.len() <= 256 << 10);
        assert!(d.state.terminal_output["term-1"].len() <= 256 << 10);
    }

    /// Review note: under Auto-accept edits a served command still asks
    /// (edits only are auto-accepted); a write does not.
    #[test]
    fn auto_accept_edits_still_asks_before_a_client_command() {
        let mut d = AcpDriver::grok();
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"g"}}"#);
        d.state.access = RuntimeAccessMode::AutoAcceptEdits;
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":"c1","method":"terminal/create","params":{"sessionId":"g","command":"rm -rf build"}}"#);
        assert!(s.events.iter().any(|e| matches!(
            e,
            DriverEvent::Request {
                kind: RequestKind::Command,
                ..
            }
        )));
        assert!(!s
            .events
            .iter()
            .any(|e| matches!(e, DriverEvent::ClientCall { .. })));
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":"w1","method":"fs/write_text_file","params":{"sessionId":"g","path":"/tmp/x/a.txt","content":"hi"}}"#);
        assert!(
            s.events
                .iter()
                .any(|e| matches!(e, DriverEvent::ClientCall { .. })),
            "an edit goes ahead"
        );
    }

    /// Review R5: a client call in flight at a daemon restart is answered
    /// with an error once the restored driver sees the agent's next line.
    #[test]
    fn a_client_call_in_flight_at_a_restart_is_answered_with_an_error() {
        let mut d = AcpDriver::grok();
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"g"}}"#);
        d.state.access = RuntimeAccessMode::FullAccess;
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":"w9","method":"terminal/wait_for_exit","params":{"sessionId":"g","terminalId":"term-1"}}"#);
        assert!(s
            .events
            .iter()
            .any(|e| matches!(e, DriverEvent::ClientCall { .. })));
        let snap = d.snapshot_state();
        let mut adopted = AcpDriver::grok();
        adopted.restore_state(&snap).unwrap();
        let s = adopted.on_line(r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"g","update":{"sessionUpdate":"usage_update","used":1,"size":2}}}"#);
        assert_eq!(sent(&s, 0)["id"], "w9");
        assert_eq!(sent(&s, 0)["error"]["code"], -32000);
        let s = adopted.on_line(r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"g","update":{"sessionUpdate":"usage_update","used":1,"size":2}}}"#);
        assert!(s.writes.is_empty(), "answered once");
    }

    /// Finding 1: a confined (Seatbelt) grok gets no `fs` capability and its
    /// `fs/*` calls are refused — the daemon would write past the profile.
    /// Terminals stay (they run under the same profile).
    #[test]
    fn a_confined_agent_is_not_served_fs() {
        let init_of = |confined: bool| {
            let mut d = AcpDriver::grok();
            let mut spec = LaunchSpec {
                program: if confined {
                    "/usr/bin/sandbox-exec".into()
                } else {
                    "grok".into()
                },
                args: if confined {
                    vec!["-p".into(), "(version 1)".into(), "grok".into()]
                } else {
                    Vec::new()
                },
                tail_args: Vec::new(),
                env: Vec::new(),
            };
            d.adopt_spec(&mut spec);
            let s = d.start(&ctx(None));
            (d, sent(&s, 0))
        };
        let (_, open) = init_of(false);
        assert_eq!(
            open["params"]["clientCapabilities"]["fs"]["writeTextFile"],
            true
        );
        let (mut d, init) = init_of(true);
        let caps = &init["params"]["clientCapabilities"];
        assert_eq!(caps["fs"]["writeTextFile"], false, "{caps}");
        assert_eq!(caps["fs"]["readTextFile"], false, "{caps}");
        assert_eq!(caps["terminal"], true);
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"g"}}"#);
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":7,"method":"fs/write_text_file","params":{"sessionId":"g","path":"/w/.git/hooks/post-checkout","content":"x"}}"#);
        assert_eq!(sent(&s, 0)["error"]["code"], -32601);
        assert!(!s
            .events
            .iter()
            .any(|e| matches!(e, DriverEvent::ClientCall { .. })));
    }

    #[test]
    fn decisions_map_onto_acp_option_kinds() {
        let mut d = AcpDriver::gemini("gemini");
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#);
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":"p7","method":"session/request_permission","params":{"sessionId":"s1","toolCall":{"toolCallId":"c1","kind":"execute","title":"ls"},"options":[{"optionId":"a1","name":"Allow","kind":"allow_once"},{"optionId":"a2","name":"Always","kind":"allow_always"},{"optionId":"r1","name":"Deny","kind":"reject_once"}]}}"#);
        let DriverEvent::Request {
            id, kind, options, ..
        } = &s.events[0]
        else {
            panic!("{s:?}")
        };
        assert_eq!((id.as_str(), *kind), ("acp-p7", RequestKind::Command));
        let decisions: Vec<ApprovalDecision> = options.iter().map(|o| o.decision).collect();
        assert_eq!(
            decisions,
            vec![
                ApprovalDecision::Accept,
                ApprovalDecision::AcceptForSession,
                ApprovalDecision::Decline
            ]
        );
        let r = d
            .respond("acp-p7", ApprovalDecision::AcceptForSession)
            .unwrap();
        assert_eq!(sent(&r, 0)["id"], "p7");
        assert_eq!(sent(&r, 0)["result"]["outcome"]["optionId"], "a2");
        assert!(
            d.respond("acp-p7", ApprovalDecision::Accept).is_err(),
            "answered once"
        );
        // No session grant offered: accept-for-session narrows to allow_once
        // instead of cancelling the turn.
        d.on_line(r#"{"jsonrpc":"2.0","id":"p8","method":"session/request_permission","params":{"sessionId":"s1","toolCall":{"toolCallId":"c2","kind":"execute","title":"ls"},"options":[{"optionId":"a1","name":"Allow","kind":"allow_once"},{"optionId":"r1","name":"Deny","kind":"reject_once"}]}}"#);
        let r = d
            .respond("acp-p8", ApprovalDecision::AcceptForSession)
            .unwrap();
        assert_eq!(sent(&r, 0)["result"]["outcome"]["optionId"], "a1");
    }

    /// T3 Code PR #34526 ("only approved execute requests may open client
    /// terminals") does not apply: Otto advertises no `terminal` capability
    /// and refuses every `terminal/*` call, unapproved or approved alike, so
    /// an agent can never make the client run a command.
    #[test]
    fn terminal_requests_are_refused_whether_or_not_the_execute_was_approved() {
        let mut d = AcpDriver::gemini("gemini");
        let init = d.start(&ctx(None));
        assert_eq!(
            sent(&init, 0)["params"]["clientCapabilities"]["terminal"],
            false
        );
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#);
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let create = r#"{"jsonrpc":"2.0","id":"t9","method":"terminal/create","params":{"sessionId":"s1","command":"rm","args":["-rf","/w"]}}"#;
        // Before any approval: refused, nothing reaches the runtime.
        let s = d.on_line(create);
        assert_eq!(sent(&s, 0)["error"]["code"], -32601);
        assert!(s.events.is_empty(), "{s:?}");
        // After a person approved an execute request: still refused.
        d.on_line(r#"{"jsonrpc":"2.0","id":"p7","method":"session/request_permission","params":{"sessionId":"s1","toolCall":{"toolCallId":"c1","kind":"execute","title":"rm"},"options":[{"optionId":"a1","name":"Allow","kind":"allow_once"},{"optionId":"r1","name":"Deny","kind":"reject_once"}]}}"#);
        d.respond("acp-p7", ApprovalDecision::Accept).unwrap();
        let s = d.on_line(create);
        assert_eq!(sent(&s, 0)["error"]["code"], -32601);
        assert!(s.events.is_empty(), "{s:?}");
        for m in ["terminal/output", "terminal/kill", "terminal/release"] {
            let s = d.on_line(&format!(
                r#"{{"jsonrpc":"2.0","id":"x","method":"{m}","params":{{"sessionId":"s1","terminalId":"t"}}}}"#
            ));
            assert_eq!(sent(&s, 0)["error"]["code"], -32601, "{m}");
        }
    }

    #[test]
    fn a_multi_file_edit_renders_one_patch() {
        let patch = multi_file_patch(&[
            ("/w/a.rs".into(), Some("x".into()), "y".into()),
            ("/w/b.rs".into(), None, "new".into()),
        ]);
        assert!(patch.contains(
            "diff --git a/w/a.rs b/w/a.rs\n--- a/w/a.rs\n+++ b/w/a.rs\n@@ -1,1 +1,1 @@\n-x\n+y\n"
        ));
        assert!(patch.contains(
            "diff --git a/w/b.rs b/w/b.rs\n--- /dev/null\n+++ b/w/b.rs\n@@ -0,0 +1,1 @@\n+new\n"
        ));
    }

    #[test]
    fn chunks_accumulate_and_unsupported_client_methods_are_refused() {
        let mut d = AcpDriver::gemini("gemini");
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#);
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let chunk = |t: &str| {
            format!(
                r#"{{"jsonrpc":"2.0","method":"session/update","params":{{"sessionId":"s1","update":{{"sessionUpdate":"agent_message_chunk","content":{{"type":"text","text":"{t}"}}}}}}}}"#
            )
        };
        d.on_line(&chunk("Hel"));
        let s = d.on_line(&chunk("lo"));
        let DriverEvent::TurnItem(item) = &s.events[0] else {
            panic!()
        };
        assert!(
            matches!(&item.kind, TurnItemKind::AssistantMessage { text, streaming: true, .. } if text == "Hello")
        );
        let fs = d.on_line(r#"{"jsonrpc":"2.0","id":9,"method":"fs/read_text_file","params":{}}"#);
        assert_eq!(sent(&fs, 0)["error"]["code"], -32601);
        let end = d.on_line(r#"{"jsonrpc":"2.0","id":3,"result":{"stopReason":"cancelled"}}"#);
        assert!(matches!(&end.events[0], DriverEvent::TurnItem(i) if !i.is_streaming()));
        assert!(matches!(
            end.events.last(),
            Some(DriverEvent::Terminal {
                status: TurnTerminalStatus::Interrupted,
                ..
            })
        ));
    }

    /// T3 Code #37335: Grok steers through `_x.ai/interject` (text and
    /// images); anything but `queued` is reported; other ACP agents refuse.
    #[test]
    fn grok_steers_into_the_running_prompt_through_interject() {
        let mut g = AcpDriver::gemini("gemini");
        assert!(!g.capabilities().steer);
        assert!(g.steer(&RuntimePromptReq::of("x")).is_err());

        let mut d = AcpDriver::grok();
        assert!(d.capabilities().steer);
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#);
        assert!(
            d.steer(&RuntimePromptReq::of("early")).is_err(),
            "no prompt yet"
        );
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let mut req = RuntimePromptReq::of("Look at this");
        req.images.push(otto_core::agent_runtime::PromptImage {
            mime: "image/png".into(),
            data_base64: "aGk=".into(),
            name: None,
        });
        let s = d.steer(&req).unwrap();
        let w = sent(&s, 0);
        assert_eq!(w["method"], "_x.ai/interject");
        assert_eq!(w["params"]["sessionId"], "s1");
        assert_eq!(w["params"]["text"], "Look at this");
        assert_eq!(w["params"]["content"][0]["mimeType"], "image/png");
        let ok = d.on_line(
            &json!({"jsonrpc":"2.0","id":w["id"],"result":{"status":"queued"}}).to_string(),
        );
        assert!(ok.events.is_empty());
        let w = sent(&d.steer(&RuntimePromptReq::of("Late")).unwrap(), 0);
        assert!(w["params"].get("content").is_none());
        let no = d.on_line(
            &json!({"jsonrpc":"2.0","id":w["id"],"result":{"status":"rejected"}}).to_string(),
        );
        assert!(matches!(&no.events[0], DriverEvent::Error(f) if f.message.contains("mid-turn")));
    }

    /// The answer after a steer is a new segment: it is not appended to the
    /// text before the steer (which would sort it above the steer bubble).
    #[test]
    fn a_steer_closes_the_open_text_segment() {
        let mut d = AcpDriver::grok();
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#);
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let chunk = |t: &str| {
            json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1",
                "update":{"sessionUpdate":"agent_message_chunk","content":{"type":"text","text":t}}}})
            .to_string()
        };
        let item = |s: &DriverStep| {
            s.events.iter().rev().find_map(|e| match e {
                DriverEvent::TurnItem(i) => Some(i.clone()),
                _ => None,
            })
        };
        let before = item(&d.on_line(&chunk("Before."))).unwrap();
        let s = d.steer(&RuntimePromptReq::of("now this")).unwrap();
        let closed = item(&s).expect("the steer settles the open segment");
        assert_eq!(closed.id, before.id);
        assert!(!closed.is_streaming());
        let after = item(&d.on_line(&chunk("After."))).unwrap();
        assert_ne!(after.id, before.id, "a new segment after the steer");
        assert!(after.started_at >= closed.started_at);
        assert!(
            matches!(&after.kind, TurnItemKind::AssistantMessage { text, .. } if text == "After.")
        );
    }

    /// T3 Code #28797: a refusal or a limit stop fails the turn with a
    /// visible reason; `end_turn` still completes.
    #[test]
    fn refusal_and_limit_stops_fail_the_turn_with_a_reason() {
        for (stop, failed) in [
            ("refusal", true),
            ("max_tokens", true),
            ("max_turn_requests", true),
            ("end_turn", false),
        ] {
            let mut d = AcpDriver::gemini("gemini");
            d.start(&ctx(None));
            d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
            d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#);
            d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
            let end = d.on_line(&format!(
                r#"{{"jsonrpc":"2.0","id":3,"result":{{"stopReason":"{stop}"}}}}"#
            ));
            let Some(DriverEvent::Terminal {
                status,
                failure,
                reason,
            }) = end.events.last()
            else {
                panic!("{stop}: no terminal")
            };
            if failed {
                assert_eq!(*status, TurnTerminalStatus::Failed, "{stop}");
                let f = failure.as_ref().expect(stop);
                assert_eq!(f.code.as_deref(), Some(stop));
                assert!(!f.message.is_empty());
                assert_eq!(reason.as_deref(), Some(stop));
            } else {
                assert_eq!(*status, TurnTerminalStatus::Completed);
                assert!(failure.is_none() && reason.is_none());
            }
        }
    }

    #[test]
    fn access_modes_pick_t3_argv_and_gemini_switches_live_when_advertised() {
        let at = |access_mode| LaunchContext {
            mcp_servers: Vec::new(),
            access_mode,
            ..ctx(None)
        };
        let g = AcpDriver::grok();
        let full = g.launch(&at(RuntimeAccessMode::FullAccess));
        assert!(full.args.is_empty());
        assert_eq!(full.tail_args, ["agent", "--always-approve", "stdio"]);
        // grok 1.0.92's own accept-edits mode.
        let edits = g.launch(&at(RuntimeAccessMode::AutoAcceptEdits));
        assert_eq!(edits.args, ["--permission-mode", "acceptEdits"]);
        assert_eq!(edits.tail_args, ["agent", "stdio"]);
        assert!(!g.switches_access_live(RuntimeAccessMode::FullAccess));

        let mut d = AcpDriver::gemini("gemini");
        let spec = d.launch(&at(RuntimeAccessMode::AutoAcceptEdits));
        assert_eq!(spec.args, ["--acp", "--approval-mode", "auto_edit"]);
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1","modes":{"currentModeId":"default","availableModes":[{"id":"default","name":"D"},{"id":"autoEdit","name":"E"}]}}}"#);
        assert!(d.switches_access_live(RuntimeAccessMode::AutoAcceptEdits));
        assert!(!d.switches_access_live(RuntimeAccessMode::FullAccess));
        let req = RuntimePromptReq {
            access_mode: Some(RuntimeAccessMode::AutoAcceptEdits),
            ..RuntimePromptReq::of("go")
        };
        let s = d.prompt("t1", &req).unwrap();
        assert_eq!(sent(&s, 0)["method"], "session/set_mode");
        assert_eq!(sent(&s, 0)["params"]["modeId"], "autoEdit");
        assert_eq!(sent(&s, 1)["method"], "session/prompt");
        let e =
            d.on_line(r#"{"jsonrpc":"2.0","id":3,"error":{"code":-32602,"message":"bad mode"}}"#);
        assert!(matches!(&e.events[0], DriverEvent::Error(f) if f.message == "bad mode"));
    }

    #[test]
    fn pdfs_need_embedded_context_and_failed_tools_carry_their_text() {
        let pdf = RuntimePromptReq {
            files: vec![otto_core::agent_runtime::PromptFile {
                path: None,
                mime: "application/pdf".into(),
                name: "a.pdf".into(),
                data_base64: "JVBERg==".into(),
            }],
            ..RuntimePromptReq::of("read")
        };
        for embedded in [false, true] {
            let mut d = AcpDriver::gemini("gemini");
            d.start(&ctx(None));
            let caps = json!({"protocolVersion":1,"agentCapabilities":{"promptCapabilities":{"embeddedContext":embedded}}});
            d.on_line(&json!({"jsonrpc":"2.0","id":1,"result":caps}).to_string());
            d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#);
            let Ok(s) = d.prompt("t1", &pdf) else {
                assert!(!embedded);
                continue;
            };
            assert!(embedded);
            let block = &sent(&s, 0)["params"]["prompt"][0];
            assert_eq!(block["type"], "resource");
            assert_eq!(block["resource"]["blob"], "JVBERg==");
            assert_eq!(block["resource"]["mimeType"], "application/pdf");
            // A permission request names its tool's item; a failed update
            // carries its content as the error.
            d.on_line(r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"tool_call","toolCallId":"c1","title":"fetch","kind":"fetch","status":"pending"}}}"#);
            let r = d.on_line(r#"{"jsonrpc":"2.0","id":"p1","method":"session/request_permission","params":{"sessionId":"s1","toolCall":{"toolCallId":"c1"},"options":[]}}"#);
            assert!(
                matches!(&r.events[0], DriverEvent::Request { tool_item_id: Some(t), .. } if t == "t1:tool:c1")
            );
            let f = d.on_line(r#"{"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{"sessionUpdate":"tool_call_update","toolCallId":"c1","status":"failed","content":[{"type":"content","content":{"type":"text","text":"403 Forbidden"}}]}}}"#);
            let DriverEvent::TurnItem(i) = &f.events[0] else {
                panic!()
            };
            assert_eq!(
                (i.status, i.error.as_deref()),
                (TurnItemStatus::Failed, Some("403 Forbidden"))
            );
        }
    }
    #[test]
    fn form_elicitation_preserves_values_and_rejects_wrong_session() {
        use otto_core::agent_runtime::QuestionAnswer;
        let mut d = AcpDriver::gemini("gemini");
        d.state.session_id = Some("s1".into());
        let form = json!({"jsonrpc":"2.0","id":7,"method":"elicitation/create","params":{"sessionId":"s1","mode":"form","message":"Choose","requestedSchema":{"type":"object","properties":{"strategy":{"type":"string","oneOf":[{"const":"safe_v1","title":"Safe"}]}},"required":["strategy"]}}});
        let s = d.on_line(&form.to_string());
        let DriverEvent::Question { id, .. } = &s.events[0] else {
            panic!("question event missing")
        };
        let response = d
            .respond_question(
                id,
                &[QuestionAnswer::Single {
                    question_id: "strategy".into(),
                    value: "safe_v1".into(),
                }],
            )
            .unwrap();
        assert_eq!(
            sent(&response, 0),
            json!({"jsonrpc":"2.0","id":7,"result":{"action":"accept","content":{"strategy":"safe_v1"}}})
        );
        let mut invalid = form;
        invalid["params"]["sessionId"] = json!("foreign");
        assert_eq!(
            sent(&d.on_line(&invalid.to_string()), 0)["error"]["code"],
            -32602
        );
    }
    /// T3 Code #34867: an optional choice with an `<id>__other` text field
    /// is one question; typed text answers the companion, a pick the choice.
    #[test]
    fn a_choice_with_an_other_field_is_one_question_answered_either_way() {
        use otto_core::agent_runtime::QuestionAnswer;
        let mut d = AcpDriver::gemini("gemini");
        d.state.session_id = Some("s1".into());
        let form = |id: u64| {
            json!({"jsonrpc":"2.0","id":id,"method":"elicitation/create","params":{"sessionId":"s1","mode":"form","message":"Pick a database","requestedSchema":{"type":"object","properties":{
                "q0":{"type":"string","description":"Which database?","enum":["Postgres","MySQL"]},
                "q0__other":{"type":"string"}
            }}}})
            .to_string()
        };
        let s = d.on_line(&form(1));
        let DriverEvent::Question { id, questions, .. } = &s.events[0] else {
            panic!("question event missing")
        };
        assert_eq!(questions.len(), 1, "{questions:?}");
        assert!(questions[0].allow_free_text);
        let typed = d
            .respond_question(
                id,
                &[QuestionAnswer::Text {
                    question_id: "q0".into(),
                    text: "SQLite".into(),
                }],
            )
            .unwrap();
        assert_eq!(
            sent(&typed, 0)["result"]["content"],
            json!({"q0__other":"SQLite"})
        );
        let s = d.on_line(&form(2));
        let DriverEvent::Question { id, .. } = &s.events[0] else {
            panic!("question event missing")
        };
        let picked = d
            .respond_question(
                id,
                &[QuestionAnswer::Single {
                    question_id: "q0".into(),
                    value: "MySQL".into(),
                }],
            )
            .unwrap();
        assert_eq!(sent(&picked, 0)["result"]["content"], json!({"q0":"MySQL"}));
    }
    #[test]
    fn required_unconstrained_acp_fields_keep_empty_values() {
        use otto_core::agent_runtime::QuestionAnswer;
        let mut d = AcpDriver::gemini("gemini");
        d.state.session_id = Some("s1".into());
        let step = d.on_line(&json!({"jsonrpc":"2.0","id":"empty-form","method":"elicitation/create","params":{"sessionId":"s1","mode":"form","requestedSchema":{"type":"object","properties":{"tags":{"type":"array","items":{"type":"string","enum":["api"]}},"name":{"type":"string"}},"required":["tags","name"]}}}).to_string());
        let DriverEvent::Question { id, .. } = &step.events[0] else {
            panic!("missing question")
        };
        let response = d
            .respond_question(
                id,
                &[
                    QuestionAnswer::Multi {
                        question_id: "tags".into(),
                        values: vec![],
                    },
                    QuestionAnswer::Text {
                        question_id: "name".into(),
                        text: "".into(),
                    },
                ],
            )
            .unwrap();
        assert_eq!(
            sent(&response, 0)["result"]["content"],
            json!({"tags":[],"name":""})
        );
    }

    /// T3 Code #34782: a model write lost to anything but a refusal is
    /// written again before the next prompt; a refused value is not.
    #[test]
    fn a_model_config_write_that_failed_is_retried_before_the_next_prompt() {
        let mut d = AcpDriver::opencode();
        let c = LaunchContext {
            model: Some("anthropic/claude-sonnet".into()),
            ..ctx(None)
        };
        d.start(&c);
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        let s = d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"ses_1"}}"#);
        let set_id = sent(&s, 0)["id"].clone();
        let lost =
            json!({"jsonrpc":"2.0","id":set_id,"error":{"code":-32603,"message":"internal error"}});
        let s = d.on_line(&lost.to_string());
        assert!(matches!(&s.events[0], DriverEvent::Error(_)));
        let s = d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let again = sent(&s, 0);
        assert_eq!(again["method"], "session/set_config_option");
        assert_eq!(again["params"]["configId"], "model");
        assert_eq!(again["params"]["value"], "anthropic/claude-sonnet");
        let prompt = sent(&s, 1);
        assert_eq!(prompt["method"], "session/prompt");
        // A refusal (-32602) is final: the next prompt goes alone.
        let refused = json!({"jsonrpc":"2.0","id":again["id"],"error":{"code":-32602,"message":"unknown model"}});
        let s = d.on_line(&refused.to_string());
        // …and says so, so the runtime puts the thread back on its model.
        assert!(matches!(&s.events[0], DriverEvent::Error(f)
            if f.code.as_deref() == Some(otto_core::agent_runtime::MODEL_SWITCH_REFUSED)));
        let done = json!({"jsonrpc":"2.0","id":prompt["id"],"result":{"stopReason":"end_turn"}});
        d.on_line(&done.to_string());
        let s = d.prompt("t2", &RuntimePromptReq::of("again")).unwrap();
        assert_eq!(s.writes.len(), 1);
        assert_eq!(sent(&s, 0)["method"], "session/prompt");
    }

    /// T3 Code #33604: a named message that resumes after a thought keeps
    /// the text streamed before it, as one item in its first place.
    #[test]
    fn a_resumed_message_keeps_its_streamed_text() {
        let mut d = AcpDriver::gemini("gemini");
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#);
        d.prompt("t1", &RuntimePromptReq::of("go")).unwrap();
        let update = |kind: &str, text: &str, mid: Option<&str>| {
            let mut u = json!({"sessionUpdate":kind,"content":{"type":"text","text":text}});
            if let Some(m) = mid {
                u["messageId"] = json!(m);
            }
            json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":u}})
                .to_string()
        };
        let last_item = |s: &DriverStep| {
            s.events
                .iter()
                .rev()
                .find_map(|e| match e {
                    DriverEvent::TurnItem(i) => Some(i.clone()),
                    _ => None,
                })
                .expect("an item")
        };
        let first = last_item(&d.on_line(&update("agent_message_chunk", "First. ", Some("a"))));
        d.on_line(&update("agent_thought_chunk", "Thinking", None));
        let s = d.on_line(&update("agent_message_chunk", "Second.", Some("a")));
        let resumed = last_item(&s);
        assert_eq!(resumed.id, first.id, "one item, in its first place");
        assert!(
            matches!(&resumed.kind, TurnItemKind::AssistantMessage { text, message_id, .. }
                if text == "First. Second." && message_id == "a"),
            "{:?}",
            resumed.kind
        );
        // Interleaved named messages a, b, a: each keeps its own text.
        let b = last_item(&d.on_line(&update("agent_message_chunk", "B", Some("b"))));
        assert_ne!(b.id, first.id);
        let a = last_item(&d.on_line(&update("agent_message_chunk", " Third.", Some("a"))));
        assert_eq!(a.id, first.id);
        assert!(
            matches!(&a.kind, TurnItemKind::AssistantMessage { text, .. } if text == "First. Second. Third.")
        );
        // Anonymous (ACP v1) chunks still split at a thought.
        let x = last_item(&d.on_line(&update("agent_message_chunk", "x", None)));
        d.on_line(&update("agent_thought_chunk", "hm", None));
        let y = last_item(&d.on_line(&update("agent_message_chunk", "y", None)));
        assert_ne!(x.id, y.id);
    }

    #[test]
    fn autonomous_wakeup_turns_track_background_tasks_and_idle_settle_cleanly() {
        let mut d = AcpDriver::agy("agy");
        d.start(&ctx(None));
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s1"}}"#);

        // Turn 1 schedules a timer + spawns a background command, then finishes.
        let p1 = d.prompt("t1", &RuntimePromptReq::of("monitor")).unwrap();
        let pid = sent(&p1, 0)["id"].clone();
        d.on_line(
            &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{
                "sessionUpdate":"tool_call","toolCallId":"sc1","title":"schedule","kind":"other","status":"pending",
                "rawInput":{"DurationSeconds":30,"Prompt":"check deploy"}
            }}})
            .to_string(),
        );
        let bg = d.on_line(
            &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{
                "sessionUpdate":"tool_call_update","toolCallId":"sc1","status":"completed"
            }}})
            .to_string(),
        );
        assert!(bg.events.iter().any(|e| matches!(
            e,
            DriverEvent::BackgroundTasks(tasks)
                if tasks.len() == 1 && tasks[0].kind == "timer" && tasks[0].label.contains("check deploy")
        )));
        d.on_line(
            &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{
                "sessionUpdate":"tool_call","toolCallId":"cmd1","title":"run_command","kind":"execute","status":"pending",
                "rawInput":{"CommandLine":"npm test"}
            }}})
            .to_string(),
        );
        let bg2 = d.on_line(
            &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{
                "sessionUpdate":"tool_call_update","toolCallId":"cmd1","status":"completed",
                "content":[{"type":"content","content":{"type":"text","text":"Command is running in background task with task id: task_42."}}]
            }}})
            .to_string(),
        );
        assert!(bg2.events.iter().any(|e| matches!(
            e,
            DriverEvent::BackgroundTasks(tasks) if tasks.len() == 2 && tasks.iter().any(|t| t.id == "task_42")
        )));
        d.on_line(
            &json!({"jsonrpc":"2.0","id":pid,"result":{"stopReason":"end_turn"}}).to_string(),
        );
        assert!(!d.needs_idle_turn_settle());

        // Autonomous wakeup #1 arrives with no session/prompt RPC.
        let w1 = d.on_line(
            &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{
                "sessionUpdate":"agent_thought_chunk","content":{"type":"text","text":"Timer fired"}
            }}})
            .to_string(),
        );
        assert!(w1
            .events
            .iter()
            .any(|e| matches!(e, DriverEvent::BackgroundTasks(tasks) if tasks.is_empty())));
        assert!(w1
            .events
            .iter()
            .any(|e| matches!(e, DriverEvent::TurnStarted { turn_id } if turn_id == "auto-1")));
        assert!(d.needs_idle_turn_settle());

        // While a tool call is pending in the autonomous turn, do NOT idle-settle early.
        d.on_line(
            &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{
                "sessionUpdate":"tool_call","toolCallId":"t_auto","title":"ls","kind":"execute","status":"in_progress"
            }}})
            .to_string(),
        );
        assert!(!d.needs_idle_turn_settle());
        d.on_line(
            &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{
                "sessionUpdate":"tool_call_update","toolCallId":"t_auto","status":"completed"
            }}})
            .to_string(),
        );
        // Raw ACP v2 message variant + unrecognized experimental variant are handled safely.
        let msg = d.on_line(
            &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{
                "sessionUpdate":"agent_message","content":[{"type":"text","text":"Deploy healthy!"}]
            }}})
            .to_string(),
        );
        assert!(msg.events.iter().any(|e| matches!(
            e,
            DriverEvent::TurnItem(i)
                if i.id.starts_with("auto-1:")
                    && matches!(&i.kind, TurnItemKind::AssistantMessage { text, streaming: true, .. } if text == "Deploy healthy!")
        )));
        assert!(d
            .on_line(
                &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{
                    "sessionUpdate":"experimental_heartbeat","seq":1
                }}})
                .to_string(),
            )
            .events
            .is_empty());

        assert!(d.needs_idle_turn_settle());
        let settled = d.on_idle_settle();
        assert!(settled.events.iter().any(|e| matches!(
            e,
            DriverEvent::TurnItem(i)
                if matches!(&i.kind, TurnItemKind::AssistantMessage { text, streaming: false, .. } if text == "Deploy healthy!")
        )));
        assert!(settled.events.iter().any(|e| matches!(
            e,
            DriverEvent::Terminal {
                status: TurnTerminalStatus::Completed,
                ..
            }
        )));
        assert!(!d.needs_idle_turn_settle());

        // Second autonomous wakeup 20 minutes later opens auto-2 cleanly.
        let w2 = d.on_line(
            &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s1","update":{
                "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Wakeup 2"}
            }}})
            .to_string(),
        );
        assert!(w2
            .events
            .iter()
            .any(|e| matches!(e, DriverEvent::TurnStarted { turn_id } if turn_id == "auto-2")));
    }

    #[test]
    fn agy_switches_model_live_and_clamps_thought_level_effort_to_advertised_options() {
        let mut d = AcpDriver::agy("agy");
        assert!(d.switches_model_live());
        assert!(!AcpDriver::gemini("gemini").switches_model_live());
        assert!(!AcpDriver::grok().switches_model_live());
        assert!(AcpDriver::opencode().switches_model_live());

        let c = LaunchContext {
            mcp_servers: Vec::new(),
            session_id: "s".into(),
            cwd: "/w".into(),
            model: Some("gemini-4-argon-medium".into()),
            resume: None,
            sandboxed: false,
            access_mode: RuntimeAccessMode::ApprovalRequired,
        };
        d.start(&c);
        d.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d.on_line(
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": {
                    "sessionId": "s1",
                    "modes": {"currentModeId": "default", "availableModes": [{"id": "default"}]},
                    "configOptions": [{
                        "id": "thought_level",
                        "options": [{"value": "low"}, {"value": "medium"}, {"value": "high"}]
                    }]
                }
            })
            .to_string(),
        );

        let req = RuntimePromptReq {
            model: Some("gemini-4-argon-high".into()),
            effort: Some("max".into()),
            ..RuntimePromptReq::of("think")
        };
        let s = d.prompt("t1", &req).unwrap();
        assert_eq!(s.writes.len(), 3);
        assert_eq!(sent(&s, 0)["method"], "session/set_model");
        assert_eq!(sent(&s, 0)["params"]["modelId"], "gemini-4-argon-high");
        assert_eq!(sent(&s, 1)["method"], "session/set_config_option");
        assert_eq!(sent(&s, 1)["params"]["configId"], "thought_level");
        assert_eq!(sent(&s, 1)["params"]["value"], "high");
        assert_eq!(sent(&s, 2)["method"], "session/prompt");

        // Switching to a unified family (`gemini-4-hydrogen`) that advertises
        // `xhigh` in `models.availableModels` updates `effort_options` before
        // resolving `thought_level`, so `xhigh` is preserved without clamping
        // to the previous family's `high` ceiling.
        let mut d2 = AcpDriver::agy("agy");
        d2.start(&ctx(Some("gemini-4-carbon")));
        d2.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d2.on_line(
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": {
                    "sessionId": "s2",
                    "models": {"availableModels": [
                        {"modelId": "gemini-4-carbon-low"},
                        {"modelId": "gemini-4-carbon-medium"},
                        {"modelId": "gemini-4-carbon-high"},
                        {"modelId": "gemini-4-hydrogen-low"},
                        {"modelId": "gemini-4-hydrogen-medium"},
                        {"modelId": "gemini-4-hydrogen-high"},
                        {"modelId": "gemini-4-hydrogen-xhigh"},
                        {"modelId": "gemini-4-flash-minimal"},
                        {"modelId": "gemini-4-flash-ultra"},
                        {"modelId": "gemini-pro-orchestrator"}
                    ]},
                    "configOptions": [{
                        "id": "thought_level",
                        "options": [{"value": "low"}, {"value": "medium"}, {"value": "high"}]
                    }]
                }
            })
            .to_string(),
        );
        let s2 = d2
            .prompt(
                "t2",
                &RuntimePromptReq {
                    model: Some("gemini-4-hydrogen".into()),
                    effort: Some("xhigh".into()),
                    ..RuntimePromptReq::of("deep")
                },
            )
            .unwrap();
        assert_eq!(sent(&s2, 0)["method"], "session/set_model");
        assert_eq!(sent(&s2, 0)["params"]["modelId"], "gemini-4-hydrogen");
        assert_eq!(sent(&s2, 1)["method"], "session/set_config_option");
        assert_eq!(sent(&s2, 1)["params"]["value"], "xhigh");
        let p2_id = sent(&s2, 2)["id"].clone();
        d2.on_line(
            &json!({"jsonrpc":"2.0","id":p2_id,"result":{"stopReason":"end_turn"}}).to_string(),
        );

        // Switching to an untiered model (`gemini-pro-orchestrator`) clears
        // stale `xhigh` options and omits unsupported `thought_level` writes.
        let s3 = d2
            .prompt(
                "t3",
                &RuntimePromptReq {
                    model: Some("gemini-pro-orchestrator".into()),
                    effort: Some("xhigh".into()),
                    ..RuntimePromptReq::of("orchestrate")
                },
            )
            .unwrap();
        assert_eq!(s3.writes.len(), 2);
        assert_eq!(sent(&s3, 0)["method"], "session/set_model");
        assert_eq!(sent(&s3, 0)["params"]["modelId"], "gemini-pro-orchestrator");
        assert_eq!(sent(&s3, 1)["method"], "session/prompt");
        let p3_id = sent(&s3, 1)["id"].clone();
        d2.on_line(
            &json!({"jsonrpc":"2.0","id":p3_id,"result":{"stopReason":"end_turn"}}).to_string(),
        );

        // Switching to `-minimal` / `-ultra` variants maps to canonical `low` / `max`,
        // and launching directly with a unified model (`LaunchContext.model`) resolves
        // its family effort options during `session/new`.
        let mut d3 = AcpDriver::agy("agy");
        d3.start(&ctx(Some("gemini-4-flash")));
        d3.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1}}"#);
        d3.on_line(
            &json!({
                "jsonrpc": "2.0",
                "id": 2,
                "result": {
                    "sessionId": "s3",
                    "models": {"availableModels": [
                        {"modelId": "gemini-4-flash-minimal"},
                        {"modelId": "gemini-4-flash-ultra"}
                    ]}
                }
            })
            .to_string(),
        );
        let s4 = d3
            .prompt(
                "t4",
                &RuntimePromptReq {
                    effort: Some("ultra".into()),
                    ..RuntimePromptReq::of("fast-max")
                },
            )
            .unwrap();
        assert_eq!(sent(&s4, 0)["method"], "session/set_config_option");
        assert_eq!(sent(&s4, 0)["params"]["value"], "max");
    }

    #[test]
    fn snapshot_and_restore_preserve_cancel_barrier_and_drop_stale_pre_cancel_deltas() {
        let mut d1 = AcpDriver::agy("agy");
        d1.start(&ctx(None));
        d1.on_line(r#"{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":1,"agentCapabilities":{"promptCapabilities":{"image":true}}}}"#);
        d1.on_line(r#"{"jsonrpc":"2.0","id":2,"result":{"sessionId":"s-cancel"}}"#);

        let p1 = d1.prompt("t1", &RuntimePromptReq::of("work")).unwrap();
        let prompt_id = sent(&p1, 0)["id"].clone();
        d1.on_line(
            &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s-cancel","update":{
                "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Before cancel"}
            }}})
            .to_string(),
        );
        let cancel_step = d1.interrupt();
        assert_eq!(sent(&cancel_step, 0)["method"], "session/cancel");

        // Snapshot mid-cancel and restore into a fresh driver instance.
        let snap = d1.snapshot_state();
        let mut d2 = AcpDriver::agy("agy");
        d2.restore_state(&snap).unwrap();

        // Stale pre-cancel delta and late permission request arriving before the
        // cancelled prompt response must be dropped/cancelled cleanly.
        let stale = d2.on_line(
            &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s-cancel","update":{
                "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":" STALE"}
            }}})
            .to_string(),
        );
        assert!(
            stale.events.is_empty(),
            "stale pre-cancel delta must be dropped"
        );

        let late_perm = d2.on_line(
            r#"{"jsonrpc":"2.0","id":"p-late","method":"session/request_permission","params":{"sessionId":"s-cancel","toolCall":{"toolCallId":"c1","kind":"execute","title":"ls"},"options":[]}}"#,
        );
        assert!(late_perm.events.is_empty());
        assert_eq!(
            sent(&late_perm, 0)["result"]["outcome"]["outcome"],
            "cancelled"
        );

        // Cancelled prompt settles as Interrupted even if agent returns end_turn.
        let done = d2.on_line(
            &json!({"jsonrpc":"2.0","id":prompt_id,"result":{"stopReason":"end_turn"}}).to_string(),
        );
        assert!(done.events.iter().any(|e| matches!(
            e,
            DriverEvent::Terminal {
                status: TurnTerminalStatus::Interrupted,
                ..
            }
        )));

        // Next prompt starts cleanly with preserved image capability and fresh text.
        let p2 = d2
            .prompt(
                "t2",
                &RuntimePromptReq {
                    images: vec![otto_core::agent_runtime::PromptImage {
                        mime: "image/png".into(),
                        data_base64: "AAAA".into(),
                        name: None,
                    }],
                    ..RuntimePromptReq::of("steered follow-up")
                },
            )
            .unwrap();
        let next_id = sent(&p2, 0)["id"].clone();
        let fresh = d2.on_line(
            &json!({"jsonrpc":"2.0","method":"session/update","params":{"sessionId":"s-cancel","update":{
                "sessionUpdate":"agent_message_chunk","content":{"type":"text","text":"Fresh reply"}
            }}})
            .to_string(),
        );
        assert!(fresh.events.iter().any(|e| matches!(
            e,
            DriverEvent::TurnItem(i)
                if matches!(&i.kind, TurnItemKind::AssistantMessage { text, streaming: true, .. } if text == "Fresh reply")
        )));
        let fin2 = d2.on_line(
            &json!({"jsonrpc":"2.0","id":next_id,"result":{"stopReason":"end_turn"}}).to_string(),
        );
        assert!(fin2.events.iter().any(|e| matches!(
            e,
            DriverEvent::Terminal {
                status: TurnTerminalStatus::Completed,
                ..
            }
        )));
    }
}
