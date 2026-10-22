//! The shutdown sweep and the spawn refusal while it runs
//! (concurrency-async-5-04; moved out of `manager.rs`).

use super::*;

impl SessionManager {
    /// Kill every live PTY and mark the sessions exited. Used when the app
    /// closes (no orphaned agent processes left running) and on daemon
    /// shutdown. Returns the number of sessions terminated.
    ///
    /// The app closing is NOT the daemon exiting (under launchd it keeps
    /// running for the next window): spawns are refused only while the kill
    /// sweep runs, so a resume racing it can't fork a fresh CLI, and the flag
    /// is cleared again afterwards — left set, every later resume/restart
    /// answered "the daemon is shutting down" until a daemon restart.
    pub async fn shutdown_all(&self) -> usize {
        self.shutting_down
            .store(true, std::sync::atomic::Ordering::SeqCst);
        let ids: Vec<Id> = self.live.iter().map(|e| e.key().clone()).collect();
        let killed = self.shutdown_ids(ids).await;
        self.shutting_down
            .store(false, std::sync::atomic::Ordering::SeqCst);
        killed
    }

    /// `Conflict` while a shutdown sweep runs — a spawn now would land after
    /// it iterated `live` and outlive it (create and resume both check).
    pub(super) fn refuse_while_shutting_down(&self) -> Result<()> {
        if self.shutting_down.load(std::sync::atomic::Ordering::SeqCst) {
            Err(Error::Conflict("the daemon is shutting down".into()))
        } else {
            Ok(())
        }
    }

    /// [`SessionManager::spawn_session_pty`] bracketed by the shutdown check:
    /// refused before the spawn, and — since the flag may flip while the PTY
    /// spawns — the fresh CLI is killed afterwards so the caller takes the
    /// same cleanup path as a failed spawn (revoke the MCP token, drop the row).
    pub(super) async fn spawn_unless_shutting_down(
        &self,
        session: &Session,
        spec: CommandSpec,
        cols: u16,
        rows: u16,
    ) -> Result<PtyHandle> {
        self.refuse_while_shutting_down()?;
        let handle = self.spawn_session_pty(session, spec, cols, rows).await?;
        if let Err(e) = self.refuse_while_shutting_down() {
            let _ = handle.kill();
            return Err(e);
        }
        Ok(handle)
    }
}
