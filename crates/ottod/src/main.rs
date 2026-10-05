//! ottod — the Otto daemon binary.
//!
//! Opens the SQLite store, wires secrets + RBAC + the event bus into
//! `otto_server::build_router`, and serves on 127.0.0.1:<port> (plus an
//! optional 0.0.0.0 listener controlled by the `network_listener` setting).

// The MCP tool catalog (`mcp_tools::tool_catalog`) is one large `json!` literal;
// it outgrew the default macro recursion limit as tools were added.
#![recursion_limit = "512"]

mod config;
mod housekeeping;
mod mcp_server;
mod mcp_tools;
#[cfg(feature = "embed-ui")]
mod ui_assets;
mod usage_tailer;

use std::future::IntoFuture;
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
    build_router_with_assets, spawn_budget_sampler, spawn_metrics_sampler,
    spawn_session_event_listener, spawn_usage_recorder, spawn_workflow_event_trigger_listener,
    AuthScanner, CredentialMonitor, ServerCtx,
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
use tracing_subscriber::Layer;

use crate::config::Config;

fn main() -> ExitCode {
    // `ottod pty-holder`: a detached process that owns ONE session's PTY so the
    // session survives a daemon restart (otto_pty::holder). Dispatched before
    // anything else — it inherits the daemon's already-augmented environment,
    // must not open the daemon's logs, and talks to its launcher on stdout.
    if std::env::args().nth(1).as_deref() == Some("pty-holder") {
        return ExitCode::from(otto_sessions::pty_holder::run_from_stdin().clamp(0, 255) as u8);
    }
    if std::env::args().nth(1).as_deref() == Some("room-ocr") {
        return if otto_server::run_room_ocr_helper() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
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
        // One stdio relay per agent session: a current-thread runtime, not a
        // worker per core (a multi-thread runtime started ~18 threads and
        // ~12 MB RSS in every per-session bridge). Blocking work (stdin, DB
        // file I/O) still goes to tokio's on-demand blocking pool.
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
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

    // `ottod mcp-server` runs the OUTWARD "Otto as an MCP server" over stdio: an
    // external agent (Claude Code, Copilot, …) launches it with an `OTTO_API_TOKEN`
    // (a restricted `kind='mcp'` token) and calls the eight `otto.*` tools, every
    // one governed by the control plane (design §7).
    if std::env::args().nth(1).as_deref() == Some("mcp-server") {
        // One stdio relay per agent session: a current-thread runtime, not a
        // worker per core (a multi-thread runtime started ~18 threads and
        // ~12 MB RSS in every per-session bridge). Blocking work (stdin, DB
        // file I/O) still goes to tokio's on-demand blocking pool.
        let runtime = match tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
        {
            Ok(rt) => rt,
            Err(e) => {
                eprintln!("ottod mcp-server: tokio runtime: {e}");
                return ExitCode::FAILURE;
            }
        };
        return match runtime.block_on(mcp_server::run()) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("ottod mcp-server: {e}");
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
    // Daily files, the newest MAX_LOG_FILES kept (the appender prunes older
    // `ottod.log.*` on rotation — they used to accumulate forever).
    let file_appender = match tracing_appender::rolling::RollingFileAppender::builder()
        .rotation(tracing_appender::rolling::Rotation::DAILY)
        .filename_prefix("ottod.log")
        .max_log_files(MAX_LOG_FILES)
        .build(&log_dir)
    {
        Ok(a) => a,
        Err(e) => {
            eprintln!("ottod: cannot open log file in {}: {e}", log_dir.display());
            return ExitCode::FAILURE;
        }
    };
    let (file_writer, _guard) = tracing_appender::non_blocking(file_appender);
    // Under launchd, stderr is an append-only file (StandardErrorPath) that
    // nothing rotates: mirror only warnings and errors there — the full log
    // is in the rotating file. A terminal / test harness still gets it all.
    let stderr_level = if std::env::var("XPC_SERVICE_NAME").as_deref() == Ok("com.otto.daemon") {
        tracing_subscriber::filter::LevelFilter::WARN
    } else {
        tracing_subscriber::filter::LevelFilter::TRACE
    };
    tracing_subscriber::registry()
        .with(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,sqlx=warn".into()),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stderr)
                .with_filter(stderr_level),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(file_writer),
        )
        .init();
    install_panic_hook();
    if let Some(was) = housekeeping::cap_stderr_log(
        &log_dir.join("ottod.stderr.log"),
        housekeeping::STDERR_LOG_MAX_BYTES,
    ) {
        tracing::info!("truncated ottod.stderr.log ({was} bytes)");
    }

    raise_nofile_limit();

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

/// Daily `ottod.log.*` files kept (≈ a month).
const MAX_LOG_FILES: usize = 30;

/// How long the HTTP servers may drain in-flight requests after the shutdown
/// signal. launchd SIGKILLs at `ExitTimeOut` (30 s in the plist); the drain
/// plus the bounded teardown steps below must fit well inside it.
const HTTP_DRAIN_CAP: std::time::Duration = std::time::Duration::from_secs(3);

/// Route panics through tracing (file log, with a backtrace) and straight to
/// stderr (unbuffered — launchd's StandardErrorPath), then chain the default
/// hook. Before this a panicking worker left no trace in ottod.log: tokio
/// caught it and the default hook wrote one line to a stderr nobody kept.
fn install_panic_hook() {
    let default_hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let backtrace = std::backtrace::Backtrace::force_capture();
        let thread = std::thread::current()
            .name()
            .unwrap_or("<unnamed>")
            .to_string();
        tracing::error!(target: "panic", %thread, "PANIC: {info}\nbacktrace:\n{backtrace}");
        // The file writer is non-blocking (a background thread): if this panic
        // is about to abort the process that line may never land — stderr is
        // written synchronously, so the backtrace survives either way.
        eprintln!("ottod PANIC on thread {thread}: {info}\nbacktrace:\n{backtrace}");
        default_hook(info);
    }));
}

/// Raise the soft `RLIMIT_NOFILE` as far as the hard limit allows (capped at
/// 65536). launchd starts agents with a 256-descriptor soft limit, and every
/// live PTY session costs ~3 descriptors on top of sockets, SQLite and
/// ClickHouse — a daemon carrying a fleet of agent sessions hit the cap and
/// EVERYTHING failed at once: `accept()` (the UI's "slow keystrokes"), spawns
/// ("spawn claude: Too many open files (os error 24)"), state writes. Raising
/// the soft limit needs no privileges. On macOS `setrlimit` rejects values
/// above `OPEN_MAX` when the hard limit is unlimited, so fall back to that.
fn raise_nofile_limit() {
    use rustix::process::{getrlimit, setrlimit, Resource, Rlimit};
    let lim = getrlimit(Resource::Nofile);
    let target = lim.maximum.map_or(65536, |h| h.min(65536));
    match lim.current {
        None => return,                       // already unlimited
        Some(cur) if cur >= target => return, // already high enough
        _ => {}
    }
    let try_set = |cur: u64| {
        setrlimit(
            Resource::Nofile,
            Rlimit {
                current: Some(cur),
                maximum: lim.maximum,
            },
        )
        .is_ok()
    };
    // macOS rejects soft values above OPEN_MAX (10240) when the hard limit is
    // reported unlimited — retry at that ceiling.
    let new = if try_set(target) {
        target
    } else if target > 10240 && try_set(10240) {
        10240
    } else {
        tracing::warn!(
            "could not raise open-file limit past {:?}; heavy agent load may hit it",
            lim.current
        );
        return;
    };
    tracing::info!("raised open-file soft limit {:?} -> {new}", lim.current);
}

async fn run(cfg: Config) -> Result<(), String> {
    tracing::info!(
        "ottod {} starting (data dir {})",
        env!("CARGO_PKG_VERSION"),
        cfg.data_dir.display()
    );

    // Single-instance lock FIRST: holding the loopback port is what proves no
    // other ottod owns this data dir. Everything below mutates shared state —
    // the usage engine's reclaim_dir kills whatever ClickHouse server holds the
    // data dir, plugin supervisors spawn sidecars, schedulers fire. A second
    // instance (launchd respawn racing a still-running daemon) used to get all
    // the way through those side effects — murdering the live daemon's
    // ClickHouse — before dying on this very bind. Losers must exit HERE, first.
    let loopback = tokio::net::TcpListener::bind(("127.0.0.1", cfg.port))
        .await
        .map_err(|e| format!("bind 127.0.0.1:{}: {e}", cfg.port))?;
    // Second LOOPBACK host for the same port (TRANSPORT_PLAN §3d). Browsers
    // cap HTTP/1.1 sockets per host (~6, shared by every Otto window), so the
    // UI sends background polls + known-slow calls to `[::1]` and keeps
    // `127.0.0.1` for what the user clicked. Still loopback-only; fail-soft
    // (IPv6 off / port taken → no alias, and `/meta` advertises none, so the
    // UI never sends its token to an address this daemon does not hold).
    // `OTTO_ALT_LOOPBACK=0` turns it off.
    let alt_loopback = if std::env::var("OTTO_ALT_LOOPBACK").as_deref() == Ok("0") {
        None
    } else {
        match tokio::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, cfg.port)).await {
            Ok(l) => Some(l),
            Err(e) => {
                tracing::warn!(
                    "alt loopback [::1]:{} not bound ({e}) — single-host transport",
                    cfg.port
                );
                None
            }
        }
    };

    // Per-phase boot timing (perf2/03 N4/N7): one `boot: ready` line with the
    // breakdown, read by scripts/perf/daemon-budget.mjs.
    let mut boot = BootPhases::start();

    // Unclean-shutdown detection: the marker is removed only at the end of a
    // clean stop, so finding one means the last run crashed, was SIGKILLed
    // (launchd ExitTimeOut, OOM) or the machine lost power.
    if let Some(prev) = housekeeping::mark_running(
        &cfg.data_dir,
        std::process::id(),
        std::time::SystemTime::now(),
    ) {
        tracing::warn!("previous run did not shut down cleanly ({prev})");
    }

    // Offline compaction (perf2/03 N1): a large, fragmented otto.db is
    // rewritten HERE, before the pool opens — nothing can be writing, so no
    // write stalls and none made after the snapshot is lost. The old file is
    // kept until the new one has opened and migrated (confirm below); an
    // unconfirmed swap is rolled back at the next start.
    match otto_state::maintenance::offline_compact_at_boot(&cfg.db_path()).await {
        otto_state::maintenance::OfflineOutcome::NotNeeded => {}
        otto_state::maintenance::OfflineOutcome::Compacted(r) => tracing::info!(
            "db maintenance: compacted offline {} → {} bytes in {} ms",
            r.before_bytes,
            r.after_bytes,
            r.duration_ms
        ),
        otto_state::maintenance::OfflineOutcome::RolledBack => tracing::warn!(
            "db maintenance: the last start never confirmed the compacted database — \
             restored the original file (no automatic retry)"
        ),
        otto_state::maintenance::OfflineOutcome::Skipped(why) => {
            tracing::warn!("db maintenance: offline compaction skipped: {why}")
        }
    }
    boot.mark("db_compact");

    let pool = otto_state::open(&cfg.db_path())
        .await
        .map_err(|e| format!("open database: {e}"))?;
    if otto_state::maintenance::confirm_offline_compaction(&cfg.db_path()) {
        tracing::info!("db maintenance: compacted database opened cleanly; old file removed");
    }
    boot.mark("db_open");
    otto_state::database_changes::DatabaseChangesRepo::new(pool.clone())
        .recover_interrupted()
        .await
        .map_err(|e| format!("recover interrupted database changes: {e}"))?;
    // Planner statistics from the first query (sqlite_stat1), plus the
    // one-time compaction when a third of the file is free pages and the live
    // data is small enough to rewrite in a second or two (perf F1) — normally
    // already done by the offline pass above; this inline VACUUM only catches
    // a file that pass skipped. A bigger one waits for the next start.
    {
        let t = std::time::Instant::now();
        match otto_state::maintenance::boot(&pool).await {
            Ok(b) => {
                if let Some(r) = b.compacted {
                    tracing::info!(
                        "db maintenance: compacted at boot {} → {} bytes in {} ms",
                        r.before_bytes,
                        r.after_bytes,
                        r.duration_ms
                    );
                } else if b.deferred_compaction {
                    tracing::info!(
                        "db maintenance: {} of {} bytes free — compaction deferred to the next start",
                        b.stats.free_bytes(),
                        b.stats.size_bytes()
                    );
                }
                tracing::debug!("db maintenance: boot pass {} ms", t.elapsed().as_millis());
            }
            Err(e) => tracing::warn!("db boot maintenance failed: {e}"),
        }
    }
    boot.mark("maintenance");
    // Self-improvement runs are in-process: nothing can still be running at
    // boot, and an orphaned `running` row blocks that workspace's runs forever.
    match ImprovementsRepo::new(pool.clone())
        .fail_orphaned_runs()
        .await
    {
        Ok(0) => {}
        Ok(n) => tracing::warn!("marked {n} interrupted self-improvement run(s) failed"),
        Err(e) => tracing::warn!("recover interrupted self-improvement runs: {e}"),
    }
    let secrets = otto_keychain::from_env(&cfg.data_dir);
    let (events, _) = broadcast::channel::<Event>(1024);
    // Every MCP approval write (governance pipeline, outward MCP, assistant,
    // live browser) pushes `mcp_approval_changed` — the tray, the MCP badge
    // and Home stop polling `/mcp/approvals` for it.
    {
        let tx = events.clone();
        otto_state::set_approval_change_hook(move |c| {
            let _ = tx.send(Event::McpApprovalChanged {
                approval_id: c.approval_id,
                workspace_id: c.workspace_id,
                status: c.status,
            });
        });
    }

    // Module construction (Task A9): provider registry (with settings
    // overrides), session manager, connections service, spawner bridge,
    // git store and the orchestrator.
    let settings = SettingsRepo::new(pool.clone());
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
                otto_server::routes::mcp_servers::DbMcpServerProvider::new(
                    pool.clone(),
                    secrets.clone(),
                ),
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
            .with_pty_holders_opt(pty_holder_config(&cfg.data_dir)),
    );
    // The guard writes keystrokes back via the manager; wire the (weak) handle
    // now that the Arc exists.
    prompt_guard.set_manager(Arc::downgrade(&manager));
    let workspaces = WorkspacesRepo::new(pool.clone());
    // The system-owned scratch workspace (workspace-less sessions) lives at the
    // daemon user's home; created on first boot, healed on every later one.
    // Never `/`: this root is the default cwd of every workspace-less session
    // AND the folder they are pre-trusted for ("may write anywhere under"), so
    // a bare launchd context with no `$HOME` must fall back to the daemon's own
    // data dir — the same resolution the session manager uses below.
    let home = dirs::home_dir()
        .unwrap_or_else(|| cfg.data_dir.clone())
        .to_string_lossy()
        .into_owned();
    workspaces
        .ensure_scratch(&home)
        .await
        .map_err(|e| format!("ensure scratch workspace: {e}"))?;
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
        secrets.clone(),
        // Persist an audit row for every broker write (produce/delete/config/offset-reset).
        Some(otto_state::BrokerAuditRepo::new(pool.clone())),
    ));
    // Idle-connection reapers. Unlike agent sessions, nothing else closes an
    // idle Kafka client or DB SSH tunnel — POOL_TTL/lazy-eviction only fire on
    // the *next* access, which a truly idle cluster/connection never triggers.
    // So on a mostly-idle daemon the librdkafka handles + orphaned `ssh` children
    // would linger until edit/delete/restart. Sweep on a timer (every 5 min) and
    // evict anything idle past the conservative ~30 min window.
    {
        let brokers = Arc::clone(&brokers);
        let interval = std::time::Duration::from_secs(5 * 60); // every 5 min
        let idle = std::time::Duration::from_secs(30 * 60); // 30 min idle
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let n = brokers.reap_idle(idle);
                if n > 0 {
                    tracing::info!("brokers: reaped {n} idle cluster connection(s)");
                }
            }
        });
    }
    {
        let db = Arc::clone(&db_explorer);
        let interval = std::time::Duration::from_secs(5 * 60); // every 5 min
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let n = db.reap_idle().await;
                if n > 0 {
                    tracing::info!("db explorer: reaped {n} idle SSH tunnel(s)");
                }
                let n = db.reap_idle_handles().await;
                if n > 0 {
                    tracing::info!("db explorer: dropped {n} idle client handle(s)");
                }
            }
        });
    }

    // MCP Control Plane: outbound MCP client + governance pipeline. Server config
    // is the augmented `mcp_servers`; secrets resolve from the same Keychain.
    let mcp = Arc::new(otto_mcp::McpService::new(pool.clone(), secrets.clone()));
    let spawner = Arc::new(PtySpawner {
        pool: pool.clone(),
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
    let attachment_repo = otto_state::ProductAttachmentRepo::new(pool.clone());
    let discovery_repo = otto_state::ProductDiscoveryRepo::new(pool.clone());
    let refinement_repo = otto_state::ProductRefinementRepo::new(pool.clone());
    let mockup_repo = otto_state::ProductMockupRepo::new(pool.clone());
    let discovery_chat_repo = otto_state::DiscoveryChatRepo::new(pool.clone());
    let canvas_repo = otto_state::CanvasRepo::new(pool.clone());
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
            Arc::new(otto_memory::MemoryService::with_defaults(pool.clone()).with_vault(dir))
        }
        _ => memory,
    };
    // Vault v3 — the file-backed docs home (markdown vaults on disk, derived
    // SQLite index, OKF validation). No embeddings anywhere.
    let vault = Arc::new(otto_vault::VaultEngine::new(pool.clone()));

    // One shared auth-lookup cache: the authenticator fills it, the grants route
    // (via ServerCtx.auth_cache) evicts from it on set_grants. Clones share the
    // same backing map (Arc-backed).
    let auth_cache = otto_rbac::AuthCache::new();

    // Runtime custom-plugin supervisor. Plugins live under $OTTO_PLUGINS_HOME (or
    // ~/otto-plugins); enabled ones are spawned below as sidecar processes Otto
    // reverse-proxies. The sidecars call back to the scoped host API at this base.
    let plugins_home = std::env::var_os("OTTO_PLUGINS_HOME")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            dirs::home_dir()
                .unwrap_or_else(|| cfg.data_dir.clone())
                .join("otto-plugins")
        });
    let plugins = std::sync::Arc::new(otto_server::plugins::PluginManager::new(
        otto_state::PluginsRepo::new(pool.clone()),
        plugins_home,
        cfg.data_dir.clone(),
        format!("http://127.0.0.1:{}/api/v1/plugin-host", cfg.port),
    ));

    // Resolve the root user once — owns sessions spawned on behalf of inbound
    // channel/webhook messages. Used both for the webhook bridge here and the
    // channel manager further down. None until onboarding creates a root user.
    let root_user_id: Option<String> = UsersRepo::new(pool.clone())
        .list()
        .await
        .ok()
        .and_then(|users| users.into_iter().find(|u| u.is_root).map(|u| u.id));

    // Webhook-channel bridge: its own Bridge + Mirror, independent of the live
    // Slack/Telegram supervisor, so the public inbound HTTP handler can turn a
    // `POST /webhooks/{ws}` into an agent session. None until a root user exists.
    let channel_bridge = root_user_id.as_ref().map(|uid| {
        otto_channels::Bridge::new(
            Arc::clone(&manager),
            workspaces.clone(),
            SettingsRepo::new(pool.clone()),
            otto_channels::Mirror::new(Arc::clone(&manager)),
            uid.clone(),
        )
    });

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
        base_url: format!("http://127.0.0.1:{}", cfg.port),
        data_dir: cfg.data_dir.clone(),
        plugins: plugins.clone(),
        manager: Arc::clone(&manager),
        workspaces: workspaces.clone(),
        connections,
        db_explorer,
        db_assist: otto_server::db_assist::new_registry(),
        transcript_cache: Default::default(),
        rooms: Default::default(),
        brokers,
        mcp,
        spawner,
        git_store: GitStore::new(pool.clone()),
        issues_store: IssuesRepo::new(pool.clone()),
        integrations_store: IntegrationsRepo::new(pool.clone()),
        channel_bridge,
        reviews_store: ReviewsRepo::new(pool.clone()),
        findings_store: ReviewFindingsRepo::new(pool.clone()),
        finding_events_store: otto_state::FindingEventsRepo::new(pool.clone()),
        repo_rules_store: otto_state::RepoRulesRepo::new(pool.clone()),
        proof_packs_store: otto_state::ReviewProofPacksRepo::new(pool.clone()),
        skill_evals_store: SkillEvalsRepo::new(pool.clone()),
        golden_tasks_store: otto_state::GoldenTasksRepo::new(pool.clone()),
        eval_matrices_store: otto_state::EvalMatricesRepo::new(pool.clone()),
        skill_eval_cancels: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        skill_reviews_store: otto_state::SkillReviewsRepo::new(pool.clone()),
        skill_review_cancels: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        review_cancels: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        review_agent_cancels: Arc::new(std::sync::Mutex::new(std::collections::HashMap::new())),
        wf_skip_current: Arc::new(std::sync::Mutex::new(std::collections::HashSet::new())),
        orchestrator: Arc::clone(&orchestrator),
        improve_engine: Arc::clone(&improve_engine),
        context_library: context_library.clone(),
        usage: Arc::clone(&usage),
        telemetry: Some(Arc::clone(&telemetry)),
        product,
        product_repo,
        attachment_repo,
        discovery_repo,
        refinement_repo,
        mockup_repo,
        discovery_chat_repo,
        canvas_repo,
        product_agent_cancels: otto_server::product_run::new_cancel_registry(),
        design_jobs: otto_server::design_blender::new_job_registry(),
        memory,
        vault,
        vault_docs_runs: otto_server::vault_docs_agent::new_run_registry(),
        vault_docs_refine: otto_server::vault_docs_agent::new_refine_registry(),
        swarm,
        swarm_repo,
        swarm_coords: otto_server::swarm_runtime::new_registry(),
        swarm_run_cancels: otto_server::swarm_run::new_cancel_registry(),
        goal_loops_repo: otto_state::GoalLoopsRepo::new(pool.clone()),
        goal_loops: otto_server::goal_loop::new_registry(),
        workgraph: Arc::new(otto_workgraph::WorkGraphService::new(
            otto_state::WorkGraphRepo::new(pool.clone()),
            events.clone(),
        )),
        scheduled_tasks: otto_state::ScheduledTasksRepo::new(pool.clone()),
        // Proof media lives in a content-addressed file store, not the state DB.
        proof_repo: otto_state::ProofRepo::new(pool.clone())
            .with_media_dir(cfg.data_dir.join("proof-media")),
        proof_locks: otto_server::proof::new_locks(),
        runs: otto_state::RunsRepo::new(pool.clone()),
        runs_engine: otto_server::run_engine::RunEngine::new(),
        browser_tabs: otto_state::BrowserTabsRepo::new(pool.clone()),
        browser_annotations: otto_state::BrowserAnnotationsRepo::new(pool.clone()),
        browser_credentials: otto_state::BrowserCredentialsRepo::new(pool.clone()),
        ui_bridge: Default::default(),
        // Config is captured now; the Lightpanda sidecar itself is only
        // located/started on the first `/browser/page` request (see
        // `BrowserEngineHandle`) so daemon boot never waits on it.
        browser: Arc::new(otto_server::routes::browser::BrowserEngineHandle::new(
            std::env::var("OTTO_LIGHTPANDA_BIN").ok(),
            cfg.data_dir.clone(),
        )),
    };

    boot.mark("modules");

    // Restore sessions from the previous daemon run: resumable agent
    // sessions respawn, everything else becomes reconnectable.
    let ws_paths: std::collections::HashMap<String, String> = workspaces
        .list_all()
        .await
        .map_err(|e| format!("list workspaces: {e}"))?
        .into_iter()
        .map(|w| (w.id, w.root_path))
        .collect();
    match manager
        .restore_all(&move |ws_id| ws_paths.get(ws_id.as_str()).cloned())
        .await
    {
        Ok(summary) => {
            tracing::info!(
                kept_running = summary.kept_running,
                suspended = summary.suspended,
                "session restore"
            );
            otto_server::transport::set_boot_restore(otto_server::transport::BootRestore {
                kept_running: summary.kept_running,
                suspended: summary.suspended,
            });
        }
        Err(e) => tracing::warn!("session restore: {e}"),
    }
    boot.mark("restore");

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
        Ok(n) if n > 0 => {
            tracing::info!("skill-eval recovery: marked {n} orphaned run(s) as error")
        }
        Ok(_) => {}
        Err(e) => tracing::warn!("skill-eval recovery: {e}"),
    }

    // Do not replay API requests with uncertain external outcomes after a crash.
    otto_state::api_runs::ApiRunsRepo(ctx.pool.clone())
        .recover_interrupted()
        .await
        .map_err(|e| e.to_string())?;

    // Workflow recovery: runs a dead daemon left EXECUTING are RESUMED from
    // their persisted per-node progress (adopting finished steps, re-entering
    // at the interrupted one) unless the workflow opts out via
    // `on_restart = 'fail'`, the resume cap is hit, or the interrupted step
    // has external side effects (unknown outcome → failed with a pointer at
    // the manual retry-a-step flow). QUEUED runs (fresh `pending`, parked
    // behind the parallel-run gate) re-enqueue in order so the persistent run
    // queue survives the restart. Order matters: reconcile flips resumable
    // rows back to `pending` BEFORE the worktree sweep below, so their
    // provisioned worktrees survive for the resumed steps.
    {
        let (resumed, settled) =
            otto_server::workflow_engine::reconcile_interrupted_runs(&ctx).await;
        if resumed > 0 || settled > 0 {
            tracing::info!(
                "workflow recovery: resumed {resumed} interrupted run(s), settled {settled}"
            );
        }
    }
    {
        let n = otto_server::workflow_engine::resume_queued_runs(&ctx).await;
        if n > 0 {
            tracing::info!("workflow recovery: re-enqueued {n} queued run(s)");
        }
    }
    // Work no route depends on runs AFTER the listener starts serving
    // (perf2/03 N7): plugin sidecars (a request before they are up gets the
    // proxy's "not running" answer, as during any plugin restart), the
    // launchd-job sweep and the worktree sweep (git subprocesses per run).
    {
        let ctx = ctx.clone();
        tokio::spawn(async move {
            let t = std::time::Instant::now();
            // Spawn enabled runtime plugins (sidecar processes Otto supervises
            // + proxies). Best-effort: per-plugin spawn failures are logged
            // inside, never fatal.
            ctx.plugins.start_enabled().await;
            let plugins_ms = t.elapsed().as_millis();
            // Sweep stray `com.otto.deploy.*` launchd jobs. deploy.sh detaches
            // with nohup — never launchd — so any job under that prefix is an
            // agent's improvisation, and launchd re-runs a submitted job every
            // time it exits: build → app swap → daemon restart → script exits
            // → launchd runs it again, forever (seen 2026-07-16 as
            // `com.otto.deploy.okfv3`). Removing them here caps any such loop
            // at the first restart it causes.
            let _ = tokio::task::spawn_blocking(sweep_stray_deploy_jobs).await;
            // Dead files earlier versions left in the data dir (exact
            // patterns only — see housekeeping::sweep_data_dir).
            let data_dir = ctx.data_dir.clone();
            if let Ok(removed) = tokio::task::spawn_blocking(move || {
                housekeeping::sweep_data_dir(&data_dir, std::time::SystemTime::now())
            })
            .await
            {
                for path in removed {
                    tracing::info!("boot sweep: removed dead file {}", path.display());
                }
            }
            // And leftover workflow run worktrees (+ safe otto-wf/<id> branch
            // cleanup) — finalize-time reaping can't run for a crashed daemon,
            // and pre-reap versions left one worktree per run in the user's
            // real repos. Runs the reconciler chose to resume (above, before
            // this task was spawned) are `pending` again and keep theirs.
            otto_server::workflow_engine::sweep_stale_run_worktrees(&ctx).await;
            tracing::info!(
                "boot: post-listen work done in {} ms (plugins={plugins_ms})",
                t.elapsed().as_millis()
            );
        });
    }

    // Goal loops: each loop's controller dies with the process, so a row left
    // running/paused/blocked is orphaned. Pause active loops, preserve blocked
    // decisions and all working files, and reap only execution resources.
    match ctx
        .goal_loops_repo
        .fail_running("Interrupted by a daemon restart — Resume to continue with preserved work.")
        .await
    {
        Ok(loops) => {
            if !loops.is_empty() {
                tracing::info!(
                    "goal-loop recovery: preserved {} interrupted loop(s)",
                    loops.len()
                );
            }
            for l in &loops {
                otto_server::goal_loop::cleanup_executor_sessions(&ctx, &l.workspace_id, &l.id)
                    .await;
            }
        }
        Err(e) => tracing::warn!("goal-loop recovery: {e}"),
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

    // Nested-agent capture: every ~30s, look inside every LIVE plain `shell`
    // session for an agent CLI the user launched by hand (`claude`, `codex`,
    // `agy`) and record that conversation's id on the session row. A terminal
    // running an agent is no longer stateless — reopening it after a daemon
    // restart respawns the shell and types the provider's resume command, so
    // the conversation comes back instead of dead-ending.
    {
        let manager = Arc::clone(&manager);
        let interval = std::time::Duration::from_secs(30);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let n = manager.capture_nested_agents().await;
                if n > 0 {
                    tracing::info!("captured {n} nested agent conversation(s) in shell session(s)");
                }
            }
        });
    }

    // Opt-in auto-archive: hourly, archive agent sessions idle beyond the
    // `session_auto_archive_days` setting (0/absent = off). Archive keeps the
    // row + history and stays reversible via unarchive.
    {
        let manager = Arc::clone(&manager);
        let interval = std::time::Duration::from_secs(60 * 60);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                let n = manager.auto_archive_stale().await;
                if n > 0 {
                    tracing::info!("auto-archived {n} stale session(s)");
                }
            }
        });
    }

    // Provider-title auto-namer: every ~20s, rename any LIVE foreground agent
    // session the user hasn't named to the provider's own session title (the
    // first user prompt in claude's transcript / codex's rollout). Skips
    // user-named and already-adopted sessions with no disk work, so it stays
    // cheap; each rename broadcasts `SessionRenamed` for a live UI refresh.
    {
        let manager = Arc::clone(&manager);
        let interval = std::time::Duration::from_secs(20);
        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                // Nothing live → no transcript is growing; skip the SQL
                // candidate scan (the first sweep after a session goes live
                // catches anything left unnamed).
                if manager.live_count() == 0 {
                    continue;
                }
                let n = manager.refresh_provider_titles().await;
                if n > 0 {
                    tracing::info!("auto-named {n} session(s) from provider title");
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
            // First pass checks every session; later passes only the sessions
            // that received trail rows since the previous pass started (with
            // a margin for clock skew between writers), which is all that can
            // have grown past the cap.
            let mut since: Option<std::time::SystemTime> = None;
            loop {
                let started = std::time::SystemTime::now();
                let n = manager.prune_activity_trail(KEEP_PER_SESSION, since).await;
                if n > 0 {
                    tracing::info!("pruned {n} old activity-trail row(s)");
                }
                since = started.checked_sub(std::time::Duration::from_secs(10 * 60));
                tokio::time::sleep(interval).await;
            }
        });
    }

    // Audit/event-table retention (work_events, mcp_tool_calls, mcp_call_log,
    // audit_log — all append-only with no other delete path). Policy comes from
    // the `data_retention` setting, re-read each pass so a change applies
    // within the hour; windows are floored in otto-state (never deletes a row
    // younger than its window). See docs/features/backup-restore.md.
    {
        let pool = pool.clone();
        let auth_cache = auth_cache.clone();
        let interval = std::time::Duration::from_secs(60 * 60); // hourly
        let manager = Arc::clone(&manager);
        tokio::spawn(async move {
            let settings = SettingsRepo::new(pool.clone());
            let auth = otto_rbac::AuthRepo::with_cache(pool.clone(), auth_cache);
            let maint_pool = pool.clone();
            let repo = otto_state::RetentionRepo::new(pool);
            let mut compaction_logged = false;
            loop {
                let raw = settings
                    .get(otto_state::retention::SETTING_KEY)
                    .await
                    .ok()
                    .flatten();
                let policy = otto_state::RetentionPolicy::from_setting(raw.as_ref());
                match repo.prune(&policy).await {
                    Ok(r) if r.total() > 0 => tracing::info!(
                        "retention: pruned {} work_events, {} mcp_tool_calls, \
                         {} mcp_call_log, {} audit_log, {} review_agent_prompts, \
                         {} review_diffs, {} notifications, {} room_messages row(s); \
                         run history {:?}",
                        r.work_events,
                        r.mcp_tool_calls,
                        r.mcp_call_log,
                        r.audit_log,
                        r.review_agent_prompts,
                        r.review_diffs,
                        r.notifications,
                        r.room_messages,
                        r.run_history
                    ),
                    Ok(_) => {}
                    Err(e) => tracing::warn!("retention prune failed: {e}"),
                }
                // Expired credentials (14-daemon-perf P10): a week of grace
                // past expiry, then gone; the cache drops each hash too.
                match auth.purge_expired(7).await {
                    Ok(n) if n > 0 => {
                        tracing::info!("retention: purged {n} expired auth session(s)")
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("expired auth-session purge failed: {e}"),
                }
                // SQLite housekeeping (P3): planner stats, WAL truncate, and
                // incremental vacuum once the DB was compacted by an admin.
                match otto_state::maintenance::hourly(&maint_pool).await {
                    Ok(m) => {
                        if m.checkpoint_fell_back {
                            tracing::debug!("db maintenance: WAL busy, ran a PASSIVE checkpoint");
                        }
                        if m.vacuumed_pages > 0 {
                            tracing::info!(
                                "db maintenance: reclaimed {} free page(s)",
                                m.vacuumed_pages
                            );
                        }
                    }
                    Err(e) => tracing::warn!("db maintenance failed: {e}"),
                }
                // One-time auto-compaction (perf F1). A SMALL file (≤ the
                // boot-inline cap, ~1–2 s) is rewritten in place while no
                // session is live. A large one is never rewritten under a
                // running daemon — its write lock would stall every route for
                // the whole VACUUM; the next start compacts it offline before
                // the pool opens (perf2/03 N1, `offline_compact_at_boot`).
                if let Ok(s) = otto_state::maintenance::stats(&maint_pool).await {
                    if otto_state::maintenance::needs_compaction(&s) {
                        let live = s.size_bytes() - s.free_bytes();
                        if live > otto_state::maintenance::BOOT_COMPACT_MAX_LIVE_BYTES {
                            if !compaction_logged {
                                compaction_logged = true;
                                tracing::info!(
                                    "db maintenance: {} of {} bytes free — compacting offline at \
                                     the next start (≈{} ms, no write stall)",
                                    s.free_bytes(),
                                    s.size_bytes(),
                                    otto_state::maintenance::estimate_offline_compact_ms(&s)
                                );
                            }
                        } else if manager.live_count() == 0 {
                            match otto_state::maintenance::compact(&maint_pool).await {
                                Ok(r) => tracing::info!(
                                    "db maintenance: auto-compacted {} → {} bytes in {} ms",
                                    r.before_bytes,
                                    r.after_bytes,
                                    r.duration_ms
                                ),
                                Err(e) => tracing::warn!("db auto-compaction failed: {e}"),
                            }
                        }
                    }
                }
                tokio::time::sleep(interval).await;
            }
        });
    }

    // Workflow run-history retention (08-workflows R1): daily, first pass at
    // startup. Per workflow keep the newest 200 terminal runs AND every run
    // younger than 30 days; never active/approval-parked runs or runs a
    // Proof Pack / scheduled-task run references (all enforced in
    // `WorkflowsRepo::prune_runs`). Each pruned run's
    // `workflow-context/<run_id>/` dir is removed, confined to that root.
    {
        let pool = pool.clone();
        let ctx_root = cfg.data_dir.join("workflow-context");
        let interval = std::time::Duration::from_secs(24 * 60 * 60);
        tokio::spawn(async move {
            let repo = otto_state::WorkflowsRepo::new(pool);
            loop {
                match repo
                    .prune_runs(
                        otto_state::workflows::RUN_RETENTION_KEEP,
                        otto_state::workflows::RUN_RETENTION_DAYS,
                    )
                    .await
                {
                    Ok(ids) if !ids.is_empty() => {
                        let root = ctx_root.clone();
                        let n = ids.len();
                        // Runs inside spawn_blocking (std fs is fine there).
                        #[allow(clippy::disallowed_methods)]
                        let dirs = tokio::task::spawn_blocking(move || {
                            ids.iter()
                                .filter_map(|id| otto_core::paths::confine_join(&root, id))
                                .filter(|d| d.is_dir() && std::fs::remove_dir_all(d).is_ok())
                                .count()
                        })
                        .await
                        .unwrap_or(0);
                        tracing::info!(
                            "retention: pruned {n} workflow run(s), removed {dirs} context dir(s)"
                        );
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!("workflow run retention failed: {e}"),
                }
                tokio::time::sleep(interval).await;
            }
        });
    }

    // Gated by OTTO_SELF_IMPROVE (enabled by default; 0/false/off disables it).
    // Computed here so the channel manager can decide whether to wire the
    // self-improvement-on-interaction hook below.
    let self_improve_enabled = !matches!(
        std::env::var("OTTO_SELF_IMPROVE").as_deref(),
        Ok("0") | Ok("false") | Ok("off")
    );

    // --- Channel Manager (Telegram-first, Slack-ready) ---
    // Spawns agent sessions on behalf of incoming channel messages as the root
    // user resolved above (None when onboarding hasn't created one yet → skip).
    let _channel_handle = if let Some(uid) = root_user_id {
        let mut cm = ChannelManager::new(
            Arc::clone(&manager),
            workspaces.clone(),
            IntegrationsRepo::new(pool.clone()),
            SettingsRepo::new(pool.clone()),
            secrets.clone(),
            uid.clone(),
            // Share the daemon event bus so the proactive self-improvement
            // notifier can mirror Improvement* events to the user's channels
            // (opt-in via the `channels.notify_self_improvement` setting).
            Some(events.clone()),
        )
        // An inbound message on a swarm-bound channel launches that swarm.
        .with_swarm_trigger(std::sync::Arc::new(
            otto_server::swarm_channels::SwarmTriggerImpl { ctx: ctx.clone() },
        ))
        // An inbound `/run <ref>` (or `approve`/`reject` reply) drives a Run with
        // Otto run on the root user's behalf (the channel-trust model).
        .with_run_trigger(std::sync::Arc::new(
            otto_server::run_channels::ChannelRunTrigger::new(ctx.clone(), uid.clone()),
        ))
        // A structured `Action: Workflow` message starts a workflow run by name.
        .with_workflow_trigger(std::sync::Arc::new(
            otto_server::workflow_chat::WorkflowChatTriggerImpl { ctx: ctx.clone() },
        ));
        // When self-improvement is on, learn from each finished channel
        // interaction and reply with the result in the same thread (per-workspace
        // `self_improvement.enabled` is re-checked inside the hook).
        if self_improve_enabled {
            cm = cm.with_improver(std::sync::Arc::new(
                otto_server::improve_channels::InteractionImproverImpl::new(
                    Arc::clone(&improve_engine),
                    workspaces.clone(),
                    SessionsRepo::new(pool.clone()),
                    ImprovementsRepo::new(pool.clone()),
                ),
            ));
        }
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
    // Gated by OTTO_SELF_IMPROVE (computed above): enabled by default; set
    // OTTO_SELF_IMPROVE=0 (or false/off) to disable both the per-workspace
    // self-reflection scheduler and the live in-loop evolver.
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
    // Agent UI control: a removed session's in-flight UI commands end.
    otto_server::ui_bridge::spawn_session_watch(ctx.clone());
    tracing::info!("credential monitor + session-event notices started");

    // --- Conversation view (docs/design/conversation-view.md) ---
    // Board→agent nudge sweep: hands `POST /sessions/{id}/tasks` rows to the
    // agent's PTY once the session is idle (SessionStatus events + a 15 s tick).
    // History index: a low-priority boot walk of ~/.claude/projects and
    // ~/.codex/sessions (skips unchanged files); `POST …/history/rescan`
    // re-runs it on demand.
    let _nudge_handle = otto_server::agent_tasks_nudge::spawn(ctx.clone());
    otto_server::history_index::spawn_scan(ctx.clone(), None);
    tracing::info!("conversation view: nudge sweep + history index scan started");

    // --- Orphan reaper: auto-resume analysis agents stranded by a restart ---
    // Runs once at startup; any analysis agent still 'running'/'waiting' has no
    // surviving task, so it is re-run (capped) or marked errored + notified.
    tokio::spawn(otto_server::product_run::reap_orphaned_agents_on_startup(
        ctx.clone(),
    ));

    // --- Design Hall: FTS index + idempotent legacy import (background) ---
    // Mirrors Product-arena design attachments and Canvas scenes into the
    // design graph (graph rows only; the legacy rows/files are never touched)
    // and re-syncs a `sync` version when a legacy source changed.
    otto_server::design_hall::spawn_startup_import(&ctx);
    // Move legacy inline proof media into the file store + daily GC.
    otto_server::proof::spawn_media_maintenance(ctx.proof_repo.clone());

    // --- Vault docs-runs recovery: this restart killed any in-flight run ---
    // Flip still-non-terminal persisted runs to 'interrupted' and soft-trash
    // their orphaned `_drafts/docs-run-*` dirs (multi-writer runs only).
    tokio::spawn(otto_server::vault_docs_agent::recover_interrupted(
        ctx.clone(),
    ));

    // --- Insights scheduler: opt-in, catch-up usage reports ---
    // Background supervisor that ticks ~hourly and, for each ENABLED cadence
    // (daily/weekly/monthly — all default OFF), runs the `insights` skill for the
    // most-recent missed period iff it has no report yet. Runs the due-check on
    // startup (catch-up after the app was closed), then hourly.
    let _insights_scheduler_handle =
        otto_insights::InsightsScheduler::new(ctx.clone()).start();
    tracing::info!("insights scheduler started");

    // Daily CLI auto-update: updates the agent CLIs (claude/codex/…) at a
    // user-configurable local time (default 07:00, opt-out via settings) and
    // force-reloads open agent sessions onto the new binary (resume-aware).
    // Catch-up on a missed window via a last-run cursor, like insights.
    // OTTO_CLI_UPDATE=0 disables the scheduler entirely — throwaway dev/E2E
    // daemons have no last-run cursor, so the catch-up fires at startup and
    // its session reload kills agent sessions seconds after they spawn.
    let _cli_update_handle = if std::env::var("OTTO_CLI_UPDATE").is_ok_and(|v| v == "0") {
        tracing::info!("cli auto-update scheduler disabled (OTTO_CLI_UPDATE=0)");
        None
    } else {
        let h = otto_server::cli_update::CliUpdateScheduler::new(ctx.clone()).start();
        tracing::info!("cli auto-update scheduler started");
        Some(h)
    };

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
        Ok(n) if n > 0 => tracing::info!("swarm recovery: marked {n} orphaned run(s) as stopped"),
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

    // --- Workflow event-trigger listener (B8) ---
    // Subscribes to the daemon event bus; for each event whose kind matches an
    // enabled `event`-kind trigger's `event_kind` spec field, starts a workflow
    // run via the same path as schedule/webhook triggers. Best-effort: errors
    // inside the listener are logged and never propagate to the event producer.
    let _workflow_event_trigger_handle = spawn_workflow_event_trigger_listener(ctx.clone());
    // Notification-center notices for failed / waiting workflow runs and goal
    // loops (review 08 · N1); scheduled tasks and personal agents notify inline.
    otto_server::run_notices::spawn_listener(ctx.clone());
    tracing::info!("workflow event-trigger listener started");

    // --- Workflow schedule-trigger scheduler ---
    // 60 s tick that fires `schedule`-kind triggers on their cadence (interval /
    // daily / weekly / cron, timezone-aware via the shared cadence engine) and
    // starts a workflow run. Mirrors the swarm / scheduled-tasks supervisors.
    let _workflow_schedule_trigger_handle =
        otto_server::workflow_trigger_scheduler::start(ctx.clone());
    tracing::info!("workflow schedule-trigger scheduler started");

    // --- Kubernetes monitor supervisor ---
    // One collector loop per cluster with monitoring enabled; reconciles every
    // 15 s against `k8s_monitor_configs` and restarts a loop on config change.
    let _k8s_monitor_handle = otto_server::k8s_monitor_scheduler::start(ctx.clone());
    tracing::info!("k8s monitor scheduler started");

    // --- Mission Control / work-graph projector ---
    // Subscribes to the daemon event bus and materializes every agentic activity
    // into the unified work graph (live), plus a 60 s reconcile + boot backfill
    // that re-derive from the authoritative repos and refresh per-session cost.
    let _workgraph_projector_handle = otto_server::workgraph_projector::spawn(ctx.clone());
    tracing::info!("workgraph projector started");

    // --- Scheduled Tasks ---
    // Reap runs a previous daemon life left `running` BEFORE the router serves:
    // reaping inside the spawned supervisor raced the first "Run now" and
    // marked that fresh run "interrupted by daemon restart" (like the
    // workflow/swarm recovery above, this is awaited). Then fire due recurring
    // jobs (interval/daily/weekly/cron) with bounded concurrency.
    otto_server::scheduled_tasks_scheduler::reap_interrupted(&ctx).await;
    let _scheduled_tasks_handle = otto_server::scheduled_tasks_scheduler::start(ctx.clone());
    tracing::info!("scheduled tasks scheduler started");

    // --- Personal Agents ---
    // Fires every enabled schedule of every enabled personal agent (per-schedule
    // cursor), reaps interrupted runs on startup, and bounds concurrency.
    let _personal_agents_handle = otto_server::personal_agents_scheduler::start(ctx.clone());
    tracing::info!("personal agents scheduler started");

    // --- Otto Assistant ---
    // 30 s tick: fires due reminders (`once`), reports finished delegations
    // into their thread, syncs approvals decided in the MCP queue, and deletes
    // incognito threads 24 h after their last turn.
    let _assistant_handle = otto_server::assistant::start(ctx.clone());
    tracing::info!("assistant supervisor started");

    // --- Run with Otto ---
    // Boot reaper (fail interrupted runs, re-drive resumable ones) + a 30 s tick
    // that re-drives still-active runs. The engine drives the stage machine.
    otto_server::run_scheduler::spawn(ctx.clone());
    tracing::info!("run-with-otto scheduler started");

    // --- Dynamic model catalog ---
    // Hourly per-provider model discovery (CLI probe / docs scrape / models.dev
    // fallback); a failed chain keeps the last good list.
    otto_server::model_catalog::spawn_refresher(ctx.clone());
    tracing::info!("model-catalog refresher started");

    // --- Usage tracking + system metrics (embedded ClickHouse) ---
    // The recorder mines usage from the activity-trail event stream; the
    // metrics sampler writes CPU/RAM; the budget sampler rides the metrics tick
    // to check spend-vs-cap and emit BudgetExceeded events (with dedupe). All
    // three are cheap no-ops until ClickHouse is available.
    spawn_usage_recorder(ctx.clone());
    spawn_metrics_sampler(ctx.clone());
    spawn_budget_sampler(ctx.clone());
    // ClickHouse comes up in the background (UsageEngine::start returns at
    // once), so `available()` here is almost always still false — report the
    // outcome once the bring-up has actually resolved.
    {
        let usage = Arc::clone(&usage);
        tokio::spawn(async move {
            if usage.wait_ready(std::time::Duration::from_secs(120)).await {
                tracing::info!("usage tracking started (embedded clickhouse)");
            } else {
                tracing::info!(
                    "usage tracking idle (embedded clickhouse not available after 120 s — \
                     not installed, disabled, or still starting)"
                );
            }
        });
    }

    // --- Usage tailer: real token usage from Claude + Codex CLI transcripts ---
    // Tails the CLIs' on-disk JSONL transcripts and records exact per-turn token
    // usage/cost into the usage store (a persistent byte-offset cursor prevents
    // double-counting; pre-existing history is skipped to avoid misdated rows).
    // agy is unsupported (its on-disk usage is encrypted).
    let _usage_tailer_handle = {
        // An isolated E2E daemon must not import the developer's real provider
        // history. Besides privacy, that startup scan distorts load baselines.
        let transcript_home = if std::env::var("OTTO_E2E").as_deref() == Ok("1") {
            cfg.data_dir.join("fixture-home")
        } else {
            dirs::home_dir().unwrap_or_else(|| cfg.data_dir.clone())
        };
        let tailer = usage_tailer::UsageTailer::new(
            Arc::clone(&ctx.usage),
            pool.clone(),
            cfg.data_dir.clone(),
            transcript_home,
        );
        let handle = tailer.start();
        tracing::info!("usage tailer: started (claude+codex; agy unsupported)");
        handle
    };

    // MCP Control Plane: periodic health sweep of managed servers (ping/initialize
    // → status+latency). Interval from `mcp_health_interval_secs` (default 300;
    // 0 disables). Best-effort; failures only update a server's health row.
    // Lazy for stdio servers: only those used in the last 6 h are checked (see
    // `McpService::health_sweep`), so an idle daemon spawns no MCP processes.
    {
        let mcp = Arc::clone(&ctx.mcp);
        let settings = otto_state::SettingsRepo::new(pool.clone());
        tokio::spawn(async move {
            loop {
                let secs = settings
                    .get("mcp_health_interval_secs")
                    .await
                    .ok()
                    .flatten()
                    .and_then(|v| v.as_u64())
                    .unwrap_or(300);
                if secs == 0 {
                    tokio::time::sleep(std::time::Duration::from_secs(300)).await;
                    continue;
                }
                tokio::time::sleep(std::time::Duration::from_secs(secs)).await;
                mcp.health_sweep().await;
            }
        });
        tracing::info!("mcp control plane: health sweep started");
    }

    let (api_extras, root_extras) = module_routers(&ctx);
    // Kept past `build_router` (which takes the ctx) so shutdown can stop the
    // remote live browser's Chromium processes.
    let browser_handle = ctx.browser.clone();
    #[cfg(feature = "embed-ui")]
    let assets: Option<otto_server::spa::AssetLoader> = Some(ui_assets::load);
    #[cfg(not(feature = "embed-ui"))]
    let assets = None;
    let router = build_router_with_assets(ctx, api_extras, root_extras, assets);

    // Graceful shutdown signal (ctrl_c or SIGTERM) fanned out via watch.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    tokio::spawn(async move {
        wait_for_signal().await;
        tracing::info!("shutdown signal received");
        // Wake the long-poll handlers first so the drain below has nothing
        // left that would otherwise hold it for 25–30 s.
        otto_server::shutdown::begin();
        let _ = shutdown_tx.send(true);
    });

    // (Bound at the very top of `run` — the single-instance lock.)
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
                        shutdown_handle.graceful_shutdown(Some(HTTP_DRAIN_CAP));
                    });
                    network_task = Some(tokio::spawn(async move {
                        // `into_make_service_with_connect_info::<SocketAddr>` makes
                        // each request's real socket peer available to handlers via
                        // `ConnectInfo<SocketAddr>` (used by the login throttle, S5).
                        if let Err(e) = axum_server::bind_rustls(addr, tls)
                            .handle(handle)
                            .serve(
                                router
                                    .into_make_service_with_connect_info::<std::net::SocketAddr>(),
                            )
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

    let alt_task = alt_loopback.map(|listener| {
        // Advertised as `localhost`, not `[::1]`: a distinct pool key from
        // `127.0.0.1` either way, CSP host-sources cannot express IPv6
        // literals (the Tauri CSP already allows `localhost:7700`), and with
        // BOTH loopback addresses held by this process `localhost` reaches
        // this daemon whichever one the resolver picks.
        let base = format!("http://localhost:{}", cfg.port);
        tracing::info!("alt loopback lane on {base} ([::1] + 127.0.0.1 both held)");
        otto_server::transport::set_alt_loopback_base(base);
        let router = router.clone();
        let mut rx = shutdown_rx.clone();
        tokio::spawn(async move {
            let shutdown = async move {
                let _ = rx.changed().await;
            };
            if let Err(e) = axum::serve(
                listener,
                router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
            )
            .with_graceful_shutdown(shutdown)
            .await
            {
                tracing::error!("alt loopback listener: {e}");
            }
        })
    });

    boot.finish();

    let mut rx = shutdown_rx.clone();
    let shutdown = async move {
        let _ = rx.changed().await;
    };
    // `into_make_service_with_connect_info::<SocketAddr>` exposes the real socket
    // peer to handlers via `ConnectInfo<SocketAddr>` — the login throttle keys on
    // it instead of a spoofable `X-Forwarded-For` header (audit S5).
    let serve = axum::serve(
        loopback,
        router.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown)
    .into_future();
    // The graceful drain waits for EVERY in-flight request, unbounded. Cap it
    // once the signal fires: long-polls already return early (shutdown::begin),
    // and anything still running after HTTP_DRAIN_CAP is abandoned so the
    // teardown below finishes inside launchd's ExitTimeOut.
    let mut drain_rx = shutdown_rx.clone();
    let drain_deadline = async move {
        let _ = drain_rx.wait_for(|stop| *stop).await;
        tokio::time::sleep(HTTP_DRAIN_CAP).await;
    };
    tokio::select! {
        res = serve => res.map_err(|e| format!("serve: {e}"))?,
        _ = drain_deadline => tracing::warn!(
            "http drain exceeded {} s — abandoning in-flight requests",
            HTTP_DRAIN_CAP.as_secs()
        ),
    }

    // The other listeners got the same signal; they get no extra time.
    for task in [network_task, alt_task].into_iter().flatten() {
        if tokio::time::timeout(std::time::Duration::from_millis(500), task)
            .await
            .is_err()
        {
            tracing::warn!("secondary listener did not drain in time — abandoned");
        }
    }

    // Terminate every live PTY so a daemon stop / system shutdown never leaves
    // orphaned agent processes behind — except the sessions running in PTY
    // holders (setting `session_persistence`, default on): those are detached
    // and re-adopted, still running, by the next daemon start.
    // Every teardown step is bounded: ExitTimeOut (30 s) minus the drain cap
    // leaves ~27 s, and a hung step must not cost the ClickHouse flush after it.
    let (killed, kept) = match tokio::time::timeout(
        std::time::Duration::from_secs(12),
        manager.shutdown_for_restart(),
    )
    .await
    {
        Ok(counts) => counts,
        Err(_) => {
            tracing::warn!("session shutdown exceeded 12 s — continuing teardown");
            (0, 0)
        }
    };
    if kept > 0 {
        tracing::info!(
            "left {kept} session(s) running in their pty holders for the next daemon start"
        );
    }
    // Close remote live sessions and stop their Chromium processes (no-op when
    // the remote live view was never used this run).
    if tokio::time::timeout(
        std::time::Duration::from_secs(3),
        browser_handle.shutdown_live(),
    )
    .await
    .is_err()
    {
        tracing::warn!("live browser shutdown exceeded 3 s");
    }
    if killed > 0 {
        tracing::info!("terminated {killed} live session(s) on shutdown");
    }
    // Stop the embedded ClickHouse server cleanly (SIGTERM → flush) so its data
    // dir lock is released and the next daemon start doesn't have to reclaim it.
    if tokio::time::timeout(std::time::Duration::from_secs(3), telemetry.shutdown())
        .await
        .is_err()
    {
        tracing::warn!("telemetry shutdown exceeded 3 s");
    }
    if tokio::time::timeout(std::time::Duration::from_secs(6), usage.shutdown())
        .await
        .is_err()
    {
        tracing::warn!("usage (clickhouse) shutdown exceeded 6 s");
    }
    housekeeping::clear_running(&cfg.data_dir);
    tracing::info!("ottod stopped");
    Ok(())
}

/// Where this daemon's PTY holders live (`<data_dir>/pty-holders`, or a
/// per-user temp fallback when that path is too long for a unix socket) and
/// how to start one: this very binary, `ottod pty-holder`. `None` (logged)
/// disables session persistence for this run.
fn pty_holder_config(
    data_dir: &std::path::Path,
) -> Option<otto_sessions::pty_holder::HolderConfig> {
    use otto_sessions::pty_holder::{HolderConfig, HolderLauncher};
    let launcher = match HolderLauncher::current_exe(vec!["pty-holder".into()]) {
        Ok(l) => l,
        Err(e) => {
            tracing::warn!("session persistence off: cannot locate the ottod binary: {e}");
            return None;
        }
    };
    let config = HolderConfig::for_data_dir(data_dir, launcher);
    if config.is_none() {
        tracing::warn!(
            data_dir = %data_dir.display(),
            "session persistence off: no pty-holder socket directory fits the unix socket path limit"
        );
    }
    config
}

/// launchd starts agents with a bare PATH (`/usr/bin:/bin:...`), which hides
/// user-installed CLIs (claude in ~/.local/bin, codex in ~/.bun/bin, homebrew
/// git, language servers in ~/go/bin or a custom npm prefix, ...). Prepend the
/// usual tool directories — plus the *discovered* npm-global and GOPATH bins —
/// so detection and PTY spawns see the same commands the user's shell does.
/// Remove any `com.otto.deploy.*` launchd jobs left by a previous run. Otto's
/// own deploy path (packaging/deploy.sh) only registers `com.otto.deploy-finish`
/// (a hyphen, not matched here; packaging/deploy.sh also boots out exited
/// `com.otto.deploy-once.*` leftovers), so a job under this prefix can only be a coding agent's ad-hoc
/// `launchctl submit` — and launchd relaunches a submitted job on every exit,
/// turning one deploy into an endless build → swap → restart loop that
/// outlives the session that started it. Best-effort and macOS-only by
/// construction (launchctl is absent elsewhere, and probe_cmd just fails).
/// Wall time per boot phase (perf2/03 N4/N7). `finish` logs ONE line —
/// `boot: ready in N ms (db_compact=… db_open=… …)` — that the perf budget
/// script parses for its boot_ms and per-phase budgets.
struct BootPhases {
    started: std::time::Instant,
    last: std::time::Instant,
    phases: Vec<(&'static str, u128)>,
}

impl BootPhases {
    fn start() -> Self {
        let now = std::time::Instant::now();
        Self {
            started: now,
            last: now,
            phases: Vec::new(),
        }
    }

    /// Close the phase that ended now.
    fn mark(&mut self, phase: &'static str) {
        let now = std::time::Instant::now();
        self.phases.push((phase, (now - self.last).as_millis()));
        self.last = now;
    }

    fn line(&self) -> String {
        let parts: Vec<String> = self
            .phases
            .iter()
            .map(|(p, ms)| format!("{p}={ms}"))
            .collect();
        format!(
            "boot: ready in {} ms ({})",
            self.started.elapsed().as_millis(),
            parts.join(" ")
        )
    }

    fn finish(&mut self) {
        self.mark("recovery");
        tracing::info!("{}", self.line());
    }
}

// Sync by design: spawned on the blocking pool after the listener starts.
#[allow(clippy::disallowed_methods)]
fn sweep_stray_deploy_jobs() {
    let Some(list) = probe_cmd("launchctl", &["list"]) else {
        return;
    };
    // `launchctl list` rows are "PID\tStatus\tLabel"; the label is column 3.
    for label in list
        .lines()
        .filter_map(|l| l.split_whitespace().nth(2))
        .filter(|l| l.starts_with("com.otto.deploy."))
    {
        match std::process::Command::new("launchctl")
            .args(["remove", label])
            .status()
        {
            Ok(st) if st.success() => {
                tracing::warn!(%label, "removed stray deploy launchd job (would re-run deploy.sh on every exit)")
            }
            Ok(st) => tracing::warn!(%label, "stray deploy launchd job: remove exited {st}"),
            Err(e) => tracing::warn!(%label, "stray deploy launchd job: remove failed: {e}"),
        }
    }
}

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
        .filter(|p| {
            !current.split(':').any(|c| c == p.as_str()) && std::path::Path::new(p).is_dir()
        })
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
// Sync by design: called from `augment_path` before the runtime starts and
// from `sweep_stray_deploy_jobs`, which runs on the blocking pool.
#[allow(clippy::disallowed_methods)]
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
        let key_pem = cert.signing_key.serialize_pem();
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
