//! Machine-aware admission for UNATTENDED headless turns.
//!
//! [`Orchestrator::run_agent`] spawns its own claude PTY: it never passes
//! through `SessionManager::create`, so the daemon's admission gate
//! (`otto_sessions::admission`: load per core, memory pressure, a ceiling on
//! live background agents) never saw these turns. On 2026-11-12 the
//! self-improvement producer, the story watcher and the scheduled-task
//! fallback kept starting claude turns while the machine sat at load 60.
//!
//! The host installs an [`AdmissionHook`] once at boot
//! ([`Orchestrator::set_admission_hook`]); without one (tests, bare
//! harnesses) nothing waits. Only the `*_background` entry points take a
//! place — engines no person is waiting on. Interactive callers (⌘K planning,
//! handover briefs, query and workflow drafts) keep calling
//! [`Orchestrator::run_agent`] and never wait.

use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

use otto_core::auth::BoxFuture;

use crate::Orchestrator;

/// A place under the background-agent ceiling, held for the whole turn;
/// dropping it gives the place back. Opaque here (the host's permit type).
pub type AdmissionGuard = Box<dyn Any + Send>;

/// Waits for a place for one unattended turn labelled `source` (an
/// `ADMISSION_GATED_SOURCES` name). Must return promptly for a source the
/// host does not hold back, and must give way after its own bounded wait —
/// it never fails.
pub type AdmissionHook = Arc<dyn Fn(&str) -> BoxFuture<'static, AdmissionGuard> + Send + Sync>;

impl Orchestrator {
    /// Install the host's admission hook (once, at boot; a second call is
    /// ignored and logged).
    pub fn set_admission_hook(&self, hook: AdmissionHook) {
        if self.admission.set(hook).is_err() {
            tracing::warn!("orchestrator admission hook already installed");
        }
    }

    /// Wait for a place for an unattended `source` turn: `None` without a
    /// hook, or under the offline E2E stub (no process is spawned there).
    pub async fn admit_background(&self, source: &str) -> Option<AdmissionGuard> {
        if otto_core::e2e::isolated() {
            return None;
        }
        let hook = self.admission.get()?;
        Some(hook(source).await)
    }

    /// [`Orchestrator::run_agent`] for an UNATTENDED engine turn: waits for
    /// admission first and holds the place until the turn ends.
    pub async fn run_agent_background(
        &self,
        source: &str,
        prompt: &str,
        cwd: &str,
        model: Option<&str>,
        no_progress: Duration,
    ) -> otto_core::Result<String> {
        let _place = self.admit_background(source).await;
        self.run_agent(prompt, cwd, model, no_progress).await
    }

    /// [`Orchestrator::run_agent_untrusted`] for an UNATTENDED engine turn
    /// (the story watcher's reconcile): admission first, as above.
    pub async fn run_agent_untrusted_background(
        &self,
        source: &str,
        prompt: &str,
        model: Option<&str>,
        no_progress: Duration,
    ) -> otto_core::Result<String> {
        let _place = self.admit_background(source).await;
        self.run_agent_untrusted(prompt, model, no_progress).await
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Mutex;

    use super::*;

    /// A guard that counts its own drop.
    struct Place(Arc<AtomicUsize>);
    impl Drop for Place {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[tokio::test]
    async fn no_hook_admits_at_once_and_a_hook_sees_the_source() {
        let orch = Orchestrator::new("claude");
        assert!(orch.admit_background("self_improvement").await.is_none());

        let seen = Arc::new(Mutex::new(Vec::<String>::new()));
        let released = Arc::new(AtomicUsize::new(0));
        let (s, r) = (Arc::clone(&seen), Arc::clone(&released));
        orch.set_admission_hook(Arc::new(
            move |source: &str| -> BoxFuture<'static, AdmissionGuard> {
                s.lock().unwrap().push(source.to_string());
                let r = Arc::clone(&r);
                Box::pin(async move { Box::new(Place(r)) as AdmissionGuard })
            },
        ));
        let place = orch.admit_background("self_improvement").await;
        assert!(place.is_some());
        assert_eq!(*seen.lock().unwrap(), vec!["self_improvement".to_string()]);
        assert_eq!(
            released.load(Ordering::SeqCst),
            0,
            "held while the turn runs"
        );
        drop(place);
        assert_eq!(released.load(Ordering::SeqCst), 1, "given back afterwards");

        // A second install is ignored: the first hook stays.
        orch.set_admission_hook(Arc::new(|_: &str| -> BoxFuture<'static, AdmissionGuard> {
            Box::pin(async { Box::new(()) as AdmissionGuard })
        }));
        drop(orch.admit_background("product").await);
        assert_eq!(seen.lock().unwrap().len(), 2);
    }
}
