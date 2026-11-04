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
    ApprovalDecision, PlanArtifact, ProviderFailure, RuntimeProvider, RuntimeRequest,
    ThreadTokenUsage, TurnItem, TurnTerminalStatus, TurnTokenUsage,
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
}

/// The child's command line, before the safety pipeline wraps it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LaunchSpec {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChildSignal {
    Interrupt,
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
    /// Upsert a timeline item. `ordinal` is ignored (the runtime assigns it).
    TurnItem(TurnItem),
    Plan(PlanArtifact),
    CommandOutput {
        item_id: String,
        chunk: String,
    },
    /// A new pending request plus its `approval_request` item.
    Request {
        request: RuntimeRequest,
        item: TurnItem,
    },
    Usage(TurnTokenUsage),
    ContextUsage(ThreadTokenUsage),
    /// The protocol says the running turn ended.
    Terminal {
        status: TurnTerminalStatus,
        failure: Option<ProviderFailure>,
        reason: Option<String>,
    },
    /// A session-level problem (bad line, refused handshake).
    Error(ProviderFailure),
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DriverError(pub String);

impl std::fmt::Display for DriverError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// One provider's protocol translator.
pub trait StructuredDriver: Send + 'static {
    fn provider(&self) -> RuntimeProvider;

    fn launch(&self, ctx: &LaunchContext) -> LaunchSpec;

    /// Called once right after spawn: the handshake lines.
    fn start(&mut self, ctx: &LaunchContext) -> DriverStep;

    /// Send the prompt of turn `turn_id` (one turn at a time). A driver whose
    /// handshake is still in flight queues it.
    fn prompt(&mut self, turn_id: &str, text: &str) -> Result<DriverStep, DriverError>;

    /// Cancel the running turn; the driver still reports the terminal when the
    /// provider acknowledges. Open protocol requests are answered here.
    fn interrupt(&mut self) -> DriverStep;

    /// Answer a pending request with a person's decision.
    fn respond(
        &mut self,
        request_id: &str,
        decision: ApprovalDecision,
    ) -> Result<DriverStep, DriverError>;

    fn on_line(&mut self, line: &str) -> DriverStep;

    fn on_exit(&mut self, _code: Option<i32>) -> DriverStep {
        DriverStep::default()
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
