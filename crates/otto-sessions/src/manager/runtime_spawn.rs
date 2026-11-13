//! The PTY spawn's safety pipeline, applied to a structured-runtime child
//! (`otto-agent-protocol`; design `docs/features/threads-structured-runtime.md`
//! §Safety). Same helpers, same order as [`SessionManager::create`]: folder
//! refusal, trust pre-seed, context provisioning, then the Seatbelt wrap as the
//! very last step over the fully-built spec. A named provider account
//! (`meta.account_id`) and a network profile (`meta.network_profile_id`)
//! apply exactly as for a terminal session: the account's home and
//! credentials, the profile's SSH tunnels and proxy env (kept up until the
//! child is retired).

use super::*;

/// The exit signal of each structured child's active network profile: its
/// tunnels stop when the child is retired (or respawned).
static RUNTIME_NETWORK_EXITS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<Id, tokio::sync::watch::Sender<Option<i32>>>>,
> = std::sync::LazyLock::new(Default::default);

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

/// A personal agent's session (`meta.personal_agent`): otto-assistant's
/// `ensure_agent_workspace` wrote its bundle (the workspace context plus the
/// agent's persona, rules and team) just before creating it.
fn carries_persona(meta: &serde_json::Value) -> bool {
    meta.get("personal_agent")
        .and_then(serde_json::Value::as_str)
        .is_some_and(|a| !a.is_empty())
}

/// [`carries_persona`] for a stored session: the terminal spawn paths in
/// `manager.rs` (create and restart) had the same overwrite.
pub(super) fn persona(session: &Session) -> bool {
    carries_persona(&session.meta)
}

/// [`context_injection`] under a short name for `manager.rs`, which may not grow.
pub(super) fn inject(
    hook: &dyn PreSpawnHook,
    ws: &Workspace,
    cwd: &str,
    provider: &str,
    persona: bool,
) -> otto_core::hooks::SpawnInjection {
    context_injection(hook, ws, cwd, provider, persona)
}

/// The context injection of a structured spawn. A persona session launches
/// on the bundle already on disk: re-materializing `(provider, cwd)` from the
/// workspace settings alone would overwrite it and start the agent without
/// its persona. With no bundle to read back it falls back to the workspace
/// context, as every other session gets.
fn context_injection(
    hook: &dyn PreSpawnHook,
    ws: &Workspace,
    cwd: &str,
    provider: &str,
    persona: bool,
) -> otto_core::hooks::SpawnInjection {
    if persona {
        let kept = hook.resume_injection(cwd, provider);
        if !kept.args.is_empty() || !kept.env.is_empty() {
            return kept;
        }
    }
    hook.before_spawn(ws, cwd, provider)
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
    /// [`Self::check_session_cwd`]); the daemon is shutting down; the named
    /// account is unknown, someone else's or another provider's; the network
    /// profile is not usable (another workspace's, archived, its connection
    /// refused) — never a silent fallback to the default account or a direct
    /// connection.
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
        let account = self
            .resolve_account(&session.created_by, &session.provider, &session.meta)
            .await?;
        let network = self.networks.prepare(session).await?;
        let read_only = session.meta.get("read_only").and_then(|v| v.as_bool()) == Some(true);
        let unconfined = !read_only && !self.process_sandbox_enabled().await;
        self.check_session_cwd(ws, &session.cwd, unconfined)?;
        let _ = std::fs::create_dir_all(&session.cwd);
        // A named account's CLI keeps trust in its own home (as `create`).
        if account.is_none() {
            crate::trust::ensure_trusted_async(&session.provider, &session.cwd).await;
        }
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
            let persona = carries_persona(&session.meta);
            // A named account gets its bundle scoped to its home (`create`).
            let context = account
                .as_ref()
                .map(|(_, home)| otto_core::hooks::SessionSpawnContext {
                    namespace: session.id.clone(),
                    provider_home: Some(home.clone()),
                    extra_context_md: String::new(),
                });
            let injection = tokio::task::spawn_blocking(move || match context {
                Some(context) => hook.before_spawn_session(&ws_owned, &cwd, &prov, &context),
                None => context_injection(hook.as_ref(), &ws_owned, &cwd, &prov, persona),
            })
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
        // The PTY spawn's build budget (`manager/resources.rs`); priority and
        // admission stay with the runtime, which spawns the child itself.
        self.apply_machine_budget(session, spec).await;
        if let Some((account, home)) = &account {
            crate::accounts::apply_account(spec, account, home)?;
        }
        if let Some(network) = &network {
            spec.env
                .retain(|(key, _)| !network.env.iter().any(|(mapped, _)| mapped == key));
            spec.env.extend(network.env.clone());
        }
        // The profile's tunnels live until this child is retired.
        let (exit_tx, exit_rx) = tokio::sync::watch::channel(None);
        if network.is_some() {
            RUNTIME_NETWORK_EXITS
                .lock()
                .unwrap()
                .insert(session.id.clone(), exit_tx);
        }
        self.networks.activate(session.id.clone(), network, exit_rx);
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
        if let Some(exit) = RUNTIME_NETWORK_EXITS.lock().unwrap().remove(&session.id) {
            let _ = exit.send(Some(0));
        }
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

#[cfg(test)]
mod persona_tests {
    use super::*;
    use otto_core::hooks::SpawnInjection;
    use serde_json::json;

    /// A bundle on disk (`resume_injection` reads it back) or not; records
    /// whether the spawn re-materialized from the workspace settings.
    struct Hook {
        bundle: bool,
        rematerialized: std::sync::Mutex<bool>,
    }

    impl PreSpawnHook for Hook {
        fn before_spawn(&self, _ws: &Workspace, _cwd: &str, _p: &str) -> SpawnInjection {
            *self.rematerialized.lock().unwrap() = true;
            SpawnInjection {
                args: vec!["--workspace-only".into()],
                env: Vec::new(),
            }
        }
        fn resume_injection(&self, cwd: &str, _p: &str) -> SpawnInjection {
            SpawnInjection {
                args: if self.bundle {
                    vec![format!("--persona-bundle={cwd}")]
                } else {
                    Vec::new()
                },
                env: Vec::new(),
            }
        }
    }

    fn ws() -> Workspace {
        serde_json::from_value(json!({
            "id": "w1", "name": "W", "root_path": "/tmp", "settings": {},
            "archived": false, "created_at": "2026-11-12T00:00:00Z"
        }))
        .unwrap()
    }

    fn hook(bundle: bool) -> Hook {
        Hook {
            bundle,
            rematerialized: std::sync::Mutex::new(false),
        }
    }

    #[test]
    fn a_personal_agent_thread_keeps_the_persona_bundle() {
        assert!(carries_persona(
            &json!({"source": "personal_agent", "personal_agent": "a1"})
        ));
        let h = hook(true);
        let inj = context_injection(&h, &ws(), "/agents/a1", "claude", true);
        assert_eq!(inj.args, vec!["--persona-bundle=/agents/a1".to_string()]);
        assert!(
            !*h.rematerialized.lock().unwrap(),
            "the persona bundle must not be overwritten from the workspace settings"
        );
    }

    #[test]
    fn other_sessions_and_a_missing_bundle_get_the_workspace_context() {
        for meta in [
            json!({}),
            json!({"personal_agent": ""}),
            json!({"personal_agent": 5}),
        ] {
            assert!(!carries_persona(&meta), "{meta}");
        }
        let h = hook(true);
        let inj = context_injection(&h, &ws(), "/repo", "claude", false);
        assert_eq!(inj.args, vec!["--workspace-only".to_string()]);
        assert!(*h.rematerialized.lock().unwrap());
        let h = hook(false);
        let inj = context_injection(&h, &ws(), "/agents/a1", "codex", true);
        assert_eq!(inj.args, vec!["--workspace-only".to_string()]);
    }
}
