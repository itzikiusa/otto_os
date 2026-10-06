//! Per-run cancellation flags shared by the daemon's long-running engines
//! (skill evals, swarm runs, product analysis, reviews).
//!
//! A run registers an `Arc<AtomicBool>` under its id; the Stop handler flips
//! it; the run's loop polls it. One alias instead of three identical copies.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

/// `run id → cancel flag`.
pub type CancelRegistry = Arc<Mutex<HashMap<String, Arc<AtomicBool>>>>;

/// An empty registry.
pub fn new_cancel_registry() -> CancelRegistry {
    Arc::new(Mutex::new(HashMap::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn a_flag_registered_by_a_run_is_seen_by_the_canceller() {
        let reg = new_cancel_registry();
        let flag = Arc::new(AtomicBool::new(false));
        reg.lock().unwrap().insert("run".into(), flag.clone());
        reg.lock().unwrap()["run"].store(true, Ordering::SeqCst);
        assert!(flag.load(Ordering::SeqCst));
    }
}
