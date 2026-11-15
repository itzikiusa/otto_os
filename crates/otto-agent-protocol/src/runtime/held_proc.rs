//! Detached `runtime-holder` spawn, live event pump, and checkpoint sync for [`super::AgentRuntime`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::Utc;
use otto_core::agent_runtime::{
    CancelReason, ProviderFailure, ProviderSessionStatus, RuntimeSession, TurnItem, TurnItemStatus,
    TurnTerminalStatus,
};
use otto_core::{Error, Id, Result};
use tokio::sync::{mpsc, oneshot};

use super::{
    apply_step, cancel_pending, descendants, finish_turn, flush_held, idle_settle_deadline,
    interrupt_turn, retire, set_session_status, settle_background, stall_deadline, Inner, Live,
    PersistOp, Proc, ProcTransport, RequestIds, State,
};
use crate::driver::{DriverEvent, DriverStep, LaunchContext, StructuredDriver};
use crate::holder::{
    socket_path_for, spawn_held, HolderCheckpoint, HolderChildCommand, HolderEvent, HolderLauncher,
    HolderSpawnSpec, HolderTurnSnapshot, LineStream,
};
use crate::process::ChildCommand;

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

    let mut streaming_map: HashMap<Id, TurnItem> = st
        .proj
        .items
        .values()
        .filter(|i| i.is_streaming())
        .map(|i| (i.id.clone(), i.clone()))
        .collect();
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

pub(super) fn sync_holder_checkpoint(st: &State) {
    let Some(cp) = build_checkpoint(st) else {
        return;
    };
    if let Some(Proc {
        transport: ProcTransport::Held { client, .. },
        ..
    }) = st.proc.as_ref()
    {
        client.update_checkpoint(cp);
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
    retire(inner, &st.session);
    let step = proc.driver.on_exit(code);
    apply_step(inner, live, st, step);
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
    holders_dir: &Path,
    launcher: &HolderLauncher,
) -> Result<()> {
    let socket_path = socket_path_for(holders_dir, &session.id).map_err(|e| {
        Error::Internal(format!(
            "cannot prepare runtime holder socket for {}: {e}",
            session.id
        ))
    })?;
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
        },
        exit_linger_ms: None,
    };

    let held = spawn_held(launcher, &socket_path, &spec)
        .await
        .map_err(|e| {
            Error::Invalid(format!(
                "could not start {} ({}): {e}",
                session.provider.as_str(),
                cmd.program
            ))
        })?;
    let cli_pid = held.child_pid;
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
            },
            driver,
        });
        st.reserved = st.next_seq + inner.config.seq_reserve;
        let _ = live.persist.send(PersistOp::Reserve(st.reserved));
        let (ftx, frx) = oneshot::channel();
        let _ = live.persist.send(PersistOp::Flush(ftx));
        set_session_status(inner, live, &mut st, ProviderSessionStatus::Starting, None);
        apply_step(inner, live, &mut st, start_step);
        (epoch, frx)
    };
    if claude_login.is_some() {
        descendants::watch(Arc::clone(live), epoch, cli_pid);
    }
    tokio::spawn(pump_held(
        Arc::clone(inner),
        Arc::clone(live),
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
                    next_sideband.get_or_insert_with(|| Instant::now() + iv);
                }
                None => {
                    next_sideband = None;
                }
            }
            let held = st.held_deadline(window);
            let settle = idle_settle_deadline(&st, &inner.config);
            let stall = stall_deadline(&st, &inner.config);
            held.into_iter()
                .chain(settle)
                .chain(stall)
                .chain(next_sideband)
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
                if next_sideband.is_some_and(|at| at <= Instant::now()) {
                    next_sideband = None;
                    for _ in 0..8 {
                        if !super::poll_driver_sideband(&inner, &live, Some(epoch)).await {
                            break;
                        }
                    }
                }
                let mut st = live.lock();
                if st.proc.as_ref().map(|p| p.epoch) != Some(epoch) {
                    return;
                }
                flush_held(&inner, &live, &mut st);
                let settled = idle_settle_deadline(&st, &inner.config)
                    .is_some_and(|at| at <= Instant::now());
                if settled {
                    if let Some(proc) = st.proc.as_mut() {
                        let step = proc.driver.on_idle_settle();
                        apply_step(&inner, &live, &mut st, step);
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
                    interrupt_turn(&inner, &live);
                }
                continue;
            }
        };
        let Some(ev) = ev else { return };
        match ev {
            HolderEvent::Disconnected => return,
            HolderEvent::Line(entry) => {
                if entry.stream == LineStream::Out && entry.overlong_bytes.is_none() {
                    if let Some(dir) = &claude_login {
                        otto_usage::limits::record_claude_line(
                            dir.as_deref(),
                            &entry.line,
                            Utc::now(),
                        );
                    }
                }
                let mut st = live.lock();
                if st.proc.as_ref().map(|p| p.epoch) != Some(epoch) {
                    return;
                }
                if entry.stream == LineStream::Out {
                    let step = if let Some(n) = entry.overlong_bytes {
                        DriverStep {
                            events: vec![DriverEvent::Error(ProviderFailure {
                                class: "protocol_error".into(),
                                message: format!("dropped an agent output line of {n} bytes"),
                                code: None,
                                retryable: None,
                            })],
                            ..DriverStep::default()
                        }
                    } else {
                        st.proc
                            .as_mut()
                            .expect("checked above")
                            .driver
                            .on_line(&entry.line)
                    };
                    apply_step(&inner, &live, &mut st, step);
                }
                if let Some(Proc {
                    transport: ProcTransport::Held { acked_seq, .. },
                    ..
                }) = st.proc.as_mut()
                {
                    *acked_seq = entry.seq;
                }
                sync_holder_checkpoint(&st);
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
                finalize_child_exit(&inner, &live, &mut st, proc, code, stderr_tail);
                return;
            }
        }
    }
}
