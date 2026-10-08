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
