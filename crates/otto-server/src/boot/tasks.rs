//! Phase 4 — the long-running background tasks: sweeps, retention,
//! supervisors and schedulers.
//!
//! [`spawn_background`] is the ordered list; every task lives in its own
//! small function so a subsystem moving crates only touches its one line
//! (and its one `use` path). A few steps are awaited inline (channel
//! supervisor start, self-improvement scheduler start, swarm and
//! scheduled-task reaps) — they finish before the router serves, exactly as
//! they did when this was one inline function in `ottod::run`.
//!
//! A supervisor's handle is returned as `impl Send + 'static` and parked in
//! [`Background`]: the boot sequence only has to keep it alive, so it never
//! names the handle type (one less path to fix when a subsystem moves).

use std::sync::Arc;
use std::time::Duration;

use otto_improve::{LiveEvolver, Scheduler};
use otto_state::{ImprovementsRepo, SessionsRepo, SettingsRepo};

use crate::monitor::{
    spawn_budget_sampler, spawn_metrics_sampler, spawn_session_event_listener,
    spawn_usage_recorder, CredentialMonitor,
};
use crate::state::ServerCtx;

/// Handles of the started supervisors. Several stop their task on drop, so
/// the daemon keeps this alive until it has shut down.
#[derive(Default)]
pub struct Background {
    handles: Vec<Box<dyn Send>>,
}

impl Background {
    /// Keep `handle` alive for as long as this value lives.
    pub fn keep<H: Send + 'static>(&mut self, handle: H) {
        self.handles.push(Box::new(handle));
    }
}

/// Start every background task, in boot order. `root_user_id` gates the
/// channel supervisor (none until onboarding).
pub async fn spawn_background(ctx: &ServerCtx, root_user_id: Option<String>) -> Background {
    let mut bg = Background::default();

    // Idle-connection reapers + session sweeps.
    spawn_brokers_reaper(ctx);
    spawn_db_explorer_reaper(ctx);
    spawn_channel_session_archiver(ctx);
    spawn_archived_channel_session_purge(ctx);
    spawn_idle_session_suspender(ctx);
    spawn_nested_agent_capture(ctx);
    spawn_stale_session_archiver(ctx);
    spawn_provider_title_namer(ctx);
    spawn_dead_session_pruner(ctx);
    spawn_activity_trail_pruner(ctx);
    // Retention.
    spawn_data_retention(ctx);
    spawn_workflow_run_retention(ctx);
    spawn_memory_fts_warmup(ctx);

    // Gated by OTTO_SELF_IMPROVE (enabled by default; 0/false/off disables it).
    // Computed here so the channel manager can decide whether to wire the
    // self-improvement-on-interaction hook.
    let self_improve_enabled = !matches!(
        std::env::var("OTTO_SELF_IMPROVE").as_deref(),
        Ok("0") | Ok("false") | Ok("off")
    );
    if let Some(h) = start_channel_manager(ctx, root_user_id, self_improve_enabled).await {
        bg.keep(h);
    }
    bg.keep(start_story_watcher(ctx));
    if self_improve_enabled {
        start_self_improvement(ctx, &mut bg).await;
    } else {
        tracing::info!("self-improvement disabled (OTTO_SELF_IMPROVE=off)");
    }

    start_notices(ctx);
    bg.keep(start_conversation_view(ctx));
    spawn_product_orphan_reaper(ctx);
    start_design_hall_and_proof_media(ctx);
    spawn_vault_docs_recovery(ctx);
    bg.keep(start_insights_scheduler(ctx));
    if let Some(h) = start_cli_update_scheduler(ctx) {
        bg.keep(h);
    }
    bg.keep(start_swarm(ctx).await);
    bg.keep(start_workflow_event_triggers(ctx));
    bg.keep(start_workflow_schedule_triggers(ctx));
    bg.keep(start_k8s_monitor(ctx));
    bg.keep(start_workgraph_projector(ctx));
    bg.keep(start_scheduled_tasks(ctx).await);
    bg.keep(start_personal_agents(ctx));
    bg.keep(start_assistant(ctx));
    start_run_scheduler(ctx);
    start_model_catalog(ctx);
    start_usage_tracking(ctx);
    spawn_mcp_health_sweep(ctx);
    bg
}

/// Reconcile the memory FTS index off the request path: it used to run
/// lazily on the first memory call after boot (an O(n) body compare under
/// `BEGIN IMMEDIATE`), so that call — often a session's recall brief — paid
/// for it. Single-flight with the lazy path, so a racing request just waits.
fn spawn_memory_fts_warmup(ctx: &ServerCtx) {
    let memory = ctx.memory.clone();
    tokio::spawn(async move {
        let t = std::time::Instant::now();
        let ok = memory.warm_fts().await;
        tracing::debug!(
            fts = ok,
            ms = t.elapsed().as_millis() as u64,
            "memory: FTS warmed"
        );
    });
}

// -- Idle-connection reapers ----------------------------------------------
//
// Unlike agent sessions, nothing else closes an idle Kafka client or DB SSH
// tunnel — POOL_TTL/lazy-eviction only fire on the *next* access, which a
// truly idle cluster/connection never triggers. So on a mostly-idle daemon
// the librdkafka handles + orphaned `ssh` children would linger until
// edit/delete/restart. Sweep on a timer (every 5 min) and evict anything idle
// past the conservative ~30 min window.

fn spawn_brokers_reaper(ctx: &ServerCtx) {
    let brokers = Arc::clone(&ctx.brokers);
    let interval = Duration::from_secs(5 * 60); // every 5 min
    let idle = Duration::from_secs(30 * 60); // 30 min idle
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

fn spawn_db_explorer_reaper(ctx: &ServerCtx) {
    let db = Arc::clone(&ctx.db_explorer);
    let interval = Duration::from_secs(5 * 60); // every 5 min
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

// -- Session sweeps -------------------------------------------------------

/// Periodically auto-archive idle channel (ticket/chat) sessions so they
/// don't accumulate in the sidebar. A later message respawns a fresh one.
/// At ticketing volume (100-200/day) a long window floods the sidebar, so
/// we archive after 1h idle and sweep every 10 min.
fn spawn_channel_session_archiver(ctx: &ServerCtx) {
    let manager = Arc::clone(&ctx.manager);
    let interval = Duration::from_secs(10 * 60); // every 10 min
    let max_idle = Duration::from_secs(60 * 60); // 1h idle
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

/// Retention: permanently delete archived channel (ticket/chat) sessions
/// whose last activity is older than 30 days, so the DB doesn't grow without
/// bound at ticketing volume. Runs at startup, then daily.
fn spawn_archived_channel_session_purge(ctx: &ServerCtx) {
    let manager = Arc::clone(&ctx.manager);
    let interval = Duration::from_secs(24 * 60 * 60); // daily
    let max_age = Duration::from_secs(30 * 24 * 60 * 60); // 30 days
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

/// Idle+unattached suspend sweep: every ~60s, release the PTY of any LIVE
/// resumable session that has been idle past the grace window and has no
/// attached WS viewer. The session stays resumable (reopening auto-resumes
/// via --resume), so this frees RAM without ever losing a conversation.
fn spawn_idle_session_suspender(ctx: &ServerCtx) {
    let manager = Arc::clone(&ctx.manager);
    let interval = Duration::from_secs(60);
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

/// Nested-agent capture: every ~30s, look inside every LIVE plain `shell`
/// session for an agent CLI the user launched by hand (`claude`, `codex`,
/// `agy`) and record that conversation's id on the session row. A terminal
/// running an agent is no longer stateless — reopening it after a daemon
/// restart respawns the shell and types the provider's resume command, so
/// the conversation comes back instead of dead-ending.
fn spawn_nested_agent_capture(ctx: &ServerCtx) {
    let manager = Arc::clone(&ctx.manager);
    let interval = Duration::from_secs(30);
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

/// Opt-in auto-archive: hourly, archive agent sessions idle beyond the
/// `session_auto_archive_days` setting (0/absent = off). Archive keeps the
/// row + history and stays reversible via unarchive.
fn spawn_stale_session_archiver(ctx: &ServerCtx) {
    let manager = Arc::clone(&ctx.manager);
    let interval = Duration::from_secs(60 * 60);
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

/// Provider-title auto-namer: every ~20s, rename any LIVE foreground agent
/// session the user hasn't named to the provider's own session title (the
/// first user prompt in claude's transcript / codex's rollout). Skips
/// user-named and already-adopted sessions with no disk work, so it stays
/// cheap; each rename broadcasts `SessionRenamed` for a live UI refresh.
fn spawn_provider_title_namer(ctx: &ServerCtx) {
    let manager = Arc::clone(&ctx.manager);
    let interval = Duration::from_secs(20);
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

/// Existence-check pruner: once at startup, then every ~6h. For non-live
/// resumable agent sessions, delete the row only when the provider's local
/// transcript is positively gone (un-resumable). Sessions whose transcript
/// still exists — or whose resumability can't be verified — are kept.
fn spawn_dead_session_pruner(ctx: &ServerCtx) {
    let manager = Arc::clone(&ctx.manager);
    let interval = Duration::from_secs(6 * 60 * 60); // every 6h
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

/// Activity-trail retention: cap each session's trail at the newest N rows so
/// long-lived sessions don't grow it unbounded. Runs at startup then hourly.
fn spawn_activity_trail_pruner(ctx: &ServerCtx) {
    let manager = Arc::clone(&ctx.manager);
    const KEEP_PER_SESSION: i64 = 1_000;
    let interval = Duration::from_secs(60 * 60); // hourly
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
            since = started.checked_sub(Duration::from_secs(10 * 60));
            tokio::time::sleep(interval).await;
        }
    });
}

// -- Retention ------------------------------------------------------------

/// Audit/event-table retention (work_events, mcp_tool_calls, mcp_call_log,
/// audit_log — all append-only with no other delete path). Policy comes from
/// the `data_retention` setting, re-read each pass so a change applies
/// within the hour; windows are floored in otto-state (never deletes a row
/// younger than its window). See docs/features/backup-restore.md. The same
/// hourly pass purges expired credentials and runs SQLite housekeeping.
fn spawn_data_retention(ctx: &ServerCtx) {
    let pool = ctx.pool.clone();
    let auth_cache = ctx.auth_cache.clone();
    let interval = Duration::from_secs(60 * 60); // hourly
    let manager = Arc::clone(&ctx.manager);
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

/// Workflow run-history retention (08-workflows R1): daily, first pass at
/// startup. Per workflow keep the newest 200 terminal runs AND every run
/// younger than 30 days; never active/approval-parked runs or runs a
/// Proof Pack / scheduled-task run references (all enforced in
/// `WorkflowsRepo::prune_runs`). Each pruned run's
/// `workflow-context/<run_id>/` dir is removed, confined to that root.
fn spawn_workflow_run_retention(ctx: &ServerCtx) {
    let pool = ctx.pool.clone();
    let ctx_root = ctx.data_dir.join("workflow-context");
    let interval = Duration::from_secs(24 * 60 * 60);
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

// -- Channels, product, self-improvement ----------------------------------

/// Channel Manager (Telegram-first, Slack-ready). Spawns agent sessions on
/// behalf of incoming channel messages as the root user (None when
/// onboarding hasn't created one yet → skip).
async fn start_channel_manager(
    ctx: &ServerCtx,
    root_user_id: Option<String>,
    self_improve_enabled: bool,
) -> Option<impl Send + 'static> {
    let Some(uid) = root_user_id else {
        tracing::info!("channel manager: skipping (no root user yet — run onboarding first)");
        return None;
    };
    let mut cm = otto_channels::ChannelManager::new(
        Arc::clone(&ctx.manager),
        ctx.workspaces.clone(),
        otto_state::IntegrationsRepo::new(ctx.pool.clone()),
        SettingsRepo::new(ctx.pool.clone()),
        ctx.secrets.clone(),
        uid.clone(),
        // Share the daemon event bus so the proactive self-improvement
        // notifier can mirror Improvement* events to the user's channels
        // (opt-in via the `channels.notify_self_improvement` setting).
        Some(ctx.events.clone()),
    )
    // An inbound message on a swarm-bound channel launches that swarm.
    .with_swarm_trigger(Arc::new(otto_swarm::runtime::channels::SwarmTriggerImpl {
        ctx: ctx.swarm_rt(),
    }))
    // An inbound `/run <ref>` (or `approve`/`reject` reply) drives a Run with
    // Otto run on the root user's behalf (the channel-trust model).
    .with_run_trigger(Arc::new(crate::run_channels::ChannelRunTrigger::new(
        ctx.clone(),
        uid.clone(),
    )))
    // A structured `Action: Workflow` message starts a workflow run by name.
    .with_workflow_trigger(Arc::new(crate::workflow_chat::WorkflowChatTriggerImpl {
        ctx: ctx.clone(),
    }));
    // When self-improvement is on, learn from each finished channel
    // interaction and reply with the result in the same thread (per-workspace
    // `self_improvement.enabled` is re-checked inside the hook).
    if self_improve_enabled {
        cm = cm.with_improver(Arc::new(
            crate::improve_channels::InteractionImproverImpl::new(
                Arc::clone(&ctx.improve_engine),
                ctx.workspaces.clone(),
                SessionsRepo::new(ctx.pool.clone()),
                ImprovementsRepo::new(ctx.pool.clone()),
            ),
        ));
    }
    let handle = cm.start().await;
    tracing::info!("channel manager: supervisor started (adapters track config live)");
    Some(handle)
}

/// Story watcher (polls watched stories for new comments).
fn start_story_watcher(ctx: &ServerCtx) -> impl Send + 'static {
    let watcher = otto_product::watcher::WatcherManager::new(
        otto_state::ProductRepo::new(ctx.pool.clone()),
        Arc::clone(&ctx.product),
        Arc::clone(&ctx.orchestrator),
        Arc::clone(&ctx.improve_engine),
        ctx.events.clone(),
        "claude".to_string(),
    );
    let handle = watcher.start();
    tracing::info!("story watcher: supervisor started");
    handle
}

/// Self-improvement: the per-workspace self-reflection scheduler and the
/// live in-loop skill evolver (both gated by OTTO_SELF_IMPROVE).
async fn start_self_improvement(ctx: &ServerCtx, bg: &mut Background) {
    // Background supervisor that fires due per-workspace self-reflection runs.
    let scheduler = Scheduler::new(Arc::clone(&ctx.improve_engine), ctx.workspaces.clone())
        .start()
        .await;
    tracing::info!("self-improvement scheduler started");
    bg.keep(scheduler);

    // Subscribes to the event bus; evolves a watched session's skills after
    // its interaction goes idle (workspace `live_evolve` / session `meta.evolve`).
    let evolver = LiveEvolver::new(
        Arc::clone(&ctx.improve_engine),
        ctx.workspaces.clone(),
        SessionsRepo::new(ctx.pool.clone()),
    )
    .start(ctx.events.subscribe());
    tracing::info!("live skill evolver started");
    bg.keep(evolver);
}

// -- Notices, conversation view, recovery tasks ---------------------------

/// Credential monitor + session-event notices (wave 2). Background loop:
/// token-expiry + agent-CLI health checks (startup, then every ~6h).
/// Event-bus listener: session-progress notices.
fn start_notices(ctx: &ServerCtx) {
    CredentialMonitor::new(ctx.clone()).spawn();
    spawn_session_event_listener(ctx.clone());
    // Agent UI control: a removed session's in-flight UI commands end.
    crate::ui_bridge::spawn_session_watch(ctx.clone());
    tracing::info!("credential monitor + session-event notices started");
}

/// Conversation view (docs/design/conversation-view.md). Board→agent nudge
/// sweep: hands `POST /sessions/{id}/tasks` rows to the agent's PTY once the
/// session is idle (SessionStatus events + a 15 s tick). History index: a
/// low-priority boot walk of ~/.claude/projects and ~/.codex/sessions (skips
/// unchanged files); `POST …/history/rescan` re-runs it on demand.
fn start_conversation_view(ctx: &ServerCtx) -> impl Send + 'static {
    let nudge = crate::agent_tasks_nudge::spawn(ctx.clone());
    crate::history_index::spawn_scan(ctx.clone(), None);
    tracing::info!("conversation view: nudge sweep + history index scan started");
    nudge
}

/// Orphan reaper: auto-resume analysis agents stranded by a restart. Runs
/// once at startup; any analysis agent still 'running'/'waiting' has no
/// surviving task, so it is re-run (capped) or marked errored + notified.
fn spawn_product_orphan_reaper(ctx: &ServerCtx) {
    tokio::spawn(otto_product::run::reap_orphaned_agents_on_startup(
        ctx.clone(),
    ));
}

/// Design Hall: FTS index + idempotent legacy import (background). Mirrors
/// Product-arena design attachments and Canvas scenes into the design graph
/// (graph rows only; the legacy rows/files are never touched) and re-syncs a
/// `sync` version when a legacy source changed. Then moves legacy inline
/// proof media into the file store + daily GC.
fn start_design_hall_and_proof_media(ctx: &ServerCtx) {
    crate::design_hall::spawn_startup_import(ctx);
    crate::proof::spawn_media_maintenance(ctx.proof_repo.clone());
}

/// Vault docs-runs recovery: this restart killed any in-flight run. Flip
/// still-non-terminal persisted runs to 'interrupted' and soft-trash their
/// orphaned `_drafts/docs-run-*` dirs (multi-writer runs only).
fn spawn_vault_docs_recovery(ctx: &ServerCtx) {
    tokio::spawn(crate::vault_docs_agent::recover_interrupted(ctx.clone()));
}

// -- Schedulers -----------------------------------------------------------

/// Insights scheduler: opt-in, catch-up usage reports. Ticks ~hourly and, for
/// each ENABLED cadence (daily/weekly/monthly — all default OFF), runs the
/// `insights` skill for the most-recent missed period iff it has no report
/// yet. Runs the due-check on startup (catch-up after the app was closed).
fn start_insights_scheduler(ctx: &ServerCtx) -> impl Send + 'static {
    let h = otto_insights::InsightsScheduler::new(ctx.clone()).start();
    tracing::info!("insights scheduler started");
    h
}

/// Daily CLI auto-update: updates the agent CLIs (claude/codex/…) at a
/// user-configurable local time (default 07:00, opt-out via settings) and
/// force-reloads open agent sessions onto the new binary (resume-aware).
/// Catch-up on a missed window via a last-run cursor, like insights.
/// OTTO_CLI_UPDATE=0 disables the scheduler entirely — throwaway dev/E2E
/// daemons have no last-run cursor, so the catch-up fires at startup and
/// its session reload kills agent sessions seconds after they spawn.
fn start_cli_update_scheduler(ctx: &ServerCtx) -> Option<impl Send + 'static> {
    if std::env::var("OTTO_CLI_UPDATE").is_ok_and(|v| v == "0") {
        tracing::info!("cli auto-update scheduler disabled (OTTO_CLI_UPDATE=0)");
        return None;
    }
    let h = crate::cli_update::CliUpdateScheduler::new(ctx.clone()).start();
    tracing::info!("cli auto-update scheduler started");
    Some(h)
}

/// Agent Swarm: reconcile stale runs, then scheduler + restore coordinators.
/// A swarm run's background task dies with the process. A row left
/// queued/running/waiting would permanently consume the parallel cap and
/// block its agent's one-turn-at-a-time gate, so fail them BEFORE restarting
/// any coordinator (mirrors the review/skill-eval recovery).
async fn start_swarm(ctx: &ServerCtx) -> impl Send + 'static {
    match ctx
        .swarm_repo
        .fail_running("Interrupted by a daemon restart — the coordinator will re-run the task.")
        .await
    {
        Ok(n) if n > 0 => tracing::info!("swarm recovery: marked {n} orphaned run(s) as stopped"),
        Ok(_) => {}
        Err(e) => tracing::warn!("swarm recovery: {e}"),
    }
    let scheduler = otto_swarm::runtime::scheduler::start(ctx.swarm_rt());
    match ctx.swarm_repo.list_all_active_swarms().await {
        Ok(active) => {
            for s in active {
                otto_swarm::runtime::engine::start_coordinator(ctx.swarm_rt(), s.id.clone());
            }
            tracing::info!("swarm scheduler started; coordinators restored");
        }
        Err(e) => tracing::warn!("swarm restore: {e}"),
    }
    scheduler
}

/// Workflow event-trigger listener (B8). Subscribes to the daemon event bus;
/// for each event whose kind matches an enabled `event`-kind trigger's
/// `event_kind` spec field, starts a workflow run via the same path as
/// schedule/webhook triggers. Best-effort: errors inside the listener are
/// logged and never propagate to the event producer. Also starts the
/// notification-center notices for failed / waiting workflow runs and goal
/// loops (review 08 · N1); scheduled tasks and personal agents notify inline.
fn start_workflow_event_triggers(ctx: &ServerCtx) -> impl Send + 'static {
    let h = crate::workflow_trigger_scheduler::spawn_workflow_event_trigger_listener(ctx.clone());
    crate::run_notices::spawn_listener(ctx.clone());
    tracing::info!("workflow event-trigger listener started");
    h
}

/// Workflow schedule-trigger scheduler: 60 s tick that fires
/// `schedule`-kind triggers on their cadence (interval / daily / weekly /
/// cron, timezone-aware via the shared cadence engine) and starts a workflow
/// run. Mirrors the swarm / scheduled-tasks supervisors.
fn start_workflow_schedule_triggers(ctx: &ServerCtx) -> impl Send + 'static {
    let h = crate::workflow_trigger_scheduler::start(ctx.clone());
    tracing::info!("workflow schedule-trigger scheduler started");
    h
}

/// Kubernetes monitor supervisor: one collector loop per cluster with
/// monitoring enabled; reconciles every 15 s against `k8s_monitor_configs`
/// and restarts a loop on config change.
fn start_k8s_monitor(ctx: &ServerCtx) -> impl Send + 'static {
    let h = crate::k8s_monitor_scheduler::start(ctx.clone());
    tracing::info!("k8s monitor scheduler started");
    h
}

/// Mission Control / work-graph projector: subscribes to the daemon event
/// bus and materializes every agentic activity into the unified work graph
/// (live), plus a 60 s reconcile + boot backfill that re-derive from the
/// authoritative repos and refresh per-session cost.
fn start_workgraph_projector(ctx: &ServerCtx) -> impl Send + 'static {
    let h = crate::workgraph_projector::spawn(ctx.clone());
    tracing::info!("workgraph projector started");
    h
}

/// Scheduled Tasks. Reap runs a previous daemon life left `running` BEFORE
/// the router serves: reaping inside the spawned supervisor raced the first
/// "Run now" and marked that fresh run "interrupted by daemon restart" (like
/// the workflow/swarm recovery, this is awaited). Then fire due recurring
/// jobs (interval/daily/weekly/cron) with bounded concurrency.
async fn start_scheduled_tasks(ctx: &ServerCtx) -> impl Send + 'static {
    crate::scheduled_tasks_scheduler::reap_interrupted(ctx).await;
    let h = crate::scheduled_tasks_scheduler::start(ctx.clone());
    tracing::info!("scheduled tasks scheduler started");
    h
}

/// Personal Agents: fires every enabled schedule of every enabled personal
/// agent (per-schedule cursor), reaps interrupted runs on startup, and bounds
/// concurrency.
fn start_personal_agents(ctx: &ServerCtx) -> impl Send + 'static {
    let h = crate::personal_agents_scheduler::start(ctx.clone());
    tracing::info!("personal agents scheduler started");
    h
}

/// Otto Assistant: 30 s tick that fires due reminders (`once`), reports
/// finished delegations into their thread, syncs approvals decided in the
/// MCP queue, and deletes incognito threads 24 h after their last turn.
fn start_assistant(ctx: &ServerCtx) -> impl Send + 'static {
    let h = crate::assistant::start(ctx.clone());
    tracing::info!("assistant supervisor started");
    h
}

/// Run with Otto: boot reaper (fail interrupted runs, re-drive resumable
/// ones) + a 30 s tick that re-drives still-active runs. The engine drives
/// the stage machine.
fn start_run_scheduler(ctx: &ServerCtx) {
    crate::run_scheduler::spawn(ctx.clone());
    tracing::info!("run-with-otto scheduler started");
}

/// Dynamic model catalog: hourly per-provider model discovery (CLI probe /
/// docs scrape / models.dev fallback); a failed chain keeps the last good
/// list.
fn start_model_catalog(ctx: &ServerCtx) {
    crate::model_catalog::spawn_refresher(ctx.clone());
    tracing::info!("model-catalog refresher started");
}

/// Usage tracking + system metrics (embedded ClickHouse). The recorder mines
/// usage from the activity-trail event stream; the metrics sampler writes
/// CPU/RAM; the budget sampler rides the metrics tick to check spend-vs-cap
/// and emit BudgetExceeded events (with dedupe). All three are cheap no-ops
/// until ClickHouse is available.
fn start_usage_tracking(ctx: &ServerCtx) {
    spawn_usage_recorder(ctx.clone());
    spawn_metrics_sampler(ctx.clone());
    spawn_budget_sampler(ctx.clone());
    // ClickHouse comes up in the background (UsageEngine::start returns at
    // once), so `available()` here is almost always still false — report the
    // outcome once the bring-up has actually resolved.
    let usage = Arc::clone(&ctx.usage);
    tokio::spawn(async move {
        if usage.wait_ready(Duration::from_secs(120)).await {
            tracing::info!("usage tracking started (embedded clickhouse)");
        } else {
            tracing::info!(
                "usage tracking idle (embedded clickhouse not available after 120 s — \
                 not installed, disabled, or still starting)"
            );
        }
    });
}

/// MCP Control Plane: periodic health sweep of managed servers
/// (ping/initialize → status+latency). Interval from
/// `mcp_health_interval_secs` (default 300; 0 disables). Best-effort;
/// failures only update a server's health row. Lazy for stdio servers: only
/// those used in the last 6 h are checked (see `McpService::health_sweep`),
/// so an idle daemon spawns no MCP processes.
fn spawn_mcp_health_sweep(ctx: &ServerCtx) {
    let mcp = Arc::clone(&ctx.mcp);
    let settings = SettingsRepo::new(ctx.pool.clone());
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
                tokio::time::sleep(Duration::from_secs(300)).await;
                continue;
            }
            tokio::time::sleep(Duration::from_secs(secs)).await;
            mcp.health_sweep().await;
        }
    });
    tracing::info!("mcp control plane: health sweep started");
}
