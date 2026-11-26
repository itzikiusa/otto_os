//! Shared bounded pipe capture for scheduled and verification commands.

/// Keep a bounded preview while continuing to drain each pipe. Discarding the
/// excess is essential: a full stderr pipe must not stall a stdout-heavy child.
pub(crate) const STREAM_BYTES: usize = 512 * 1024;
pub(crate) async fn drain(
    mut stream: impl tokio::io::AsyncRead + Unpin,
) -> std::io::Result<Vec<u8>> {
    use tokio::io::AsyncReadExt;
    let mut kept = Vec::new();
    let mut omitted = 0u64;
    let mut chunk = [0u8; 8192];
    loop {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        let take = n.min(STREAM_BYTES - kept.len());
        kept.extend_from_slice(&chunk[..take]);
        omitted = omitted.saturating_add((n - take) as u64);
    }
    if omitted > 0 {
        kept.extend_from_slice(format!("\n[{} bytes omitted]\n", omitted).as_bytes());
    }
    Ok(kept)
}

/// Like [`drain`] but keeps the NEWEST `cap` bytes (a failing check's
/// cause is at the end), and appends every byte to `log` when given.
/// `stop` resolving ends the read with what arrived so far.
pub(crate) async fn drain_tail(
    mut stream: impl tokio::io::AsyncRead + Unpin,
    cap: usize,
    log: Option<&std::sync::Mutex<std::fs::File>>,
    stop: impl std::future::Future<Output = ()>,
) -> std::io::Result<Vec<u8>> {
    use std::io::Write;
    use tokio::io::AsyncReadExt;
    let mut kept = Vec::new();
    let mut omitted = 0usize;
    let mut chunk = [0u8; 8192];
    tokio::pin!(stop);
    loop {
        let n = tokio::select! {
            biased;
            n = stream.read(&mut chunk) => n?,
            _ = &mut stop => break,
        };
        if n == 0 {
            break;
        }
        if let Some(log) = log {
            log.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .write_all(&chunk[..n])?;
        }
        kept.extend_from_slice(&chunk[..n]);
        // Compact only past twice the cap, so the copy stays amortized O(n).
        if kept.len() > cap.saturating_mul(2) {
            let excess = kept.len() - cap;
            kept.drain(..excess);
            omitted += excess;
        }
    }
    if kept.len() > cap {
        let excess = kept.len() - cap;
        kept.drain(..excess);
        omitted += excess;
    }
    if omitted > 0 {
        let mut out = format!("[{omitted} bytes omitted]\n").into_bytes();
        out.extend_from_slice(&kept);
        kept = out;
    }
    Ok(kept)
}
