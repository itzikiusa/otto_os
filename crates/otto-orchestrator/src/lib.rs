//! otto-orchestrator — ⌘K plain English → typed `ActionPlan`. The deterministic
//! literal parser in the SPA handles common intents with no LLM at all; this
//! crate is the AI-fallback planner, which drives a *real* interactive claude
//! session in a PTY (no `-p`), types the prompt into it, and reads the
//! completed reply from claude's session JSONL transcript (see [`claude_pty`]).

pub mod claude_pty;
pub mod e2e_stub;
pub mod parse;

use std::time::Duration;

use otto_core::api::{Action, ActionPlan, OrchestrateReq, OrchestrateResp};
use otto_core::auth::BoxFuture;
use otto_core::domain::{Connection, Session};
use otto_core::{Id, Result};
use serde::Serialize;

use crate::claude_pty::ClaudePty;

/// "Stuck" window for a ⌘K planner turn — NOT a wall-clock cap. The turn may
/// run as long as it keeps making progress; only a stall this long (no
/// transcript growth and no PTY activity) is treated as wedged and retried.
/// Generous because a cold claude spawn takes 25-30s (hook init + MCP load)
/// before the session JSONL even appears.
const CLAUDE_NO_PROGRESS: Duration = Duration::from_secs(120);

/// Model for planning/optimizing turns — haiku keeps them fast and cheap.
const PLANNER_MODEL: &str = "haiku";

/// Providers the model may reference in `spawn_sessions` actions.
const DEFAULT_PROVIDERS: &[&str] = &["claude", "codex", "agy", "shell"];

/// Workspace context given to the planner (and used for plan validation).
pub struct OrchestratorContext {
    pub sessions: Vec<Session>,
    pub connections: Vec<Connection>,
    /// Working directory the planner claude session runs in — it also
    /// determines which `~/.claude/projects/<enc>` dir its JSONL lands in.
    pub cwd: String,
    /// The effective default agent CLI for this workspace (per-workspace
    /// default, else global, else "claude"). Steers `spawn_sessions` when the
    /// user doesn't name a specific CLI.
    pub default_provider: String,
}

/// Plain-English → ActionPlan orchestrator backed by real claude sessions.
pub struct Orchestrator {
    claude: ClaudePty,
}

impl Orchestrator {
    pub fn new(claude_bin: impl Into<String>) -> Self {
        Self {
            claude: ClaudePty::new(claude_bin),
        }
    }

    /// Turn `req.text` into a validated `ActionPlan` (never executes it).
    ///
    /// `optimize`: first rewrites the instruction via a claude turn and
    /// plans from the rewritten text. On planning failure with
    /// `ai_fallback` + a focused session, falls back to a single
    /// `run_command` sending the raw text to the focused session.
    pub async fn orchestrate(
        &self,
        req: OrchestrateReq,
        ctx: OrchestratorContext,
    ) -> Result<OrchestrateResp> {
        let mut optimized_text: Option<String> = None;
        if req.optimize {
            match self.optimize_prompt(&req.text, &ctx.cwd).await {
                Ok(text) if !text.trim().is_empty() => optimized_text = Some(text),
                Ok(_) => {}
                Err(e) => tracing::warn!("prompt optimize failed, using original text: {e}"),
            }
        }
        let plan_input = optimized_text.as_deref().unwrap_or(&req.text);

        let allowed: Vec<String> = DEFAULT_PROVIDERS.iter().map(|s| s.to_string()).collect();
        let planned: Result<ActionPlan> = async {
            let prompt = build_plan_prompt(plan_input, &ctx);
            let text = self
                .claude
                .run_prompt(&prompt, &ctx.cwd, Some(PLANNER_MODEL), CLAUDE_NO_PROGRESS)
                .await?;
            let plan = parse::parse_plan(&text)?;
            parse::validate_plan(&plan, &ctx, &allowed)?;
            Ok(plan)
        }
        .await;

        match planned {
            Ok(plan) => Ok(OrchestrateResp {
                plan,
                optimized_text,
            }),
            Err(e) => {
                if req.ai_fallback {
                    if let Some(focused) = req.focused_session_id.clone() {
                        tracing::info!("plan parse failed ({e}); using ai_fallback");
                        return Ok(OrchestrateResp {
                            plan: vec![Action::RunCommand {
                                session_id: focused,
                                text: req.text.clone(),
                            }],
                            optimized_text,
                        });
                    }
                }
                Err(e)
            }
        }
    }

    /// Run a single headless agent turn with the given `prompt` in `cwd` and
    /// return the assistant's full reply text. Used by the PR review workflow
    /// and the swarm planner/recruiter. `no_progress` is the stuck window, not a
    /// wall-clock cap (see [`ClaudePty::run_prompt`]).
    pub async fn run_agent(
        &self,
        prompt: &str,
        cwd: &str,
        model: Option<&str>,
        no_progress: std::time::Duration,
    ) -> otto_core::Result<String> {
        // Deterministic, offline agent replies for Playwright E2E. The throwaway
        // E2E daemon sets OTTO_E2E=1 so feature flows (discovery chat, canvas
        // assist) get a canned, prompt-routed reply instead of spawning a real
        // claude PTY (which the E2E harness deliberately makes fail-fast). Only a
        // truthy value enables it, so `OTTO_E2E=0` can't silently turn it on.
        if matches!(std::env::var("OTTO_E2E").as_deref(), Ok("1") | Ok("true")) {
            return Ok(crate::e2e_stub::canned_reply(prompt));
        }
        self.claude.run_prompt(prompt, cwd, model, no_progress).await
    }

    /// Rewrite an instruction into a precise prompt (returns ONLY the rewrite).
    pub async fn optimize_prompt(&self, text: &str, cwd: &str) -> Result<String> {
        let prompt = format!(
            "Rewrite the following instruction to be a precise, unambiguous prompt \
             for a coding agent. Reply with ONLY the rewritten prompt.\n\n{text}"
        );
        let text = self
            .claude
            .run_prompt(&prompt, cwd, Some(PLANNER_MODEL), CLAUDE_NO_PROGRESS)
            .await?;
        Ok(text.trim().to_string())
    }
}

/// Build the constrained planning prompt: workspace context + the exact
/// serde-tagged JSON schema of `otto_core::api::Action`.
fn build_plan_prompt(text: &str, ctx: &OrchestratorContext) -> String {
    let mut prompt = String::from(
        "You are the orchestrator of a development environment. Convert the user's \
         instruction into a JSON action plan.\n\nWorkspace context:\nSessions:\n",
    );
    if ctx.sessions.is_empty() {
        prompt.push_str("  (none)\n");
    }
    for s in &ctx.sessions {
        prompt.push_str(&format!(
            "  - id={} provider={} title={:?} status={}\n",
            s.id,
            s.provider,
            s.title,
            s.status.as_str()
        ));
    }
    prompt.push_str("Connections:\n");
    if ctx.connections.is_empty() {
        prompt.push_str("  (none)\n");
    }
    for c in &ctx.connections {
        prompt.push_str(&format!(
            "  - id={} name={:?} kind={}\n",
            c.id,
            c.name,
            c.kind.as_str()
        ));
    }
    prompt.push_str(
        "\nRespond with ONLY a JSON array of 1 to 10 action objects, no prose, no \
         markdown fences. Each object is tagged by a string field \"action\" with \
         snake_case variant names. The exact schema (serde tag = \"action\"):\n\
         - {\"action\":\"spawn_sessions\",\"provider\":\"claude\"|\"codex\"|\"shell\",\"count\":<1-255>}\n\
         - {\"action\":\"broadcast\",\"text\":\"<sent to every running agent session>\"}\n\
         - {\"action\":\"open_connection\",\"connection_id\":\"<a connection id from the context>\"}\n\
         - {\"action\":\"run_command\",\"session_id\":\"<a session id from the context>\",\"text\":\"<sent to that session>\"}\n\
         Only reference session/connection ids listed in the context.\n",
    );
    // When the user doesn't name an agent CLI, steer spawn_sessions to the
    // configured default agent (per-workspace, else global, else claude).
    if !ctx.default_provider.trim().is_empty() {
        prompt.push_str(&format!(
            "When the user does not name a specific agent CLI, use \"{}\" as the provider for spawn_sessions.\n",
            ctx.default_provider.trim()
        ));
    }
    prompt.push_str("\nInstruction: ");
    prompt.push_str(text);
    prompt
}

// ---------------------------------------------------------------------------
// Headless CLI exec helper (for codex / agy and other non-claude providers)
// ---------------------------------------------------------------------------

/// Run `program exec_args… prompt` in `cwd` capturing stdout. Returns stdout
/// on success; returns `Error::Upstream` on timeout or non-zero exit.
pub async fn run_cli_exec(
    program: &str,
    exec_args: &[&str],
    prompt: &str,
    cwd: &str,
    timeout: Duration,
) -> otto_core::Result<String> {
    use tokio::process::Command;

    let child_fut = async {
        let output = Command::new(program)
            .args(exec_args)
            .arg(prompt)
            .current_dir(cwd)
            .kill_on_drop(true)
            .output()
            .await
            .map_err(|e| otto_core::Error::Upstream(format!("failed to spawn '{program}': {e}")))?;

        if output.status.success() {
            let stdout = String::from_utf8_lossy(&output.stdout).into_owned();
            Ok(stdout)
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr);
            Err(otto_core::Error::Upstream(format!(
                "'{program}' exited with {}: {}",
                output.status,
                stderr.chars().take(200).collect::<String>()
            )))
        }
    };

    match tokio::time::timeout(timeout, child_fut).await {
        Ok(result) => result,
        Err(_) => Err(otto_core::Error::Upstream(format!(
            "'{program}' timed out after {}s",
            timeout.as_secs()
        ))),
    }
}

// ---------------------------------------------------------------------------
// Deterministic plan executor
// ---------------------------------------------------------------------------

/// Spawns sessions for plan execution (implemented at integration time on
/// top of `SessionManager` / `ConnectionsService`, scoped to one workspace
/// and acting user).
pub trait PlanSpawner: Send + Sync {
    /// Spawn one agent session of `provider`; returns the new session.
    fn spawn_agent<'a>(&'a self, provider: &'a str) -> BoxFuture<'a, Result<Session>>;
    /// Open a saved connection as a new session.
    fn open_connection<'a>(&'a self, connection_id: &'a Id) -> BoxFuture<'a, Result<Session>>;
}

/// Sends text into sessions for plan execution.
pub trait PlanIo: Send + Sync {
    /// Send `text` (a newline is appended by the implementation) to every
    /// running agent session in the workspace; returns the session ids hit.
    fn broadcast<'a>(&'a self, text: &'a str) -> BoxFuture<'a, Result<Vec<Id>>>;
    /// Send `text` to one specific session.
    fn run_command<'a>(&'a self, session_id: &'a Id, text: &'a str) -> BoxFuture<'a, Result<()>>;
}

/// Per-action execution result (contract endpoint #24 response item).
#[derive(Debug, Clone, Serialize)]
pub struct ActionResult {
    pub action_index: usize,
    pub ok: bool,
    pub detail: String,
    pub session_ids: Vec<Id>,
}

/// Contract endpoint #24 response: `{"results":[...]}`.
#[derive(Debug, Clone, Serialize)]
pub struct ExecuteResp {
    pub results: Vec<ActionResult>,
}

/// Execute a confirmed plan deterministically, action by action. Failures
/// are recorded per-action; execution continues with the next action.
pub async fn execute(plan: &ActionPlan, spawner: &dyn PlanSpawner, io: &dyn PlanIo) -> ExecuteResp {
    let mut results = Vec::with_capacity(plan.len());
    for (action_index, action) in plan.iter().enumerate() {
        let result = match action {
            Action::SpawnSessions { provider, count } => {
                let mut session_ids = Vec::new();
                let mut error = None;
                for _ in 0..*count {
                    match spawner.spawn_agent(provider).await {
                        Ok(session) => session_ids.push(session.id),
                        Err(e) => {
                            error = Some(e);
                            break;
                        }
                    }
                }
                match error {
                    None => ActionResult {
                        action_index,
                        ok: true,
                        detail: format!("spawned {} {provider} session(s)", session_ids.len()),
                        session_ids,
                    },
                    Some(e) => ActionResult {
                        action_index,
                        ok: false,
                        detail: format!(
                            "spawned {} of {count} {provider} session(s): {e}",
                            session_ids.len()
                        ),
                        session_ids,
                    },
                }
            }
            Action::Broadcast { text } => match io.broadcast(text).await {
                Ok(session_ids) => ActionResult {
                    action_index,
                    ok: true,
                    detail: format!("broadcast to {} session(s)", session_ids.len()),
                    session_ids,
                },
                Err(e) => ActionResult {
                    action_index,
                    ok: false,
                    detail: format!("broadcast failed: {e}"),
                    session_ids: vec![],
                },
            },
            Action::OpenConnection { connection_id } => {
                match spawner.open_connection(connection_id).await {
                    Ok(session) => ActionResult {
                        action_index,
                        ok: true,
                        detail: format!("opened connection {connection_id}"),
                        session_ids: vec![session.id],
                    },
                    Err(e) => ActionResult {
                        action_index,
                        ok: false,
                        detail: format!("open connection failed: {e}"),
                        session_ids: vec![],
                    },
                }
            }
            Action::RunCommand { session_id, text } => {
                match io.run_command(session_id, text).await {
                    Ok(()) => ActionResult {
                        action_index,
                        ok: true,
                        detail: "command sent".into(),
                        session_ids: vec![session_id.clone()],
                    },
                    Err(e) => ActionResult {
                        action_index,
                        ok: false,
                        detail: format!("run command failed: {e}"),
                        session_ids: vec![],
                    },
                }
            }
        };
        results.push(result);
    }
    ExecuteResp { results }
}
