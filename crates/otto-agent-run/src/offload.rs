//! Run blocking filesystem / CPU work off the tokio workers.
//!
//! Transcript watchers used to read and parse whole JSONL files inside `async
//! fn`s — 40–70 ms synchronous chunks on a runtime worker every second per
//! running agent, which is exactly what makes terminal keystrokes and HTTP
//! handlers scheduled on that worker lag. Anything that touches a transcript
//! (read, parse, fold, directory walk) goes through [`blocking`].

/// `spawn_blocking(f).await`, with a panic in `f` re-raised on the caller (as
/// if `f` had run inline) instead of surfacing as a `JoinError` every call
/// site would have to invent handling for.
pub async fn blocking<T, F>(f: F) -> T
where
    F: FnOnce() -> T + Send + 'static,
    T: Send + 'static,
{
    match otto_telemetry::context::measure("server.blocking", tokio::task::spawn_blocking(f)).await
    {
        Ok(v) => v,
        Err(e) => match e.try_into_panic() {
            Ok(payload) => std::panic::resume_unwind(payload),
            // Only on runtime shutdown — nothing is waiting for the answer.
            Err(e) => panic!("blocking task cancelled: {e}"),
        },
    }
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    async fn returns_the_value_and_reraises_panics() {
        assert_eq!(super::blocking(|| 2 + 2).await, 4);
        let caught =
            tokio::spawn(async { super::blocking(|| -> u8 { panic!("boom") }).await }).await;
        assert!(caught.unwrap_err().is_panic());
    }
}
