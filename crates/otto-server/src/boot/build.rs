//! Phase 2 — construct every module and assemble the [`ServerCtx`].
//!
//! Only construction happens here (plus the few idempotent boot writes the
//! constructors need: provider settings, skill seeding, the scratch
//! workspace). No background task is spawned — that is
//! [`super::spawn_background`].

use std::sync::Arc;

use otto_core::event::Event;
use otto_core::secrets::SecretStore;
use otto_improve::{ImprovementEngine, RealProposalProducer};
use otto_orchestrator::Orchestrator;
use otto_sessions::{ProviderRegistry, SessionManager};
use otto_state::{
    ActivityRepo, DbPool, ImprovementsRepo, SessionsRepo, SettingsRepo, UsersRepo, WorkspacesRepo,
};
use tokio::sync::broadcast;

use super::ctx::CtxParts;
use super::BootConfig;
use crate::monitor::AuthScanner;
use crate::state::ServerCtx;

/// The assembled context plus the boot facts later phases still need.
pub struct BuiltCtx {
    pub ctx: ServerCtx,
    /// The root user (owns sessions spawned for inbound channel/webhook
    /// messages). `None` until onboarding creates one.
    pub root_user_id: Option<String>,
}

/// Build the daemon's full [`ServerCtx`] over an opened `pool`.
pub async fn build_ctx(
    cfg: &BootConfig,
    pool: DbPool,
    secrets: Arc<dyn SecretStore>,
) -> Result<BuiltCtx, String> {
    let (events, _) = broadcast::channel::<Event>(1024);
    wire_approval_change_hook(&events);

    // Module construction (Task A9): provider registry (with settings
    // overrides), session manager, connections service, spawner bridge,
    // git store and the orchestrator.
    let settings = SettingsRepo::new(pool.clone());
    let providers = provider_registry(&settings).await?;
    let (usage, telemetry) = start_usage_and_telemetry(cfg, &settings).await?;
    let context_library = context_library(cfg);
    let manager = session_manager(cfg, &pool, &events, &secrets, providers, &context_library);
    let workspaces = WorkspacesRepo::new(pool.clone());
    ensure_scratch_workspace(cfg, &workspaces).await?;
    // Message Brokers (Kafka) viewer: cluster profiles (secrets in the Keychain)
    // + an rdkafka client pool for overview/topics/peek/produce/groups/metrics.
    let brokers = Arc::new(otto_brokers::BrokersService::new(
        otto_state::BrokerClustersRepo::new(pool.clone()),
        secrets.clone(),
        // Persist an audit row for every broker write (produce/delete/config/offset-reset).
        Some(otto_state::BrokerAuditRepo::new(pool.clone())),
    ));
    let spawner = Arc::new(crate::modules::PtySpawner {
        pool: pool.clone(),
        manager: Arc::clone(&manager),
        workspaces: workspaces.clone(),
    });
    let orchestrator = orchestrator();
    let improve_engine = improve_engine(&pool, &events, &workspaces, &orchestrator, cfg);
    // Agent Swarm: seed role skills + preset souls into the library (only if
    // absent). The Coordinator runtime + scheduler start in spawn_background.
    otto_swarm::presets::seed(&context_library);
    let memory = memory_service(&pool);
    let plugins = plugin_manager(cfg, &pool);
    let root_user_id = root_user_id(&pool).await;
    let channel_bridge = webhook_channel_bridge(&pool, &manager, &workspaces, &root_user_id);

    let ctx = ServerCtx::from_parts(CtxParts {
        pool,
        secrets,
        events,
        version: cfg.version.clone(),
        base_url: format!("http://127.0.0.1:{}", cfg.port),
        data_dir: cfg.data_dir.clone(),
        plugins,
        manager,
        spawner,
        brokers,
        channel_bridge,
        orchestrator,
        improve_engine,
        context_library,
        usage,
        telemetry: Some(telemetry),
        memory,
        proof_media_dir: Some(cfg.data_dir.join("proof-media")),
        // Config is captured now; the Lightpanda sidecar itself is only
        // located/started on the first `/browser/page` request (see
        // `BrowserEngineHandle`) so daemon boot never waits on it.
        browser: Arc::new(crate::routes::browser::BrowserEngineHandle::new(
            std::env::var("OTTO_LIGHTPANDA_BIN").ok(),
            cfg.data_dir.clone(),
        )),
        cache_auth_lookups: true,
    });
    Ok(BuiltCtx { ctx, root_user_id })
}

/// Every MCP approval write (governance pipeline, outward MCP, assistant,
/// live browser) pushes `mcp_approval_changed` — the tray, the MCP badge
/// and Home stop polling `/mcp/approvals` for it.
fn wire_approval_change_hook(events: &broadcast::Sender<Event>) {
    let tx = events.clone();
    otto_state::set_approval_change_hook(move |c| {
        let _ = tx.send(Event::McpApprovalChanged {
            approval_id: c.approval_id,
            workspace_id: c.workspace_id,
            status: c.status,
        });
    });
}

/// The provider registry with the admin's settings applied: per-provider
/// overrides, the permission-bypass opt-out and the excluded providers.
async fn provider_registry(settings: &SettingsRepo) -> Result<ProviderRegistry, String> {
    let provider_overrides = settings
        .get("providers")
        .await
        .map_err(|e| format!("read providers setting: {e}"))?;
    let providers = ProviderRegistry::new(provider_overrides.as_ref());
    // Apply the opt-out for "skip permission prompts" (default ON / unattended).
    // When an admin turns `agent_skip_permissions` off, built-in agent CLIs launch
    // without their bypass flag and fall back to their own ask/auto permission mode.
    let skip_permissions = settings
        .get("agent_skip_permissions")
        .await
        .map_err(|e| format!("read agent_skip_permissions setting: {e}"))?
        .and_then(|v| v.as_bool())
        .unwrap_or(true);
    providers.set_skip_permissions(skip_permissions, provider_overrides.as_ref());
    // Apply the admin's EXCLUDED providers (`disabled_providers`): a JSON array of
    // provider names hidden from every picker (they might have a CLI installed but
    // not want to use it). Specs stay registered so existing sessions still resume.
    let disabled_providers: Vec<String> = settings
        .get("disabled_providers")
        .await
        .map_err(|e| format!("read disabled_providers setting: {e}"))?
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    providers.set_disabled(&disabled_providers);
    Ok(providers)
}

/// Embedded ClickHouse usage + metrics store (config in the `usage` setting;
/// degrades to a no-op when the binary isn't installed) and the opt-in
/// application telemetry riding on it.
async fn start_usage_and_telemetry(
    cfg: &BootConfig,
    settings: &SettingsRepo,
) -> Result<
    (
        Arc<otto_usage::UsageEngine>,
        Arc<otto_telemetry::TelemetryService>,
    ),
    String,
> {
    let usage_config = otto_usage::UsageConfig::from_json(
        settings
            .get("usage")
            .await
            .map_err(|e| format!("read usage setting: {e}"))?
            .as_ref(),
    );
    let usage = otto_usage::UsageEngine::start(usage_config, cfg.data_dir.clone()).await;
    let telemetry_config = settings
        .get("application_telemetry")
        .await
        .map_err(|e| format!("read telemetry setting: {e}"))?
        .and_then(|v| serde_json::from_value::<otto_telemetry::TelemetryConfig>(v).ok())
        .filter(|c| c.validate().is_ok())
        .unwrap_or_default();
    let telemetry = otto_telemetry::TelemetryService::start(
        Arc::clone(&usage),
        cfg.data_dir.clone(),
        telemetry_config,
    )
    .await;
    Ok((usage, telemetry))
}

/// Otto context library (skills/souls/context) under the data dir; the
/// Provisioner materializes a workspace's active set into each CLI at spawn.
fn context_library(cfg: &BootConfig) -> otto_context::Library {
    let context_library = otto_context::Library::new(cfg.data_dir.join("library"));
    // Seed the product-analysis skills into the library (write-if-absent, so user
    // and self-improvement edits are preserved across restarts).
    if let Err(e) = otto_product::seed_skills(&context_library) {
        tracing::warn!("failed to seed product skills: {e}");
    }
    context_library
}

/// The session manager with its output scanners, spawn hook and every
/// optional repo wired in.
fn session_manager(
    cfg: &BootConfig,
    pool: &DbPool,
    events: &broadcast::Sender<Event>,
    secrets: &Arc<dyn SecretStore>,
    providers: ProviderRegistry,
    context_library: &otto_context::Library,
) -> Arc<SessionManager> {
    // Mid-session re-auth detector: scans live PTY output for re-auth prompts
    // and raises a Credential notice. Attached to the manager so each session's
    // status task streams output into it.
    let auth_scanner = AuthScanner::new(pool.clone(), events.clone());
    // Prompt guard: auto-accepts known "trust this folder / approve?" prompts on
    // every session (normal, channel, review) so nothing gets stuck. Composed
    // with the auth scanner since the manager exposes a single scanner slot.
    let prompt_guard = otto_sessions::PromptGuard::new();
    let scanner = otto_sessions::CompositeScanner::new(vec![
        auth_scanner as Arc<dyn otto_sessions::OutputScanner>,
        prompt_guard.clone() as Arc<dyn otto_sessions::OutputScanner>,
    ]);

    // The context provisioner is the single PreSpawnHook every session flows
    // through.
    let provisioner = Arc::new(otto_context::Provisioner::new(context_library.clone()));
    let manager = Arc::new(
        SessionManager::new(SessionsRepo::new(pool.clone()), events.clone(), providers)
            // Runtime-configurable idle-suspend grace + per-session keep-alive pin.
            .with_settings_repo(SettingsRepo::new(pool.clone()))
            .with_provider_accounts(
                otto_state::provider_accounts::ProviderAccountsRepo::new(pool.clone()),
                cfg.data_dir.join("provider-accounts"),
            )
            // Auto-name new agent sessions from the creating user's active theme.
            .with_name_themes_repo(otto_state::NameThemesRepo::new(pool.clone()))
            .with_pre_spawn_hook(provisioner.clone())
            .with_output_scanner(scanner)
            // User-configured MCP servers merged into `.mcp.json` on agent spawn
            // (Keychain-backed secret env values resolved at merge time).
            .with_mcp_servers(Arc::new(
                crate::routes::mcp_servers::DbMcpServerProvider::new(pool.clone(), secrets.clone()),
            ))
            // Agent activity hooks post back to this loopback daemon.
            .with_ingest_base(format!("http://127.0.0.1:{}", cfg.port))
            // First-party read-only MCP tool server (Task B2b): mint a per-session
            // token when `otto_mcp_enabled` is on for the workspace, and inject the
            // `otto` server (runs `ottod mcp-tools`) into the workspace `.mcp.json`.
            .with_auth_repo(otto_rbac::AuthRepo::new(pool.clone()))
            // Record Otto-side lifecycle + user actions to the activity trail.
            .with_activity_repo(ActivityRepo::new(pool.clone()))
            // Sessions the user started run in PTY holders (`ottod pty-holder`)
            // and survive a daemon restart (setting `session_persistence`).
            .with_pty_holders_opt(cfg.pty_holders.clone()),
    );
    // The guard writes keystrokes back via the manager; wire the (weak) handle
    // now that the Arc exists.
    prompt_guard.set_manager(Arc::downgrade(&manager));
    manager
}

/// The system-owned scratch workspace (workspace-less sessions) lives at the
/// daemon user's home; created on first boot, healed on every later one.
/// Never `/`: this root is the default cwd of every workspace-less session
/// AND the folder they are pre-trusted for ("may write anywhere under"), so
/// a bare launchd context with no `$HOME` must fall back to the daemon's own
/// data dir — the same resolution the session manager uses.
async fn ensure_scratch_workspace(
    cfg: &BootConfig,
    workspaces: &WorkspacesRepo,
) -> Result<(), String> {
    let home = dirs::home_dir()
        .unwrap_or_else(|| cfg.data_dir.clone())
        .to_string_lossy()
        .into_owned();
    workspaces
        .ensure_scratch(&home)
        .await
        .map_err(|e| format!("ensure scratch workspace: {e}"))?;
    Ok(())
}

/// The planner drives a real claude session in a PTY; CLAUDE_BIN lets
/// operators point at a non-PATH binary (mirrors loom).
fn orchestrator() -> Arc<Orchestrator> {
    let claude_bin = std::env::var("CLAUDE_BIN")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "claude".to_string());
    Arc::new(Orchestrator::new(claude_bin))
}

/// Self-improvement engine: reuses the orchestrator's claude driver to run
/// the analysis agent, and shares the event bus so run/edit events reach the
/// /ws/events stream.
fn improve_engine(
    pool: &DbPool,
    events: &broadcast::Sender<Event>,
    workspaces: &WorkspacesRepo,
    orchestrator: &Arc<Orchestrator>,
    cfg: &BootConfig,
) -> Arc<ImprovementEngine> {
    Arc::new(ImprovementEngine {
        improvements: ImprovementsRepo::new(pool.clone()),
        sessions: SessionsRepo::new(pool.clone()),
        workspaces: workspaces.clone(),
        producer: Arc::new(RealProposalProducer::new(Arc::clone(orchestrator))),
        events: events.clone(),
        library_root: cfg.data_dir.join("library"),
    })
}

/// Memory backend: local SQLite by default, or a shared host Otto when
/// OTTO_MEMORY_REMOTE_URL is set (so a team shares one memory across
/// machines); optionally with an Obsidian-vault write-through.
fn memory_service(pool: &DbPool) -> Arc<otto_memory::MemoryService> {
    let memory = match std::env::var("OTTO_MEMORY_REMOTE_URL") {
        Ok(url) if !url.trim().is_empty() => {
            let token = std::env::var("OTTO_MEMORY_REMOTE_TOKEN").unwrap_or_default();
            tracing::info!("memory: shared remote backend at {url}");
            Arc::new(otto_memory::MemoryService::remote(pool.clone(), url, token))
        }
        _ => Arc::new(otto_memory::MemoryService::with_defaults(pool.clone())),
    };
    // Optional Obsidian-vault write-through (git-shareable notes). Wrap only the
    // local service; a remote-backed one writes notes on the host instead.
    match std::env::var("OTTO_MEMORY_VAULT_DIR") {
        Ok(dir) if !dir.trim().is_empty() && std::env::var("OTTO_MEMORY_REMOTE_URL").is_err() => {
            tracing::info!("memory: vault write-through at {dir}");
            Arc::new(otto_memory::MemoryService::with_defaults(pool.clone()).with_vault(dir))
        }
        _ => memory,
    }
}

/// Runtime custom-plugin supervisor. Plugins live under $OTTO_PLUGINS_HOME (or
/// ~/otto-plugins); enabled ones are spawned after listen as sidecar processes
/// Otto reverse-proxies. The sidecars call back to the scoped host API.
fn plugin_manager(cfg: &BootConfig, pool: &DbPool) -> Arc<crate::plugins::PluginManager> {
    let plugins_home = std::env::var_os("OTTO_PLUGINS_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| cfg.data_dir.clone())
                .join("otto-plugins")
        });
    Arc::new(crate::plugins::PluginManager::new(
        otto_state::PluginsRepo::new(pool.clone()),
        plugins_home,
        cfg.data_dir.clone(),
        format!("http://127.0.0.1:{}/api/v1/plugin-host", cfg.port),
    ))
}

/// Resolve the root user once — owns sessions spawned on behalf of inbound
/// channel/webhook messages. Used both for the webhook bridge and the channel
/// manager. None until onboarding creates a root user.
async fn root_user_id(pool: &DbPool) -> Option<String> {
    UsersRepo::new(pool.clone())
        .list()
        .await
        .ok()
        .and_then(|users| users.into_iter().find(|u| u.is_root).map(|u| u.id))
}

/// Webhook-channel bridge: its own Bridge + Mirror, independent of the live
/// Slack/Telegram supervisor, so the public inbound HTTP handler can turn a
/// `POST /webhooks/{ws}` into an agent session. None until a root user exists.
fn webhook_channel_bridge(
    pool: &DbPool,
    manager: &Arc<SessionManager>,
    workspaces: &WorkspacesRepo,
    root_user_id: &Option<String>,
) -> Option<Arc<otto_channels::Bridge>> {
    root_user_id.as_ref().map(|uid| {
        otto_channels::Bridge::new(
            Arc::clone(manager),
            workspaces.clone(),
            SettingsRepo::new(pool.clone()),
            otto_channels::Mirror::new(Arc::clone(manager)),
            uid.clone(),
        )
    })
}
