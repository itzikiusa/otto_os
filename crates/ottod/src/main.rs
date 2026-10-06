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

use otto_server::boot::{self, BootConfig, BootPhases, BuiltCtx};
use otto_server::modules::module_routers;
use otto_server::{build_router_with_assets, ServerCtx};
use otto_state::SettingsRepo;
use tokio::sync::watch;
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
    // `ottod apiclient-script`: one API-client pre/post script, out of
    // process so a runaway / OOM script never takes the daemon down.
    if std::env::args().nth(1).as_deref() == Some(otto_server::API_SCRIPT_HELPER_ARG) {
        return if otto_server::run_api_script_helper() {
            ExitCode::SUCCESS
        } else {
            ExitCode::FAILURE
        };
    }
    if let Ok(exe) = std::env::current_exe() {
        otto_server::register_api_script_host(exe);
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
    let code = match runtime.block_on(run(cfg)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("ottod failed: {e}");
            ExitCode::FAILURE
        }
    };
    // Dropping a Runtime waits, unbounded, for in-flight `spawn_blocking`
    // work (a usage-tailer transcript rebuild, a long `git` call): a stop
    // during one would hang past "ottod stopped" until launchd's SIGKILL.
    runtime.shutdown_timeout(RUNTIME_SHUTDOWN_CAP);
    code
}

/// Daily `ottod.log.*` files kept (≈ a month).
const MAX_LOG_FILES: usize = 30;

/// How long the HTTP servers may drain in-flight requests after the shutdown
/// signal. launchd SIGKILLs at `ExitTimeOut` (30 s in the plist); the drain
/// plus the bounded teardown steps below must fit well inside it.
const HTTP_DRAIN_CAP: std::time::Duration = std::time::Duration::from_secs(3);

/// The whole post-drain teardown (sessions, live browser, telemetry,
/// ClickHouse) shares this one deadline. Worst case from SIGTERM: the 3 s
/// drain, 1 s of secondary listeners, this 18 s and the 2 s runtime shutdown
/// make 24 s, leaving ≥6 s of launchd's 30 s `ExitTimeOut` for the log flush.
const TEARDOWN_BUDGET: std::time::Duration = std::time::Duration::from_secs(18);

/// Reserved out of [`TEARDOWN_BUDGET`] for the ClickHouse stop (SIGTERM →
/// flush → release the data-dir lock): the earlier steps can never eat it.
const CLICKHOUSE_RESERVE: std::time::Duration = std::time::Duration::from_secs(6);

/// How long the tokio runtime may wait for blocking tasks after `run`.
const RUNTIME_SHUTDOWN_CAP: std::time::Duration = std::time::Duration::from_secs(2);

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

    // Graceful shutdown signal (ctrl_c or SIGTERM) fanned out via watch —
    // installed FIRST, before any boot step: with the default action a
    // SIGTERM mid-boot (a deploy kickstart, the supervisor, a user quit)
    // killed the process inside the pre-migration snapshot or a compaction.
    // A signal during boot is remembered; boot runs to its end and the
    // listener below then drains at once and tears down normally.
    let (shutdown_tx, shutdown_rx) = watch::channel(false);
    let signals = ShutdownSignals::install();
    tokio::spawn(async move {
        signals.recv().await;
        tracing::info!("shutdown signal received");
        // Wake the long-poll handlers first so the drain below has nothing
        // left that would otherwise hold it for 25–30 s.
        otto_server::shutdown::begin();
        let _ = shutdown_tx.send(true);
    });

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
    let alt_loopback = bind_alt_loopback(cfg.port).await;

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

    // Composition root (otto_server::boot): state DB → modules + ServerCtx →
    // crash recovery → background tasks. Every step is awaited in order, so
    // all reaps finish before the router serves.
    let boot_cfg = BootConfig {
        data_dir: cfg.data_dir.clone(),
        db_path: cfg.db_path(),
        port: cfg.port,
        version: env!("CARGO_PKG_VERSION").to_string(),
        pty_holders: pty_holder_config(&cfg.data_dir),
    };
    let pool = boot::open_state(&boot_cfg, &mut boot).await?;
    let secrets = otto_keychain::from_env(&cfg.data_dir);
    let BuiltCtx { ctx, root_user_id } = boot::build_ctx(&boot_cfg, pool, secrets).await?;
    boot.mark("modules");
    boot::recover_before_serve(&ctx, &mut boot).await?;
    boot::spawn_post_listen_work(&ctx, post_listen_housekeeping);
    boot::recover_goal_loops(&ctx).await;
    let mut background = boot::spawn_background(&ctx, root_user_id).await;
    background.keep(start_usage_tailer(&cfg, &ctx));

    // Kept past `build_router` (which takes the ctx) for the listeners and
    // the teardown.
    let pool = ctx.pool.clone();
    let manager = Arc::clone(&ctx.manager);
    let usage = Arc::clone(&ctx.usage);
    let telemetry = ctx.telemetry.clone();
    // Shutdown stops the remote live browser's Chromium processes.
    let browser_handle = ctx.browser.clone();
    let (api_extras, root_extras) = module_routers(&ctx);
    #[cfg(feature = "embed-ui")]
    let assets: Option<otto_server::spa::AssetLoader> = Some(ui_assets::load);
    #[cfg(not(feature = "embed-ui"))]
    let assets = None;
    let router = build_router_with_assets(ctx, api_extras, root_extras, assets);
    if *shutdown_rx.borrow() {
        tracing::info!("shutdown requested during boot — boot finished, stopping now");
    }

    // (Bound at the very top of `run` — the single-instance lock.)
    tracing::info!("listening on http://127.0.0.1:{}", cfg.port);
    let network_task = serve_network_listener(&cfg, &pool, &router, &shutdown_rx).await?;
    let alt_task = alt_loopback.map(|l| serve_alt_loopback(l, cfg.port, &router, &shutdown_rx));

    boot.finish();

    serve_loopback(loopback, router, &shutdown_rx).await?;

    // The other listeners got the same signal; they get no extra time.
    for task in [network_task, alt_task].into_iter().flatten() {
        if tokio::time::timeout(std::time::Duration::from_millis(500), task)
            .await
            .is_err()
        {
            tracing::warn!("secondary listener did not drain in time — abandoned");
        }
    }

    teardown(&manager, &browser_handle, telemetry.as_deref(), &usage).await;
    housekeeping::clear_running(&cfg.data_dir);
    tracing::info!("ottod stopped");
    drop(background);
    Ok(())
}

/// Second LOOPBACK host for the same port (TRANSPORT_PLAN §3d). Browsers
/// cap HTTP/1.1 sockets per host (~6, shared by every Otto window), so the
/// UI sends background polls + known-slow calls to `[::1]` and keeps
/// `127.0.0.1` for what the user clicked. Still loopback-only; fail-soft
/// (IPv6 off / port taken → no alias, and `/meta` advertises none, so the
/// UI never sends its token to an address this daemon does not hold).
/// `OTTO_ALT_LOOPBACK=0` turns it off.
async fn bind_alt_loopback(port: u16) -> Option<tokio::net::TcpListener> {
    if std::env::var("OTTO_ALT_LOOPBACK").as_deref() == Ok("0") {
        return None;
    }
    match tokio::net::TcpListener::bind((std::net::Ipv6Addr::LOCALHOST, port)).await {
        Ok(l) => Some(l),
        Err(e) => {
            tracing::warn!("alt loopback [::1]:{port} not bound ({e}) — single-host transport");
            None
        }
    }
}

/// The binary's own post-listen sweeps, run on the blocking pool by
/// `boot::spawn_post_listen_work` once plugins are up.
fn post_listen_housekeeping(data_dir: std::path::PathBuf) {
    // Sweep stray `com.otto.deploy.*` launchd jobs. deploy.sh's own
    // finish job is `com.otto.deploy-finish` (a hyphen — not matched), so
    // any job under the dotted prefix is an agent's improvisation, and
    // launchd re-runs a submitted job every time it exits: build → app
    // swap → daemon restart → script exits → launchd runs it again,
    // forever (seen 2026-07-16 as `com.otto.deploy.okfv3`). Removing them
    // here caps any such loop at the first restart it causes.
    sweep_stray_deploy_jobs();
    // Dead files earlier versions left in the data dir (exact
    // patterns only — see housekeeping::sweep_data_dir).
    for path in housekeeping::sweep_data_dir(&data_dir, std::time::SystemTime::now()) {
        tracing::info!("boot sweep: removed dead file {}", path.display());
    }
}

/// Usage tailer: real token usage from Claude + Codex CLI transcripts. Tails
/// the CLIs' on-disk JSONL transcripts and records exact per-turn token
/// usage/cost into the usage store (a persistent byte-offset cursor prevents
/// double-counting; pre-existing history is skipped to avoid misdated rows).
/// agy is unsupported (its on-disk usage is encrypted).
fn start_usage_tailer(cfg: &Config, ctx: &ServerCtx) -> usage_tailer::UsageTailerHandle {
    // An isolated E2E daemon must not import the developer's real provider
    // history. Besides privacy, that startup scan distorts load baselines.
    let transcript_home = if std::env::var("OTTO_E2E").as_deref() == Ok("1") {
        cfg.data_dir.join("fixture-home")
    } else {
        dirs::home_dir().unwrap_or_else(|| cfg.data_dir.clone())
    };
    let tailer = usage_tailer::UsageTailer::new(
        Arc::clone(&ctx.usage),
        ctx.pool.clone(),
        cfg.data_dir.clone(),
        transcript_home,
    );
    let handle = tailer.start();
    tracing::info!("usage tailer: started (claude+codex; agy unsupported)");
    handle
}

/// Optional network listener from the settings table. Unlike loopback, the
/// 0.0.0.0 listener is reachable from the LAN, so it is served over TLS
/// (rustls) — never plain HTTP (audit S3). The cert+key live under
/// <data_dir>/tls and are auto-generated (self-signed) on first use.
async fn serve_network_listener(
    cfg: &Config,
    pool: &otto_state::DbPool,
    router: &axum::Router,
    shutdown_rx: &watch::Receiver<bool>,
) -> Result<Option<tokio::task::JoinHandle<()>>, String> {
    let Some(value) = SettingsRepo::new(pool.clone())
        .get("network_listener")
        .await
        .map_err(|e| format!("read network_listener setting: {e}"))?
    else {
        return Ok(None);
    };
    let enabled = value
        .get("enabled")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    if !enabled {
        return Ok(None);
    }
    let port = value
        .get("port")
        .and_then(serde_json::Value::as_u64)
        .and_then(|p| u16::try_from(p).ok())
        .unwrap_or(cfg.port);
    let tls = match load_or_make_tls_config(&cfg.data_dir).await {
        Ok(tls) => tls,
        Err(e) => {
            tracing::error!("network listener TLS setup failed: {e}");
            return Ok(None);
        }
    };
    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], port));
    // Bind HERE (not inside `serve`) so a taken port is known before anything
    // advertises the listener: share links read the recorded port, never the
    // setting (S20-303).
    let listener = match std::net::TcpListener::bind(addr).and_then(|l| {
        l.set_nonblocking(true)?;
        Ok(l)
    }) {
        Ok(l) => l,
        Err(e) => {
            tracing::error!("network listener: bind {addr} failed: {e}");
            return Ok(None);
        }
    };
    let server = match axum_server::from_tcp_rustls(listener, tls) {
        Ok(server) => server,
        Err(e) => {
            tracing::error!("network listener: {e}");
            return Ok(None);
        }
    };
    tracing::info!("network listener on https://0.0.0.0:{port} (TLS)");
    otto_server::transport::set_network_listener_port(Some(port));
    let router = router.clone();
    // axum-server drives shutdown via its own Handle; bridge the watch signal
    // into a graceful_shutdown so the TLS listener drains in step with the
    // loopback one.
    let handle = axum_server::Handle::new();
    let mut rx = shutdown_rx.clone();
    let shutdown_handle = handle.clone();
    tokio::spawn(async move {
        let _ = rx.changed().await;
        shutdown_handle.graceful_shutdown(Some(HTTP_DRAIN_CAP));
    });
    Ok(Some(tokio::spawn(async move {
        // `into_make_service_with_connect_info::<SocketAddr>` makes each
        // request's real socket peer available to handlers via
        // `ConnectInfo<SocketAddr>` (used by the login throttle, S5).
        if let Err(e) = server
            .handle(handle)
            .serve(router.into_make_service_with_connect_info::<std::net::SocketAddr>())
            .await
        {
            tracing::error!("network listener: {e}");
        }
        // No longer serving: stop advertising it.
        otto_server::transport::set_network_listener_port(None);
    })))
}

/// Serve the `[::1]` alias lane (see [`bind_alt_loopback`]).
fn serve_alt_loopback(
    listener: tokio::net::TcpListener,
    port: u16,
    router: &axum::Router,
    shutdown_rx: &watch::Receiver<bool>,
) -> tokio::task::JoinHandle<()> {
    // Advertised as `localhost`, not `[::1]`: a distinct pool key from
    // `127.0.0.1` either way, CSP host-sources cannot express IPv6
    // literals (the Tauri CSP already allows `localhost:7700`), and with
    // BOTH loopback addresses held by this process `localhost` reaches
    // this daemon whichever one the resolver picks.
    let base = format!("http://localhost:{port}");
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
}

/// Serve the primary loopback listener until the shutdown signal, with the
/// graceful drain capped at [`HTTP_DRAIN_CAP`].
async fn serve_loopback(
    loopback: tokio::net::TcpListener,
    router: axum::Router,
    shutdown_rx: &watch::Receiver<bool>,
) -> Result<(), String> {
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
    // teardown finishes inside launchd's ExitTimeOut.
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
    Ok(())
}

/// Terminate every live PTY so a daemon stop / system shutdown never leaves
/// orphaned agent processes behind — except the sessions running in PTY
/// holders (setting `session_persistence`, default on): those are detached
/// and re-adopted, still running, by the next daemon start. Then stop the
/// live browser, telemetry and the embedded ClickHouse.
/// Every step shares one deadline ([`TEARDOWN_BUDGET`]) with the ClickHouse
/// flush's slot reserved ([`CLICKHOUSE_RESERVE`]): a hung step can neither
/// push the stop past launchd's SIGKILL nor cost the flush after it.
async fn teardown(
    manager: &otto_sessions::SessionManager,
    browser: &otto_server::routes::browser::BrowserEngineHandle,
    telemetry: Option<&otto_telemetry::TelemetryService>,
    usage: &otto_usage::UsageEngine,
) {
    use std::time::Duration;
    let deadline = std::time::Instant::now() + TEARDOWN_BUDGET;
    let slot = |cap: Duration, reserve: Duration| {
        step_budget(
            deadline.saturating_duration_since(std::time::Instant::now()),
            cap,
            reserve,
        )
    };
    let sessions_slot = slot(Duration::from_secs(12), CLICKHOUSE_RESERVE);
    let (killed, kept) =
        match tokio::time::timeout(sessions_slot, manager.shutdown_for_restart()).await {
            Ok(counts) => counts,
            Err(_) => {
                tracing::warn!(
                    "session shutdown exceeded {} ms — continuing teardown",
                    sessions_slot.as_millis()
                );
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
        slot(Duration::from_secs(3), CLICKHOUSE_RESERVE),
        browser.shutdown_live(),
    )
    .await
    .is_err()
    {
        tracing::warn!("live browser shutdown exceeded its slot");
    }
    if killed > 0 {
        tracing::info!("terminated {killed} live session(s) on shutdown");
    }
    if let Some(telemetry) = telemetry {
        if tokio::time::timeout(
            slot(Duration::from_secs(3), CLICKHOUSE_RESERVE),
            telemetry.shutdown(),
        )
        .await
        .is_err()
        {
            tracing::warn!("telemetry shutdown exceeded its slot");
        }
    }
    // Stop the embedded ClickHouse server cleanly (SIGTERM → flush) so its data
    // dir lock is released and the next daemon start doesn't have to reclaim it.
    // Whatever is left of the budget (never less than its reserve, which
    // the steps above could not touch).
    let ch_slot = slot(TEARDOWN_BUDGET, Duration::ZERO);
    if tokio::time::timeout(ch_slot, usage.shutdown())
        .await
        .is_err()
    {
        tracing::warn!(
            "usage (clickhouse) shutdown exceeded {} ms",
            ch_slot.as_millis()
        );
    }
}

/// One teardown step's timeout: its own `cap`, but never more than what is
/// left of the shared deadline (`remaining`) after keeping `reserve` back for
/// the steps behind it.
fn step_budget(
    remaining: std::time::Duration,
    cap: std::time::Duration,
    reserve: std::time::Duration,
) -> std::time::Duration {
    cap.min(remaining.saturating_sub(reserve))
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

/// Remove any `com.otto.deploy.*` launchd jobs left by a previous run. Otto's
/// own deploy path (packaging/deploy.sh) only registers `com.otto.deploy-finish`
/// (a hyphen, not matched here; packaging/deploy.sh also boots out exited
/// `com.otto.deploy-once.*` leftovers), so a job under this prefix can only be a coding agent's ad-hoc
/// `launchctl submit` — and launchd relaunches a submitted job on every exit,
/// turning one deploy into an endless build → swap → restart loop that
/// outlives the session that started it. Best-effort and macOS-only by
/// construction (launchctl is absent elsewhere, and probe_cmd just fails).
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

/// SIGTERM/SIGINT listeners, registered synchronously by [`install`] — the
/// default (terminate) action is replaced the moment this returns, not when a
/// spawned task first gets polled.
///
/// [`install`]: ShutdownSignals::install
struct ShutdownSignals {
    #[cfg(unix)]
    sigterm: tokio::signal::unix::Signal,
    #[cfg(unix)]
    sigint: tokio::signal::unix::Signal,
}

impl ShutdownSignals {
    fn install() -> Self {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{signal, SignalKind};
            Self {
                sigterm: signal(SignalKind::terminate()).expect("install SIGTERM handler"),
                sigint: signal(SignalKind::interrupt()).expect("install SIGINT handler"),
            }
        }
        #[cfg(not(unix))]
        Self {}
    }

    async fn recv(self) {
        #[cfg(unix)]
        {
            let Self {
                mut sigterm,
                mut sigint,
            } = self;
            tokio::select! {
                _ = sigterm.recv() => {}
                _ = sigint.recv() => {}
            }
        }
        #[cfg(not(unix))]
        {
            let _ = tokio::signal::ctrl_c().await;
        }
    }
}

#[cfg(test)]
mod shutdown_budget_tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn steps_never_eat_the_clickhouse_reserve() {
        let s = Duration::from_secs;
        // Plenty left: the step's own cap.
        assert_eq!(step_budget(s(18), s(12), CLICKHOUSE_RESERVE), s(12));
        // A slow first step left 8 s: the next gets 2 s, keeping 6 back.
        assert_eq!(step_budget(s(8), s(3), CLICKHOUSE_RESERVE), s(2));
        // Nothing beyond the reserve: zero, never negative.
        assert_eq!(step_budget(s(5), s(3), CLICKHOUSE_RESERVE), Duration::ZERO);
        // The last step takes what is left.
        assert_eq!(step_budget(s(7), TEARDOWN_BUDGET, Duration::ZERO), s(7));
    }

    #[test]
    fn worst_case_stop_fits_inside_launchd_exit_timeout() {
        // drain + two secondary listeners at 500 ms + teardown + runtime.
        let worst =
            HTTP_DRAIN_CAP + Duration::from_millis(1000) + TEARDOWN_BUDGET + RUNTIME_SHUTDOWN_CAP;
        let exit_timeout = Duration::from_secs(30);
        assert!(
            worst + Duration::from_secs(5) <= exit_timeout,
            "worst-case stop {worst:?} leaves <5 s of ExitTimeOut"
        );
        assert!(CLICKHOUSE_RESERVE < TEARDOWN_BUDGET);
    }
}
