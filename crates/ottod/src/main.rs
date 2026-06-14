//! ottod — the Otto daemon binary.
//!
//! Opens the SQLite store, wires secrets + RBAC + the event bus into
//! `otto_server::build_router`, and serves on 127.0.0.1:<port> (plus an
//! optional 0.0.0.0 listener controlled by the `network_listener` setting).

mod config;

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
    build_router, spawn_session_event_listener, AuthScanner, CredentialMonitor, ServerCtx,
};
use otto_sessions::{ProviderRegistry, SessionManager};
use otto_state::{
    ConnectionSectionsRepo, ConnectionsRepo, GitStore, ImprovementsRepo, IntegrationsRepo,
    IssuesRepo, ReviewsRepo, SessionsRepo, SettingsRepo, UsersRepo, WorkspacesRepo,
};
use tokio::sync::{broadcast, watch};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

use crate::config::Config;

fn main() -> ExitCode {
    augment_path();
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

    // Otto context library (skills/souls/context) lives under the data dir; the
    // Provisioner materializes a workspace's active set into each CLI at spawn.
    let library_root = cfg.data_dir.join("library");
    let context_library = otto_context::Library::new(library_root.clone());

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
            .with_pre_spawn_hook(Arc::new(otto_context::Provisioner::new(
                context_library.clone(),
            )))
            .with_output_scanner(scanner),
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

    let ctx = ServerCtx {
        pool: pool.clone(),
        secrets: secrets.clone(),
        events: events.clone(),
        authenticator: Arc::new(RbacAuthenticator::new(pool.clone())),
        roles: Arc::new(RbacRoleChecker::new(pool.clone())),
        version: env!("CARGO_PKG_VERSION").to_string(),
        manager: Arc::clone(&manager),
        workspaces: workspaces.clone(),
        connections,
        spawner,
        git_store: GitStore::new(pool.clone()),
        issues_store: IssuesRepo::new(pool.clone()),
        integrations_store: IntegrationsRepo::new(pool.clone()),
        reviews_store: ReviewsRepo::new(pool.clone()),
        orchestrator: Arc::clone(&orchestrator),
        improve_engine: Arc::clone(&improve_engine),
        context_library: context_library.clone(),
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
        );
        let handle = cm.start().await;
        tracing::info!("channel manager: supervisor started (adapters track config live)");
        Some(handle)
    } else {
        tracing::info!("channel manager: skipping (no root user yet — run onboarding first)");
        None
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

    // Optional network listener from the settings table.
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
            match tokio::net::TcpListener::bind(("0.0.0.0", port)).await {
                Ok(listener) => {
                    tracing::info!("network listener on http://0.0.0.0:{port}");
                    let router = router.clone();
                    let mut rx = shutdown_rx.clone();
                    network_task = Some(tokio::spawn(async move {
                        let shutdown = async move {
                            let _ = rx.changed().await;
                        };
                        if let Err(e) = axum::serve(listener, router)
                            .with_graceful_shutdown(shutdown)
                            .await
                        {
                            tracing::error!("network listener: {e}");
                        }
                    }));
                }
                Err(e) => tracing::error!("network listener bind 0.0.0.0:{port}: {e}"),
            }
        }
    }

    let mut rx = shutdown_rx.clone();
    let shutdown = async move {
        let _ = rx.changed().await;
    };
    axum::serve(loopback, router)
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
/// git, ...). Prepend the usual tool directories so detection and PTY spawns
/// see the same commands the user's shell does.
fn augment_path() {
    let home = std::env::var("HOME").unwrap_or_default();
    let extra = [
        format!("{home}/.local/bin"),
        format!("{home}/.bun/bin"),
        format!("{home}/.claude/local"),
        format!("{home}/.cargo/bin"),
        format!("{home}/bin"),
        "/opt/homebrew/bin".to_string(),
        "/usr/local/bin".to_string(),
    ];
    let current = std::env::var("PATH").unwrap_or_default();
    let mut parts: Vec<String> = extra
        .into_iter()
        .filter(|p| !current.split(':').any(|c| c == p) && std::path::Path::new(p).is_dir())
        .collect();
    if parts.is_empty() {
        return;
    }
    parts.push(current);
    std::env::set_var("PATH", parts.join(":"));
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
