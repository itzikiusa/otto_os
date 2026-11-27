//! Graceful daemon-restart detach and boot adoption + gap replay for [`AgentRuntime`].

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use otto_core::agent_runtime::{
    ProviderSessionStatus, ProviderTurn, ProviderTurnStatus, RuntimeEventBody,
};
use otto_core::{Id, Result};
use tokio::sync::oneshot;

use super::held_proc::{build_checkpoint, finalize_child_exit, pump_held, sync_holder_checkpoint};
use super::{
    apply_step, descendants, dispatch_child_stdout, emit, evict_if_dead_after_flush, flush_held,
    has_pending, record_claude_stdout_line, set_session_status, ActiveTurn, AgentRuntime, Live,
    PersistOp, Proc, ProcTransport, RequestIds,
};
use crate::holder::{adopt_socket, best_effort_shutdown_socket, list_holder_sockets, LineStream};
use crate::process::ChildControl;

/// What [`AgentRuntime::recover_and_adopt`] re-attached after a restart.
#[derive(Debug, Default)]
pub struct Adopted {
    /// Holders whose CLI is still running.
    pub live_ids: HashSet<Id>,
    /// Every holder adopted, the cleanly finished ones included.
    pub handled_ids: HashSet<Id>,
}

impl AgentRuntime {
    /// What [`Self::shutdown_for_restart`] would keep (a child in a
    /// runtime-holder, persistence on) and end (a direct child) right now —
    /// the restart dialog's count. Sessions without a process are not counted.
    pub async fn restart_impact(&self) -> Vec<otto_core::restart::ImpactItem> {
        let persist = self.inner.host.session_persistence_enabled().await;
        self.inner
            .live
            .iter()
            .filter_map(|e| {
                let st = e.value().lock();
                let held = matches!(st.proc.as_ref()?.transport, ProcTransport::Held { .. });
                // Engine-owned: a background `meta.source` or a non-manual
                // `meta.work.origin` (the sessions' `is_user_started` rule).
                let meta = st.session.meta.as_ref();
                let field =
                    |path: &str| meta.and_then(|m| m.pointer(path)).and_then(|v| v.as_str());
                let background = field("/source")
                    .is_some_and(|s| otto_core::domain::BACKGROUND_SESSION_SOURCES.contains(&s))
                    || field("/work/origin").is_some_and(|o| o != "manual");
                let title = st.session.title.clone();
                Some(otto_core::restart::ImpactItem::session(
                    persist && held,
                    false,
                    background,
                    title,
                ))
            })
            .collect()
    }

    /// Graceful daemon restart teardown: flush coalesced streaming items, push
    /// a final [`crate::holder::HolderCheckpoint`] + `DETACH` to every live
    /// `runtime-holder` without killing child CLIs or Claude background
    /// descendants, and flush pending SQLite writes. Non-held direct processes
    /// (if any) still terminate cleanly.
    pub async fn shutdown_for_restart(&self) {
        if !self.inner.host.session_persistence_enabled().await {
            self.shutdown().await;
            return;
        }
        let lives: Vec<Arc<Live>> = self
            .inner
            .live
            .iter()
            .map(|e| Arc::clone(e.value()))
            .collect();
        let mut detach_tasks = Vec::new();
        for live in &lives {
            live.detaching_for_restart.store(true, Ordering::Release);
            let taken = {
                let mut st = live.lock();
                flush_held(&self.inner, live, &mut st);
                let cp = build_checkpoint(&st);
                let taken = st.proc.take().map(|p| (p, cp));
                if taken.is_some() {
                    live.proc_ended.notify_waiters();
                }
                // Client terminals are the daemon's own children: nothing
                // reaches them after this (pending calls are refused on adopt).
                super::client_calls::drop_session(&st.session.id);
                taken
            };
            if let Some((proc, cp)) = taken {
                match proc.transport {
                    ProcTransport::Held { client, .. } => {
                        if let Some(cp) = cp {
                            detach_tasks.push(tokio::spawn(async move {
                                client.detach_with_checkpoint(cp).await;
                            }));
                        }
                    }
                    ProcTransport::Direct { stdin, control } => {
                        drop(stdin);
                        let _ = control.send(ChildControl::Signal(libc::SIGTERM));
                    }
                }
            }
        }
        for task in detach_tasks {
            let _ = task.await;
        }
        // Queue all flushes up front and await them concurrently so a daemon
        // with 10+ loaded workflow/review threads does not exhaust the restart
        // detach window waiting on sequential SQLite commits.
        let flushes: Vec<_> = lives
            .into_iter()
            .filter_map(|live| {
                let (tx, rx) = oneshot::channel();
                live.persist.send(PersistOp::Flush(tx)).ok().map(|()| rx)
            })
            .collect();
        let _ = futures_util::future::join_all(flushes).await;
    }

    /// Scan `<holders_dir>/*.sock`, re-attach live `runtime-holder` processes,
    /// replay any lines emitted while `ottod` was down, and settle only the
    /// remaining un-adopted SQLite sessions via [`Self::recover`]. Returns the
    /// adopted live `session_id`s (so session restore keeps their
    /// `OTTO_MCP_TOKEN` credentials active) and every holder it handled.
    pub async fn recover_and_adopt(&self) -> Result<Adopted> {
        let mut live_ids = HashSet::new();
        let mut handled_ids = HashSet::new();
        let mut failed = Vec::new();
        let persist_enabled = self.inner.host.session_persistence_enabled().await;
        if let Some(holders_dir) = self.inner.config.holders_dir.clone() {
            for sock in list_holder_sockets(&holders_dir) {
                if !persist_enabled {
                    best_effort_shutdown_socket(&sock).await;
                    let _ = tokio::fs::remove_file(&sock).await;
                    continue;
                }
                if let Some((sid, is_alive)) = self.adopt_holder_socket(&sock, &mut failed).await {
                    if is_alive {
                        live_ids.insert(sid.clone());
                    }
                    // Exclude both alive and cleanly-drained-exit holders from
                    // `recover_excluding` so a turn that finished during the
                    // restart window keeps its `Completed` status.
                    handled_ids.insert(sid);
                }
            }
        }
        self.recover_excluding(&handled_ids).await?;
        // Recovery settled the cut-off turn; the thread also says why.
        for (sid, why) in failed {
            let _ = self
                .notice(
                    &sid,
                    &format!(
                        "Otto restarted and could not reattach to this agent ({why}), so its \
                         running work was stopped. Send a message to continue the conversation."
                    ),
                )
                .await;
        }
        Ok(Adopted {
            live_ids,
            handled_ids,
        })
    }

    /// The reviews (`meta.review_id`) with an agent among `adopted.handled_ids`
    /// — a holder still running, or one whose CLI finished cleanly during the
    /// restart (its turn stays `Completed`, its findings file is written):
    /// review recovery resumes those instead of failing them (DR-115.2).
    pub async fn held_reviews(&self, adopted: &Adopted) -> HashSet<Id> {
        let mut out = HashSet::new();
        for id in &adopted.handled_ids {
            let meta = self.get(id).await.ok().and_then(|s| s.meta);
            if let Some(r) = meta
                .as_ref()
                .and_then(|m| m.get("review_id"))
                .and_then(|v| v.as_str())
            {
                out.insert(Id::from(r));
            }
        }
        out
    }

    /// `failed` collects threads whose saved driver state did not load (a
    /// notice tells each one after recovery).
    async fn adopt_holder_socket(
        &self,
        socket_path: &Path,
        failed: &mut Vec<(Id, String)>,
    ) -> Option<(Id, bool)> {
        let adopted = match adopt_socket(socket_path).await {
            Ok(a) => a,
            Err(e) => {
                if !matches!(
                    e.kind(),
                    std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
                ) {
                    tracing::warn!(
                        socket = %socket_path.display(),
                        "could not adopt runtime holder ({e}); shutting it down before unlinking"
                    );
                    best_effort_shutdown_socket(socket_path).await;
                }
                let _ = tokio::fs::remove_file(socket_path).await;
                return None;
            }
        };
        let sid = adopted.checkpoint.session_id.clone();
        // A fresh start or provider switch cut mid-handshake never took a
        // prompt, and its process may run another provider than the row
        // names: not adopted, the thread resumes as it was.
        if adopted.checkpoint.fresh_starting {
            adopted.client.shutdown(Duration::from_millis(500)).await;
            let _ = tokio::fs::remove_file(socket_path).await;
            return None;
        }
        let rec = match self.inner.repo.get(&sid).await {
            Ok(r) if !r.closed => r,
            _ => {
                adopted.client.shutdown(Duration::from_millis(500)).await;
                let _ = tokio::fs::remove_file(socket_path).await;
                return None;
            }
        };
        let Some(mut driver) = self.inner.drivers.create(rec.session.provider) else {
            adopted.client.shutdown(Duration::from_millis(500)).await;
            let _ = tokio::fs::remove_file(socket_path).await;
            return None;
        };
        if let Err(e) = driver.restore_state(&adopted.checkpoint.driver_state) {
            tracing::warn!(session = %sid, "runtime holder driver restore failed: {e}");
            failed.push((sid.clone(), format!("its saved state did not load: {e}")));
            adopted.client.shutdown(Duration::from_millis(500)).await;
            let _ = tokio::fs::remove_file(socket_path).await;
            return None;
        }
        let Ok(live) = self.live(&sid).await else {
            adopted.client.shutdown(Duration::from_millis(500)).await;
            let _ = tokio::fs::remove_file(socket_path).await;
            return None;
        };

        let child_pid = adopted.child_pid;
        let cp = adopted.checkpoint;
        let claude_login = cp.claude_login.clone();
        let exited = adopted.exited;
        let replay_lines = adopted.replay_lines;
        let client = adopted.client;
        let events = adopted.events;

        let (epoch, still_alive, kick) = {
            let mut st = live.lock();
            live.detaching_for_restart.store(false, Ordering::Release);
            st.epoch += 1;
            let epoch = st.epoch;
            st.fresh_starting = cp.fresh_starting;
            for item in cp.streaming_items {
                st.proj.next_item_ordinal = st.proj.next_item_ordinal.max(item.ordinal);
                if st
                    .proj
                    .items
                    .get(&item.id)
                    .is_none_or(|cur| cur.is_streaming())
                {
                    st.proj.upsert_item_entry(item);
                }
            }
            if let Some(t_snap) = cp.active_turn {
                let non_terminal = st
                    .proj
                    .turns
                    .get(&t_snap.id)
                    .is_none_or(|pt| !pt.status.is_terminal());
                if non_terminal {
                    if !st.proj.turns.contains_key(&t_snap.id) {
                        st.proj.next_turn_ordinal += 1;
                        let now = Utc::now();
                        let pt = ProviderTurn {
                            id: t_snap.id.clone(),
                            ordinal: st.proj.next_turn_ordinal,
                            status: ProviderTurnStatus::Running,
                            started_at: Some(now),
                            completed_at: None,
                            turn_token_usage: None,
                            origin: t_snap.origin,
                            model: st.session.model.clone(),
                            effort: st.session.effort.clone(),
                        };
                        emit(
                            &self.inner,
                            &live,
                            &mut st,
                            RuntimeEventBody::ProviderTurnUpdated { provider_turn: pt },
                        );
                    }
                    st.turn = Some(ActiveTurn {
                        id: t_snap.id,
                        origin: t_snap.origin,
                        interrupting: t_snap.interrupting,
                        stalled: t_snap.stalled,
                    });
                }
            }
            // The agent is as it was: what it takes, and its background
            // work still runs.
            st.session.capabilities = driver.capabilities();
            if let Some(ps) = &st.proj.session {
                st.session.background_tasks = ps.background_tasks.clone();
            }
            st.reserved = st.next_seq + self.inner.config.seq_reserve;
            let _ = live.persist.send(PersistOp::Reserve(st.reserved));
            st.proc = Some(Proc {
                epoch,
                request_ids: RequestIds::from_prefix(cp.request_ids_prefix),
                pid: child_pid,
                transport: ProcTransport::Held {
                    client,
                    acked_seq: cp.acked_seq,
                    claude_login: claude_login.clone(),
                    last_checkpoint_at: None,
                    checkpoint_dirty: false,
                    #[cfg(test)]
                    checkpoint_writes: 0,
                },
                driver,
            });
            let status = if has_pending(&st) {
                ProviderSessionStatus::Waiting
            } else if st.turn.is_some() {
                ProviderSessionStatus::Running
            } else if st
                .proj
                .session
                .as_ref()
                .is_some_and(|s| s.status == ProviderSessionStatus::Starting)
            {
                ProviderSessionStatus::Starting
            } else {
                ProviderSessionStatus::Ready
            };
            set_session_status(&self.inner, &live, &mut st, status, None);

            for entry in replay_lines {
                if let Some(Proc {
                    transport: ProcTransport::Held { acked_seq, .. },
                    ..
                }) = st.proc.as_mut()
                {
                    *acked_seq = entry.seq;
                }
                if entry.stream == LineStream::Out {
                    if st.proc.is_none() {
                        break;
                    }
                    if entry.overlong_bytes.is_none() {
                        record_claude_stdout_line(&claude_login, &entry.line);
                    }
                    let line_or_overlong =
                        entry.overlong_bytes.map_or(Ok(entry.line.as_str()), Err);
                    dispatch_child_stdout(&self.inner, &live, &mut st, line_or_overlong, false);
                }
            }

            if let Some(exit_status) = exited {
                if let Some(proc) = st.proc.take() {
                    if let ProcTransport::Held { client, .. } = &proc.transport {
                        client.shutdown_fire_and_forget(Duration::ZERO);
                    }
                    finalize_child_exit(
                        &self.inner,
                        &live,
                        &mut st,
                        proc,
                        exit_status.code,
                        exit_status.stderr_tail,
                    );
                }
                (epoch, false, false)
            } else {
                if let Some(proc) = st.proc.as_mut() {
                    let step = proc.driver.after_adopt();
                    apply_step(&self.inner, &live, &mut st, step);
                }
                sync_holder_checkpoint(&mut st, true);
                let alive = st.proc.is_some();
                let kick = alive && st.turn.is_none() && !st.proj.queue.is_empty();
                (epoch, alive, kick)
            }
        };

        if still_alive {
            if claude_login.is_some() {
                descendants::watch(
                    Arc::clone(&self.inner),
                    Arc::clone(&live),
                    sid.clone(),
                    epoch,
                    child_pid,
                );
            }
            let pumped = pump_held(
                Arc::clone(&self.inner),
                Arc::clone(&live),
                sid.clone(),
                epoch,
                events,
                claude_login,
            );
            let session = live.lock().session.clone();
            super::unattended::register_and_pump(&self.inner, &session, child_pid, pumped).await;
        }
        drop(live);
        self.flush(&sid).await;
        if !still_alive {
            evict_if_dead_after_flush(&self.inner, &sid).await;
        }
        if kick {
            self.kick_queue(&sid);
        }
        Some((sid, still_alive))
    }
}
