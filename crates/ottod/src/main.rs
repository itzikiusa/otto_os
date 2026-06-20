//! ottod — the Otto daemon binary.
//!
//! Opens the SQLite store, wires secrets + RBAC + the event bus into
//! `otto_server::build_router`, and serves on 127.0.0.1:<port> (plus an
//! optional 0.0.0.0 listener controlled by the `network_listener` setting).

mod config;
mod mcp_tools;
mod usage_tailer;

use std::process::ExitCode;
use std::sync::Arc;

use otto_channels::ChannelManager;
use otto_connections::ConnectionsService;
use otto_core::event::Event;
use otto_improve::{ImprovementEngine, LiveEvolver, RealProposalProducer, Scheduler};
use otto_orchestrator::Orchestrator;
use otto_rbac::{RbacAuthenticator, RbacRoleChecker};
use otto_server::modules::{module_routers, PtySpawner};
use otto_server::{
    build_router, spawn_budget_sampler, spawn_metrics_sampler, spawn_session_event_listener,
    spawn_usage_recorder, AuthScanner, CredentialMonitor, ServerCtx,
};
use otto_sessions::{ProviderRegistry, SessionManager};
use otto_state::{
    ActivityRepo, ConnectionSectionsRepo, ConnectionsRepo, GitStore, ImprovementsRepo,
    IntegrationsRepo, IssuesRepo, ReviewFindingsRepo, ReviewsRepo, SessionsRepo, SettingsRepo,
    SkillEvalsRepo, UsersRepo, WorkspacesRepo,
};
use tokio::sync::{broadcast, watch};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::config::Config;

fn main() -> ExitCode {
    augment_path();

    // Subcommand dispatch. `ottod mcp-tools` runs the first-party read-only MCP
    // tool server (Task B2b) over stdio instead of starting the daemon. It is
    // spawned as a child of an agent's CLI via the workspace `.mcp.json`, talks
    // JSON-RPC on stdin/stdout, and calls back into the running daemon. No
    // tracing-to-stderr setup here — stdout/stdin are the MCP transport.
    // Only `mcp-tools` is intercepted; any other argv falls through to the
    // daemon (back-compat: the daemon historically ignores extra argv, and a
    // leading flag like `--version` should keep the daemon's behaviour).
    if std::env::args().nth(1).as_deref() == Some("mcp-tools") {
        let runtime = match tokio::runtime::Runtime::new() {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("ottod mcp-tools: tokio runtime: {e}");
                return ExitCode::FAILURE;
            }
        };
        return match runtime.block_on(mcp_tools::run()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("ottod mcp-tools: {e}");
                ExitCode::FAILURE
            }
        };
    }

    let cfg = Config::load();

    // Tracing: daily-rolling file in ~/Library/Logs/Otto/ AND stderr.
    let log_dir = cfg.log_dir();
    if let Err(e) = std::fs::create_dir_all(&log_dir) {
        eprintln!("ottod: cannot create log dir {}: {e}", log_dir.display());
        return ExitCode::FAILURE;
    }
    let file_appender = tracing_appender::rolling::daily(&log_dir, "ottod.log");
    let (file_writer, _guard) = tracing_appender::non_blocking(file_appender);
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .with(tracing_subscriber::fmt::layer().with_writer(std::io::stderr))
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(file_writer),
        )
        .init();

    let runtime = match tokio::runtime::Runtime::new() {
        Ok(rt) => rt,
        Err(e) => {
            tracing::error!("tokio runtime: {e}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(cfg)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("ottod failed: {e}");
            ExitCode::FAILURE
        }
    }
}

async fn run(cfg: Config) -> Result<(), String> {
    tracing::info!(
        "ottod {} starting (data dir {})",
        env!("CARGO_PKG_VERSION"),
        cfg.data_dir.display()
    );

    let pool = otto_state::open(&cfg.db_path())
        .await
        .map_err(|e| format!("open database: {e}"))?;
    let secrets = otto_keychain::from_env(&cfg.data_dir);
    let (events, _) = broadcast::channel::<Event>(1024);

    // Module construction (Task A9): provider registry (with settings
    // overrides), session manager, connections service, spawner bridge,
    // git store and the orchestrator.
    let settings = SettingsRepo::new(pool.clone());
    let provider_overrides = settings
        .get("providers")
        .await
        .map_err(|e| format!("read providers setting: {e}"))?;
    let providers = ProviderRegistry::new(provider_overrides.as_ref());

    // Embedded ClickHouse usage + metrics store. Config lives in the settings
    // table (`usage` key); degrades to a no-op when the binary isn't installed.
    let usage_config = otto_usage::UsageConfig::from_json(
        settings
            .get("usage")
            .await
            .map_err(|e| format!("read usage setting: {e}"))?
            .as_ref(),
    );
    let usage = otto_usage::UsageEngine::start(usage_config, cfg.data_dir.clone()).await;

    // Otto context library (skills/souls/context) lives under the data dir; the
    // Provisioner materializes a workspace's active set into each CLI at spawn.
    let library_root = cfg.data_dir.join("library");
    let context_library = otto_context::Library::new(library_root.clone());
    // Seed the product-analysis skills into the library (write-if-absent, so user
    // and self-improvement edits are preserved across restarts).
    if let Err(e) = otto_product::seed_skills(&context_library) {
        tracing::warn!("failed to seed product skills: {e}");
    }

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

    let manager = Arc::new(
        SessionManager::new(SessionsRepo::new(pool.clone()), events.clone(), providers)
            // Runtime-configurable idle-suspend grace + per-session keep-alive pin.
            .with_settings_repo(SettingsRepo::new(pool.clone()))
            .with_pre_spawn_hook(Arc::new(otto_context::Provisioner::new(
                context_library.clone(),
            )))
            .with_output_scanner(scanner)
            // User-configured MCP servers merged into `.mcp.json` on agent spawn.
            .with_mcp_servers(Arc::new(
                otto_server::routes::mcp_servers::DbMcpServerProvider::new(pool.clone()),
            ))
            // Agent activity hooks post back to this loopback daemon.
            .with_ingest_base(format!("http://127.0.0.1:{}", cfg.port))
            // First-party read-only MCP tool server (Task B2b): mint a per-session
            // token when `otto_mcp_enabled` is on for the workspace, and inject the
            // `otto` server (runs `ottod mcp-tools`) into the workspace `.mcp.json`.
            .with_auth_repo(otto_rbac::AuthRepo::new(pool.clone()))
            // Record Otto-side lifecycle + user actions to the activity trail.
            .with_activity_repo(ActivityRepo::new(pool.clone())),
    );
    // The guard writes keystrokes back via the manager; wire the (weak) handle
    // now that the Arc exists.
    prompt_guard.set_manager(Arc::downgrade(&manager));
    let workspaces = WorkspacesRepo::new(pool.clone());
    let secrets_arc = secrets.clone();
    let connections = Arc::new(ConnectionsService::new(
        ConnectionsRepo::new(pool.clone()),
        ConnectionSectionsRepo::new(pool.clone()),
        secrets_arc,
    ));
    // Native data-access layer for the DB Explorer: reuses connection profiles +
    // keychain secrets, persists saved queries / history / dashboards.
    let db_explorer = Arc::new(otto_dbviewer::DbViewerService::new(
        ConnectionsRepo::new(pool.clone()),
        secrets.clone(),
        otto_state::DbExplorerRepo::new(pool.clone()),
    ));
    // Message Brokers (Kafka) viewer: cluster profiles (secrets in the Keychain)
    // + an rdkafka client pool for overview/topics/peek/produce/groups/metrics.
    let brokers = Arc::new(otto_brokers::BrokersService::new(
        otto_state::BrokerClustersRepo::new(pool.clone()),
        otto_state::BrokerClusterSectionsRepo::new(pool.clone()),
        secrets.clone(),
        // Persist an audit row for every broker write (produce/delete/config/offset-reset).
        Some(otto_state::BrokerAuditRepo::new(pool.clone())),
    ));
    let spawner = Arc::new(PtySpawner {
        manager: Arc::clone(&manager),
        workspaces: workspaces.clone(),
    });

    // The planner drives a real claude session in a PTY; CLAUDE_BIN lets
    // operators point at a non-PATH binary (mirrors loom).
    let claude_bin = std::env::var("CLAUDE_BIN")
        .ok()
        .filter(|v| !v.trim().is_empty())
        .unwrap_or_else(|| "claude".to_string());
    let orchestrator = Arc::new(Orchestrator::new(claude_bin));

    // Self-improvement engine: reuses the orchestrator's claude driver to run
    // the analysis agent, and shares the event bus so run/edit events reach the
    // /ws/events stream.
    let improve_engine = Arc::new(ImprovementEngine {
        improvements: ImprovementsRepo::new(pool.clone()),
        sessions: SessionsRepo::new(pool.clone()),
        workspaces: workspaces.clone(),
        producer: Arc::new(RealProposalProducer::new(Arc::clone(&orchestrator))),
        events: events.clone(),
        library_root: library_root.clone(),
    });

    let product_repo = otto_state::ProductRepo::new(pool.clone());
    let product = std::sync::Arc::new(otto_product::service::ProductService::new(
        product_repo.clone(),
        IssuesRepo::new(pool.clone()),
        secrets.clone(),
    ));

    // Agent Swarm: persistence + CRUD service. The Coordinator runtime + scheduler
    // are started below once the full ServerCtx exists.
    let swarm_repo = otto_state::SwarmRepo::new(pool.clone());
    let swarm = Arc::new(otto_swarm::SwarmService::new(swarm_repo.clone()));
    // Seed swarm role skills + preset souls into the library (only if absent).
    otto_swarm::presets::seed(&context_library);

    // Memory backend: local SQLite by default, or a shared host Otto when
    // OTTO_MEMORY_REMOTE_URL is set (so a team shares one memory across machines).
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
    let memory = match std::env::var("OTTO_MEMORY_VAULT_DIR") {
        Ok(dir) if !dir.trim().is_empty() && std::env::var("OTTO_MEMORY_REMOTE_URL").is_err() => {
            tracing::info!("memory: vault write-through at {dir}");
            Arc::new(
                otto_memory::MemoryService::with_defaults(pool.clone()).with_vault(dir),
            )
        }
        _ => memory,
    };

    // One shared auth-lookup cache: the authenticator fills it, the grants route
    // (via ServerCtx.auth_cache) evicts from it on set_grants. Clones share the
    // same backing map (Arc-backed).
    let auth_cache = otto_rbac::AuthCache::new();
    let ctx = ServerCtx {
        pool: pool.clone(),
        secrets: secrets.clone(),
        events: events.clone(),
        authenticator: Arc::new(RbacAuthenticator::new_with_cache(
            pool.clone(),
            auth_cache.clone(),
        )),
        roles: Arc::new(RbacRoleChecker::new(pool.clone())),
        auth_cache: auth_cache.clone(),
        version: env!("CARGO_PKG_VERSION").to_string(),
        data_dir: cfg.data_dir.clone(),
        manager: Arc::clone(&manager),
        workspaces: workspaces.clone(),
        connections,
        db_explorer,
        brokers,
        spawner,
        git_store: GitStore::new(pool.clone()),
        issues_store: IssuesRepo::new(pool.clone()),
        integrations_store: IntegrationsRepo::new(pool.clone()),
        reviews_store: ReviewsRepo::new(pool.clone()),
        findings_store: ReviewFindingsRepo::new(pool.clone()),
        skill_evals_store: SkillEvalsRepo::new(pool.clone()),
        skill_eval_cancels: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        orchestrator: Arc::clone(&orchestrator),
        improve_engine: Arc::clone(&improve_engine),
        context_library: context_library.clone(),
        usage: Arc::clone(&usage),
        product,
        product_repo,
        product_agent_cancels: otto_server::product_run::new_cancel_registry(),
        memory,
        swarm,
        swarm_repo,
        swarm_coords: otto_server::swarm_runtime::new_registry(),
        swarm_run_cancels: otto_server::swarm_run::new_cancel_registry(),
    };

    // Restore sessions from the previous daemon run: resumable agent
    // sessions respawn, everything else becomes reconnectable.
    let ws_paths: std::collections::HashMap<String, String> = workspaces
        .list_all()
        .await
        .map_err(|e| format!("list workspaces: {e}"))?
        .into_iter()
        .map(|w| (w.id, w.root_path))
        .collect();
    if let Err(e) = manager
        .restore_all(&move |ws_id| ws_paths.get(ws_id.as_str()).cloned())
        .await
    {
        tracing::warn!("session restore: {e}");
    }

    // Fail any reviews orphaned by the previous process exit: a review's
    // background task dies with the process, so a row left `running` would
    // otherwise poll forever in the UI. Mark them error so they're re-runnable.
    match ReviewsRepo::new(pool.clone())
        .fail_running("Interrupted by a daemon restart — re-run the review.")
        .await
    {
        Ok(n) if n > 0 => tracing::info!("review recovery: marked {n} orphaned review(s) as error"),
        Ok(_) => {}
        Err(e) => tracing::warn!("review recovery: {e}"),
    }

    // Same recovery for orphaned skill-evaluation runs.
    match SkillEvalsRepo::new(pool.clone())
        .fail_running("Interrupted by a daemon restart — re-run the evaluation.")
        .await
    {
        Ok(n) if n > 0 => tracing::info!("skill-eval recovery: marked {n} orphaned run(s) as error"),
        Ok(_) => {}
        Err(e) => tracing::warn!("skill-eval recovery: {e}"),
    }

    // Same recovery for orphaned workflow runs: a run executes in a background
    // task that dies with the process, so any row left `pending`/`running` would
    // otherwise poll forever in the UI. Mark them error so they're re-runnable.
    match otto_server::workflow_engine::reap_orphaned_runs(&pool).await {
        Ok(n) if n > 0 => tracing::info!("workflow recovery: marked {n} orphaned run(s) as error"),
        Ok(_) => {}
        Err(e) => tracing::warn!("workflow recovery: {e}"),
    }

    // Periodically auto-archive idle channel (ticket/chat) sessions so they
    // don't accumulate in the sidebar. A later message respawns a fresh one.
    // At ticketing volume (100-200/day) a long window floods the sidebar, so
    // we archive after 1h idle and sweep every 10 min.
    {
        let manager = Arc::clone(&manager);
        let interval = std::time::Duration::from_secs(10 * 60); // every 10 min
        let max_idle = std::time::Duration::from_secs(60 * 60); // 1h idle
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let n = manager.reap_idle_channel_sessions(max_idle).await;
                if n > 0 {
                    tracing::info!("auto-archived {n} idle channel session(s)");
                }
            }
        });
    }

    // Retention: permanently delete archived channel (ticket/chat) sessions
    // whose last activity is older than 30 days, so the DB doesn't grow without
    // bound at ticketing volume. Runs at startup, then daily.
    {
        let manager = Arc::clone(&manager);
        let interval = std::time::Duration::from_secs(24 * 60 * 60); // daily
        let max_age = std::time::Duration::from_secs(30 * 24 * 60 * 60); // 30 days
        tokio::spawn(async move {
            loop {
                let n = manager.purge_old_archived_channel_sessions(max_age).await;
                if n > 0 {
                    tracing::info!("purged {n} archived channel session(s) older than 30 days");
                }
                tokio::time::sleep(interval).await;
            }
        });
    }

    // Idle+unattached suspend sweep: every ~60s, release the PTY of any LIVE
    // resumable session that has been idle past the grace window and has no
    // attached WS viewer. The session stays resumable (reopening auto-resumes
    // via --resume), so this frees RAM without ever losing a conversation.
    {
        let manager = Arc::clone(&manager);
        let interval = std::time::Duration::from_secs(60);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let n = manager.suspend_idle_unattached().await;
                if n > 0 {
                    tracing::info!("suspended {n} idle, unattached session(s)");
                }
            }
        });
    }

    // Existence-check pruner: once at startup, then every ~6h. For non-live
    // resumable agent sessions, delete the row only when the provider's local
    // transcript is positively gone (un-resumable). Sessions whose transcript
    // still exists — or whose resumability can't be verified — are kept.
    {
        let manager = Arc::clone(&manager);
        let interval = std::time::Duration::from_secs(6 * 60 * 60); // every 6h
        tokio::spawn(async move {
            loop {
                let n = manager.prune_dead_sessions().await;
                if n > 0 {
                    tracing::info!("pruned {n} un-resumable session(s)");
                }
                tokio::time::sleep(interval).await;
            }
        });
    }

    // Activity-trail retention: cap each session's trail at the newest N rows so
    // long-lived sessions don't grow it unbounded. Runs at startup then hourly.
    {
        let manager = Arc::clone(&manager);
        const KEEP_PER_SESSION: i64 = 1_000;
        let interval = std::time::Duration::from_secs(60 * 60); // hourly
        tokio::spawn(async move {
            loop {
                let n = manager.prune_activity_trail(KEEP_PER_SESSION).await;
                if n > 0 {
                    tracing::info!("pruned {n} old activity-trail row(s)");
                }
                tokio::time::sleep(interval).await;
            }
        });
    }

    // --- Channel Manager (Telegram-first, Slack-ready) ---
    // Resolve the root user id for spawning agent sessions on behalf of
    // incoming channel messages.  We use the first root user in the DB;
    // if onboarding hasn't happened yet there are no users and we skip.
    let root_user_id: Option<String> = UsersRepo::new(pool.clone())
        .list()
        .await
        .ok()
        .and_then(|users| users.into_iter().find(|u| u.is_root).map(|u| u.id));

    let _channel_handle = if let Some(uid) = root_user_id {
        let cm = ChannelManager::new(
            Arc::clone(&manager),
            workspaces.clone(),
            IntegrationsRepo::new(pool.clone()),
            SettingsRepo::new(pool.clone()),
            secrets.clone(),
            uid,
            // Share the daemon event bus so the proactive self-improvement
            // notifier can mirror Improvement* events to the user's channels
            // (opt-in via the `channels.notify_self_improvement` setting).
            Some(events.clone()),
        );
        let handle = cm.start().await;
        tracing::info!("channel manager: supervisor started (adapters track config live)");
        Some(handle)
    } else {
        tracing::info!("channel manager: skipping (no root user yet — run onboarding first)");
        None
    };

    // --- Story watcher (polls watched stories for new comments) ---
    let _watcher_handle = {
        let watcher = otto_server::product_watcher::WatcherManager::new(
            otto_state::ProductRepo::new(pool.clone()),
            Arc::clone(&ctx.product),
            Arc::clone(&orchestrator),
            Arc::clone(&improve_engine),
            events.clone(),
            "claude".to_string(),
        );
        let handle = watcher.start();
        tracing::info!("story watcher: supervisor started");
        handle
    };

    // --- Self-improvement (scheduler + live skill evolver) ---
    // Gated by OTTO_SELF_IMPROVE so it can be killed without a rebuild: enabled
    // by default; set OTTO_SELF_IMPROVE=0 (or false/off) to disable both the
    // per-workspace self-reflection scheduler and the live in-loop evolver.
    let self_improve_enabled = !matches!(
        std::env::var("OTTO_SELF_IMPROVE").as_deref(),
        Ok("0") | Ok("false") | Ok("off")
    );
    let (_scheduler_handle, _live_evolver_handle) = if self_improve_enabled {
        // Background supervisor that fires due per-workspace self-reflection runs.
        let scheduler = Scheduler::new(Arc::clone(&improve_engine), workspaces.clone())
            .start()
            .await;
        tracing::info!("self-improvement scheduler started");

        // Subscribes to the event bus; evolves a watched session's skills after
        // its interaction goes idle (workspace `live_evolve` / session `meta.evolve`).
        let evolver = LiveEvolver::new(
            Arc::clone(&improve_engine),
            workspaces.clone(),
            SessionsRepo::new(pool.clone()),
        )
        .start(events.subscribe());
        tracing::info!("live skill evolver started");

        (Some(scheduler), Some(evolver))
    } else {
        tracing::info!("self-improvement disabled (OTTO_SELF_IMPROVE=off)");
        (None, None)
    };

    // --- Credential monitor + session-event notices (wave 2) ---
    // Background loop: token-expiry + agent-CLI health checks (startup, then
    // every ~6h). Event-bus listener: session-progress notices.
    CredentialMonitor::new(ctx.clone()).spawn();
    spawn_session_event_listener(ctx.clone());
    tracing::info!("credential monitor + session-event notices started");

    // --- Orphan reaper: auto-resume analysis agents stranded by a restart ---
    // Runs once at startup; any analysis agent still 'running'/'waiting' has no
    // surviving task, so it is re-run (capped) or marked errored + notified.
    tokio::spawn(otto_server::product_run::reap_orphaned_agents_on_startup(
        ctx.clone(),
    ));

    // --- Insights scheduler: opt-in, catch-up usage reports ---
    // Background supervisor that ticks ~hourly and, for each ENABLED cadence
    // (daily/weekly/monthly — all default OFF), runs the `insights` skill for the
    // most-recent missed period iff it has no report yet. Runs the due-check on
    // startup (catch-up after the app was closed), then hourly.
    let _insights_scheduler_handle =
        otto_server::insights::InsightsScheduler::new(ctx.clone()).start();
    tracing::info!("insights scheduler started");

    // Daily CLI auto-update: updates the agent CLIs (claude/codex/…) at a
    // user-configurable local time (default 07:00, opt-out via settings) and
    // force-reloads open agent sessions onto the new binary (resume-aware).
    // Catch-up on a missed window via a last-run cursor, like insights.
    let _cli_update_handle = otto_server::cli_update::CliUpdateScheduler::new(ctx.clone()).start();
    tracing::info!("cli auto-update scheduler started");

    // --- Agent Swarm: reconcile stale runs, then scheduler + restore coords ---
    // A swarm run's background task dies with the process. A row left
    // queued/running/waiting would permanently consume the parallel cap and
    // block its agent's one-turn-at-a-time gate, so fail them BEFORE restarting
    // any coordinator (mirrors the review/skill-eval recovery above).
    match ctx
        .swarm_repo
        .fail_running("Interrupted by a daemon restart — the coordinator will re-run the task.")
        .await
    {
        Ok(n) if n > 0 => tracing::info!("swarm recovery: marked {n} orphaned run(s) as error"),
        Ok(_) => {}
        Err(e) => tracing::warn!("swarm recovery: {e}"),
    }
    let _swarm_scheduler_handle = otto_server::swarm_scheduler::start(ctx.clone());
    match ctx.swarm_repo.list_all_active_swarms().await {
        Ok(active) => {
            for s in active {
                otto_server::swarm_runtime::start_coordinator(ctx.clone(), s.id.clone());
            }
            tracing::info!("swarm scheduler started; coordinators restored");
        }
        Err(e) => tracing::warn!("swarm restore: {e}"),
    }

    // --- Usage tracking + system metrics (embedded ClickHouse) ---
    // The recorder mines usage from the activity-trail event stream; the
    // metrics sampler writes CPU/RAM; the budget sampler rides the metrics tick
    // to check spend-vs-cap and emit BudgetExceeded events (with dedupe). All
    // three are cheap no-ops until ClickHouse is available.
    spawn_usage_recorder(ctx.clone());
    spawn_metrics_sampler(ctx.clone());
    spawn_budget_sampler(ctx.clone());
    if usage.available() {
        tracing::info!("usage tracking started (embedded clickhouse)");
    } else {
        tracing::info!("usage tracking idle (clickhouse not installed)");
    }

    // --- Usage tailer: real token usage from Claude + Codex CLI transcripts ---
    // Tails the CLIs' on-disk JSONL transcripts and records exact per-turn token
    // usage/cost into the usage store (a persistent byte-offset cursor prevents
    // double-counting; pre-existing history is skipped to avoid misdated rows).
    // agy is unsupported (its on-disk usage is encrypted).
    let _usage_tailer_handle = {
        let tailer = usage_tailer::UsageTailer::new(
            Arc::clone(&ctx.usage),
            pool.clone(),
            cfg.data_dir.clone(),
            dirs::home_dir().unwrap_or_else(|| cfg.data_dir.clone()),
        );
        let handle = tailer.start();
        tracing::info!("usage tailer: started (claude+codex; agy unsupported)");
        handle
    };

    let (api_extras, root_extras) = module_routers(&ctx);
    let router = build_router(ctx, api_extras, root_extras);

    // Graceful shutdown signal (ctrl_c or SIGTERM) fanned out via watch.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(async move {
        wait_for_signal().await;
        tracing::info!("shutdown signal received");
        let _ = shutdown_tx.send(true);
    });

    let loopback = tokio::net::TcpListener::bind(("127.0.0.1", cfg.port))
        .await
        .map_err(|e| format!("bind 127.0.0.1:{}: {e}", cfg.port))?;
    tracing::info!("listening on http://127.0.0.1:{}", cfg.port);

    // Optional network listener from the settings table. Unlike loopback, the
    // 0.0.0.0 listener is reachable from the LAN, so it is served over TLS
    // (rustls) — never plain HTTP (audit S3). The cert+key live under
    // <data_dir>/tls and are auto-generated (self-signed) on first use.
    let mut network_task = None;
    if let Some(value) = SettingsRepo::new(pool.clone())
        .get("network_listener")
        .await
        .map_err(|e| format!("read network_listener setting: {e}"))?
    {
        let enabled = value
            .get("enabled")
            .and_then(serde_json::Value::as_bool)
            .unwrap_or(false);
        if enabled {
            let port = value
                .get("port")
                .and_then(serde_json::Value::as_u64)
                .and_then(|p| u16::try_from(p).ok())
                .unwrap_or(cfg.port);
            match load_or_make_tls_config(&cfg.data_dir).await {
                Ok(tls) => {
                    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
                    tracing::info!("network listener on https://0.0.0.0:{port} (TLS)");
                    let router = router.clone();
                    // axum-server drives shutdown via its own Handle; bridge the
                    // watch signal into a graceful_shutdown so the TLS listener
                    // drains in step with the loopback one.
                    let handle = axum_server::Handle::new();
                    let mut rx = shutdown_rx.clone();
                    let shutdown_handle = handle.clone();
                    tokio::spawn(async move {
                        let _ = rx.changed().await;
                        shutdown_handle
                            .graceful_shutdown(Some(std::time::Duration::from_secs(5)));
                    });
                    network_task = Some(tokio::spawn(async move {
                        // `into_make_service_with_connect_info::<SocketAddr>` makes
                        // each request's real socket peer available to handlers via
                        // `ConnectInfo<SocketAddr>` (used by the login throttle, S5).
                        if let Err(e) = axum_server::bind_rustls(addr, tls)
                            .handle(handle)
                            .serve(router.into_make_service_with_connect_info::<std::net::SocketAddr>())
                            .await
                        {
                            tracing::error!("network listener: {e}");
                        }
                    }));
                }
                Err(e) => {
                    tracing::error!("network listener TLS setup failed: {e}");
                }
            }
        }
    }

    let mut rx = shutdown_rx.clone();
    let shutdown = async move {
        let _ = rx.changed().await;
    };
    // `into_make_service_with_connect_info::<SocketAddr>` exposes the real socket
    // peer to handlers via `ConnectInfo<SocketAddr>` — the login throttle keys on
    // it instead of a spoofable `X-Forwarded-For` header (audit S5).
    axum::serve(
        loopback,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .await
    .map_err(|e| format!("serve: {e}"))?;

    if let Some(task) = network_task {
        let _ = task.await;
    }

    // Terminate every live PTY so a daemon stop / system shutdown never leaves
    // orphaned agent processes behind.
    let killed = manager.shutdown_all().await;
    if killed > 0 {
        tracing::info!("terminated {killed} live session(s) on shutdown");
    }
    tracing::info!("ottod stopped");
    Ok(())
}

/// launchd starts agents with a bare PATH (`/usr/bin:/bin:...`), which hides
/// user-installed CLIs (claude in ~/.local/bin, codex in ~/.bun/bin, homebrew
/// git, language servers in ~/go/bin or a custom npm prefix, ...). Prepend the
/// usual tool directories — plus the *discovered* npm-global and GOPATH bins —
/// so detection and PTY spawns see the same commands the user's shell does.
fn augment_path() {
    let home = std::env::var("HOME").unwrap_or_default();
    prepend_path(&[
        format!("{home}/.local/bin"),
        format!("{home}/.bun/bin"),
        format!("{home}/.claude/local"),
        format!("{home}/.cargo/bin"),
        format!("{home}/bin"),
        // Go binaries (gopls, etc.) install here by default.
        format!("{home}/go/bin"),
        // Otto's own bin dir, where the usage feature installs `clickhouse`.
        format!("{home}/Library/Application Support/Otto/bin"),
        "/opt/homebrew/bin".to_string(),
        "/usr/local/bin".to_string(),
    ]);
    // Discover the npm global-prefix bin and GOPATH bin (best-effort; npm/go are
    // now resolvable via the prepends above). This catches servers installed to a
    // custom prefix (e.g. `npm config set prefix ~/.hermes/node`) that the user
    // never added to their own PATH.
    if let Some(out) = probe_cmd("npm", &["prefix", "-g"]) {
        let dir = format!("{}/bin", out.trim());
        prepend_path(std::slice::from_ref(&dir));
    }
    if let Some(out) = probe_cmd("go", &["env", "GOPATH"]) {
        let gopath = out.trim();
        if !gopath.is_empty() {
            let dir = format!("{gopath}/bin");
            prepend_path(std::slice::from_ref(&dir));
        }
    }
}

/// Prepend `dirs` (those that exist and aren't already present) to `$PATH`.
fn prepend_path(dirs: &[String]) {
    let current = std::env::var("PATH").unwrap_or_default();
    let mut parts: Vec<String> = dirs
        .iter()
        .filter(|p| !current.split(':').any(|c| c == p.as_str()) && std::path::Path::new(p).is_dir())
        .cloned()
        .collect();
    if parts.is_empty() {
        return;
    }
    parts.push(current);
    std::env::set_var("PATH", parts.join(":"));
}

/// Run `cmd args`, returning trimmed stdout on success (best-effort; `None` if
/// the command is missing or fails). Used to discover tool install prefixes.
fn probe_cmd(cmd: &str, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new(cmd).args(args).output().ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Build the rustls config for the 0.0.0.0 network listener from a PEM cert+key
/// under `<data_dir>/tls`. On first use (no cert present) a self-signed cert is
/// generated, persisted, and its SHA-256 fingerprint logged so operators can pin
/// it. Errors (bad/unreadable PEM, generation failure) abort the network
/// listener rather than silently falling back to plain HTTP.
async fn load_or_make_tls_config(
    data_dir: &std::path::Path,
) -> Result<axum_server::tls_rustls::RustlsConfig, String> {
    use axum_server::tls_rustls::RustlsConfig;

    // Both `ring` and `aws-lc-rs` are linked into rustls in this tree, so the
    // process-default crypto provider is ambiguous. Install `ring` explicitly
    // (idempotent: a no-op if a provider is already installed) before building
    // any TLS config, otherwise rustls panics at first use.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let tls_dir = data_dir.join("tls");
    let cert_path = tls_dir.join("cert.pem");
    let key_path = tls_dir.join("key.pem");

    if !cert_path.exists() || !key_path.exists() {
        std::fs::create_dir_all(&tls_dir)
            .map_err(|e| format!("create {}: {e}", tls_dir.display()))?;
        // Self-signed cert valid for loopback + LAN hostnames. The listener is
        // reachable by IP on the LAN, so include both names and the loopback IP.
        let sans = vec![
            "localhost".to_string(),
            "otto.local".to_string(),
            "127.0.0.1".to_string(),
        ];
        let cert = rcgen::generate_simple_self_signed(sans)
            .map_err(|e| format!("generate self-signed cert: {e}"))?;
        let cert_pem = cert.cert.pem();
        let key_pem = cert.key_pair.serialize_pem();
        std::fs::write(&cert_path, &cert_pem)
            .map_err(|e| format!("write {}: {e}", cert_path.display()))?;
        std::fs::write(&key_path, &key_pem)
            .map_err(|e| format!("write {}: {e}", key_path.display()))?;
        // Lock the private key down to the owner.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&key_path, std::fs::Permissions::from_mode(0o600));
        }
        tracing::info!(
            "network TLS: generated self-signed cert at {} (fingerprint {})",
            cert_path.display(),
            cert_fingerprint(cert.cert.der())
        );
    } else if let Ok(der) = std::fs::read(&cert_path) {
        // Log the fingerprint of the existing cert too, so it's discoverable.
        if let Some(fp) = pem_cert_fingerprint(&der) {
            tracing::info!("network TLS: using cert at {} ({fp})", cert_path.display());
        }
    }

    RustlsConfig::from_pem_file(&cert_path, &key_path)
        .await
        .map_err(|e| format!("load TLS cert/key: {e}"))
}

/// SHA-256 fingerprint of a DER cert, formatted as colon-separated hex.
fn cert_fingerprint(der: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(der);
    digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(":")
}

/// Fingerprint the first certificate in a PEM file's bytes, if parseable.
fn pem_cert_fingerprint(pem_bytes: &[u8]) -> Option<String> {
    let mut reader = std::io::BufReader::new(pem_bytes);
    let certs: Vec<_> = rustls_pemfile::certs(&mut reader)
        .filter_map(|c| c.ok())
        .collect();
    certs.first().map(|c| cert_fingerprint(c.as_ref()))
}

async fn wait_for_signal() {
    let ctrl_c = tokio::signal::ctrl_c();
    #[cfg(unix)]
    {
        let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("install SIGTERM handler");
        tokio::select! {
            _ = ctrl_c => {}
            _ = sigterm.recv() => {}
        }
    }
    #[cfg(not(unix))]
    {
        let _ = ctrl_c.await;
    }
}
