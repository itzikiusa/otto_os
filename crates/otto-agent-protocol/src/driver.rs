//! The provider driver contract — Otto's port of T3 Code's provider adapter
//! split (MIT, Copyright (c) 2026 T3 Tools Inc.; `ProviderAdapterV2` in
//! `packages/provider-core/src/server/ProviderAdapter.ts`, the ACP adapter in
//! `packages/provider-acp/src/server/adapter.ts`).
//!
//! In T3 Code an adapter owns its process and emits entity upserts. Here the
//! split is sans-IO: the runtime owns the child (so every spawn goes through
//! Otto's safety pipeline) and the driver only translates — each command and
//! each stdout line becomes a [`DriverStep`]: [`DriverEvent`]s (the
//! adapter-produced subset of the runtime event model), stdin lines, and
//! rarely a signal. The runtime owns what T3 Code's orchestration layer owns:
//! turns (ids, ordinals, status), item ordinals, sequencing, request
//! resolution and cancellation, and the process-death terminal.

use otto_core::agent_runtime::{
    ApprovalDecision, ApprovalOption, BackgroundTask, PlanStep, ProviderFailure, RequestKind,
    RuntimeAccessMode, RuntimeCapabilities, RuntimePromptReq, RuntimeProvider,
    RuntimeRequestStatus, ThreadTokenUsage, TurnItem, TurnItemKind, TurnItemStatus,
    TurnTerminalStatus, TurnTokenUsage,
};

#[derive(Debug, Clone)]
pub struct LaunchContext {
    /// The Otto runtime session id (stable across respawns).
    pub session_id: String,
    pub cwd: String,
    pub model: Option<String>,
    /// The provider's own session / thread id to resume (`None` = fresh).
    pub resume: Option<String>,
    /// The child will run under Otto's Seatbelt profile.
    pub sandboxed: bool,
    pub access_mode: RuntimeAccessMode,
    /// MCP servers for a [`McpDelivery::Protocol`] driver's handshake
    /// (filled by the host's spawn preparation; empty at `launch`).
    pub mcp_servers: Vec<McpServerDef>,
}

/// How a driver's agent receives MCP servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum McpDelivery {
    /// The CLI's own launcher config / flags (what its terminal session gets).
    Launcher,
    /// In the protocol handshake ([`LaunchContext::mcp_servers`]).
    Protocol,
}

/// One stdio MCP server handed over the protocol. `command` is absolute.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct McpServerDef {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// The child's command line, before the safety pipeline wraps it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchSpec {
    pub program: String,
    pub args: Vec<String>,
    /// Argv that must come AFTER the context hook's injected flags — a
    /// subcommand whose parser rejects top-level flags after it (`grok …
    /// agent stdio` refuses a trailing `--rules=`). The safety pipeline
    /// appends these after its injection and before the Seatbelt wrap.
    pub tail_args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildSignal {
    Interrupt,
}

/// What a [`DriverEvent::ClientCall`] asks the runtime to do. Paths are
/// absolute; `root` is the folder a file must stay inside. A terminal's
/// `program` / `args` already carry the session's sandbox wrapper.
#[derive(Debug, Clone, PartialEq)]
pub enum ClientOp {
    ReadFile {
        root: String,
        path: String,
        line: Option<u64>,
        limit: Option<u64>,
    },
    WriteFile {
        root: String,
        path: String,
        content: String,
    },
    TerminalCreate {
        terminal_id: String,
        program: String,
        args: Vec<String>,
        env: Vec<(String, String)>,
        cwd: String,
        byte_limit: Option<u64>,
    },
    TerminalOutput {
        terminal_id: String,
    },
    TerminalWait {
        terminal_id: String,
    },
    TerminalKill {
        terminal_id: String,
    },
    TerminalRelease {
        terminal_id: String,
    },
}

/// What a driver reports. `turn_item` / `plan` / `command_output` /
/// `request` map 1:1 onto `turn_item.updated`, `plan.updated`,
/// `command_output.delta` and `runtime_request.updated`.
#[derive(Debug, Clone, PartialEq)]
pub enum DriverEvent {
    /// The protocol session is ready (`provider_session` → ready). `native_thread_id`
    /// is the resume key; `None` keeps the one already known.
    Ready {
        native_thread_id: Option<String>,
        model: Option<String>,
    },
    /// Upsert a timeline item. `ordinal` and a missing `turn_id` are filled
    /// in by the runtime.
    TurnItem(TurnItem),
    /// The turn's plan: `plan.updated` plus its `todo_list` item.
    Plan {
        steps: Vec<PlanStep>,
        explanation: Option<String>,
    },
    CommandOutput {
        item_id: String,
        chunk: String,
    },
    /// A new pending request; the runtime adds its `approval_request` item.
    Request {
        id: String,
        kind: RequestKind,
        title: Option<String>,
        prompt: Option<String>,
        reason: Option<String>,
        options: Vec<ApprovalOption>,
        /// The turn item id of the tool call it gates, when known.
        tool_item_id: Option<String>,
    },
    /// A typed user question, never a permission approval.
    Question {
        id: String,
        title: Option<String>,
        questions: Vec<otto_core::agent_runtime::RuntimeQuestion>,
        tool_item_id: Option<String>,
    },
    /// The provider withdrew or settled an open request itself (Claude
    /// `control_cancel_request`, Codex `serverRequest/resolved`).
    RequestClosed {
        request_id: String,
        status: RuntimeRequestStatus,
    },
    Usage(TurnTokenUsage),
    /// What the running turn cost in US dollars (merged into its usage).
    TurnCost(f64),
    ContextUsage(ThreadTokenUsage),
    /// An ACP client method the agent called (`fs/*`, `terminal/*`), for the
    /// runtime to run off the session lock; the answer comes back through
    /// [`StructuredDriver::on_client_result`].
    ClientCall {
        id: String,
        op: ClientOp,
    },
    /// The agent changed its mode or model itself (ACP `current_mode_update`,
    /// `config_option_update`): the thread's chips follow it.
    AgentSettings {
        model: Option<String>,
        access: Option<otto_core::agent_runtime::RuntimeAccessMode>,
    },
    /// The agent's own slash commands, whole (`commands.updated`).
    Commands(Vec<otto_core::agent_runtime::ProviderCommand>),
    /// MCP servers as the CLI reports them (merged by name into the thread's
    /// list), or — `spawn`, emitted by the runtime — the list Otto gave the
    /// new process, whole (`mcp_servers.updated`).
    McpServers {
        servers: Vec<otto_core::agent_runtime::McpServerStatus>,
        spawn: bool,
    },
    /// The agent named the conversation (Codex `thread/name/updated`, an
    /// ACP `session_info_update`). Adopted while the thread still has the
    /// title Otto gave it.
    Title(String),
    /// The agent started a turn on its own (no prompt of ours is running —
    /// Claude Code continues when background work it started finishes).
    /// `turn_id` is the driver's; the runtime opens it as an `agent` turn
    /// and ends it on the next [`DriverEvent::Terminal`].
    TurnStarted {
        turn_id: String,
    },
    /// The agent's pending background work, whole (replaces the last list).
    BackgroundTasks(Vec<BackgroundTask>),
    /// The provider answered a [`TaskStop::Native`] call for `task_id`:
    /// `error` is its refusal. Acceptance is not the end of the task — that
    /// still comes as the task leaving [`DriverEvent::BackgroundTasks`].
    TaskStopAnswered {
        task_id: String,
        error: Option<String>,
    },
    /// The protocol says the running turn ended.
    Terminal {
        status: TurnTerminalStatus,
        failure: Option<ProviderFailure>,
        reason: Option<String>,
    },
    /// A session-level problem the session survives (a bad line).
    Error(ProviderFailure),
    /// The handshake was refused (not signed in, bad argv, …): the runtime
    /// fails the running turn with it and stops the child; the next prompt
    /// respawns and tries again.
    Fatal(ProviderFailure),
}

#[derive(Debug, Default, Clone, PartialEq)]
pub struct DriverStep {
    pub events: Vec<DriverEvent>,
    /// Complete JSON documents, one per stdin line.
    pub writes: Vec<String>,
    pub signal: Option<ChildSignal>,
}

impl DriverStep {
    pub fn push(&mut self, ev: DriverEvent) {
        self.events.push(ev);
    }

    pub fn write(&mut self, line: String) {
        self.writes.push(line);
    }
}

/// A timeline item as drivers build it (the runtime sets ordinal + turn).
pub fn item(
    id: String,
    status: TurnItemStatus,
    title: Option<String>,
    kind: TurnItemKind,
) -> TurnItem {
    let now = chrono::Utc::now();
    TurnItem {
        id,
        turn_id: None,
        ordinal: 0,
        status,
        title,
        started_at: Some(now),
        completed_at: (!status.is_open()).then_some(now),
        updated_at: now,
        error: None,
        parent_item_id: None,
        detail_omitted: false,
        kind,
    }
}

/// The provider as a person reads it in a notice.
pub fn provider_name(p: RuntimeProvider) -> &'static str {
    match p {
        RuntimeProvider::Claude => "Claude Code",
        RuntimeProvider::Codex => "Codex",
        RuntimeProvider::Pi => "Pi",
        RuntimeProvider::Agy => "Antigravity",
        RuntimeProvider::Grok => "Grok",
        RuntimeProvider::Muse => "Muse",
        RuntimeProvider::Gemini => "Gemini",
        RuntimeProvider::Opencode => "OpenCode",
    }
}

/// A system notice row (an Otto-side remark in the timeline).
pub fn notice(id: String, message: String) -> TurnItem {
    item(
        id,
        TurnItemStatus::Completed,
        None,
        TurnItemKind::SystemNotice {
            message,
            task_settlement: None,
        },
    )
}

/// The id suffix of a turn's provider-retry notice. Its updates are not
/// activity: the stall watchdog keeps counting while a provider retries, so a
/// retry loop cannot keep a turn alive forever (T3 Code #29208).
pub const RETRY_NOTICE_SUFFIX: &str = ":provider-retry";

/// The turn's one provider-retry row (updated in place on every attempt),
/// worded the same for every provider (T3 Code #33151).
pub fn retry_notice(
    turn: &str,
    attempt: Option<u64>,
    max: Option<u64>,
    why: Option<&str>,
) -> TurnItem {
    let mut message = match (attempt, max) {
        (Some(a), Some(m)) => format!("Provider retry {a} of {m}"),
        (Some(a), None) => format!("Provider retry {a}"),
        _ => "Provider retry".to_string(),
    };
    if let Some(why) = why.map(str::trim).filter(|w| !w.is_empty()) {
        message.push_str(" · ");
        message.push_str(why);
    }
    notice(format!("{turn}{RETRY_NOTICE_SUFFIX}"), message)
}

/// The turn's compaction row: "Compacting context…" while it runs (`done`
/// false), then "Context compacted" — the wording Codex's own item uses.
pub fn compaction_notice(turn: &str, done: bool) -> TurnItem {
    let (status, message) = if done {
        (TurnItemStatus::Completed, "Context compacted")
    } else {
        (TurnItemStatus::Running, "Compacting context…")
    };
    item(
        format!("{turn}:compaction"),
        status,
        None,
        TurnItemKind::SystemNotice {
            message: message.into(),
            task_settlement: None,
        },
    )
}

/// `req` without an effort `provider` does not take, plus the notice that
/// says so: an unsupported level runs the turn at the agent's default
/// instead of refusing the whole prompt (an "Ask another agent" create).
pub fn drop_unsupported_effort(
    provider: RuntimeProvider,
    efforts: &[String],
    req: &RuntimePromptReq,
) -> (RuntimePromptReq, Option<TurnItem>) {
    let Some(effort) = req.effort.as_deref() else {
        return (req.clone(), None);
    };
    if efforts.iter().any(|e| e == effort) {
        return (req.clone(), None);
    }
    let message = format!(
        "{} does not take the reasoning effort \u{201c}{effort}\u{201d} \u{2014} this turn runs at its default effort.",
        provider_name(provider)
    );
    let req = RuntimePromptReq {
        effort: None,
        ..req.clone()
    };
    (
        req,
        Some(notice(
            format!("effort-dropped:{}", otto_core::new_id()),
            message,
        )),
    )
}

/// A failed tool's error text for [`TurnItem::error`]: trimmed, at most
/// ~2 KB (cut on a char boundary), `None` when empty.
pub fn tool_error(text: &str) -> Option<String> {
    let t = text.trim();
    let mut end = t.len().min(2048);
    while !t.is_char_boundary(end) {
        end -= 1;
    }
    (!t.is_empty()).then(|| t[..end].to_string())
}

/// `text` with a line per attached file the agent must read from disk (an
/// agent whose input has no document type): T3 Code's
/// `providerMessageTextWithAttachmentPaths`.
pub fn with_file_paths(text: &str, files: &[otto_core::agent_runtime::PromptFile]) -> String {
    let mut out = text.to_string();
    for f in files {
        let line = match &f.path {
            Some(p) => format!(
                "[Attached file \"{}\" ({}) is saved at: {p}]",
                f.name, f.mime
            ),
            None => format!(
                "[Attached file \"{}\" is unavailable: it was not saved]",
                f.name
            ),
        };
        if !out.is_empty() {
            out.push_str("\n\n");
        }
        out.push_str(&line);
    }
    out
}

/// A JSON-RPC id as a map key.
pub fn rpc_key(id: &serde_json::Value) -> String {
    match id {
        serde_json::Value::String(s) => s.clone(),
        other => other.to_string(),
    }
}

/// A JSON-RPC `error` object as a failure.
pub fn rpc_failure(class: &str, e: &serde_json::Value) -> ProviderFailure {
    ProviderFailure {
        class: class.into(),
        message: e
            .get("message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("agent error")
            .to_string(),
        code: e.get("code").map(|c| c.to_string()),
        retryable: None,
    }
}

/// Sideband storage source polled asynchronously off the session lock when an
/// ACP agent buffers between-prompt notifications in memory while persisting
/// real-time session steps on disk (`agy`, `grok`, `gemini`, `opencode`, or an
/// explicit `OTTO_ACP_SIDEBAND_FILE` stream).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebandSource {
    /// No primary session transcript poll (active child tails / timer ticks only).
    None,
    /// Append-only `\n`-terminated JSONL (`agy` `transcript_full.jsonl`, `grok`
    /// `updates.jsonl`, or `OTTO_ACP_SIDEBAND_FILE`).
    AppendLines {
        path: std::path::PathBuf,
        fallback_path: Option<std::path::PathBuf>,
        /// Optional `<sessions_root>/<project_dir>/<session_id>/updates.jsonl`
        /// scan fallback when `cwd` was symlink-resolved differently by the CLI.
        scan_sessions_dir: Option<(std::path::PathBuf, String)>,
        offset: u64,
        using_fallback: bool,
    },
    /// Rewritten JSON session snapshot (`gemini --acp` `chats/session-*.json`),
    /// read only when `(mtime_ns, len)` changes.
    GeminiJson {
        gemini_home: std::path::PathBuf,
        cwd: String,
        session_id: String,
        resolved_path: Option<std::path::PathBuf>,
        last_stamp: (u64, u64),
        allow_scan: bool,
    },
    /// Read-only SQLite `part` + `message` rows (`opencode acp` `opencode.db`),
    /// queried only when `(db_mtime_ns, db_len, wal_mtime_ns, wal_len)` changes.
    OpencodeSqlite {
        db_path: std::path::PathBuf,
        session_id: String,
        min_time_ms: i64,
        min_part_id: String,
        last_stamp: (u64, u64, u64, u64),
    },
}

/// Kind of auxiliary background file tailed off-lock alongside the session's
/// primary sideband stream.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum ActiveTailKind {
    CommandLog,
    AgySubagentTranscript,
}

/// One active background command `.log` or `agy` child `transcript.jsonl` to
/// tail off-lock.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTailSource {
    pub task_id: String,
    pub kind: ActiveTailKind,
    pub path: std::path::PathBuf,
    pub fallback_path: Option<std::path::PathBuf>,
    pub offset: u64,
    pub using_fallback: bool,
    pub allowed_cwd: Option<std::path::PathBuf>,
    pub agy_brain_dir: Option<std::path::PathBuf>,
    pub allowed_agy_conv_ids: Option<Vec<String>>,
    pub otto_session_id: Option<String>,
}

/// Off-lock read chunk from an [`ActiveTailSource`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveTailChunk {
    pub task_id: String,
    pub kind: ActiveTailKind,
    pub using_fallback: bool,
    pub next_offset: u64,
    pub bytes: Vec<u8>,
}

/// Periodic sideband poll descriptor returned by a sans-IO driver while idle
/// between `session/prompt` RPCs (or while tailing active background tasks).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SidebandPoll {
    pub source: SidebandSource,
    pub interval: std::time::Duration,
    pub active_tails: Vec<ActiveTailSource>,
    pub timer_expired: bool,
}

/// Initial watermark seeded asynchronously before spawning a resumed session so
/// historical on-disk steps are skipped in `O(1)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebandResumeSeed {
    AppendOffset {
        resolved_path: Option<std::path::PathBuf>,
        using_fallback: bool,
        offset: u64,
    },
    IdleSinceMs(u64),
}

/// Off-lock read result delivered to [`StructuredDriver::on_sideband`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SidebandPayload {
    AppendLines {
        resolved_path: Option<std::path::PathBuf>,
        using_fallback: bool,
        next_offset: u64,
        bytes: Vec<u8>,
    },
    GeminiJson {
        resolved_path: std::path::PathBuf,
        stamp: (u64, u64),
        doc: serde_json::Value,
    },
    OpencodeSqlite {
        stamp: (u64, u64, u64, u64),
        rows_jsonl: Vec<u8>,
        has_more: bool,
    },
    /// Directory scan attempted to locate an unresolved transcript file and
    /// found no matching file; arms a negative-lookup backoff on the driver.
    Unresolved,
    /// Combined primary sideband payload, active child/command tail chunks,
    /// and/or due timer tick.
    Batch {
        primary: Option<Box<SidebandPayload>>,
        tails: Vec<ActiveTailChunk>,
        timer_tick: bool,
    },
}

/// How a driver stops one piece of its agent's background work
/// ([`StructuredDriver::stop_task`]).
#[derive(Debug, Clone, PartialEq)]
pub enum TaskStop {
    /// The provider's own stop call is in `step`; its answer comes back as
    /// [`DriverEvent::TaskStopAnswered`].
    Native(DriverStep),
    /// This provider has no stop call for it: the runtime asks the agent.
    Unsupported,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverError(pub String);

impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// [`StructuredDriver::restore_state`] for a driver that snapshots itself
/// whole: a non-null `state` replaces `driver`, which keeps its own `program`
/// (the field `program` picks), never part of a snapshot.
pub(crate) fn restore_keeping_program<T: serde::de::DeserializeOwned>(
    driver: &mut T,
    state: &serde_json::Value,
    program: fn(&mut T) -> &mut String,
) -> Result<(), DriverError> {
    if !state.is_null() {
        let mut restored: T =
            serde_json::from_value(state.clone()).map_err(|e| DriverError(e.to_string()))?;
        std::mem::swap(program(&mut restored), program(driver));
        *driver = restored;
    }
    Ok(())
}

/// One provider's protocol translator.
pub trait StructuredDriver: Send + 'static {
    fn provider(&self) -> RuntimeProvider;

    fn launch(&self, ctx: &LaunchContext) -> LaunchSpec;

    /// Where this driver's agent takes its MCP servers from.
    fn mcp_delivery(&self) -> McpDelivery {
        McpDelivery::Launcher
    }

    /// The host finished `launch`'s spec (context injection, MCP, sandbox):
    /// a driver may take what it delivers over the protocol out of it. Runs
    /// on a blocking thread, so it may read files.
    fn adopt_spec(&mut self, _spec: &mut LaunchSpec) {}

    /// Called once right after spawn: the handshake lines.
    fn start(&mut self, ctx: &LaunchContext) -> DriverStep;

    /// Send the prompt of turn `turn_id` (one turn at a time). A driver whose
    /// handshake is still in flight queues it. An error refuses the prompt
    /// (400): the provider cannot take it (images, effort).
    fn prompt(&mut self, turn_id: &str, req: &RuntimePromptReq) -> Result<DriverStep, DriverError>;

    /// Whether `req.model` switches on the running process. Otherwise the
    /// runtime restarts the child with the new model, resuming its thread.
    fn switches_model_live(&self) -> bool {
        false
    }

    /// Whether `steer` can switch `req.model` mid-turn without queuing for the
    /// next turn boundary.
    fn steers_model_live(&self) -> bool {
        false
    }

    /// Whether the running process can switch to access mode `to` (its next
    /// `prompt` carries it). Otherwise the runtime restarts the child in the
    /// new mode, resuming its thread.
    fn switches_access_live(&self, _to: RuntimeAccessMode) -> bool {
        false
    }

    /// What this agent takes (steer, images, PDFs, efforts): the provider
    /// baseline, refined by a driver that learns it in its handshake. Read
    /// by the runtime when the driver reports `Ready`.
    fn capabilities(&self) -> RuntimeCapabilities {
        RuntimeCapabilities::for_provider(self.provider())
    }

    /// A transcript the agent writes its own title for the conversation
    /// into (Claude Code's `ai-title` lines): the host reads it after a turn
    /// while the thread's title is still Otto's placeholder.
    fn title_transcript(&self) -> Option<std::path::PathBuf> {
        None
    }

    /// Whether `steer` can switch to access mode `to` mid-turn without queuing
    /// for the next turn boundary.
    fn steers_access_live(&self, _to: RuntimeAccessMode) -> bool {
        false
    }

    /// Add a person's message to the RUNNING turn (steering): the agent
    /// reads it at its next step. A driver that cannot refuses, and the
    /// runtime queues the message for after the turn instead.
    fn steer(&mut self, _req: &RuntimePromptReq) -> Result<DriverStep, DriverError> {
        Err(DriverError(
            "this agent cannot take a message mid-turn".into(),
        ))
    }

    /// Cancel the running turn; the driver still reports the terminal when the
    /// provider acknowledges. Open protocol requests are answered here.
    fn interrupt(&mut self) -> DriverStep;

    /// Stop one background task (`task_id` from [`DriverEvent::BackgroundTasks`])
    /// with the provider's own call. The default — and any task the driver
    /// cannot address natively — is [`TaskStop::Unsupported`]; never a kill
    /// of a process the driver cannot prove is that task's.
    fn stop_task(&mut self, _task_id: &str) -> TaskStop {
        TaskStop::Unsupported
    }

    /// Answer a pending request with a person's decision.
    fn respond(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<DriverStep, DriverError>;

    fn respond_question(
        &mut self,
        _request_id: &str,
        _answers: &[otto_core::agent_runtime::QuestionAnswer],
    ) -> Result<DriverStep, DriverError> {
        Err(DriverError(
            "this provider does not support structured answers".into(),
        ))
    }

    fn dismiss_question(
        &mut self,
        _request_id: &str,
        _action: otto_core::agent_runtime::QuestionDismissAction,
    ) -> Result<DriverStep, DriverError> {
        Err(DriverError(
            "this provider cannot dismiss structured questions".into(),
        ))
    }

    fn on_line(&mut self, line: &str) -> DriverStep;

    /// Whether the running agent-initiated (`origin: agent`) turn has no
    /// protocol-level end frame (e.g. ACP notifications) and is ready to settle
    /// once stdout stays quiet for `RuntimeConfig::autonomous_settle`.
    fn needs_idle_turn_settle(&self) -> bool {
        false
    }

    /// Settle an idle agent-initiated turn once `autonomous_settle` elapses.
    fn on_idle_settle(&mut self) -> DriverStep {
        DriverStep::default()
    }

    /// Optional sideband file tail descriptor while the driver is idle between
    /// prompts. All disk I/O runs asynchronously outside the session lock.
    fn sideband_poll(&self) -> Option<SidebandPoll> {
        None
    }

    /// Optional sideband source for `resume_id` so `ensure_process` can seed
    /// the initial watermark asynchronously before spawning.
    fn sideband_resume_poll(&self, _resume_id: &str, _cwd: &str) -> Option<SidebandSource> {
        None
    }

    /// Seed the initial sideband watermark on resume (`session/load`) before
    /// the child spawns so historical steps on disk are skipped in `O(1)`.
    fn init_sideband_resume(&mut self, _seed: SidebandResumeSeed) {}

    /// Translate off-lock sideband data into a sans-IO [`DriverStep`].
    /// The runtime ran a [`DriverEvent::ClientCall`]: its JSON result, or why
    /// it failed.
    fn on_client_result(
        &mut self,
        _id: &str,
        _result: std::result::Result<serde_json::Value, String>,
    ) -> DriverStep {
        DriverStep::default()
    }

    /// A chunk of a client terminal's output, as it arrives.
    fn on_client_output(&mut self, _terminal_id: &str, _chunk: &str) -> DriverStep {
        DriverStep::default()
    }

    fn on_sideband(&mut self, _payload: SidebandPayload) -> DriverStep {
        DriverStep::default()
    }

    fn on_exit(&mut self, _code: Option<i32>) -> DriverStep {
        DriverStep::default()
    }

    /// Snapshot live protocol state so a restarted daemon adopting a detached
    /// holder can resume mid-turn without losing request correlations or
    /// negotiated session capabilities.
    fn snapshot_state(&self) -> serde_json::Value {
        serde_json::Value::Null
    }

    /// Restore live protocol state captured by [`Self::snapshot_state`].
    fn restore_state(&mut self, _state: &serde_json::Value) -> Result<(), DriverError> {
        Ok(())
    }
}

/// Builds a fresh driver per spawn.
pub trait DriverFactory: Send + Sync + 'static {
    fn create(&self, provider: RuntimeProvider) -> Option<Box<dyn StructuredDriver>>;
}

impl<F> DriverFactory for F
where
    F: Fn(RuntimeProvider) -> Option<Box<dyn StructuredDriver>> + Send + Sync + 'static,
{
    fn create(&self, provider: RuntimeProvider) -> Option<Box<dyn StructuredDriver>> {
        self(provider)
    }
}

/// Extract a clamped one-shot timer `due_at` (`0..=365d`) from tool arguments,
/// returning `None` when a recurring `CronExpression` / `cron_expression` is set
/// or when the duration is missing/negative.
pub fn one_shot_due_at(
    raw: Option<&serde_json::Value>,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<chrono::DateTime<chrono::Utc>> {
    const MAX_TIMER_SECS: i64 = 86_400 * 365;
    let v = raw?;
    let has_cron = v
        .get("CronExpression")
        .or_else(|| v.get("cron_expression"))
        .and_then(serde_json::Value::as_str)
        .is_some_and(|s| !s.trim().is_empty());
    if has_cron {
        return None;
    }
    let dur = v
        .get("DurationSeconds")
        .or_else(|| v.get("duration_seconds"))
        .or_else(|| v.get("delay_seconds"))?;
    if let Some(secs) = dur.as_i64() {
        if secs < 0 {
            return None;
        }
        return Some(now + chrono::Duration::seconds(secs.clamp(0, MAX_TIMER_SECS)));
    }
    if let Some(secs) = dur.as_u64() {
        let clamped = secs.min(MAX_TIMER_SECS as u64) as i64;
        return Some(now + chrono::Duration::seconds(clamped));
    }
    if let Some(secs_f) = dur.as_f64() {
        if secs_f.is_finite() && secs_f >= 0.0 {
            let ms = (secs_f.min(MAX_TIMER_SECS as f64) * 1_000.0) as i64;
            return Some(now + chrono::Duration::milliseconds(ms));
        }
    }
    None
}
