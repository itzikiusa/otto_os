//! Benign SSH/git stderr chatter, filtered out of command output (moved out
//! of `local.rs` unchanged).

/// stderr lines that SSH/git emit as benign chatter — never the reason a command
/// failed. We skip these when choosing the message to surface so the real git
/// error (rejected push, auth failure, …) isn't masked. Newer OpenSSH (9.x/10.x)
/// prints the post-quantum warning to stderr on every non-PQ connection and it
/// does NOT affect the exit status — yet it sorts first, so the old "first
/// non-empty line" logic reported it as the failure.
pub(crate) fn is_noise_line(l: &str) -> bool {
    let t = l.trim();
    t.is_empty()
        || t.contains("post-quantum key exchange")
        || t.starts_with("Warning: Permanently added")
}

/// Drop benign SSH/git noise lines from combined command output (used for the
/// success path so a successful push/pull doesn't surface the post-quantum
/// warning).
pub(crate) fn strip_noise(s: &str) -> String {
    s.lines()
        .filter(|l| !is_noise_line(l))
        .collect::<Vec<_>>()
        .join("\n")
        .trim_end()
        .to_string()
}
