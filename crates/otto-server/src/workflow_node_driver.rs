//! Scoped concurrency between a workflow node and its live-progress monitor.

use std::future::Future;
use tokio::sync::oneshot;

/// Keep polling the node while its monitor persists logs/session associations.
/// The node may itself hold a checkpoint transaction needed by that write.
/// Completion goes through the monitor so an in-flight progress write finishes
/// before the caller publishes final state. Neither future outlives this scope:
/// cancellation/skip or dropping the caller also drops the node and its locks.
pub(crate) async fn drive<N, M, F, R>(node: N, monitor: F) -> R
where
    N: Future<Output = R>,
    M: Future<Output = R>,
    F: FnOnce(oneshot::Receiver<R>) -> M,
{
    let (done, result) = oneshot::channel();
    let node = async {
        let _ = done.send(node.await);
        // The monitor owns completion, including finishing its pending write.
        std::future::pending::<()>().await;
    };
    tokio::select! {
        result = monitor(result) => result,
        () = node => unreachable!("node driver stays alive until its monitor finishes"),
    }
}

#[cfg(test)]
#[path = "../tests/unit/workflow_node_driver.rs"]
mod tests;
