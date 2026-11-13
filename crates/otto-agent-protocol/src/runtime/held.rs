//! Children that outlive the daemon (a deploy, a crash, an app relaunch).
//!
//! With session persistence on, a structured child runs inside a detached
//! stdio holder ([`otto_pty::stdio_holder`]) instead of as a direct child of
//! `ottod`. The holder numbers the child's stdout lines and keeps every line
//! the runtime has not acknowledged. The runtime acknowledges a line only once
//! a CHECKPOINT covering it is durable: the driver's protocol state
//! ([`crate::StructuredDriver::export_state`]) plus what the runtime itself
//! holds about the process (its running turn, its request-id generation, the
//! last line processed), stored as the session's `process` entity — through
//! the session's ordered persister, so it is never ahead of the timeline
//! rows its lines produced.
//!
//! A checkpoint is written before anything is sent to the child (so the
//! stored driver state knows every request the child may answer), at most
//! every [`CHECKPOINT_EVERY`] while lines stream, once the stream goes quiet,
//! and at shutdown. Shutdown ([`super::AgentRuntime::shutdown`]) detaches:
//! the child, its tools and its holder keep running. Boot recovery adopts
//! each live checkpoint: re-attach with "replay after line N", restore a fresh
//! driver from the state, and pump on — the turn continues, its pending
//! approvals stay answerable, the queue goes out after it. A checkpoint that
//! cannot be adopted (holder gone, other protocol, unreadable state) ends its
//! holder and falls back to today's recovery: the turn ends interrupted and
//! the next message resumes the provider's session.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use otto_core::{Error, Id, Result};
use otto_state::RuntimeEntityRow;
use serde::{Deserialize, Serialize};

use super::request_ids::RequestIds;
use super::{descendants, pump, ActiveTurn, Inner, Live, PersistOp, Proc, State};
use crate::process::adopt_held_child;

/// The checkpoint's entity kind (ignored by the projection).
pub(super) const K_PROCESS: &str = "process";
/// While lines stream, at most one checkpoint per this.
pub(super) const CHECKPOINT_EVERY: Duration = Duration::from_millis(500);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct TurnCheckpoint {
    id: Id,
    interrupting: bool,
    stalled: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Checkpoint {
    pub(super) socket: PathBuf,
    pid: u32,
    /// The last holder line the driver processed.
    line_seq: u64,
    driver: serde_json::Value,
    request_prefix: String,
    turn: Option<TurnCheckpoint>,
    fresh_starting: bool,
    /// Which Claude login the child runs as (see `ensure_process`).
    claude_login: Option<Option<PathBuf>>,
}

/// Write the process checkpoint now (no-op for a child that is not held).
/// The holder may drop the lines it covers once it is durable.
pub(super) fn checkpoint(live: &Live, st: &mut State) {
    let Some(p) = st.proc.as_mut() else {
        return;
    };
    let Some(link) = p.held.as_ref() else {
        return;
    };
    // Due again only after new lines, whatever happens below.
    p.dirty = false;
    p.checkpointed_at = Instant::now();
    let Some(driver) = p.driver.export_state() else {
        return;
    };
    let cp = Checkpoint {
        socket: link.socket.clone(),
        pid: p.pid,
        line_seq: p.line_seq,
        driver,
        request_prefix: p.request_ids.prefix().to_string(),
        turn: st.turn.as_ref().map(|t| TurnCheckpoint {
            id: t.id.clone(),
            interrupting: t.interrupting,
            stalled: t.stalled,
        }),
        fresh_starting: st.fresh_starting,
        claude_login: p.claude_login.clone(),
    };
    let row = row(
        st.session.last_sequence,
        serde_json::to_value(cp).unwrap_or_default(),
    );
    let _ = live
        .persist
        .send(PersistOp::Checkpoint(row, link.acker(), p.line_seq));
}

/// The checkpoint entity (`payload` null = none).
pub(super) fn row(sequence: u64, payload: serde_json::Value) -> RuntimeEntityRow {
    RuntimeEntityRow {
        kind: K_PROCESS.into(),
        entity_id: K_PROCESS.into(),
        sequence,
        payload,
    }
}

/// When a dirty held process is due its next checkpoint.
pub(super) fn due(st: &State) -> Option<Instant> {
    st.proc
        .as_ref()
        .filter(|p| p.dirty && p.held.is_some())
        .map(|p| p.checkpointed_at + CHECKPOINT_EVERY)
}

/// End a holder this daemon will not adopt (any protocol version).
pub(super) async fn terminate(socket: PathBuf) {
    let _ = tokio::task::spawn_blocking(move || otto_pty::holder::terminate(&socket)).await;
}

impl super::AgentRuntime {
    /// Carry on with the child a previous daemon left running (see the
    /// module docs). On error the caller terminates the holder.
    pub(super) async fn adopt(&self, id: &Id, cp: Checkpoint) -> Result<()> {
        let child = adopt_held_child(&cp.socket, cp.line_seq)
            .await
            .map_err(|e| Error::Internal(format!("adopt {}: {e}", cp.socket.display())))?;
        if child.pid != cp.pid {
            return Err(Error::Internal(format!(
                "the holder runs pid {}, the checkpoint names {}",
                child.pid, cp.pid
            )));
        }
        let live = self.live(id).await?;
        let provider = live.lock().session.provider;
        let mut driver = self
            .inner
            .drivers
            .create(provider)
            .ok_or_else(|| Error::Invalid(format!("no driver for '{}'", provider.as_str())))?;
        driver
            .import_state(cp.driver)
            .map_err(|e| Error::Internal(format!("restore the driver: {e}")))?;
        let inner: &Arc<Inner> = &self.inner;
        let epoch = {
            let mut st = live.lock();
            if st.proc.is_some() {
                return Err(Error::Conflict("the session already has a process".into()));
            }
            st.epoch += 1;
            let epoch = st.epoch;
            st.proc = Some(Proc {
                epoch,
                request_ids: RequestIds::from_prefix(cp.request_prefix),
                pid: child.pid,
                stdin: child.stdin,
                driver,
                held: child.held,
                line_seq: cp.line_seq,
                dirty: false,
                checkpointed_at: Instant::now(),
                claude_login: cp.claude_login.clone(),
            });
            st.turn = cp.turn.map(|t| ActiveTurn {
                id: t.id,
                interrupting: t.interrupting,
                stalled: t.stalled,
            });
            st.fresh_starting = cp.fresh_starting;
            // The session is as the provider left it: running, waiting on a
            // person, or ready — and its background work still runs.
            if let Some(ps) = st.proj.session.clone() {
                st.session.background_tasks = ps.background_tasks;
                st.session.status = super::session_status(ps.status);
            }
            st.reserved = st.next_seq + inner.config.seq_reserve;
            let _ = live.persist.send(PersistOp::Reserve(st.reserved));
            epoch
        };
        if cp.claude_login.is_some() {
            descendants::watch(Arc::clone(&live), epoch, cp.pid);
        }
        tokio::spawn(pump(
            Arc::clone(inner),
            Arc::clone(&live),
            epoch,
            child.messages,
            cp.claude_login,
        ));
        tracing::info!(session = %id, pid = cp.pid, "re-adopted a structured agent process");
        Ok(())
    }
}
