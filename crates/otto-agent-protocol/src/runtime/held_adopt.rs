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
    descendants, dispatch_child_stdout, emit, evict_if_dead_after_flush, flush_held, has_pending,
    record_claude_stdout_line, set_session_status, ActiveTurn, AgentRuntime, Live, PersistOp, Proc,
    ProcTransport, RequestIds,
};
use crate::holder::{adopt_socket, list_holder_sockets, LineStream};
use crate::process::signal_group;

impl AgentRuntime {
    /// Graceful daemon restart teardown: flush coalesced streaming items, push
    /// a final [`crate::holder::HolderCheckpoint`] + `DETACH` to every live
    /// `runtime-holder` without killing child CLIs or Claude background
    /// descendants, and flush pending SQLite writes. Non-held direct processes
    /// (if any) still terminate cleanly.
    pub async fn shutdown_for_restart(&self) {
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
                    ProcTransport::Direct { stdin } => {
                        drop(stdin);
                        signal_group(proc.pid, libc::SIGTERM);
                    }
                }
            }
        }
        for task in detach_tasks {
            let _ = task.await;
        }
        for live in lives {
            let (tx, rx) = oneshot::channel();
            let _ = live.persist.send(PersistOp::Flush(tx));
            let _ = rx.await;
        }
    }

    /// Scan `<holders_dir>/*.sock`, re-attach live `runtime-holder` processes,
    /// replay any lines emitted while `ottod` was down, and settle only the
    /// remaining un-adopted SQLite sessions via [`Self::recover`]. Returns the
    /// set of adopted live `session_id`s so session restore keeps their
    /// `OTTO_MCP_TOKEN` credentials active.
    pub async fn recover_and_adopt(&self) -> Result<HashSet<Id>> {
        let mut live_ids = HashSet::new();
        let mut handled_ids = HashSet::new();
        if let Some(holders_dir) = self.inner.config.holders_dir.clone() {
            for sock in list_holder_sockets(&holders_dir) {
                if let Some((sid, is_alive)) = self.adopt_holder_socket(&sock).await {
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
        Ok(live_ids)
    }

    async fn adopt_holder_socket(&self, socket_path: &Path) -> Option<(Id, bool)> {
        let adopted = match adopt_socket(socket_path).await {
            Ok(a) => a,
            Err(_) => {
                let _ = tokio::fs::remove_file(socket_path).await;
                return None;
            }
        };
        let sid = adopted.checkpoint.session_id.clone();
        let rec = match self.inner.repo.get(&sid).await {
            Ok(r) if !r.closed => r,
            _ => {
                adopted.client.shutdown(Duration::from_millis(500)).await;
                return None;
            }
        };
        let Some(mut driver) = self.inner.drivers.create(rec.session.provider) else {
            adopted.client.shutdown(Duration::from_millis(500)).await;
            return None;
        };
        if let Err(e) = driver.restore_state(&adopted.checkpoint.driver_state) {
            tracing::warn!(session = %sid, "runtime holder driver restore failed: {e}");
            adopted.client.shutdown(Duration::from_millis(500)).await;
            return None;
        }
        let Ok(live) = self.live(&sid).await else {
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
            tokio::spawn(pump_held(
                Arc::clone(&self.inner),
                Arc::clone(&live),
                sid.clone(),
                epoch,
                events,
                claude_login,
            ));
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
