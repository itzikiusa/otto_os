//! Reviving a closed or dormant session (split out of runtime.rs).

use std::sync::Arc;

use chrono::Utc;
use otto_core::agent_runtime::{ProviderSessionStatus, RuntimeSession, RuntimeSessionStatus};
use otto_core::{Id, Result};

use super::{set_session_status, AgentRuntime, PersistOp};

impl AgentRuntime {
    /// Revive a closed or dormant session: it takes prompts again and its agent is
    /// respawned now, resuming the provider's own thread (`native_thread_id`).
    /// A provider that cannot resume it (its process dies before it is
    /// ready) gets a fresh thread instead and the timeline says so (a
    /// `system_notice`). Its meta is kept. A live process is left alone.
    /// What waits in its queue (a child thread's reply) then goes out.
    pub async fn reopen(&self, id: &Id) -> Result<RuntimeSession> {
        let live = self.live(id).await?;
        let _prompt = live.prompt_lock.lock().await;
        {
            let mut st = live.lock();
            if st.session.status == RuntimeSessionStatus::Closed {
                st.session.status = RuntimeSessionStatus::Dormant;
                // After the close's own write, in order.
                let _ = live.persist.send(PersistOp::Opened);
            } else if st.proc.is_some() {
                return Ok(st.session.clone());
            }
            // A revive is activity: the idle release must not stop it again.
            st.session.last_active_at = Utc::now();
        }
        self.flush(id).await;
        self.spawn_resuming(id, &live).await?;
        let (s, queued) = {
            let st = live.lock();
            (st.session.clone(), !st.proj.queue.is_empty())
        };
        // What waited for the person — a child thread's reply above all —
        // goes out now, oldest first (after this call: the drain takes the
        // prompt lock held here).
        if queued {
            self.kick_queue(id);
        }
        Ok(s)
    }

    /// Low-latency HTTP reopen: validates & spawns the CLI process immediately
    /// (`< 30 ms`), returning the revived [`RuntimeSession`] without blocking
    /// the HTTP caller for up to `start_wait` (20 s) on MCP/CLI boot. A
    /// background task finishes the readiness check, fresh-thread fallback on
    /// refused resume, and queued reply delivery.
    pub async fn reopen_fast(&self, id: &Id) -> Result<RuntimeSession> {
        let live = self.live(id).await?;
        let (s, resume_key, need_bg_watch) = {
            let _prompt = live.prompt_lock.lock().await;
            {
                let mut st = live.lock();
                if st.session.status == RuntimeSessionStatus::Closed {
                    st.session.status = RuntimeSessionStatus::Dormant;
                    let _ = live.persist.send(PersistOp::Opened);
                } else if st.proc.is_some() {
                    return Ok(st.session.clone());
                }
                st.session.last_active_at = Utc::now();
            }
            self.flush(id).await;
            self.ensure_process(&live).await?;
            let st = live.lock();
            let resume_key = st.session.native_thread_id.clone();
            let need_bg = resume_key.is_some() || !st.proj.queue.is_empty();
            (st.session.clone(), resume_key, need_bg)
        };
        if need_bg_watch {
            let rt = self.clone();
            let sid = id.clone();
            let live_bg = Arc::clone(&live);
            tokio::spawn(async move {
                let started = resume_key.is_none() || rt.wait_started(&live_bg).await;
                if !started {
                    let _prompt = live_bg.prompt_lock.lock().await;
                    let idle = {
                        let st = live_bg.lock();
                        st.proc.is_none() && st.turn.is_none() && st.starting == 0
                    };
                    if idle {
                        if let Err(e) = rt.spawn_resuming(&sid, &live_bg).await {
                            tracing::warn!(session = %sid, "background reopen_fast resume failed: {e}");
                            let mut st = live_bg.lock();
                            if st.proc.is_none() && st.turn.is_none() {
                                set_session_status(
                                    &rt.inner,
                                    &live_bg,
                                    &mut st,
                                    ProviderSessionStatus::Error,
                                    Some(e.to_string()),
                                );
                            }
                        }
                    }
                }
                let queued = {
                    let st = live_bg.lock();
                    !st.proj.queue.is_empty() && st.turn.is_none()
                };
                if queued {
                    rt.kick_queue(&sid);
                }
            });
        }
        Ok(s)
    }
}
