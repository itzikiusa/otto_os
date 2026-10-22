//! The API client's pure REST helpers (curl parsing, OpenAPI export, JSON
//! path, assertion evaluation) now live in [`otto_apiclient::rest`]; this
//! re-export keeps every `crate::api_helpers::…` path working
//! (architecture-1-07). Only the server-side settle helper stays here.

pub use otto_apiclient::rest::*;

/// Poll `settled` (every 100 ms, at most ~15 s) until it reports true — a
/// handler that just stopped a background run waits for the run to wind down
/// (kill its session, cancel its workflow, settle its row) before deleting
/// what the run still references. Returns whether it settled in time.
pub(crate) async fn wait_until_settled<F, Fut>(mut settled: F) -> bool
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = bool>,
{
    for _ in 0..150 {
        if settled().await {
            return true;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    settled().await
}
