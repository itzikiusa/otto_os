//! SessionManager — owns live PTYs, the sessions DB rows and per-session
//! status tasks (working/idle/exited detection + events).

use std::sync::Arc;
use std::time::Duration;

use dashmap::DashMap;
use otto_core::api::CreateSessionReq;
use otto_core::domain::{
    Session, SessionKind, SessionStatus, TrailKind, TrailLevel, TrailSource, Workspace,
};
use otto_core::event::Event;
use otto_core::hooks::{McpServerProvider, PreSpawnHook};
use otto_core::{new_id, Error, Id, Result};
use otto_pty::{resolve_grid, CommandSpec, PtyHandle};
use otto_rbac::AuthRepo;
use otto_state::{ActivityRepo, NewSession, NewTrail, SessionsRepo};
use tokio::sync::broadcast;

use crate::providers::ProviderRegistry;

/// Build `["--add-dir", path, ...]` args for providers that support `--add-dir`
/// (claude, codex, agy — NOT shell).  Returns an empty vec for unknown/shell
/// providers or when `meta` has no `extra_dirs` array.
fn add_dir_args(provider: &str, meta: &serde_json::Value) -> Vec<String> {
    if provider == "shell" {
        return vec![];
    }
    let Some(arr) = meta.get("extra_dirs").and_then(|v| v.as_array()) else {
        return vec![];
    };
    let mut out = Vec::with_capacity(arr.len() * 2);
    for item in arr {
        if let Some(dir) = item.as_str() {
            if !dir.is_empty() {
                out.push("--add-dir".to_string());
                out.push(dir.to_string());
            }
        }
    }
    out
}

/// Build `["--model", name]` args when `meta.model` is set and the provider
/// supports an explicit `--model` flag (claude — same flag codex accepts).
/// Returns an empty vec for shell or unknown providers, or when `meta.model`
/// is absent/empty.  The model flag is provider-specific:
///   - claude/codex: `--model <name>`
///   - agy, shell: unsupported — silently omitted (agy has no model flag).
fn model_args(provider: &str, meta: &serde_json::Value) -> Vec<String> {
    if !matches!(provider, "claude" | "codex") {
        return vec![];
    }
    let Some(model) = meta.get("model").and_then(|v| v.as_str()) else {
        return vec![];
    };
    let model = model.trim();
    if model.is_empty() {
        return vec![];
    }
    vec!["--model".to_string(), model.to_string()]
}

/// Truncate `s` to at most `max` chars (char-boundary safe), appending `…`.
/// Used for one-line trail summaries.
fn trail_clip(s: &str, max: usize) -> String {
    let s = s.trim();
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max).collect();
    out.push('…');
    out
}

/// Window after the last output chunk during which a session is `working`.
const WORKING_WINDOW: Duration = Duration::from_secs(5);
/// Status poll interval.
const STATUS_TICK: Duration = Duration::from_secs(2);

/// How long a LIVE resumable session must be idle (no output) AND unattached
/// (no WS viewer) before its PTY is suspended to free RAM. The conversation
/// stays resumable, so reopening it auto-resumes via `--resume`.
pub const SUSPEND_GRACE: Duration = Duration::from_secs(5 * 60);

/// Hook that inspects live PTY output for a session, used by otto-server's
/// credential monitor to detect mid-session re-auth prompts (e.g. "run
/// `claude login`", "session expired").
///
/// Lives here (not in otto-core) to avoid a core dependency. **Best-effort:**
/// implementations MUST handle their own errors and never panic — a scan
/// failure must never disturb the session's status task. `chunk` is a raw PTY
/// output slice (may split lines); implementations should keep their own small
/// rolling context and debounce per session.
pub trait OutputScanner: Send + Sync {
    /// Called for each PTY output chunk. `provider` is the session's CLI
    /// provider ("claude", "codex", "shell", …) used as the re-auth target.
    fn on_output(&self, session_id: &Id, provider: &str, chunk: &[u8]);
}

/// Await the child exit code without holding the (non-Send) watch guard
/// across an await point. `None` when the watch sender was dropped.
pub(crate) async fn wait_exit_code(
    rx: &mut tokio::sync::watch::Receiver<Option<i32>>,
) -> Option<i32> {
    let res = rx.wait_for(|v| v.is_some()).await;
    match res {
        Ok(guard) => Some((*guard).unwrap_or(-1)),
        Err(_) => None,
    }
}

/// RAII guard for a WS terminal attachment: decrements the session's attached-
/// viewer count when dropped, on every WS `serve_terminal` return path.
pub struct AttachGuard {
    manager: Arc<SessionManager>,
    id: Id,
}

impl Drop for AttachGuard {
    fn drop(&mut self) {
        self.manager.detach(&self.id);
    }
}

/// Default daemon base URL agent hooks post activity back to. Overridden via
/// [`SessionManager::with_ingest_base`] (ottod sets it from its bind port).
const DEFAULT_INGEST_BASE: &str = "http://127.0.0.1:7700";

/// Owns live sessions: PTY handles keyed by session id plus persistence.
pub struct SessionManager {
    /// Shared so the per-session status task can evict an exited handle
    /// (otherwise dead PtyHandles — and their emulator + ring buffer — leak).
    live: Arc<DashMap<Id, Arc<PtyHandle>>>,
    /// In-memory count of attached WS terminal viewers per session. Bumped by
    /// `ws::serve_terminal` on attach/detach; read by the idle-suspend sweep so
    /// it never suspends a session someone is actively watching.
    attached: Arc<DashMap<Id, usize>>,
    /// Session ids whose PTY is being deliberately suspended (RAM release, not
    /// a real exit). The per-session status task consults this in its exit
    /// branch so it marks the session `Reconnectable` (still resumable) instead
    /// of `Exited`, winning the kill→exit race deterministically.
    suspending: Arc<DashMap<Id, ()>>,
    repo: SessionsRepo,
    events: broadcast::Sender<Event>,
    providers: ProviderRegistry,
    /// Optional context-provisioning hook, invoked before an agent spawn.
    pre_spawn_hook: Option<Arc<dyn PreSpawnHook>>,
    /// Optional resolver for the workspace's enabled user-configured MCP servers,
    /// merged into `.mcp.json` on agent spawn (alongside Otto's browser entry).
    mcp_servers: Option<Arc<dyn McpServerProvider>>,
    /// Optional live-output scanner (credential monitor's mid-session auth
    /// detection). When set, each session's status task subscribes to its PTY
    /// output and forwards chunks here.
    output_scanner: Option<Arc<dyn OutputScanner>>,
    /// Daemon base URL that injected agent hooks post their activity back to.
    ingest_base: String,
    /// Per-session ingest tokens. An agent's hooks present this token to the
    /// (otherwise unauthenticated) `/ingest/*` endpoints; the route verifies it
    /// against this map. Minted at spawn, dropped when the session is removed.
    ingest_tokens: Arc<DashMap<Id, String>>,
    /// Optional activity store: records Otto-side lifecycle and user actions to
    /// the session trail (so the trail is populated for every provider, not just
    /// ones with native hooks).
    activity: Option<ActivityRepo>,
    /// Per-session forced-disconnect signal. Attached `/ws/term` viewers
    /// subscribe via [`Self::evict_signal`]; [`Self::evict`] fires a unit to all
    /// of them so they immediately send `{"type":"terminated"}` and close.
    /// Created lazily (a session only gets a sender once someone subscribes or
    /// evicts it). A `broadcast` channel is used so every attached viewer is
    /// dropped, not just one — mirrors how `live`/`attached` are keyed by id.
    evict: Arc<DashMap<Id, broadcast::Sender<()>>>,
    /// Optional settings store used to read the configurable idle-suspend grace
    /// period (`idle_suspend_grace_secs`). Falls back to [`SUSPEND_GRACE`] when
    /// absent or when the key is not set.
    settings: Option<otto_state::SettingsRepo>,
    /// Optional auth-token repo. When set (and `otto_mcp_enabled` is on for the
    /// workspace), an agent spawn mints a per-session token for Otto's first-party
    /// read-only MCP tool server (Task B2b) and injects the `otto` entry into
    /// `.mcp.json`. Absent ⇒ the feature is entirely off.
    auth: Option<AuthRepo>,
    /// Absolute path to the `ottod` binary that backs the `otto` MCP tool server
    /// (`<path> mcp-tools`). Defaults to the running executable's own path so the
    /// tools subcommand is always the same build as the daemon.
    mcp_tools_bin: String,
    /// Per-session MCP-token ids (the auth-token row id, NOT the secret), so the
    /// token minted for the `otto` server can be revoked when the session is
    /// removed. Keyed like `ingest_tokens`.
    mcp_tokens: Arc<DashMap<Id, String>>,
}

impl SessionManager {
    pub fn new(
        repo: SessionsRepo,
        events: broadcast::Sender<Event>,
        providers: ProviderRegistry,
    ) -> Self {
        Self {
            live: Arc::new(DashMap::new()),
            attached: Arc::new(DashMap::new()),
            suspending: Arc::new(DashMap::new()),
            repo,
            events,
            providers,
            pre_spawn_hook: None,
            mcp_servers: None,
            output_scanner: None,
            ingest_base: std::env::var("OTTO_INGEST_BASE")
                .ok()
                .filter(|s| !s.trim().is_empty())
                .unwrap_or_else(|| DEFAULT_INGEST_BASE.to_string()),
            ingest_tokens: Arc::new(DashMap::new()),
            activity: None,
            evict: Arc::new(DashMap::new()),
            settings: None,
            auth: None,
            // Default to this daemon's own binary so `mcp-tools` is the same build.
            mcp_tools_bin: std::env::current_exe()
                .ok()
                .and_then(|p| p.to_str().map(str::to_owned))
                .unwrap_or_else(|| "ottod".to_string()),
            mcp_tokens: Arc::new(DashMap::new()),
        }
    }

    /// Attach the activity store so lifecycle + user actions are recorded to the
    /// session trail. Builder-style; without it the recording calls are no-ops.
    pub fn with_activity_repo(mut self, activity: ActivityRepo) -> Self {
        self.activity = Some(activity);
        self
    }

    /// Attach the settings store so the idle-suspend grace period and other
    /// runtime-configurable parameters can be read at sweep time. Builder-style;
    /// without it all parameters fall back to their compiled-in defaults.
    pub fn with_settings_repo(mut self, settings: otto_state::SettingsRepo) -> Self {
        self.settings = Some(settings);
        self
    }

    /// Attach the auth-token repo used to mint the per-session token for the
    /// first-party `otto` MCP tool server (Task B2b). Without it the feature is
    /// off even if `otto_mcp_enabled` is set. Builder-style.
    pub fn with_auth_repo(mut self, auth: AuthRepo) -> Self {
        self.auth = Some(auth);
        self
    }

    /// Override the `ottod` binary path that backs the `otto` MCP tool server
    /// (`<path> mcp-tools`). Defaults to the running executable. Builder-style.
    pub fn with_mcp_tools_bin(mut self, bin: impl Into<String>) -> Self {
        self.mcp_tools_bin = bin.into();
        self
    }

    /// Best-effort: persist a trail entry and broadcast it. Fire-and-forget so
    /// callers (lifecycle methods) never block on the DB. No-op without an
    /// activity store.
    fn record_trail(
        &self,
        session_id: &Id,
        workspace_id: &Id,
        source: TrailSource,
        kind: TrailKind,
        level: TrailLevel,
        summary: String,
    ) {
        let Some(repo) = self.activity.clone() else {
            return;
        };
        let events = self.events.clone();
        let (sid, wid) = (session_id.clone(), workspace_id.clone());
        tokio::spawn(async move {
            let new = NewTrail {
                session_id: sid.clone(),
                workspace_id: wid.clone(),
                source,
                kind,
                level,
                summary,
                detail: None,
            };
            match repo.append_trail(new).await {
                Ok(event) => {
                    let _ = events.send(Event::TrailAppended {
                        workspace_id: wid,
                        session_id: sid,
                        event,
                    });
                }
                Err(e) => tracing::warn!(session = %sid, "record trail: {e}"),
            }
        });
    }

    /// Record an Otto-side lifecycle entry (spawn/suspend/archive/…) for an
    /// agent session. Skips connection sessions to keep the trail agent-focused.
    fn record_lifecycle(&self, session: &Session, summary: impl Into<String>) {
        if session.kind != SessionKind::Agent {
            return;
        }
        self.record_trail(
            &session.id,
            &session.workspace_id,
            TrailSource::Otto,
            TrailKind::Session,
            TrailLevel::Info,
            summary.into(),
        );
    }

    /// Record a user-authored message relayed into a session (channel relay,
    /// orchestrator command). Surfaces the "by user" side of the trail for every
    /// provider. Best-effort; loads the session to resolve its workspace.
    pub async fn record_user_message(&self, session_id: &Id, text: &str) {
        if self.activity.is_none() {
            return;
        }
        let Ok(session) = self.repo.get(session_id).await else {
            return;
        };
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let summary = trail_clip(trimmed, 200);
        self.record_trail(
            session_id,
            &session.workspace_id,
            TrailSource::User,
            TrailKind::Prompt,
            TrailLevel::Info,
            summary,
        );
    }

    /// Record an auto-approved prompt-guard action on the session activity trail.
    /// Called by [`crate::prompt_guard::PromptGuard`] after injecting approval
    /// keys. Best-effort; no-op when the activity store is absent.
    pub fn record_approval_trail(&self, session_id: &Id, provider: &str) {
        let summary = format!("Auto-approved trust/permission prompt for {provider}");
        let repo = self.repo.clone();
        let sid = session_id.clone();
        let activity = self.activity.clone();
        let events = self.events.clone();
        tokio::spawn(async move {
            let Ok(session) = repo.get(&sid).await else {
                return;
            };
            let Some(activity) = activity else {
                return;
            };
            let new = NewTrail {
                session_id: sid.clone(),
                workspace_id: session.workspace_id.clone(),
                source: TrailSource::Otto,
                kind: TrailKind::Session,
                level: TrailLevel::Info,
                summary,
                detail: None,
            };
            match activity.append_trail(new).await {
                Ok(event) => {
                    let _ = events.send(Event::TrailAppended {
                        workspace_id: session.workspace_id,
                        session_id: sid,
                        event,
                    });
                }
                Err(e) => tracing::warn!("prompt-guard record trail: {e}"),
            }
        });
    }

    /// Submit a message to a live session as if a human typed it and pressed
    /// Enter. Sends the text inside a bracketed-paste pair (so multi-line text
    /// stays intact and interactive TUIs treat it as pasted content rather than
    /// keystrokes), waits briefly for the TUI to finish handling the paste, then
    /// sends a real carriage return to submit.
    ///
    /// This is the reliable "actually send" path: writing `"{text}\n"` in one
    /// burst makes bracketed-paste TUIs (Claude Code, Codex) treat the trailing
    /// newline as pasted content — it inserts a newline instead of submitting,
    /// so the message is pasted but never sent. Mirrors the handover injector.
    pub async fn submit_text(&self, id: &Id, text: &str) -> Result<()> {
        let paste = format!("\x1b[200~{text}\x1b[201~");
        self.input(id, paste.as_bytes()).await?;
        tokio::time::sleep(Duration::from_millis(200)).await;
        self.input(id, b"\r").await
    }

    /// Relay `text` verbatim to live agent sessions in workspace `ws`. When
    /// `targets` is `Some`, only those sessions are considered; otherwise every
    /// live agent session is. A session is eligible when it is an agent (not a
    /// connection) and its status is live (`Running | Working | Idle`).
    ///
    /// Each message is submitted via [`Self::submit_text`] (paste + Enter) and
    /// recorded on the session trail. Per-session failures are logged and
    /// skipped — they never abort the rest. Returns the ids that received it.
    ///
    /// Deliberately free of any AI/orchestrator involvement: it sends the literal
    /// text, nothing else.
    pub async fn broadcast_message(
        &self,
        ws: &Id,
        text: &str,
        targets: Option<&[Id]>,
    ) -> Result<Vec<Id>> {
        let sessions = self.list_by_workspace(ws).await?;
        let mut hit = Vec::new();
        for s in sessions {
            let live = matches!(
                s.status,
                SessionStatus::Running | SessionStatus::Working | SessionStatus::Idle
            );
            let targeted = targets.is_none_or(|ids| ids.iter().any(|t| t == &s.id));
            if s.kind == SessionKind::Agent && live && targeted {
                if let Err(e) = self.submit_text(&s.id, text).await {
                    tracing::warn!(session = %s.id, "broadcast failed: {e}");
                    continue;
                }
                self.record_user_message(&s.id, text).await;
                hit.push(s.id);
            }
        }
        Ok(hit)
    }

    /// Prune the activity trail to the newest `keep_per_session` rows per
    /// session. No-op without an activity store. Returns rows pruned.
    pub async fn prune_activity_trail(&self, keep_per_session: i64) -> u64 {
        let Some(repo) = self.activity.as_ref() else {
            return 0;
        };
        match repo.prune_trail(keep_per_session).await {
            Ok(n) => n,
            Err(e) => {
                tracing::warn!("prune activity trail: {e}");
                0
            }
        }
    }

    /// Set the daemon base URL agent hooks post activity back to (ottod passes
    /// its actual bind URL). Builder-style.
    pub fn with_ingest_base(mut self, base: impl Into<String>) -> Self {
        let base = base.into();
        if !base.trim().is_empty() {
            self.ingest_base = base;
        }
        self
    }

    /// Attach a pre-spawn hook (context provisioning). Builder-style so existing
    /// `new()` callers (tests, channels) stay unchanged.
    pub fn with_pre_spawn_hook(mut self, hook: Arc<dyn PreSpawnHook>) -> Self {
        self.pre_spawn_hook = Some(hook);
        self
    }

    /// Attach the user-configured MCP-server resolver, merged into `.mcp.json`
    /// on agent spawn. Builder-style; without it no user servers are written.
    pub fn with_mcp_servers(mut self, provider: Arc<dyn McpServerProvider>) -> Self {
        self.mcp_servers = Some(provider);
        self
    }

    /// Attach a live-output scanner (mid-session re-auth detection). Builder-
    /// style so existing `new()` callers stay unchanged.
    pub fn with_output_scanner(mut self, scanner: Arc<dyn OutputScanner>) -> Self {
        self.output_scanner = Some(scanner);
        self
    }

    pub fn providers(&self) -> &ProviderRegistry {
        &self.providers
    }

    /// Verify an agent hook's ingest token for `session_id`. Returns false when
    /// the session has no token (not an agent / not spawned by this daemon) or
    /// the token doesn't match. Used by the unauthenticated `/ingest/*` routes.
    pub fn verify_ingest_token(&self, session_id: &Id, token: &str) -> bool {
        !token.is_empty()
            && self
                .ingest_tokens
                .get(session_id)
                .is_some_and(|t| t.as_str() == token)
    }

    /// Mint (or reuse) the ingest token for `session_id` and return the env vars
    /// that wire an agent's injected hooks back to this daemon. Pushed onto the
    /// spawned PTY's environment so hook subprocesses inherit them.
    fn ingest_env(&self, session_id: &Id) -> Vec<(String, String)> {
        let token = self
            .ingest_tokens
            .entry(session_id.clone())
            .or_insert_with(|| uuid::Uuid::new_v4().simple().to_string())
            .clone();
        vec![
            ("OTTO_INGEST_BASE".to_string(), self.ingest_base.clone()),
            ("OTTO_SESSION_ID".to_string(), session_id.to_string()),
            ("OTTO_INGEST_TOKEN".to_string(), token),
        ]
    }

    /// Is Otto's first-party MCP tool server opted-in for `workspace_id`?
    /// Reads the `otto_mcp_enabled` setting and applies the shared precedence
    /// rules (see [`otto_state::otto_mcp_enabled_for`]); default OFF.
    async fn otto_mcp_enabled(&self, workspace_id: &str) -> bool {
        let Some(settings) = &self.settings else {
            return false;
        };
        let value = settings
            .get(otto_state::OTTO_MCP_ENABLED_KEY)
            .await
            .ok()
            .flatten();
        otto_state::otto_mcp_enabled_for(value.as_ref(), workspace_id)
    }

    /// When the workspace has opted into Otto's first-party tools, mint a
    /// per-session token and inject the read-only `otto` MCP server into the
    /// workspace `.mcp.json` (Task B2b).
    ///
    /// The token is a per-session auth token (the existing token system) minted
    /// for the session's owner; its row id is recorded in `mcp_tokens` so it is
    /// revoked when the session is removed. The tools binary is **read-only by
    /// construction** (it issues only allow-listed GETs), so the token is the
    /// owner's identity confined by what the tools choose to expose. Best-effort:
    /// any failure here is logged and never blocks the spawn.
    async fn maybe_enable_otto_tools(&self, session: &Session) {
        if !self.otto_mcp_enabled(&session.workspace_id).await {
            return;
        }
        let Some(auth) = &self.auth else {
            return; // feature wired off (no token minter)
        };
        // Mint a per-session token for the owner. Labeled so it is identifiable
        // in the token list and revoked on session removal.
        let label = format!("otto-mcp:{}", session.id);
        let (token, info) = match auth.issue_api_token(&session.created_by, Some(&label)).await {
            Ok(pair) => pair,
            Err(e) => {
                tracing::warn!("otto MCP tools: mint token failed: {e}");
                return;
            }
        };
        self.mcp_tokens.insert(session.id.clone(), info.id);

        let mut env = std::collections::BTreeMap::new();
        env.insert("OTTO_MCP_TOKEN".to_string(), token);
        env.insert("OTTO_MCP_BASE".to_string(), self.ingest_base.clone());
        env.insert("OTTO_SESSION_ID".to_string(), session.id.to_string());
        env.insert(
            "OTTO_WORKSPACE_ID".to_string(),
            session.workspace_id.to_string(),
        );
        let server = crate::mcp::OttoToolsServer {
            command: self.mcp_tools_bin.clone(),
            args: vec!["mcp-tools".to_string()],
            env,
        };
        if let Err(e) = crate::mcp::enable_otto_tools(&session.cwd, &server) {
            tracing::warn!("otto MCP tools: write .mcp.json failed: {e}");
        }
    }

    /// Revoke the per-session MCP token minted for `session_id` (if any). Called
    /// from the session-removal path so the `otto` tool server's credential dies
    /// with the session. Best-effort.
    async fn revoke_mcp_token(&self, owner: &Id, session_id: &Id) {
        if let Some((_, token_id)) = self.mcp_tokens.remove(session_id) {
            if let Some(auth) = &self.auth {
                if let Err(e) = auth.revoke_api_token(owner, &token_id).await {
                    tracing::warn!("otto MCP tools: revoke token failed: {e}");
                }
            }
        }
    }

    /// All `(provider_name, update_command)` pairs for providers that have an
    /// update command configured. Delegates to the registry.
    pub fn provider_update_commands(&self) -> Vec<(String, String)> {
        self.providers.update_commands()
    }

    /// Return the resolved program binary for `name`, or `None` if the
    /// provider is not registered. Delegates to the registry.
    pub fn provider_program(&self, name: &str) -> Option<String> {
        self.providers.program_for(name)
    }

    /// Create a session row, spawn its PTY and start the status task.
    ///
    /// `spec_override` is used by connection sessions (the connections crate
    /// prebuilds the full command, including secret env vars). For
    /// `kind=agent` without an override the command comes from the provider
    /// registry. Title default: `"<provider> #N"`; callers that open
    /// connections should pass `req.title = Some(<connection name>)`.
    pub async fn create(
        &self,
        ws: &Workspace,
        user_id: &Id,
        req: CreateSessionReq,
        spec_override: Option<CommandSpec>,
    ) -> Result<Session> {
        let cwd = req.cwd.clone().unwrap_or_else(|| ws.root_path.clone());

        let (provider, mut spec, provider_session_id) = match spec_override {
            Some(spec) => {
                let provider = req
                    .provider
                    .clone()
                    .ok_or_else(|| Error::Invalid("provider is required".into()))?;
                (provider, spec, None)
            }
            None => {
                if req.kind != SessionKind::Agent {
                    return Err(Error::Invalid(
                        "connection sessions are opened via POST /connections/{id}/open".into(),
                    ));
                }
                let provider = req.provider.clone().ok_or_else(|| {
                    Error::Invalid("provider is required for agent sessions".into())
                })?;
                // claude's --session-id flag requires a UUID, so provider
                // session ids are UUIDs (the Otto session id stays a ULID).
                let sid = uuid::Uuid::new_v4().to_string();
                let mut spec = self.providers.build_spec(&provider, &sid, &cwd, false)?;
                // Append --add-dir args from req.meta.extra_dirs
                let meta_val = req
                    .meta.clone()
                    .unwrap_or(serde_json::json!({}));
                spec.args.extend(add_dir_args(&provider, &meta_val));
                spec.args.extend(model_args(&provider, &meta_val));
                let psid = self.providers.supports_resume(&provider).then_some(sid);
                (provider, spec, psid)
            }
        };

        let title = match req.title.clone() {
            Some(t) if !t.is_empty() => t,
            _ => {
                let n = self.repo.count_by_provider(&ws.id, &provider).await? + 1;
                format!("{provider} #{n}")
            }
        };

        let session = self
            .repo
            .create(NewSession {
                workspace_id: ws.id.clone(),
                kind: req.kind,
                provider,
                title,
                cwd,
                provider_session_id,
                connection_id: req.connection_id.clone(),
                created_by: user_id.clone(),
                meta: req.meta.clone().unwrap_or_else(|| serde_json::json!({})),
            })
            .await?;

        // The cwd must exist (a missing dir makes the child fall back to
        // $HOME) and agent CLIs should already trust the workspace folder.
        let _ = std::fs::create_dir_all(&session.cwd);
        if session.kind == SessionKind::Agent {
            crate::trust::ensure_trusted(&session.provider, &session.cwd);
            // Browser tools: wire an MCP browser server into the workspace
            // when the session opted in (meta.browser == true).
            if session
                .meta
                .get("browser")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                if let Err(e) = crate::mcp::enable_browser(&session.cwd) {
                    tracing::warn!("enable browser MCP: {e}");
                }
            }
            // User-configured MCP servers: merge the workspace's *enabled* ones
            // into `.mcp.json` alongside any managed entries. Opt-in by the user
            // (each server is enabled explicitly); best-effort — never blocks spawn.
            if let Some(provider) = &self.mcp_servers {
                let specs = provider.enabled_servers(&session.workspace_id);
                if !specs.is_empty() {
                    let servers: Vec<crate::mcp::UserMcpServer> = specs
                        .into_iter()
                        .map(|s| crate::mcp::UserMcpServer {
                            name: s.name,
                            command: s.command,
                            args: s.args,
                            env: s.env,
                        })
                        .collect();
                    if let Err(e) = crate::mcp::merge_user_servers(&session.cwd, &servers) {
                        tracing::warn!("merge user MCP servers: {e}");
                    }
                }
            }
            // Otto's first-party read-only tool server: when the workspace has
            // opted in (`otto_mcp_enabled`), mint a per-session token and inject
            // the `otto` MCP entry alongside the user/browser servers. Opt-in,
            // best-effort — never blocks spawn.
            self.maybe_enable_otto_tools(&session).await;
            // Otto context provisioning: materialize the workspace's active
            // skills + soul + context into this CLI's native form. Best-effort —
            // the hook logs and swallows its own errors, never blocking spawn.
            //
            // Skipped for PR-review sessions: they all share one repo cwd, so
            // concurrent spawns would serialize on this *synchronous* materialize
            // (leaving one agent stuck "pending"); a focused diff review needs no
            // workspace skills/soul; and provisioning also pollutes the repo with
            // .otto-managed.json / CLAUDE.md.
            let is_review =
                session.meta.get("source").and_then(|v| v.as_str()) == Some("review");
            if !is_review {
                if let Some(hook) = &self.pre_spawn_hook {
                    hook.before_spawn(ws, &session.cwd, &session.provider);
                }
            }
            // Wire this session's injected hooks back to the daemon: the
            // provisioner wrote a hooks config that reads these env vars and
            // posts trail/task activity to the per-session ingest endpoint.
            spec.env.extend(self.ingest_env(&session.id));
        }

        // Restore the saved grid from `pty_cols` / `pty_rows` in the session's
        // metadata (written by `resize()`). Falls back to 80×24 when absent or
        // out-of-range so the very first spawn still gets a sane default.
        let saved_cols = session.meta.get("pty_cols").and_then(|v| v.as_u64()).map(|v| v as u16);
        let saved_rows = session.meta.get("pty_rows").and_then(|v| v.as_u64()).map(|v| v as u16);
        let (grid_cols, grid_rows) = resolve_grid(saved_cols, saved_rows);

        let handle = match PtyHandle::spawn_sized(&spec, grid_cols, grid_rows) {
            Ok(h) => Arc::new(h),
            Err(e) => {
                let _ = self.repo.delete(&session.id).await;
                return Err(e);
            }
        };

        self.live.insert(session.id.clone(), Arc::clone(&handle));
        self.start_status_task(
            session.id.clone(),
            session.workspace_id.clone(),
            session.provider.clone(),
            handle,
        );
        let _ = self.events.send(Event::SessionCreated {
            session: session.clone(),
        });
        self.record_lifecycle(&session, format!("Session started · {}", session.provider));
        Ok(session)
    }

    /// Load one session from the DB.
    pub async fn get(&self, id: &Id) -> Result<Session> {
        self.repo.get(id).await
    }

    /// All sessions of a workspace from the DB.
    pub async fn list_by_workspace(&self, ws: &Id) -> Result<Vec<Session>> {
        self.repo.list_by_workspace(ws).await
    }

    /// Sessions of a workspace owned by `user_id` — the owner-scoped variant
    /// used to list only the caller's own sessions for non-admins (#L1).
    pub async fn list_by_workspace_for_user(&self, ws: &Id, user_id: &Id) -> Result<Vec<Session>> {
        self.repo.list_by_workspace_for_user(ws, user_id).await
    }

    /// True when the session has a live PTY in this daemon process.
    pub fn is_live(&self, id: &Id) -> bool {
        self.live.contains_key(id)
    }

    /// Register a WS terminal viewer for `id` (called on attach). Returns an
    /// [`AttachGuard`] that decrements the count on drop, so every WS exit path
    /// (clean close, error, drop) releases the attachment.
    pub fn attach(self: &Arc<Self>, id: &Id) -> AttachGuard {
        *self.attached.entry(id.clone()).or_insert(0) += 1;
        AttachGuard {
            manager: Arc::clone(self),
            id: id.clone(),
        }
    }

    /// Decrement the attached-viewer count for `id`, removing the entry at zero.
    fn detach(&self, id: &Id) {
        if let Some(mut e) = self.attached.get_mut(id) {
            *e = e.saturating_sub(1);
            if *e == 0 {
                drop(e);
                self.attached.remove_if(id, |_, &v| v == 0);
            }
        }
    }

    /// Number of WS terminal viewers currently attached to `id`.
    pub fn attached_count(&self, id: &Id) -> usize {
        self.attached.get(id).map(|e| *e).unwrap_or(0)
    }

    /// True when at least one WS viewer is attached to `id`.
    pub fn is_attached(&self, id: &Id) -> bool {
        self.attached_count(id) > 0
    }

    /// Subscribe to the per-session forced-disconnect signal, lazily creating
    /// the broadcast sender for `id` if it doesn't exist yet (mirrors how the
    /// `attached` map's entry is created on demand). The attached `/ws/term`
    /// loop selects on the returned receiver; on [`Self::evict`] it sends a
    /// `{"type":"terminated"}` frame and closes the socket. Capacity is tiny —
    /// the channel only ever carries unit signals.
    pub fn evict_signal(&self, id: &Id) -> broadcast::Receiver<()> {
        self.evict
            .entry(id.clone())
            .or_insert_with(|| broadcast::channel(8).0)
            .subscribe()
    }

    /// Fire the forced-disconnect signal for `id`: every attached viewer that
    /// subscribed via [`Self::evict_signal`] is dropped. A no-op when no sender
    /// exists (no one ever subscribed); the "no receivers" send error is ignored
    /// (all viewers already detached). Used by admin terminate (Task 4.2) and
    /// mobile share-link revoke to kick live `/ws/term` viewers immediately.
    pub fn evict(&self, id: &Id) {
        if let Some(tx) = self.evict.get(id) {
            // Err means no live receivers — harmless, nothing to evict.
            let _ = tx.send(());
        }
    }

    /// Ensure the session is live, resuming it if it is an exited-but-resumable
    /// agent session. A no-op when the session is already live or cannot be
    /// resumed. Errors are logged and suppressed so callers (WS attach) can
    /// proceed optimistically.
    pub async fn ensure_live(&self, id: &Id) -> Result<()> {
        if self.is_live(id) {
            return Ok(());
        }
        let session = self.repo.get(id).await?;
        let resumable = session.kind == SessionKind::Agent
            && session.provider_session_id.is_some()
            && self.providers.supports_resume(&session.provider);
        if resumable {
            self.restart(id, None).await.map(|_| ())?;
        }
        Ok(())
    }

    /// The live PTY handle, when the session has one in this daemon.
    pub fn live_handle(&self, id: &Id) -> Option<Arc<PtyHandle>> {
        self.live.get(id).map(|h| Arc::clone(&h))
    }

    /// Write input bytes to a live session.
    pub async fn input(&self, id: &Id, data: &[u8]) -> Result<()> {
        let handle = self
            .live_handle(id)
            .ok_or_else(|| Error::Conflict("session is not live".into()))?;
        handle.write(data)
    }

    /// Resize a live session's terminal.
    pub async fn resize(&self, id: &Id, cols: u16, rows: u16) -> Result<()> {
        let handle = self
            .live_handle(id)
            .ok_or_else(|| Error::Conflict("session is not live".into()))?;
        handle.resize(cols, rows)?;
        // Persist the last known grid size so resume/reconnect can restore it
        // (prevents reflow flash on reconnect). Best-effort — no await.
        let repo = self.repo.clone();
        let sid = id.clone();
        tokio::spawn(async move {
            if let Ok(session) = repo.get(&sid).await {
                let mut base = match session.meta {
                    serde_json::Value::Object(m) => m,
                    _ => serde_json::Map::new(),
                };
                base.insert("pty_cols".to_string(), serde_json::Value::from(cols));
                base.insert("pty_rows".to_string(), serde_json::Value::from(rows));
                let _ = repo.set_meta(&sid, &serde_json::Value::Object(base)).await;
            }
        });
        Ok(())
    }

    /// Rename a session.
    pub async fn update_title(&self, id: &Id, title: &str) -> Result<Session> {
        self.repo.set_title(id, title).await?;
        self.repo.get(id).await
    }

    /// Shallow-merge `patch` (a JSON object) into the session's existing meta.
    /// Top-level null values in the patch remove that key. Non-object existing
    /// meta is replaced by an empty object before merging.
    pub async fn update_meta(&self, id: &Id, patch: serde_json::Value) -> Result<Session> {
        let session = self.repo.get(id).await?;
        let mut base = match session.meta {
            serde_json::Value::Object(m) => m,
            _ => serde_json::Map::new(),
        };
        if let serde_json::Value::Object(patch_map) = patch {
            for (k, v) in patch_map {
                if v.is_null() {
                    base.remove(&k);
                } else {
                    base.insert(k, v);
                }
            }
        }
        let merged = serde_json::Value::Object(base);
        self.repo.set_meta(id, &merged).await?;
        let updated = self.repo.get(id).await?;
        let _ = self.events.send(Event::SessionMetaUpdated {
            session_id: updated.id.clone(),
            workspace_id: updated.workspace_id.clone(),
            meta: updated.meta.clone(),
        });
        Ok(updated)
    }

    /// Kill the PTY (if live) and mark the session exited.
    pub async fn kill_session(&self, id: &Id) -> Result<()> {
        let session = self.repo.get(id).await?;
        if let Some(handle) = self.live_handle(id) {
            let _ = handle.kill();
        }
        self.repo.update_status(id, SessionStatus::Exited).await?;
        self.record_lifecycle(&session, "Killed");
        let _ = self.events.send(Event::SessionStatus {
            session_id: id.clone(),
            workspace_id: session.workspace_id,
            status: SessionStatus::Exited,
        });
        Ok(())
    }

    /// Suspend a session: release its RAM-holding PTY **without losing the
    /// session**. The conversation stays resumable, so reopening it (WS attach
    /// → `ensure_live` → `restart --resume`) brings it right back.
    ///
    /// Only meaningful for resumable agent sessions; callers (the idle-suspend
    /// sweep) gate on `supports_resume`. The row is kept (incl.
    /// `provider_session_id`); the session ends up `Reconnectable`.
    ///
    /// Status-race handling: killing the handle makes the per-session status
    /// task's exit branch fire, which would normally write `Exited`. We mark
    /// the id in `suspending` *before* killing so that branch writes
    /// `Reconnectable` instead. We also set `Reconnectable` here directly, so
    /// the final status is correct regardless of which path wins.
    pub async fn suspend(&self, id: &Id) -> Result<()> {
        let session = self.repo.get(id).await?;
        // Mark as suspending so the status task's exit branch chooses
        // Reconnectable over Exited. Cleared by that branch (or below).
        self.suspending.insert(id.clone(), ());
        if let Some((_, handle)) = self.live.remove(id) {
            let _ = handle.kill();
        }
        // Authoritatively set Reconnectable (idempotent with the status task).
        self.repo
            .update_status(id, SessionStatus::Reconnectable)
            .await?;
        // Drop the suspend flag last; the status task only reads it, and a late
        // read after this point is harmless (it would also pick Reconnectable).
        self.suspending.remove(id);
        self.record_lifecycle(&session, "Suspended (idle — freed memory, still resumable)");
        let _ = self.events.send(Event::SessionStatus {
            session_id: id.clone(),
            workspace_id: session.workspace_id,
            status: SessionStatus::Reconnectable,
        });
        Ok(())
    }

    /// One sweep of the idle-suspend policy: suspend every LIVE session that is
    /// resumable, idle (no output for ≥ [`SUSPEND_GRACE`]) and has no attached
    /// WS viewer. Working sessions, attached sessions and non-resumable
    /// providers are never touched. Returns the number suspended.
    ///
    /// Resilient: a failure on one session is logged and skipped; the loop
    /// never panics or aborts.
    pub async fn suspend_idle_unattached(&self) -> usize {
        // Read the configurable grace period from settings; fall back to the
        // compiled-in default when not set or when the key is absent.
        let grace = if let Some(ref sr) = self.settings {
            match sr.get("idle_suspend_grace_secs").await {
                Ok(Some(v)) => v
                    .as_u64()
                    .map(Duration::from_secs)
                    .unwrap_or(SUSPEND_GRACE),
                _ => SUSPEND_GRACE,
            }
        } else {
            SUSPEND_GRACE
        };

        // Snapshot live ids first (don't hold DashMap refs across awaits).
        let candidates: Vec<(Id, std::time::Instant)> = self
            .live
            .iter()
            .map(|e| (e.key().clone(), e.value().last_output_at()))
            .collect();

        let mut suspended = 0;
        for (id, last_output) in candidates {
            // Idle: no PTY output for the full grace window.
            if last_output.elapsed() < grace {
                continue;
            }
            // Unattached: nobody is watching the terminal right now.
            if self.is_attached(&id) {
                continue;
            }
            let session = match self.repo.get(&id).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!(session = %id, "idle-suspend: load failed: {e}");
                    continue;
                }
            };
            // Per-session keep-alive: never auto-suspend sessions pinned by the user.
            if session
                .meta
                .get("keep_alive")
                .and_then(|v| v.as_bool())
                .unwrap_or(false)
            {
                continue;
            }
            // Only resumable agent sessions — never lose work for a provider
            // that can't be resumed (codex/agy/shell).
            let resumable = session.kind == SessionKind::Agent
                && session.provider_session_id.is_some()
                && self.providers.supports_resume(&session.provider);
            if !resumable {
                continue;
            }
            match self.suspend(&id).await {
                Ok(()) => {
                    suspended += 1;
                    tracing::info!(
                        session = %id,
                        provider = %session.provider,
                        title = %session.title,
                        "suspended idle, unattached session (freed PTY; stays resumable)"
                    );
                }
                Err(e) => tracing::warn!(session = %id, "idle-suspend failed: {e}"),
            }
        }
        suspended
    }

    /// Existence-check pruner: for non-live agent sessions with a
    /// `provider_session_id`, verify the provider's on-disk transcript still
    /// exists. If it is positively gone (un-resumable) → delete the row. If it
    /// still exists, or existence cannot be determined → keep the session.
    ///
    /// We only ever delete what we can positively confirm is gone. `home` is
    /// the user's home dir ($HOME); when unset we keep everything.
    ///
    /// Resilient: per-session failures are logged and skipped. Returns the
    /// number of rows pruned.
    pub async fn prune_dead_sessions(&self) -> usize {
        let home = match std::env::var("HOME") {
            Ok(h) if !h.is_empty() => std::path::PathBuf::from(h),
            _ => {
                tracing::warn!("prune: HOME unset; skipping existence-check prune");
                return 0;
            }
        };
        let candidates = match self.repo.list_prunable_agent_sessions().await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("prune: list prunable sessions failed: {e}");
                return 0;
            }
        };
        let mut pruned = 0;
        for s in candidates {
            // Never prune a session that became live again between the query
            // and now (e.g. someone reopened it).
            if self.is_live(&s.id) {
                continue;
            }
            let Some(psid) = s.provider_session_id.as_deref() else {
                continue;
            };
            let verdict = crate::lifecycle::check_resumability(&home, &s.provider, &s.cwd, psid);
            match verdict {
                crate::lifecycle::Resumability::Gone => {
                    match self.remove(&s.id).await {
                        Ok(()) => {
                            pruned += 1;
                            tracing::info!(
                                session = %s.id,
                                provider = %s.provider,
                                title = %s.title,
                                "pruned un-resumable session (provider transcript gone)"
                            );
                        }
                        Err(e) => tracing::warn!(session = %s.id, "prune remove failed: {e}"),
                    }
                }
                // Exists or Unknown → keep. We never prune what we can't verify.
                crate::lifecycle::Resumability::Exists
                | crate::lifecycle::Resumability::Unknown => {}
            }
        }
        pruned
    }

    /// Kill every live PTY and mark the sessions exited. Used when the app
    /// closes (no orphaned agent processes left running) and on daemon
    /// shutdown. Returns the number of sessions terminated.
    pub async fn shutdown_all(&self) -> usize {
        let ids: Vec<Id> = self.live.iter().map(|e| e.key().clone()).collect();
        let count = ids.len();
        for id in ids {
            if let Some((_, handle)) = self.live.remove(&id) {
                let _ = handle.kill();
            }
            // Best-effort status update; ignore errors during shutdown.
            let _ = self.repo.update_status(&id, SessionStatus::Exited).await;
            if let Ok(s) = self.repo.get(&id).await {
                let _ = self.events.send(Event::SessionStatus {
                    session_id: id.clone(),
                    workspace_id: s.workspace_id,
                    status: SessionStatus::Exited,
                });
            }
        }
        count
    }

    /// Number of live (running) sessions.
    pub fn live_count(&self) -> usize {
        self.live.len()
    }

    /// Archive a session: kill its PTY, mark it archived + exited, keep the
    /// row and history. It disappears from the active list (clients hide it)
    /// but can be restored or deleted later.
    pub async fn archive(&self, id: &Id) -> Result<Session> {
        let session = self.repo.get(id).await?;
        if let Some((_, handle)) = self.live.remove(id) {
            let _ = handle.kill();
        }
        self.repo.set_archived(id, true).await?;
        self.repo.update_status(id, SessionStatus::Exited).await?;
        self.record_lifecycle(&session, "Archived");
        // Clients refresh on this event and move the row to the archive.
        let _ = self.events.send(Event::SessionStatus {
            session_id: id.clone(),
            workspace_id: session.workspace_id.clone(),
            status: SessionStatus::Exited,
        });
        self.repo.get(id).await
    }

    /// Auto-archive channel-spawned agent sessions (ticket/chat) idle longer
    /// than `max_idle`, so they don't pile up. A later message in the same
    /// conversation spawns a fresh session. Returns the number archived.
    pub async fn reap_idle_channel_sessions(&self, max_idle: std::time::Duration) -> usize {
        let stale = match self.repo.list_idle_channel_sessions(max_idle).await {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("reap idle channel sessions: {e}");
                return 0;
            }
        };
        let mut n = 0;
        for s in stale {
            match self.archive(&s.id).await {
                Ok(_) => {
                    n += 1;
                    tracing::info!(session = %s.id, title = %s.title, "reaped idle channel session");
                }
                Err(e) => tracing::warn!(session = %s.id, "reap archive failed: {e}"),
            }
        }
        n
    }

    /// Permanently delete archived channel (ticket/chat) sessions whose last
    /// activity is older than `max_age`, so closed tickets don't accumulate in
    /// the DB forever. Uses [`remove`](Self::remove) so clients drop the row
    /// from the Archived view. Returns the number deleted.
    pub async fn purge_old_archived_channel_sessions(
        &self,
        max_age: std::time::Duration,
    ) -> usize {
        let stale = match self
            .repo
            .list_archived_channel_sessions_older_than(max_age)
            .await
        {
            Ok(v) => v,
            Err(e) => {
                tracing::warn!("purge old archived channel sessions: {e}");
                return 0;
            }
        };
        let mut n = 0;
        for s in stale {
            // Skip anything that became live again between the query and now.
            if self.live.contains_key(&s.id) {
                continue;
            }
            match self.remove(&s.id).await {
                Ok(()) => {
                    n += 1;
                    tracing::info!(session = %s.id, title = %s.title, "purged old archived channel session");
                }
                Err(e) => tracing::warn!(session = %s.id, "purge delete failed: {e}"),
            }
        }
        n
    }

    /// Un-archive a session (it returns to the active list as reconnectable;
    /// agent sessions can then be restarted to resume).
    pub async fn unarchive(&self, id: &Id) -> Result<Session> {
        self.repo.set_archived(id, false).await?;
        self.repo
            .update_status(id, SessionStatus::Reconnectable)
            .await?;
        let session = self.repo.get(id).await?;
        self.record_lifecycle(&session, "Unarchived");
        Ok(session)
    }

    /// Kill the PTY, delete the DB row and emit `SessionRemoved`.
    pub async fn remove(&self, id: &Id) -> Result<()> {
        let session = self.repo.get(id).await?;
        if let Some((_, handle)) = self.live.remove(id) {
            let _ = handle.kill();
        }
        self.repo.delete(id).await?;
        self.ingest_tokens.remove(id);
        // Revoke the per-session token minted for the `otto` MCP tool server, so
        // its read-only credential dies with the session (best-effort).
        self.revoke_mcp_token(&session.created_by, id).await;
        // Drop the per-session disconnect sender; any attached viewers were
        // already evicted by the terminate path before removal.
        self.evict.remove(id);
        let _ = self.events.send(Event::SessionRemoved {
            session_id: id.clone(),
            workspace_id: session.workspace_id,
        });
        Ok(())
    }

    /// Respawn a session. Agent sessions with a `provider_session_id` use the
    /// provider's resume args. Connection sessions need a `spec_override`
    /// (rebuilt by the connections service) — without one this fails.
    pub async fn restart(&self, id: &Id, spec_override: Option<CommandSpec>) -> Result<Session> {
        let session = self.repo.get(id).await?;
        if let Some((_, handle)) = self.live.remove(id) {
            let _ = handle.kill();
        }

        let mut spec = match spec_override {
            Some(s) => s,
            None => {
                if session.kind != SessionKind::Agent {
                    return Err(Error::Invalid(
                        "connection sessions are reopened via their connection".into(),
                    ));
                }
                let resume = session.provider_session_id.is_some()
                    && self.providers.supports_resume(&session.provider);
                let sid = session.provider_session_id.clone().unwrap_or_else(new_id);
                let mut spec =
                    self.providers
                        .build_spec(&session.provider, &sid, &session.cwd, resume)?;
                // Append --add-dir and --model args from session.meta.
                spec.args.extend(add_dir_args(&session.provider, &session.meta));
                spec.args.extend(model_args(&session.provider, &session.meta));
                spec
            }
        };

        let _ = std::fs::create_dir_all(&session.cwd);
        if session.kind == SessionKind::Agent {
            crate::trust::ensure_trusted(&session.provider, &session.cwd);
            // Re-wire the per-session ingest env (the hooks config persists in
            // the workspace from the initial spawn).
            spec.env.extend(self.ingest_env(&session.id));
        }
        // Restore the saved grid — the client will confirm its own size via a
        // Resize frame on connect, but we want the PTY and emulator to agree
        // with what the user last had so the first snapshot is correctly framed.
        let saved_cols = session.meta.get("pty_cols").and_then(|v| v.as_u64()).map(|v| v as u16);
        let saved_rows = session.meta.get("pty_rows").and_then(|v| v.as_u64()).map(|v| v as u16);
        let (grid_cols, grid_rows) = resolve_grid(saved_cols, saved_rows);
        let handle = Arc::new(PtyHandle::spawn_sized(&spec, grid_cols, grid_rows)?);
        self.live.insert(id.clone(), Arc::clone(&handle));
        self.repo.update_status(id, SessionStatus::Running).await?;
        let _ = self.events.send(Event::SessionStatus {
            session_id: id.clone(),
            workspace_id: session.workspace_id.clone(),
            status: SessionStatus::Running,
        });
        self.record_lifecycle(&session, "Session resumed");
        self.start_status_task(id.clone(), session.workspace_id, session.provider, handle);
        self.repo.get(id).await
    }

    /// Daemon-boot restore. We deliberately do NOT respawn agent processes here:
    /// keeping every historical session resident would cost ~200 MB each. Instead
    /// every restorable session is marked `Reconnectable` (0 memory) and resumed
    /// lazily by [`Self::ensure_live`] the moment a client opens it — claude/codex
    /// keep their conversation in the on-disk JSONL, so `--resume` restores it in
    /// full. `_fallback_cwd` is kept for signature stability (used by resume).
    pub async fn restore_all(
        &self,
        _fallback_cwd: &(dyn Fn(&Id) -> Option<String> + Send + Sync),
    ) -> Result<()> {
        for session in self.repo.list_all_restorable().await? {
            self.repo
                .update_status(&session.id, SessionStatus::Reconnectable)
                .await?;
            let _ = self.events.send(Event::SessionStatus {
                session_id: session.id.clone(),
                workspace_id: session.workspace_id.clone(),
                status: SessionStatus::Reconnectable,
            });
        }
        Ok(())
    }

    /// Per-session status task: every 2s classify working/idle from PTY
    /// activity; on exit mark `exited` and stop. When an [`OutputScanner`] is
    /// configured, also spawns a sibling task that streams the PTY's live
    /// output into the scanner (mid-session re-auth detection).
    fn start_status_task(
        &self,
        id: Id,
        workspace_id: Id,
        provider: String,
        handle: Arc<PtyHandle>,
    ) {
        // Mid-session output scan: subscribe to the PTY broadcast and forward
        // chunks to the scanner. Ends when the PTY closes (broadcast Closed).
        if let Some(scanner) = self.output_scanner.clone() {
            let mut rx = handle.subscribe();
            let scan_id = id.clone();
            tokio::spawn(async move {
                loop {
                    match rx.recv().await {
                        Ok(chunk) => scanner.on_output(&scan_id, &provider, &chunk),
                        Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                        Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    }
                }
            });
        }

        let repo = self.repo.clone();
        let events = self.events.clone();
        let live = Arc::clone(&self.live);
        let suspending = Arc::clone(&self.suspending);
        tokio::spawn(async move {
            let mut exit_rx = handle.on_exit();
            let mut current = SessionStatus::Running;
            let mut interval = tokio::time::interval(STATUS_TICK);
            interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = interval.tick() => {
                        let next = if handle.last_output_at().elapsed() < WORKING_WINDOW {
                            SessionStatus::Working
                        } else {
                            SessionStatus::Idle
                        };
                        if next != current {
                            current = next;
                            let _ = repo.update_status(&id, next).await;
                            let _ = events.send(Event::SessionStatus {
                                session_id: id.clone(),
                                workspace_id: workspace_id.clone(),
                                status: next,
                            });
                        }
                    }
                    code = wait_exit_code(&mut exit_rx) => {
                        let _ = code;
                        // Evict the dead handle so its emulator + ring buffer
                        // are dropped (no accumulation across many sessions).
                        live.remove(&id);
                        // If this exit was caused by a deliberate suspend (PTY
                        // killed to free RAM), the session stays resumable: mark
                        // it Reconnectable, not Exited. `suspend()` also writes
                        // Reconnectable authoritatively, so either order is safe.
                        let status = if suspending.contains_key(&id) {
                            SessionStatus::Reconnectable
                        } else {
                            SessionStatus::Exited
                        };
                        let _ = repo.update_status(&id, status).await;
                        let _ = events.send(Event::SessionStatus {
                            session_id: id.clone(),
                            workspace_id: workspace_id.clone(),
                            status,
                        });
                        break;
                    }
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use otto_core::domain::{SessionKind, Workspace};
    use otto_state::NewSession;

    async fn test_manager() -> (Arc<SessionManager>, SessionsRepo, Workspace, Id) {
        // A migrated on-disk sqlite (in a tempdir) via otto-state's opener.
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("test.db");
        let pool = otto_state::open(&db).await.unwrap();
        // Keep the tempdir alive for the whole process (leak is fine in tests).
        std::mem::forget(dir);

        let user = new_id();
        let ws_id = new_id();
        let now = chrono::Utc::now().to_rfc3339();
        sqlx::query("INSERT INTO users (id, username, password_hash, display_name, is_root, created_at) VALUES (?, ?, ?, ?, 0, ?)")
            .bind(&user).bind("u").bind("x").bind("U").bind(&now)
            .execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO workspaces (id, name, root_path, created_at) VALUES (?, ?, ?, ?)")
            .bind(&ws_id).bind("w").bind("/tmp").bind(&now)
            .execute(&pool).await.unwrap();

        let repo = SessionsRepo::new(pool);
        let (events, _rx) = broadcast::channel(16);
        let providers = ProviderRegistry::new(None);
        let mgr = Arc::new(SessionManager::new(repo.clone(), events, providers));
        let ws = Workspace {
            id: ws_id,
            name: "w".into(),
            root_path: "/tmp".into(),
            settings: serde_json::json!({}),
            archived: false,
            created_at: chrono::Utc::now(),
        };
        (mgr, repo, ws, user)
    }

    async fn seed_session(repo: &SessionsRepo, ws: &Workspace, user: &Id, psid: Option<&str>) -> Id {
        let s = repo
            .create(NewSession {
                workspace_id: ws.id.clone(),
                kind: SessionKind::Agent,
                provider: "claude".into(),
                title: "t".into(),
                cwd: "/tmp".into(),
                provider_session_id: psid.map(|s| s.to_string()),
                connection_id: None,
                created_by: user.clone(),
                meta: serde_json::json!({}),
            })
            .await
            .unwrap();
        s.id
    }

    #[tokio::test]
    async fn attach_guard_counts_and_releases() {
        let (mgr, repo, ws, user) = test_manager().await;
        let id = seed_session(&repo, &ws, &user, Some("sid")).await;

        assert_eq!(mgr.attached_count(&id), 0);
        assert!(!mgr.is_attached(&id));

        let g1 = mgr.attach(&id);
        assert_eq!(mgr.attached_count(&id), 1);
        assert!(mgr.is_attached(&id));

        let g2 = mgr.attach(&id);
        assert_eq!(mgr.attached_count(&id), 2);

        drop(g1);
        assert_eq!(mgr.attached_count(&id), 1);
        assert!(mgr.is_attached(&id));

        drop(g2);
        assert_eq!(mgr.attached_count(&id), 0);
        assert!(!mgr.is_attached(&id));
    }

    #[tokio::test]
    async fn suspend_marks_reconnectable_and_keeps_row() {
        let (mgr, repo, ws, user) = test_manager().await;
        let id = seed_session(&repo, &ws, &user, Some("sid-keep")).await;

        // No live PTY in this test; suspend still drives the DB-side outcome.
        mgr.suspend(&id).await.unwrap();

        let s = repo.get(&id).await.unwrap();
        assert_eq!(s.status, SessionStatus::Reconnectable);
        // The session is NOT lost — row and resume id are preserved.
        assert_eq!(s.provider_session_id.as_deref(), Some("sid-keep"));
        // Suspend flag is cleared after the operation.
        assert!(!mgr.suspending.contains_key(&id));
    }

    #[tokio::test]
    async fn idle_suspend_skips_attached_sessions() {
        let (mgr, repo, ws, user) = test_manager().await;
        let id = seed_session(&repo, &ws, &user, Some("sid")).await;
        // Mark it as a (fake) live session so the sweep considers it, but with
        // an attachment so it must be skipped. We can't spawn a real PTY here,
        // so assert the guard semantics the sweep relies on: an attached
        // session reports is_attached == true.
        let _g = mgr.attach(&id);
        assert!(mgr.is_attached(&id));
        // The sweep over live sessions is a no-op (no live PTYs), and the
        // attachment registry it consults is correct.
        assert_eq!(mgr.suspend_idle_unattached().await, 0);
    }

    #[tokio::test]
    async fn evict_signal_fires_to_subscribers() {
        let (mgr, repo, ws, user) = test_manager().await;
        let id = seed_session(&repo, &ws, &user, Some("sid")).await;

        // Two attached viewers each subscribe to the per-session disconnect
        // signal (broadcast so every viewer is dropped, not just one).
        let mut rx1 = mgr.evict_signal(&id);
        let mut rx2 = mgr.evict_signal(&id);

        // Nothing fired yet.
        assert!(rx1.try_recv().is_err());

        // Firing the signal yields a unit to every subscriber.
        mgr.evict(&id);
        assert!(rx1.recv().await.is_ok(), "subscriber 1 must receive evict");
        assert!(rx2.recv().await.is_ok(), "subscriber 2 must receive evict");
    }

    #[tokio::test]
    async fn evict_without_subscribers_is_noop() {
        let (mgr, repo, ws, user) = test_manager().await;
        let id = seed_session(&repo, &ws, &user, None).await;
        // No receivers exist; evict must not panic or error (no-op).
        mgr.evict(&id);
        // A subscriber created afterwards does not see the earlier (lost) send.
        let mut rx = mgr.evict_signal(&id);
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn prune_keeps_session_with_existing_transcript() {
        let (mgr, repo, ws, user) = test_manager().await;
        // Point HOME at a tempdir holding a matching transcript.
        let home = tempfile::tempdir().unwrap();
        let cwd = "/tmp";
        let psid = "exists-1111";
        let proj = home
            .path()
            .join(".claude")
            .join("projects")
            .join(crate::lifecycle::claude_project_dir_name(cwd));
        std::fs::create_dir_all(&proj).unwrap();
        std::fs::write(proj.join(format!("{psid}.jsonl")), b"{}").unwrap();

        let id = seed_session(&repo, &ws, &user, Some(psid)).await;
        repo.update_status(&id, SessionStatus::Reconnectable)
            .await
            .unwrap();

        // Safe in tests: this process owns its env. Set HOME for the check.
        std::env::set_var("HOME", home.path());
        let pruned = mgr.prune_dead_sessions().await;
        assert_eq!(pruned, 0, "existing transcript must be kept");
        assert!(repo.get(&id).await.is_ok());
    }

    // ── Grid-size resolution tests ───────────────────────────────────────────

    /// `resolve_grid` returns the clamped values when both fall in-range.
    #[test]
    fn resolve_grid_in_range() {
        let (c, r) = resolve_grid(Some(132), Some(50));
        assert_eq!(c, 132);
        assert_eq!(r, 50);
    }

    /// Out-of-range cols fall back to the default; rows are accepted when valid.
    #[test]
    fn resolve_grid_cols_out_of_range_falls_back() {
        // cols = 0 is below MIN_COLS (20) → default 80
        let (c, r) = resolve_grid(Some(0), Some(40));
        assert_eq!(c, otto_pty::DEFAULT_COLS, "zero cols should yield default");
        assert_eq!(r, 40);

        // cols = 501 is above MAX_COLS (500) → default 80
        let (c, _) = resolve_grid(Some(501), Some(24));
        assert_eq!(c, otto_pty::DEFAULT_COLS, "oversized cols should yield default");
    }

    /// Rows out-of-range fall back to the default.
    #[test]
    fn resolve_grid_rows_out_of_range_falls_back() {
        let (_, r) = resolve_grid(Some(80), Some(1));
        assert_eq!(r, otto_pty::DEFAULT_ROWS, "rows below MIN_ROWS should yield default");

        let (_, r) = resolve_grid(Some(80), Some(201));
        assert_eq!(r, otto_pty::DEFAULT_ROWS, "rows above MAX_ROWS should yield default");
    }

    /// `None` values yield the defaults.
    #[test]
    fn resolve_grid_none_yields_defaults() {
        let (c, r) = resolve_grid(None, None);
        assert_eq!(c, otto_pty::DEFAULT_COLS);
        assert_eq!(r, otto_pty::DEFAULT_ROWS);
    }

    // ── model_args tests ────────────────────────────────────────────────────

    /// claude with a model set → ["--model", name].
    #[test]
    fn model_args_claude_with_model() {
        let meta = serde_json::json!({ "model": "claude-opus-4-8" });
        let args = model_args("claude", &meta);
        assert_eq!(args, vec!["--model", "claude-opus-4-8"]);
    }

    /// codex also accepts --model.
    #[test]
    fn model_args_codex_with_model() {
        let meta = serde_json::json!({ "model": "gpt-5-codex" });
        let args = model_args("codex", &meta);
        assert_eq!(args, vec!["--model", "gpt-5-codex"]);
    }

    /// agy does not support --model; args must be empty.
    #[test]
    fn model_args_agy_skipped() {
        let meta = serde_json::json!({ "model": "some-model" });
        let args = model_args("agy", &meta);
        assert!(args.is_empty(), "agy has no --model flag");
    }

    /// shell provider → empty regardless of meta.
    #[test]
    fn model_args_shell_skipped() {
        let meta = serde_json::json!({ "model": "some-model" });
        let args = model_args("shell", &meta);
        assert!(args.is_empty(), "shell has no --model flag");
    }

    /// No model in meta → empty vec.
    #[test]
    fn model_args_absent_model_empty() {
        let meta = serde_json::json!({});
        let args = model_args("claude", &meta);
        assert!(args.is_empty(), "no model key should yield no args");
    }

    /// Whitespace-only model is silently skipped.
    #[test]
    fn model_args_blank_model_empty() {
        let meta = serde_json::json!({ "model": "   " });
        let args = model_args("claude", &meta);
        assert!(args.is_empty(), "blank model should yield no args");
    }

    /// Leading/trailing whitespace is trimmed from the model name.
    #[test]
    fn model_args_model_is_trimmed() {
        let meta = serde_json::json!({ "model": "  opus  " });
        let args = model_args("claude", &meta);
        assert_eq!(args, vec!["--model", "opus"]);
    }

    /// A session spawned with saved grid meta reports that size via screen_size().
    #[tokio::test]
    async fn spawn_sized_restores_grid() {
        use otto_pty::{CommandSpec, PtyHandle};
        let spec = CommandSpec {
            program: "/bin/sh".into(),
            args: vec!["-c".into(), "exit 0".into()],
            cwd: None,
            env: vec![],
        };
        let handle = PtyHandle::spawn_sized(&spec, 132, 50).expect("spawn");
        let (rows, cols) = handle.screen_size();
        assert_eq!(cols, 132, "restored cols");
        assert_eq!(rows, 50, "restored rows");
    }
}
