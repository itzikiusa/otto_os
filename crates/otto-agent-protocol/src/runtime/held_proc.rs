//! Detached `runtime-holder` spawn, live event pump, and checkpoint sync for [`super::AgentRuntime`].

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_core::agent_runtime::{
    CancelReason, ProviderFailure, ProviderSessionStatus, RuntimeSession, TurnItem, TurnItemStatus,
    TurnTerminalStatus,
};
use otto_core::{Id, Result};
use tokio::sync::{mpsc, oneshot};

use super::{
    apply_step, apply_step_opts, cancel_pending, descendants, dispatch_child_stdout,
    evict_if_dead_after_flush, finish_turn, flush_held, idle_settle_deadline, interrupt_turn,
    record_claude_stdout_line, retire, set_session_status, settle_background, stall_deadline,
    Inner, Live, PersistOp, Proc, ProcTransport, RequestIds, State,
};
use crate::driver::{LaunchContext, StructuredDriver};
use crate::holder::{
    spawn_held, HolderCheckpoint, HolderChildCommand, HolderEvent, HolderLauncher, HolderSpawnSpec,
    HolderTurnSnapshot, LineStream,
};
use crate::process::ChildCommand;

/// Minimum interval between non-critical UDS `Checkpoint` pushes during
/// high-frequency streaming deltas. Critical transitions (`force = true`)
/// bypass this throttle and flush immediately.
pub(super) const HOLDER_CHECKPOINT_THROTTLE: Duration = Duration::from_millis(250);

pub(super) fn build_checkpoint(st: &State) -> Option<HolderCheckpoint> {
    let proc = st.proc.as_ref()?;
    let ProcTransport::Held {
        acked_seq,
        claude_login,
        ..
    } = &proc.transport
    else {
        return None;
    };

    let mut streaming_map: HashMap<Id, TurnItem> =
        HashMap::with_capacity(st.proj.streaming_ids.len() + st.held.len());
    for id in &st.proj.streaming_ids {
        if let Some(item) = st.proj.items.get(id) {
            if item.is_streaming() {
                streaming_map.insert(id.clone(), item.clone());
            }
        }
    }
    for held_item in &st.held {
        streaming_map.insert(held_item.id.clone(), held_item.clone());
    }
    let mut streaming_items: Vec<TurnItem> = streaming_map.into_values().collect();
    streaming_items.sort_by_key(|i| i.ordinal);

    let active_turn = st.turn.as_ref().map(|t| HolderTurnSnapshot {
        id: t.id.clone(),
        origin: t.origin,
        interrupting: t.interrupting,
        stalled: t.stalled,
    });
    let active_turn_id = active_turn.as_ref().map(|t| t.id.clone());

    Some(HolderCheckpoint {
        session_id: st.session.id.clone(),
        provider: st.session.provider,
        acked_seq: *acked_seq,
        driver_state: proc.driver.snapshot_state(),
        active_turn_id,
        active_turn,
        streaming_items,
        request_ids_prefix: proc.request_ids.prefix().to_string(),
        claude_login: claude_login.clone(),
        fresh_starting: st.fresh_starting,
    })
}

pub(super) fn checkpoint_flush_deadline(st: &State) -> Option<Instant> {
    let proc = st.proc.as_ref()?;
    let ProcTransport::Held {
        last_checkpoint_at,
        checkpoint_dirty,
        ..
    } = &proc.transport
    else {
        return None;
    };
    if !*checkpoint_dirty {
        return None;
    }
    Some(match *last_checkpoint_at {
        Some(t) => t + HOLDER_CHECKPOINT_THROTTLE,
        None => Instant::now(),
    })
}

pub(super) fn sync_holder_checkpoint(st: &mut State, force: bool) {
    let now = Instant::now();
    {
        let Some(Proc {
            transport:
                ProcTransport::Held {
                    last_checkpoint_at,
                    checkpoint_dirty,
                    ..
                },
            ..
        }) = st.proc.as_mut()
        else {
            return;
        };
        if !force
            && last_checkpoint_at
                .is_some_and(|t| now.saturating_duration_since(t) < HOLDER_CHECKPOINT_THROTTLE)
        {
            *checkpoint_dirty = true;
            return;
        }
    }
    let Some(cp) = build_checkpoint(st) else {
        return;
    };
    if let Some(Proc {
        transport:
            ProcTransport::Held {
                client,
                last_checkpoint_at,
                checkpoint_dirty,
                #[cfg(test)]
                checkpoint_writes,
                ..
            },
        ..
    }) = st.proc.as_mut()
    {
        client.update_checkpoint(cp);
        *last_checkpoint_at = Some(now);
        *checkpoint_dirty = false;
        #[cfg(test)]
        {
            *checkpoint_writes += 1;
        }
    }
}

pub(super) fn finalize_child_exit(
    inner: &Arc<Inner>,
    live: &Arc<Live>,
    st: &mut State,
    mut proc: Proc,
    code: Option<i32>,
    stderr_tail: String,
) {
    live.proc_ended.notify_waiters();
    st.released = false;
    retire(inner, &st.session);
    let step = proc.driver.on_exit(code);
    apply_step_opts(inner, live, st, step, true, true);
    settle_background(inner, live, st, TurnItemStatus::Interrupted);
    let detail = stderr_tail
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .map(|l| format!(": {}", l.trim()))
        .unwrap_or_default();
    let code_s = code.map_or("a signal".to_string(), |c| format!("code {c}"));
    let message = format!("the agent process exited ({code_s}){detail}");
    if let Some(turn) = st.turn.as_ref() {
        let (status, failure) = if turn.interrupting {
            (TurnTerminalStatus::Interrupted, None)
        } else {
            (
                TurnTerminalStatus::Failed,
                Some(ProviderFailure {
                    class: "transport_error".into(),
                    message: message.clone(),
                    code: code.map(|c| c.to_string()),
                    retryable: Some(true),
                }),
            )
        };
        finish_turn(inner, live, st, status, failure, None);
    } else {
        cancel_pending(inner, live, st, CancelReason::Agent);
    }
    let last_error = (code != Some(0)).then_some(message);
    set_session_status(inner, live, st, ProviderSessionStatus::Stopped, last_error);
}

#[allow(clippy::too_many_arguments)]
pub(super) async fn spawn_via_holder(
    inner: &Arc<Inner>,
    live: &Arc<Live>,
    session: &RuntimeSession,
    mut driver: Box<dyn StructuredDriver>,
    ctx: &LaunchContext,
    cmd: ChildCommand,
    claude_login: Option<Option<PathBuf>>,
    socket_path: PathBuf,
    launcher: &HolderLauncher,
) -> Result<()> {
    let request_ids = RequestIds::new();
    let fresh_starting = live.lock().fresh_starting;
    let start_step = driver.start(ctx);
    let spec = HolderSpawnSpec {
        command: HolderChildCommand {
            program: cmd.program.clone(),
            args: cmd.args,
            cwd: cmd.cwd,
            env: cmd.env,
        },
        initial_checkpoint: HolderCheckpoint {
            session_id: session.id.clone(),
            provider: session.provider,
            acked_seq: 0,
            driver_state: driver.snapshot_state(),
            active_turn_id: None,
            active_turn: None,
            streaming_items: Vec::new(),
            request_ids_prefix: request_ids.prefix().to_string(),
            claude_login: claude_login.clone(),
            fresh_starting,
        }
        .to_wire_value(),
        exit_linger_ms: None,
        orphan_ttl_ms: None,
    };

    let held = spawn_held(launcher, &socket_path, &spec)
        .await
        .map_err(|e| super::map_spawn_error(session.provider.as_str(), &cmd.program, e))?;
    let cli_pid = held.child_pid;
    let sid = session.id.clone();
    let (epoch, flushed) = {
        let mut st = live.lock();
        live.detaching_for_restart.store(false, Ordering::Release);
        st.epoch += 1;
        let epoch = st.epoch;
        st.proc = Some(Proc {
            epoch,
            request_ids,
            pid: cli_pid,
            transport: ProcTransport::Held {
                client: held.client,
                acked_seq: 0,
                claude_login: claude_login.clone(),
                last_checkpoint_at: Some(Instant::now()),
                checkpoint_dirty: false,
                #[cfg(test)]
                checkpoint_writes: 0,
            },
            driver,
        });
        st.reserved = st.next_seq + inner.config.seq_reserve;
        let _ = live.persist.send(PersistOp::Reserve(st.reserved));
        let (ftx, frx) = oneshot::channel();
        let _ = live.persist.send(PersistOp::Flush(ftx));
        set_session_status(inner, live, &mut st, ProviderSessionStatus::Starting, None);
        apply_step_opts(inner, live, &mut st, start_step, true, true);
        (epoch, frx)
    };
    if claude_login.is_some() {
        descendants::watch(
            Arc::clone(inner),
            Arc::clone(live),
            sid.clone(),
            epoch,
            cli_pid,
        );
    }
    tokio::spawn(pump_held(
        Arc::clone(inner),
        Arc::clone(live),
        sid,
        epoch,
        held.events,
        claude_login,
    ));
    let _ = flushed.await;
    Ok(())
}

pub(super) async fn pump_held(
    inner: Arc<Inner>,
    live: Arc<Live>,
    sid: Id,
    epoch: u64,
    rx: mpsc::Receiver<HolderEvent>,
    claude_login: Option<Option<PathBuf>>,
) {
    pump_held_inner(&inner, &live, epoch, rx, claude_login).await;
    drop(live);
    evict_if_dead_after_flush(&inner, &sid).await;
}

async fn pump_held_inner(
    inner: &Arc<Inner>,
    live: &Arc<Live>,
    epoch: u64,
    mut rx: mpsc::Receiver<HolderEvent>,
    claude_login: Option<Option<PathBuf>>,
) {
    let window = inner.config.stream_coalesce;
    let mut next_sideband: Option<Instant> = None;
    loop {
        let changed = live.deadline_changed.notified();
        tokio::pin!(changed);
        changed.as_mut().enable();
        let due = {
            let st = live.lock();
            if st.proc.as_ref().map(|p| p.epoch) != Some(epoch) {
                return;
            }
            let sideband_iv = st
                .proc
                .as_ref()
                .and_then(|p| p.driver.sideband_poll())
                .map(|s| s.interval);
            match sideband_iv {
                Some(iv) => {
                    let target = Instant::now() + iv;
                    next_sideband = Some(match next_sideband {
                        Some(prev) => prev.min(target),
                        None => target,
                    });
                }
                None => {
                    next_sideband = None;
                }
            }
            let held = st.held_deadline(window);
            let settle = idle_settle_deadline(&st, &inner.config);
            let stall = stall_deadline(&st, &inner.config);
            let cp_due = checkpoint_flush_deadline(&st);
            held.into_iter()
                .chain(settle)
                .chain(stall)
                .chain(next_sideband)
                .chain(cp_due)
                .min()
        };
        let ev = tokio::select! {
            _ = &mut changed => continue,
            ev = rx.recv() => ev,
            _ = async {
                match due {
                    Some(at) => tokio::time::sleep_until(at.into()).await,
                    None => std::future::pending::<()>().await,
                }
            } => {
                let need_poll = next_sideband.is_some_and(|at| at <= Instant::now()) || {
                    let st = live.lock();
                    idle_settle_deadline(&st, &inner.config)
                        .is_some_and(|at| at <= Instant::now())
                };
                if need_poll {
                    next_sideband = None;
                    for _ in 0..8 {
                        if !super::poll_driver_sideband(inner, live, Some(epoch)).await {
                            break;
                        }
                    }
                }
                let mut st = live.lock();
                if st.proc.as_ref().map(|p| p.epoch) != Some(epoch) {
                    return;
                }
                flush_held(inner, live, &mut st);
                if checkpoint_flush_deadline(&st).is_some_and(|at| at <= Instant::now()) {
                    sync_holder_checkpoint(&mut st, true);
                }
                let settled = idle_settle_deadline(&st, &inner.config)
                    .is_some_and(|at| at <= Instant::now());
                if settled {
                    if let Some(proc) = st.proc.as_mut() {
                        let step = proc.driver.on_idle_settle();
                        apply_step(inner, live, &mut st, step);
                    }
                    continue;
                }
                let stalled =
                    stall_deadline(&st, &inner.config).is_some_and(|at| at <= Instant::now());
                if stalled {
                    tracing::warn!(session = %st.session.id, "runtime turn stalled; stopping it");
                    if let Some(t) = st.turn.as_mut() {
                        t.stalled = true;
                    }
                    drop(st);
                    interrupt_turn(inner, live);
                }
                continue;
            }
        };
        let Some(ev) = ev else {
            handle_unexpected_holder_disconnect(inner, live, epoch);
            return;
        };
        match ev {
            HolderEvent::Disconnected => {
                handle_unexpected_holder_disconnect(inner, live, epoch);
                return;
            }
            HolderEvent::Line(entry) => {
                if entry.stream == LineStream::Out && entry.overlong_bytes.is_none() {
                    record_claude_stdout_line(&claude_login, &entry.line);
                }
                let mut st = live.lock();
                if st.proc.as_ref().map(|p| p.epoch) != Some(epoch) {
                    return;
                }
                if let Some(Proc {
                    transport: ProcTransport::Held { acked_seq, .. },
                    ..
                }) = st.proc.as_mut()
                {
                    *acked_seq = entry.seq;
                }
                if entry.stream == LineStream::Out {
                    let line_or_overlong =
                        entry.overlong_bytes.map_or(Ok(entry.line.as_str()), Err);
                    dispatch_child_stdout(inner, live, &mut st, line_or_overlong, true);
                } else {
                    sync_holder_checkpoint(&mut st, false);
                }
            }
            HolderEvent::Exited { code, stderr_tail } => {
                let mut st = live.lock();
                if st.proc.as_ref().map(|p| p.epoch) != Some(epoch) {
                    return;
                }
                let proc = st.proc.take().expect("checked above");
                if let ProcTransport::Held { client, .. } = &proc.transport {
                    client.shutdown_fire_and_forget(Duration::ZERO);
                }
                finalize_child_exit(inner, live, &mut st, proc, code, stderr_tail);
                return;
            }
        }
    }
}

fn handle_unexpected_holder_disconnect(inner: &Arc<Inner>, live: &Arc<Live>, epoch: u64) {
    if live.detaching_for_restart.load(Ordering::Acquire) {
        return;
    }
    let mut st = live.lock();
    if st.proc.as_ref().map(|p| p.epoch) != Some(epoch) {
        return;
    }
    let mut proc = st.proc.take().expect("checked above");
    if let ProcTransport::Held { client, .. } = &proc.transport {
        client.shutdown_fire_and_forget(Duration::from_millis(250));
    }
    live.proc_ended.notify_waiters();
    retire(inner, &st.session);
    let step = proc.driver.on_exit(None);
    apply_step_opts(inner, live, &mut st, step, true, true);
    flush_held(inner, live, &mut st);
    settle_background(inner, live, &mut st, TurnItemStatus::Interrupted);
    cancel_pending(inner, live, &mut st, CancelReason::Agent);
    let msg = "the runtime holder disconnected unexpectedly".to_string();
    tracing::warn!(session = %st.session.id, "{msg}");
    if let Some(turn) = st.turn.as_ref() {
        let (status, failure) = if turn.interrupting {
            (TurnTerminalStatus::Interrupted, None)
        } else {
            (
                TurnTerminalStatus::Failed,
                Some(ProviderFailure {
                    class: "transport_error".into(),
                    message: msg.clone(),
                    code: None,
                    retryable: Some(true),
                }),
            )
        };
        finish_turn(inner, live, &mut st, status, failure, None);
    }
    set_session_status(
        inner,
        live,
        &mut st,
        ProviderSessionStatus::Error,
        Some(msg),
    );
}
