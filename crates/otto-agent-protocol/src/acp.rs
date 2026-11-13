//! ACP (Agent Client Protocol, v1) client driver — used for Gemini
//! (`gemini --acp`), Grok (`grok … agent stdio`) and by the tests' fake ACP
//! agent. Per-agent differences live in an [`AcpFlavor`] (T3 Code: the
//! `AcpFlavor` hooks of `packages/provider-acp/src/server/adapter.ts`, Grok's
//! in `packages/provider-grok/src/server/acpSupport.ts`).
//!
//! Wire types are the official SDK's (`agent-client-protocol` 5.0.0, schema
//! 1.15.1); this module only does JSON-RPC framing + request correlation,
//! because the runtime — not the SDK's connection — owns the process (every
//! spawn passes Otto's safety pipeline) and the driver stays sans-IO.
//!
//! The lifecycle and the update → entity mapping are ported from T3 Code
//! (MIT, Copyright (c) 2026 T3 Tools Inc.):
//! * `packages/provider-acp/src/server/AcpSessionRuntime.ts` — `initialize`
//!   with no fs / terminal client capabilities (Otto offers neither, like T3
//!   Code for Claude/Gemini), then `session/load` when resuming and the agent
//!   advertises `loadSession`, else `session/new`; a failed load falls back
//!   to a fresh session; `session/cancel` on interrupt. `authenticate` is
//!   lazy: only after `session/new|load` fails with `-32000` (auth
//!   required), with the flavour's preferred method if advertised, else the
//!   first `agent`-type method, then the session call is retried once
//!   (`AcpSessionRuntime.ts:1065-1078`, `:2164-2208`).
//! * `packages/provider-acp/src/server/runtimeModel.ts` + `adapter.ts` —
//!   `agent_message_chunk` / `agent_thought_chunk` accumulate into one
//!   `assistant_message` / `reasoning` item re-emitted whole with
//!   `streaming: true` (a non-text update closes the segment); `tool_call` /
//!   `tool_call_update` merge into one `command_execution` / `file_change` /
//!   `dynamic_tool` item; `plan` → `plan.updated` + a `todo_list` item;
//!   `session/request_permission` → a pending runtime request + an
//!   `approval_request` item; answers pick `allow_once` for accept,
//!   `allow_always` for accept-for-session (narrowed to `allow_once` when
//!   the agent offers no session grant), `reject_once` for decline, and
//!   `{outcome: cancelled}` for cancel or a missing option
//!   (`adapter.ts:1255-1266`). Updates replayed by `session/load` are history
//!   the agent re-sends; Otto keeps its own, so they are dropped.
//! * access mode: the flavour's launch argv per mode (Grok,
//!   `acpSupport.ts:47-58`; Gemini's `--approval-mode`); Gemini switches live
//!   with `session/set_mode` when `session/new|load` advertised the mode
//!   (`AcpSessionRuntime.ts:2671-2688`), otherwise the runtime respawns and
//!   loads the session. A failed `tool_call_update` carries its content as
//!   the item's `error`. PDFs go as an embedded `resource` blob only when the
//!   agent advertises `promptCapabilities.embeddedContext`.

use std::collections::HashMap;

use agent_client_protocol::schema::ProtocolVersion;
use agent_client_protocol::v1::schema::{
    AuthenticateRequest, BlobResourceContents, CancelNotification, ContentBlock, EmbeddedResource,
    EmbeddedResourceResource, EnvVariable, ImageContent, InitializeRequest, InitializeResponse,
    LoadSessionRequest, McpServer, McpServerStdio, NewSessionRequest, NewSessionResponse,
    PermissionOption, PermissionOptionKind, Plan, PlanEntryStatus, PromptRequest, PromptResponse,
    RequestPermissionOutcome, RequestPermissionRequest, RequestPermissionResponse,
    SelectedPermissionOutcome, SessionNotification, SessionUpdate, SetSessionModeRequest,
    StopReason, TextContent, ToolCall, ToolCallContent, ToolCallStatus, ToolCallUpdate, ToolKind,
};
use otto_core::agent_runtime::{
    ApprovalDecision, ApprovalOption, PlanStep, PlanStepStatus, ProviderFailure, RequestKind,
    RuntimeAccessMode, RuntimeCapabilities, RuntimePromptReq, RuntimeProvider, TurnItem,
    TurnItemKind, TurnItemStatus, TurnTerminalStatus, TurnTokenUsage,
};
use serde_json::{json, Value};

use crate::driver::{
    item, rpc_failure, rpc_key, tool_error, DriverError, DriverEvent, DriverStep, LaunchContext,
    LaunchSpec, McpDelivery, McpServerDef, StructuredDriver,
};

const M_INITIALIZE: &str = "initialize";
const M_AUTHENTICATE: &str = "authenticate";
/// JSON-RPC error ACP agents return from `session/new|load` when the user
/// must authenticate first.
const AUTH_REQUIRED: i64 = -32000;
const M_SESSION_NEW: &str = "session/new";
const M_SET_CONFIG_OPTION: &str = "session/set_config_option";
const M_SESSION_LOAD: &str = "session/load";
const M_SESSION_PROMPT: &str = "session/prompt";
const M_SESSION_CANCEL: &str = "session/cancel";
const M_SESSION_UPDATE: &str = "session/update";
const M_REQUEST_PERMISSION: &str = "session/request_permission";
const M_SESSION_SET_MODE: &str = "session/set_mode";
const M_SESSION_SET_MODEL: &str = "session/set_model";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    Initializing,
    Creating { loading: bool },
    Authenticating { load: bool },
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
    /// Environment per access mode (opencode's `OPENCODE_PERMISSION`).
    pub access_env: Option<AccessEnv>,
    /// When the agent takes its model as a session config option instead of
    /// a launch flag (opencode: `session/set_config_option {configId:
    /// "model"}`), that option's id; `model_flag` is then unused.
    pub model_config_id: Option<&'static str>,
    /// The model goes as `session/set_model {modelId}` once ready (agy:
    /// `gemini-4-argon-high` — the config option would drop the effort).
    pub set_model: bool,
    /// The env var naming an Otto context bundle (`AGENTS.md` +
    /// `.agents/skills`) to send as `_meta.antigravity.systemPrompt.append`
    /// instead of a workspace root (otto-context `AGY_BUNDLE_ENV`).
    pub bundle_env: Option<&'static str>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SegmentKind {
    Message,
    Thought,
}

struct Segment {
    kind: SegmentKind,
    item_id: String,
    message_id: String,
    /// The agent named it (`messageId`): later chunks of that id append.
    named: bool,
    text: String,
}

#[derive(Default)]
struct Tool {
    title: String,
    kind: ToolKind,
    status: ToolCallStatus,
    raw_input: Option<Value>,
    output: String,
    images: Vec<otto_core::agent_runtime::ToolOutputImage>,
    file: Option<String>,
    diff: Option<(Option<String>, String)>,
    /// Every `(path, old, new)` diff of the call — a multi-file edit renders
    /// as one patch (T3 Code `acpToolCallDiffPatch`, ddd36a53a5).
    diffs: Vec<(String, Option<String>, String)>,
}

/// A whole-file unified patch over `diffs`, one `diff --git` section each.
fn multi_file_patch(diffs: &[(String, Option<String>, String)]) -> String {
    let mut out = String::new();
    for (path, old, new) in diffs {
        let p = path.trim_start_matches('/');
        out.push_str(&format!("diff --git a/{p} b/{p}\n"));
        let old_lines: Vec<&str> = old
            .as_deref()
            .map(|o| o.lines().collect())
            .unwrap_or_default();
        let new_lines: Vec<&str> = new.lines().collect();
        if old.is_none() {
            out.push_str("--- /dev/null\n");
        } else {
            out.push_str(&format!("--- a/{p}\n"));
        }
        out.push_str(&format!("+++ b/{p}\n"));
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            usize::from(!old_lines.is_empty()),
            old_lines.len(),
            usize::from(!new_lines.is_empty()),
            new_lines.len()
        ));
        for l in old_lines {
            out.push_str(&format!("-{l}\n"));
        }
        for l in new_lines {
            out.push_str(&format!("+{l}\n"));
        }
    }
    out
}

struct Pending {
    rpc_id: Value,
    options: Vec<PermissionOption>,
}

pub struct AcpDriver {
    provider: RuntimeProvider,
    flavor: AcpFlavor,
    cwd: String,
    resume: Option<String>,
    /// Handed over in `session/new|load` (Otto's `otto` tools + the user's
    /// enabled servers — the set a terminal session gets).
    mcp_servers: Vec<McpServerDef>,
    /// The model to apply through `flavor.model_config_id` / `set_model`
    /// once ready.
    model: Option<String>,
    config_req: Option<u64>,
    /// The model write failed for a reason other than a refusal: written
    /// again before the next prompt (T3 Code #34782).
    config_retry: bool,
    next_id: u64,
    phase: Phase,
    init_id: u64,
    create_id: u64,
    auth_id: u64,
    /// `authMethods` from `initialize`: `(id, type)`.
    auth_methods: Vec<(String, Option<String>)>,
    authenticated: bool,
    session_id: Option<String>,
    /// `promptCapabilities.image` / `.embeddedContext` from `initialize`.
    images: bool,
    embedded: bool,
    /// `modes.availableModes[].id` from `session/new|load`.
    modes: Vec<String>,
    access: RuntimeAccessMode,
    mode_req: Option<u64>,
    /// The bundle text for `_meta.antigravity.systemPrompt.append`.
    system_append: Option<String>,
    queued: Option<(String, RuntimePromptReq)>,
    prompt_id: Option<u64>,
    turn_id: Option<String>,
    segment: Option<Segment>,
    segments: u32,
    /// This turn's closed named segments by `messageId`: a message that
    /// resumes after a thought (or another message) keeps its text and its
    /// place (T3 Code #33604).
    closed_named: HashMap<String, Segment>,
    tools: HashMap<String, Tool>,
    pending: HashMap<String, Pending>,
    questions: HashMap<String, (Value, Vec<otto_core::agent_runtime::RuntimeQuestion>)>,
    /// The session's reasoning-effort config option (agy `thought_level`,
    /// grok `reasoning_effort`): `(configId, values low → high)`.
    effort_option: Option<(String, Vec<String>)>,
    /// The effort value last written to it.
    effort: Option<String>,
    effort_req: Option<u64>,
    /// agy swarm members this session added, by member name.
    swarm: HashMap<String, TurnItem>,
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
            cwd: String::new(),
            resume: None,
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
            access: RuntimeAccessMode::default(),
            mode_req: None,
            system_append: None,
            queued: None,
            prompt_id: None,
            turn_id: None,
            segment: None,
            segments: 0,
            closed_named: HashMap::new(),
            tools: HashMap::new(),
            pending: HashMap::new(),
            questions: HashMap::new(),
            effort_option: None,
            effort: None,
            effort_req: None,
            swarm: HashMap::new(),
        }
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
                bundle_env: Some("OTTO_AGY_CONTEXT_DIR"),
                ..AcpFlavor::default()
            },
        )
    }

    /// Grok over ACP, as T3 Code launches it (`grokAcpSpawnArgs`,
    /// `acpSupport.ts:47-58`): approval-required is `--permission-mode
    /// default agent stdio` (every tool call asks), full-access `agent
    /// --always-approve stdio`. Grok has no auto-accept-edits launch mode
    /// (T3 Code offers none either), so that mode asks like
    /// approval-required. `-m` is grok's top-level model flag; context flags
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
                        _ => (v(&["--permission-mode", "default"]), v(&["agent", "stdio"])),
                    }
                }),
                model_flag: "-m".into(),
                auth_preference: Some(auth.into()),
                initialize_meta: Some(json!({"clientType": "extension"})),
                cancel_meta: Some(json!({"cancelTrigger": "ctrl_c"})),
                ..AcpFlavor::default()
            },
        )
    }

    /// opencode over ACP (`opencode acp`; verified live against 1.19.5): no
    /// model flag — the model is the `model` session config option (value
    /// `provider/model`, the only config option it advertises); it takes
    /// stdio `mcpServers` with their env in `session/new|load`. The access
    /// mode rides `OPENCODE_PERMISSION`, a JSON permission object deep-merged
    /// OVER the user's config (a bare string like `"allow"` breaks
    /// `session/new` with a `directory` service failure): full access allows
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
                    vec![("OPENCODE_PERMISSION".into(), rules.into())]
                }),
                model_config_id: Some("model"),
                ..AcpFlavor::default()
            },
        )
    }

    fn acp_mcp_servers(&self) -> Vec<McpServer> {
        self.mcp_servers
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
        self.next_id += 1;
        let id = self.next_id;
        let line = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string();
        (id, line)
    }

    fn create_session(&mut self, step: &mut DriverStep, load: bool) {
        let line = match (&self.resume, load) {
            (Some(sid), true) => {
                let mut req = LoadSessionRequest::new(sid.clone(), self.cwd.clone())
                    .mcp_servers(self.acp_mcp_servers());
                req.meta = self.session_meta();
                let (id, l) = self.request(M_SESSION_LOAD, req);
                self.create_id = id;
                l
            }
            _ => {
                let mut req =
                    NewSessionRequest::new(self.cwd.clone()).mcp_servers(self.acp_mcp_servers());
                req.meta = self.session_meta();
                let (id, l) = self.request(M_SESSION_NEW, req);
                self.create_id = id;
                l
            }
        };
        self.phase = Phase::Creating { loading: load };
        step.write(line);
    }

    fn session_meta(&self) -> Option<serde_json::Map<String, Value>> {
        let text = self.system_append.as_ref()?;
        let meta = json!({"antigravity": {"systemPrompt": {"append": text}}});
        meta.as_object().cloned()
    }

    /// The model as a session config option (`flavor.model_config_id`) or
    /// `session/set_model` (`flavor.set_model`).
    fn write_model_config(&mut self, step: &mut DriverStep) {
        if let (true, Some(model), Some(sid)) = (
            self.flavor.set_model,
            self.model.clone(),
            self.session_id.clone(),
        ) {
            let (id, line) = self.request(
                M_SESSION_SET_MODEL,
                json!({"sessionId": sid, "modelId": model}),
            );
            self.config_req = Some(id);
            step.write(line);
        } else if let (Some(config_id), Some(model), Some(sid)) = (
            self.flavor.model_config_id,
            self.model.clone(),
            self.session_id.clone(),
        ) {
            let (id, line) = self.request(
                M_SET_CONFIG_OPTION,
                json!({"sessionId": sid, "configId": config_id, "value": model}),
            );
            self.config_req = Some(id);
            step.write(line);
        }
    }

    fn send_prompt(&mut self, step: &mut DriverStep, turn_id: String, req: &RuntimePromptReq) {
        if std::mem::take(&mut self.config_retry) {
            self.write_model_config(step);
        }
        let sid = self.session_id.clone().unwrap_or_default();
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
        if let Some(mode) = req.access_mode.filter(|m| *m != self.access) {
            self.access = mode;
            self.write_mode(step, &sid, mode);
        }
        // The effort as the session's own config option (T3 Code:
        // `antigravityThinkingConfig`, grok's `reasoning_effort`).
        let effort = req.effort.as_deref().and_then(|e| {
            let (config, levels) = self.effort_option.as_ref()?;
            Some((config.clone(), effort_value(levels, e)?))
        });
        if let Some((config, value)) = effort.filter(|(_, v)| self.effort.as_ref() != Some(v)) {
            let (id, line) = self.request(
                M_SET_CONFIG_OPTION,
                json!({"sessionId": sid, "configId": config, "value": value}),
            );
            self.effort_req = Some(id);
            self.effort = Some(value);
            step.write(line);
        }
        let (id, line) = self.request(M_SESSION_PROMPT, PromptRequest::new(sid, blocks));
        self.prompt_id = Some(id);
        self.turn_id = Some(turn_id);
        self.segments = 0;
        self.closed_named.clear();
        step.write(line);
    }

    fn write_mode(&mut self, step: &mut DriverStep, sid: &str, mode: RuntimeAccessMode) {
        if let Some(mode_id) = self.flavor.mode_id.and_then(|f| f(mode)) {
            let (id, line) = self.request(
                M_SESSION_SET_MODE,
                SetSessionModeRequest::new(sid.to_string(), mode_id),
            );
            self.mode_req = Some(id);
            step.write(line);
        }
    }

    /// T3 Code's choice (`AcpSessionRuntime.ts:1065-1074`): a preferred
    /// method the agent advertised, else the first `agent`-type (or untyped)
    /// one. `terminal` / `env_var` methods are never sent to `authenticate`.
    fn auth_method(&self) -> Option<String> {
        let advertised = |id: &str| self.auth_methods.iter().any(|(m, _)| m == id);
        self.flavor
            .auth_preference
            .iter()
            .find(|p| advertised(p))
            .cloned()
            .or_else(|| {
                self.auth_methods
                    .iter()
                    .find(|(_, ty)| ty.as_deref().is_none_or(|t| t == "agent"))
                    .map(|(id, _)| id.clone())
            })
    }

    fn turn(&self) -> String {
        self.turn_id.clone().unwrap_or_default()
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
        item(seg.item_id.clone(), status, None, kind)
    }

    fn close_segment(&mut self, step: &mut DriverStep) {
        if let Some(seg) = self.segment.take() {
            step.push(DriverEvent::TurnItem(self.segment_item(&seg, false)));
            if seg.named {
                self.closed_named.insert(seg.message_id.clone(), seg);
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
            .segment
            .as_ref()
            .is_some_and(|s| s.kind == kind && mid.as_ref().is_none_or(|m| *m == s.message_id));
        if !reuse {
            self.close_segment(step);
            let resumed = mid
                .as_ref()
                .and_then(|m| self.closed_named.remove(m))
                .filter(|seg| seg.kind == kind);
            self.segment = Some(match resumed {
                Some(seg) => seg,
                None => {
                    self.segments += 1;
                    let item_id = format!("{}:seg{}", self.turn(), self.segments);
                    Segment {
                        kind,
                        named: mid.is_some(),
                        message_id: mid.unwrap_or_else(|| item_id.clone()),
                        item_id,
                        text: String::new(),
                    }
                }
            });
        }
        let seg = self.segment.as_mut().expect("set above");
        seg.text.push_str(text);
        let item = {
            let seg = self.segment.as_ref().expect("set above");
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
            ToolKind::Edit | ToolKind::Delete | ToolKind::Move => TurnItemKind::FileChange {
                file_name: t.file.clone().unwrap_or_else(|| t.title.clone()),
                diff_str: (t.diffs.len() > 1).then(|| multi_file_patch(&t.diffs)),
                old_str: t.diff.as_ref().and_then(|d| d.0.clone()),
                new_str: t.diff.as_ref().map(|d| d.1.clone()),
            },
            _ => TurnItemKind::DynamicTool {
                tool_name: t.title.clone(),
                input: t.raw_input.clone().unwrap_or(Value::Null),
                output,
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
                    }
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

    fn on_tool_call(&mut self, step: &mut DriverStep, call: ToolCall) {
        self.close_segment(step);
        let id = call.tool_call_id.0.to_string();
        let mut t = Tool {
            title: call.title,
            kind: call.kind,
            status: call.status,
            raw_input: call.raw_input,
            file: call.locations.first().map(|l| l.path.display().to_string()),
            ..Tool::default()
        };
        Self::merge_content(&mut t, &call.content);
        Self::merge_raw_images(&mut t, call.raw_output.as_ref());
        step.push(DriverEvent::TurnItem(self.tool_item(&id, &t)));
        self.swarm_members(step, &t);
        self.tools.insert(id, t);
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
        if self.provider != RuntimeProvider::Agy
            || t.title != "Run swarm"
            || t.status != ToolCallStatus::Completed
        {
            return;
        }
        let Some(input) = t.raw_input.as_ref() else {
            return;
        };
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
                    if self.swarm.contains_key(&member) {
                        continue;
                    }
                    let mut it = item(
                        format!("{}:swarm:{member}", self.turn()),
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
                    it.turn_id = self.turn_id.clone();
                    step.push(DriverEvent::TurnItem(it.clone()));
                    self.swarm.insert(member, it);
                }
            }
            // `kill{name}` also takes a whole `name-1..k` group.
            Some("kill") => {
                let gone: Vec<String> = self
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
                    if let Some(mut it) = self.swarm.remove(&m) {
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

    fn on_tool_update(&mut self, step: &mut DriverStep, up: ToolCallUpdate) {
        self.close_segment(step);
        let id = up.tool_call_id.0.to_string();
        let mut t = self.tools.remove(&id).unwrap_or_default();
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
        if let Some(locs) = &f.locations {
            if let Some(l) = locs.first() {
                t.file = Some(l.path.display().to_string());
            }
        }
        if let Some(content) = &f.content {
            // An update's content replaces the previous content (ACP).
            t.output.clear();
            Self::merge_content(&mut t, content);
        }
        if f.content.is_none()
            && f.raw_output
                .as_ref()
                .is_some_and(|raw| raw.get("attachments").is_some() || raw.get("content").is_some())
        {
            t.images.clear();
        }
        Self::merge_raw_images(&mut t, f.raw_output.as_ref());
        step.push(DriverEvent::TurnItem(self.tool_item(&id, &t)));
        self.swarm_members(step, &t);
        self.tools.insert(id, t);
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

    fn on_update(&mut self, step: &mut DriverStep, n: SessionNotification) {
        if matches!(self.phase, Phase::Creating { loading: true }) {
            return; // history replayed by session/load
        }
        match n.update {
            SessionUpdate::AgentMessageChunk(c) => {
                if let ContentBlock::Text(t) = c.content {
                    let mid = c.message_id.map(|m| m.0.to_string());
                    self.append_text(step, SegmentKind::Message, &t.text, mid);
                }
            }
            SessionUpdate::AgentThoughtChunk(c) => {
                if let ContentBlock::Text(t) = c.content {
                    self.append_text(step, SegmentKind::Thought, &t.text, None);
                }
            }
            SessionUpdate::ToolCall(call) => self.on_tool_call(step, call),
            SessionUpdate::ToolCallUpdate(up) => self.on_tool_update(step, up),
            SessionUpdate::Plan(plan) => self.on_plan(step, plan),
            _ => {}
        }
    }

    fn on_permission(
        &mut self,
        step: &mut DriverStep,
        rpc_id: Value,
        req: RequestPermissionRequest,
    ) {
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
                // reject_always and unknown kinds have no decision of their own;
                // `cancel` (always allowed) rejects without remembering.
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
        self.pending.insert(
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
        if let Phase::Authenticating { load } = self.phase {
            if id == self.auth_id {
                if let Some(e) = error {
                    step.push(DriverEvent::Fatal(rpc_failure("auth_required", &e)));
                    return;
                }
                self.create_session(step, load);
                return;
            }
        }
        if id == self.init_id && self.phase == Phase::Initializing {
            self.auth_methods = result
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
            self.images = caps.as_ref().is_some_and(|c| c.prompt_capabilities.image);
            self.embedded = caps.is_some_and(|c| c.prompt_capabilities.embedded_context);
            if let Some(e) = error {
                step.push(DriverEvent::Fatal(rpc_failure("handshake_error", &e)));
                return;
            }
            self.create_session(step, load && self.resume.is_some());
        } else if id == self.create_id && matches!(self.phase, Phase::Creating { .. }) {
            let loading = matches!(self.phase, Phase::Creating { loading: true });
            if let Some(e) = error {
                if e.get("code").and_then(Value::as_i64) == Some(AUTH_REQUIRED)
                    && !self.authenticated
                {
                    if let Some(method) = self.auth_method() {
                        let (id, line) =
                            self.request(M_AUTHENTICATE, AuthenticateRequest::new(method));
                        self.auth_id = id;
                        self.authenticated = true;
                        self.phase = Phase::Authenticating { load: loading };
                        step.write(line);
                        return;
                    }
                }
                if loading {
                    // T3 Code: a failed load / resume starts a fresh session.
                    self.resume = None;
                    self.create_session(step, false);
                    return;
                }
                step.push(DriverEvent::Fatal(rpc_failure("handshake_error", &e)));
                return;
            }
            self.modes = result
                .as_ref()
                .and_then(|r| r.pointer("/modes/availableModes"))
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|m| Some(m.get("id")?.as_str()?.to_string()))
                        .collect()
                })
                .unwrap_or_default();
            if let Some(opts) = result
                .as_ref()
                .and_then(|r| r.get("configOptions"))
                .and_then(Value::as_array)
            {
                self.effort_option = opts.iter().find_map(effort_config);
                self.effort = self.effort_option.as_ref().and_then(|(id, _)| {
                    opts.iter()
                        .find(|o| o.get("id").and_then(Value::as_str) == Some(id.as_str()))
                        .and_then(|o| o.get("currentValue"))
                        .and_then(Value::as_str)
                        .map(str::to_string)
                });
            }
            let current_mode = result
                .as_ref()
                .and_then(|r| r.pointer("/modes/currentModeId"))
                .and_then(Value::as_str)
                .map(str::to_string);
            self.session_id = if loading {
                self.resume.clone()
            } else {
                result
                    .and_then(|r| serde_json::from_value::<NewSessionResponse>(r).ok())
                    .map(|r| r.session_id.0.to_string())
            };
            self.phase = Phase::Ready;
            step.push(DriverEvent::Ready {
                native_thread_id: self.session_id.clone(),
                model: None,
            });
            self.write_model_config(step);
            // A mode no launch flag gave (agy has none): set it now.
            if let Some(want) = self.flavor.mode_id.and_then(|f| f(self.access)) {
                if current_mode.as_deref() != Some(want) && self.modes.iter().any(|m| m == want) {
                    let sid = self.session_id.clone().unwrap_or_default();
                    self.write_mode(step, &sid, self.access);
                }
            }
            // A prompt queued before `initialize` answered goes as is; the
            // agent refuses images it cannot take.
            if let Some((turn, req)) = self.queued.take() {
                self.send_prompt(step, turn, &req);
            }
        } else if Some(id) == self.config_req {
            self.config_req = None;
            if let Some(e) = error {
                // -32602: the agent refused the value; anything else (an
                // internal error, a dropped request) was not applied.
                self.config_retry = e.get("code").and_then(Value::as_i64) != Some(-32602);
                step.push(DriverEvent::Error(rpc_failure("provider_error", &e)));
            }
        } else if Some(id) == self.effort_req {
            self.effort_req = None;
            if let Some(e) = error {
                self.effort = None;
                step.push(DriverEvent::Error(rpc_failure("provider_error", &e)));
            }
        } else if Some(id) == self.mode_req {
            self.mode_req = None;
            if let Some(e) = error {
                step.push(DriverEvent::Error(rpc_failure("provider_error", &e)));
            }
        } else if Some(id) == self.prompt_id {
            self.prompt_id = None;
            self.close_segment(step);
            for (_, t) in self.tools.drain() {
                drop(t);
            }
            let ev = match (result, error) {
                (_, Some(e)) => DriverEvent::Terminal {
                    status: TurnTerminalStatus::Failed,
                    failure: Some(rpc_failure("provider_error", &e)),
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
                        }));
                    }
                    let stop = serde_json::from_value::<PromptResponse>(r)
                        .map(|p| p.stop_reason)
                        .unwrap_or(StopReason::EndTurn);
                    let (status, reason) = match stop {
                        StopReason::Cancelled => (TurnTerminalStatus::Interrupted, None),
                        StopReason::MaxTokens => {
                            (TurnTerminalStatus::Completed, Some("max_tokens"))
                        }
                        StopReason::MaxTurnRequests => {
                            (TurnTerminalStatus::Completed, Some("max_turn_requests"))
                        }
                        StopReason::Refusal => (TurnTerminalStatus::Completed, Some("refusal")),
                        _ => (TurnTerminalStatus::Completed, None),
                    };
                    DriverEvent::Terminal {
                        status,
                        failure: None,
                        reason: reason.map(str::to_string),
                    }
                }
                (None, None) => DriverEvent::Terminal {
                    status: TurnTerminalStatus::Completed,
                    failure: None,
                    reason: None,
                },
            };
            step.push(ev);
            self.turn_id = None;
        }
    }
}

/// A `select` config option that sets reasoning effort — agy's
/// `thought_level` (category `thought_level`), grok's `reasoning_effort` —
/// as `(configId, values)`.
fn effort_config(o: &Value) -> Option<(String, Vec<String>)> {
    let id = o.get("id")?.as_str()?;
    let category = o.get("category").and_then(Value::as_str);
    if !(matches!(id, "thought_level" | "reasoning_effort") || category == Some("thought_level"))
        || o.get("type").and_then(Value::as_str) != Some("select")
    {
        return None;
    }
    // Options may come grouped (`{group, options: […]}`).
    let mut values = Vec::new();
    for entry in o.get("options")?.as_array()? {
        let nested = entry.get("options").and_then(Value::as_array);
        for v in nested.map_or_else(|| vec![entry], |n| n.iter().collect()) {
            if let Some(v) = v.get("value").and_then(Value::as_str) {
                values.push(v.to_string());
            }
        }
    }
    (!values.is_empty()).then(|| (id.to_string(), values))
}

/// Effort names low → high (agents list theirs in either order: grok
/// advertises `xhigh, high, medium, low`).
const EFFORT_RANK: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];

fn effort_rank(l: &str) -> usize {
    EFFORT_RANK.iter().position(|r| *r == l).unwrap_or(2)
}

/// The agent's levels low → high (what the effort picker lists).
fn effort_levels(levels: &[String]) -> Vec<String> {
    let mut out = levels.to_vec();
    out.sort_by_key(|l| effort_rank(l));
    out
}

/// Otto's effort name as one of the agent's `levels`: the same name, else
/// its lowest for `minimal` / its highest for `xhigh` / `max`.
fn effort_value(levels: &[String], want: &str) -> Option<String> {
    if levels.iter().any(|l| l == want) {
        return Some(want.to_string());
    }
    let sorted = effort_levels(levels);
    match want {
        "minimal" => sorted.first().cloned(),
        "xhigh" | "max" => sorted.last().cloned(),
        _ => None,
    }
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
fn command_text(raw: Option<&Value>) -> Option<String> {
    match raw?.get("command")? {
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

impl StructuredDriver for AcpDriver {
    fn provider(&self) -> RuntimeProvider {
        self.provider
    }

    /// The provider baseline, then what `initialize` / `session/new|load`
    /// advertised: `promptCapabilities` and the effort config option.
    fn capabilities(&self) -> RuntimeCapabilities {
        let mut c = RuntimeCapabilities::for_provider(self.provider);
        if self.phase == Phase::Ready {
            c.images = self.images;
            c.files = self.embedded;
            c.efforts = self
                .effort_option
                .as_ref()
                .map(|(_, l)| effort_levels(l))
                .unwrap_or_default();
        }
        c
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
        self.cwd = ctx.cwd.clone();
        self.resume = ctx.resume.clone();
        self.mcp_servers = ctx.mcp_servers.clone();
        self.model = ctx.model.clone();
        self.access = ctx.access_mode;
        let mut step = DriverStep::default();
        let mut init = InitializeRequest::new(ProtocolVersion::V1);
        if let Some(Value::Object(meta)) = &self.flavor.initialize_meta {
            init.meta = Some(meta.clone());
        }
        // Elicitation is a draft ACP capability not yet exposed by our pinned SDK.
        let mut init = serde_json::to_value(init).expect("initialize serializes");
        init["clientCapabilities"]["elicitation"] = json!({"form":{}});
        let (id, line) = self.request(M_INITIALIZE, init);
        self.init_id = id;
        step.write(line);
        step
    }

    fn prompt(&mut self, turn_id: &str, req: &RuntimePromptReq) -> Result<DriverStep, DriverError> {
        if self.prompt_id.is_some() || self.queued.is_some() {
            return Err(DriverError("a prompt is already running".into()));
        }
        let name = self.provider.as_str();
        let ready = self.phase == Phase::Ready;
        if let Some(e) = &req.effort {
            let takes = self
                .effort_option
                .as_ref()
                .is_some_and(|(_, levels)| effort_value(levels, e).is_some());
            if ready && !takes {
                return Err(DriverError(format!(
                    "{name} does not take reasoning effort {e}"
                )));
            }
        }
        if ready && !req.images.is_empty() && !self.images {
            return Err(DriverError(format!("{name} does not accept images")));
        }
        if ready && !req.files.is_empty() && !self.embedded {
            return Err(DriverError(format!(
                "{name} does not accept file attachments"
            )));
        }
        let mut step = DriverStep::default();
        if ready {
            self.send_prompt(&mut step, turn_id.to_string(), req);
        } else {
            self.turn_id = Some(turn_id.to_string());
            self.queued = Some((turn_id.to_string(), req.clone()));
        }
        Ok(step)
    }

    /// Live only through an advertised `session/set_mode`, between modes
    /// that both have one.
    fn switches_access_live(&self, to: RuntimeAccessMode) -> bool {
        self.flavor.mode_id.is_some_and(|f| {
            f(self.access).is_some() && f(to).is_some_and(|id| self.modes.iter().any(|m| m == id))
        })
    }

    fn adopt_spec(&mut self, spec: &mut LaunchSpec) {
        let Some(var) = self.flavor.bundle_env else {
            return;
        };
        if let Some(i) = spec.env.iter().position(|(k, _)| k == var) {
            let (_, dir) = spec.env.remove(i);
            self.system_append = bundle_prompt(std::path::Path::new(&dir));
        }
    }

    fn interrupt(&mut self) -> DriverStep {
        let mut step = DriverStep::default();
        // ACP: the client answers every open permission request `cancelled`.
        for (_, (rpc_id, _)) in self.questions.drain() {
            step.write(
                json!({"jsonrpc":"2.0","id":rpc_id,"result":{"action":"cancel"}}).to_string(),
            );
        }
        let mut open: Vec<(String, Pending)> = self.pending.drain().collect();
        open.sort_by(|a, b| a.0.cmp(&b.0));
        for (_, p) in open {
            let resp = RequestPermissionResponse::new(RequestPermissionOutcome::Cancelled);
            step.write(json!({"jsonrpc":"2.0","id":p.rpc_id,"result":resp}).to_string());
        }
        if self.queued.take().is_some() {
            step.push(DriverEvent::Terminal {
                status: TurnTerminalStatus::Interrupted,
                failure: None,
                reason: None,
            });
        } else if let Some(sid) = &self.session_id {
            let mut n = CancelNotification::new(sid.clone());
            if let Some(Value::Object(meta)) = &self.flavor.cancel_meta {
                n.meta = Some(meta.clone());
            }
            step.write(json!({"jsonrpc":"2.0","method":M_SESSION_CANCEL,"params":n}).to_string());
        }
        step
    }

    fn respond(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<DriverStep, DriverError> {
        let p = self
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
            .questions
            .get(request_id)
            .ok_or_else(|| DriverError("no such pending question".into()))?;
        otto_core::agent_runtime::validate_question_answers(questions, answers)
            .map_err(DriverError)?;
        use otto_core::agent_runtime::{QuestionAnswer, QuestionKind};
        let content: serde_json::Map<String, Value> = answers
            .iter()
            .map(|a| {
                let id = a.question_id();
                let single = questions
                    .iter()
                    .any(|q| q.id == id && q.kind == QuestionKind::Single);
                match a {
                    QuestionAnswer::Multi { values, .. } => (id.to_string(), json!(values)),
                    // Typed text for a single choice is its folded
                    // `<id>__other` field's answer (`questions::acp_form`).
                    QuestionAnswer::Text { text, .. } if single => {
                        (crate::questions::other_field(id), json!(text))
                    }
                    _ => (id.to_string(), json!(a.values()[0])),
                }
            })
            .collect();
        let mut step = DriverStep::default();
        step.write(
            json!({"jsonrpc":"2.0","id":rpc_id,"result":{"action":"accept","content":content}})
                .to_string(),
        );
        self.questions.remove(request_id);
        Ok(step)
    }

    fn dismiss_question(
        &mut self,
        request_id: &str,
        action: otto_core::agent_runtime::QuestionDismissAction,
    ) -> Result<DriverStep, DriverError> {
        let (rpc_id, _) = self
            .questions
            .get(request_id)
            .ok_or_else(|| DriverError("no such pending question".into()))?;
        let mut step = DriverStep::default();
        step.write(json!({"jsonrpc":"2.0","id":rpc_id,"result":{"action":action}}).to_string());
        self.questions.remove(request_id);
        Ok(step)
    }

    fn on_line(&mut self, line: &str) -> DriverStep {
        let mut step = DriverStep::default();
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
                if let Ok(n) = serde_json::from_value::<SessionNotification>(params) {
                    self.on_update(&mut step, n);
                }
            }
            (Some(_), None) => {} // unknown notification
            (Some(m), Some(id)) if m == "elicitation/create" => {
                let parsed = if params.get("sessionId").and_then(Value::as_str)
                    != self.session_id.as_deref()
                    || self.session_id.is_none()
                    || params.get("requestId").is_some()
                {
                    Err("elicitation must belong to this live session".to_string())
                } else {
                    crate::questions::acp_form(&params)
                };
                match parsed {
                    Ok(questions) => {
                        let request_id = format!("acp-question-{}", rpc_key(&id));
                        self.questions
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
                match serde_json::from_value::<RequestPermissionRequest>(params) {
                    Ok(req) => self.on_permission(&mut step, id, req),
                    Err(e) => step.write(
                        json!({"jsonrpc":"2.0","id":id,"error":{"code":-32602,"message":e.to_string()}})
                            .to_string(),
                    ),
                }
            }
            (Some(m), Some(id)) => {
                // fs/*, terminal/*, …: not offered in our client capabilities.
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
        self.pending.clear();
        self.questions.clear();
        self.tools.clear();
        self.segment = None;
        self.closed_named.clear();
        DriverStep::default()
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
            effort_value(&["xhigh".into(), "high".into(), "low".into()], "max").as_deref(),
            Some("xhigh")
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
        assert!(d2.prompt("t1", &req).is_err(), "no effort option");
        assert!(d2.capabilities().efforts.is_empty());
    }

    /// agy's swarm (live 1.5.8 frames): each `swarm add` is a background
    /// subagent card that a `swarm kill` stops; other actions stay tool rows.
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
                "OPENCODE_PERMISSION".to_string(),
                r#"{"*":"allow"}"#.to_string()
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
            let rules: Value = serde_json::from_str(&spec.env[0].1).unwrap();
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
        // No auto-accept-edits launch mode: it asks, as approval-required.
        let edits = g.launch(&at(RuntimeAccessMode::AutoAcceptEdits));
        assert_eq!(edits.args, ["--permission-mode", "default"]);
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
        d.session_id = Some("s1".into());
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
        d.session_id = Some("s1".into());
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
        d.session_id = Some("s1".into());
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
        d.on_line(&refused.to_string());
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
}
