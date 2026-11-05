//! The PTY spawn's safety pipeline, applied to a structured-runtime child
//! (`otto-agent-protocol`; design `docs/features/threads-structured-runtime.md`
//! §Safety). Same helpers, same order as [`SessionManager::create`]: folder
//! refusal, trust pre-seed, context provisioning, then the Seatbelt wrap as the
//! very last step over the fully-built spec. What the runtime cannot honour
//! yet (named provider accounts, network profiles) is REFUSED, never skipped.

use super::*;

/// How a structured driver hands MCP servers to its agent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RuntimeMcpDelivery {
    /// The CLI's own launcher config + flags, exactly as its PTY session gets
    /// them (`.mcp.json` / `.agents/mcp_config.json` / codex `-c`), with
    /// Claude additionally pointed at Otto's managed entries via
    /// `--mcp-config` (SDK mode doesn't prompt for project-server approval).
    Launcher,
    /// Over the protocol itself (ACP `session/new|load.mcpServers`): no file
    /// in the work tree, each server's env (its credential) travels with it.
    Protocol,
}

/// One stdio MCP server for [`RuntimeMcpDelivery::Protocol`]. `command` is
/// absolute (ACP requires it).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuntimeMcpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
}

/// Per-session `claude --mcp-config` document (`<tmp>/otto-mcp/<id>.claude-mcp.json`,
/// 0600). Identity-neutral (the token rides the process env), removed with
/// the session's other credentials.
fn claude_mcp_config_path(session_id: &Id) -> std::path::PathBuf {
    std::env::temp_dir()
        .join("otto-mcp")
        .join(format!("{session_id}.claude-mcp.json"))
}

/// `command` as an absolute path: kept when it already names a path, else
/// looked up on `$PATH` (plus Homebrew's prefixes — a launchd daemon's PATH
/// is minimal). `None` when it doesn't resolve.
fn absolute_command(command: &str) -> Option<String> {
    if command.contains('/') {
        return Some(command.to_string());
    }
    let mut dirs: Vec<std::path::PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    dirs.extend(["/opt/homebrew/bin", "/usr/local/bin"].map(std::path::PathBuf::from));
    dirs.into_iter()
        .map(|d| d.join(command))
        .find(|p| crate::providers::resolves_on_path(&p.to_string_lossy()))
        .map(|p| p.to_string_lossy().into_owned())
}

impl SessionManager {
    /// Prepare `spec` (the runtime driver's launch command) for spawning as
    /// agent session `session` in `ws`. `tail_args` go after the context
    /// hook's injected flags (e.g. `agent stdio` for `grok`). `session` is a synthetic, never
    /// persisted [`Session`] (`kind = Agent`) carrying the runtime session's
    /// id, provider, cwd, owner and meta — exactly the fields the PTY path's
    /// helpers read.
    ///
    /// MCP (`delivery`): Otto's `otto` tool server (when the workspace has it
    /// on — a fresh per-session token each spawn) and the workspace's enabled
    /// user servers, the same set a terminal session gets. `Launcher` applies
    /// them to `spec` and returns nothing; `Protocol` returns them for the
    /// driver's handshake.
    ///
    /// Errors: the cwd is refused (`/`, `$HOME` while confined, the data dir —
    /// [`Self::check_session_cwd`]); the daemon is shutting down; the session
    /// asks for a named provider account or the workspace selects a network
    /// profile (not supported by the runtime yet — refused so traffic and
    /// credentials never silently take a different path than a PTY session's).
    pub async fn prepare_runtime_spawn(
        &self,
        ws: &Workspace,
        session: &Session,
        spec: &mut CommandSpec,
        tail_args: Vec<String>,
        delivery: RuntimeMcpDelivery,
    ) -> Result<Vec<RuntimeMcpServer>> {
        self.refuse_while_shutting_down()?;
        if session.kind != SessionKind::Agent {
            return Err(Error::Invalid("runtime sessions are agent sessions".into()));
        }
        if session.meta.get("account_id").is_some_and(|v| !v.is_null()) {
            return Err(Error::Invalid(
                "named provider accounts are not supported for structured sessions yet".into(),
            ));
        }
        if self.networks.prepare(session).await?.is_some() {
            return Err(Error::Invalid(
                "this workspace routes agents through a network profile, which structured \
                 sessions do not support yet — use a terminal session"
                    .into(),
            ));
        }
        let read_only = session.meta.get("read_only").and_then(|v| v.as_bool()) == Some(true);
        let unconfined = !read_only && !self.process_sandbox_enabled().await;
        self.check_session_cwd(ws, &session.cwd, unconfined)?;
        let _ = std::fs::create_dir_all(&session.cwd);
        crate::trust::ensure_trusted_async(&session.provider, &session.cwd).await;
        // MCP before the context hook — the PTY spawn's order.
        let _mcp_activation = crate::mcp::activation_gate().read().await;
        let protocol_mcp = match delivery {
            RuntimeMcpDelivery::Launcher => {
                self.apply_launcher_mcp(session, spec).await;
                Vec::new()
            }
            RuntimeMcpDelivery::Protocol => self.protocol_mcp_servers(session).await,
        };
        if let Some(hook) = &self.pre_spawn_hook {
            let hook = Arc::clone(hook);
            let ws_owned = ws.clone();
            let cwd = session.cwd.clone();
            let prov = session.provider.clone();
            let injection =
                tokio::task::spawn_blocking(move || hook.before_spawn(&ws_owned, &cwd, &prov))
                    .await
                    .unwrap_or_default();
            spec.args.extend(injection.args);
            spec.env.extend(injection.env);
        }
        // A read-only session's tool list (claude: no shell, no subagents) —
        // what its terminal spawn adds; `read_only_strip` has nothing to drop
        // from a structured driver's argv.
        spec.args
            .extend(cli_flags::read_only_args(&session.provider, &session.meta));
        // A subcommand that must follow the injected top-level flags.
        spec.args.extend(tail_args);
        // Last, over the fully-built spec — as `create` does.
        self.apply_sandbox(spec, session).await;
        Ok(protocol_mcp)
    }

    /// [`RuntimeMcpDelivery::Launcher`]: the PTY spawn's MCP steps, plus
    /// `--mcp-config` for Claude.
    async fn apply_launcher_mcp(&self, session: &Session, spec: &mut CommandSpec) {
        let otto_tools = self.maybe_enable_otto_tools(session).await;
        let codex_otto = !otto_tools.args.is_empty();
        spec.args.extend(otto_tools.args);
        spec.env.extend(otto_tools.env);
        let codex_user_args = self
            .sync_workspace_mcp(session, otto_tools.server, codex_otto)
            .await;
        spec.args.extend(codex_user_args);
        if session.provider != "claude" {
            return;
        }
        let cwd = session.cwd.clone();
        let path = claude_mcp_config_path(&session.id);
        let written = tokio::task::spawn_blocking(move || -> std::result::Result<_, String> {
            let Some(doc) = crate::mcp::managed_servers_document(&cwd)? else {
                let _ = std::fs::remove_file(&path);
                return Ok(None);
            };
            let env = std::collections::BTreeMap::new();
            // Reuse the private 0600 writer, then replace its body.
            crate::mcp::write_env_file(&path, &env)?;
            std::fs::write(&path, doc).map_err(|e| format!("write {}: {e}", path.display()))?;
            Ok(Some(path))
        })
        .await
        .unwrap_or_else(|e| Err(format!("mcp config task failed: {e}")));
        match written {
            Ok(Some(path)) => spec.args.extend([
                "--mcp-config".to_string(),
                path.to_string_lossy().into_owned(),
            ]),
            Ok(None) => {}
            Err(e) => tracing::warn!(session = %session.id, "claude --mcp-config: {e}"),
        }
    }

    /// [`RuntimeMcpDelivery::Protocol`]: `otto` + the enabled user servers,
    /// each with its own env. Best-effort — a server that can't be resolved
    /// is skipped (logged), never fails the spawn.
    async fn protocol_mcp_servers(&self, session: &Session) -> Vec<RuntimeMcpServer> {
        let mut out = Vec::new();
        if let Some((_token, server)) = self.mint_otto_tools(session).await {
            out.push(RuntimeMcpServer {
                name: crate::mcp::OTTO_TOOLS_KEY.to_string(),
                command: server.command,
                args: server.args,
                env: server.env.into_iter().collect(),
            });
        }
        let (users, lookup_error) = self.resolve_user_mcp_servers(session).await;
        if let Some(e) = lookup_error {
            tracing::warn!(session = %session.id, "enabled MCP servers unavailable: {e}");
        }
        for s in users.into_iter().flatten() {
            if s.name == crate::mcp::SERVER_KEY || s.name == crate::mcp::OTTO_TOOLS_KEY {
                continue;
            }
            let Some(command) = absolute_command(&s.command) else {
                tracing::warn!(session = %session.id, server = %s.name, "MCP server command not found; skipped");
                continue;
            };
            out.push(RuntimeMcpServer {
                name: s.name,
                command,
                args: s.args,
                env: s.env.into_iter().collect(),
            });
        }
        out
    }

    /// A structured session's process ended (exit, close): revoke its `otto`
    /// MCP credential and remove its per-session MCP files, as a PTY session's
    /// retirement does. Every respawn mints a fresh one. Best-effort.
    pub async fn retire_runtime_credentials(&self, session: &Session) {
        self.retire_credentials(session).await;
        let _ = tokio::fs::remove_file(claude_mcp_config_path(&session.id)).await;
    }

    /// Whether the structured child of `session` will run Seatbelt-confined:
    /// a read-only session ALWAYS is (`apply_sandbox`'s forced profile),
    /// otherwise per the `process_sandbox` setting.
    pub async fn runtime_session_sandboxed(&self, session: &Session) -> bool {
        let read_only = session.meta.get("read_only").and_then(|v| v.as_bool()) == Some(true);
        (read_only && otto_sandbox::is_supported())
            || self.runtime_has_process_sandbox(&session.provider).await
    }

    /// Whether `provider` would run Seatbelt-confined (the runtime's drivers
    /// pass this to the provider's own sandbox flags, as
    /// `interaction_mode_apply` does for PTY sessions).
    pub async fn runtime_has_process_sandbox(&self, provider: &str) -> bool {
        self.has_process_sandbox(SessionKind::Agent, provider).await
    }
}
