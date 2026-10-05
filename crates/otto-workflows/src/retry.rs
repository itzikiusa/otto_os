//! Step-retry policy: classify a failure into a retry class and compute the
//! next backoff. Pure — moved out of otto-server's `workflow_engine`.

/// Classify a step error into a retry CLASS (and its human label). These are the
/// failures a longer pause actually cures — the provider is overloaded or the
/// daemon is out of file descriptors — as opposed to a bad prompt, which no
/// amount of waiting fixes. Case-insensitive substring match on the error text
/// (the provider strings are not structured). R5.2.
pub fn retry_class(err: &str) -> Option<&'static str> {
    let e = err.to_ascii_lowercase();
    if e.contains("529") {
        return Some("provider overloaded: 529");
    }
    if e.contains("overloaded") {
        return Some("provider overloaded");
    }
    if e.contains("rate limit") {
        return Some("rate limit");
    }
    if e.contains("too many open files") || e.contains("dup of fd") {
        return Some("fd exhaustion");
    }
    if e.contains("spawn") {
        return Some("spawn failure");
    }
    None
}

/// The next retry for a failed attempt: `(sleep_ms, effective_max_attempts,
/// reason)`, or `None` when the budget is spent. A classified failure gets a
/// ≥ 20 s jittered pause and a floor of 4 attempts; everything else keeps the
/// node's own policy verbatim. R5.2.
pub fn retry_backoff(
    err: &str,
    policy: &otto_core::workflows::RetryPolicy,
    attempt: u32,
    cur_backoff_ms: u64,
    jitter_ms: u64,
) -> Option<(u64, u32, String)> {
    // An explicit zero is a hard no-retry policy, including transient errors.
    if policy.max_attempts == 0 {
        return None;
    }
    let (sleep_ms, max_eff, reason) = match retry_class(err) {
        Some(label) => (
            cur_backoff_ms.max(20_000) + jitter_ms,
            policy.max_attempts.max(4),
            label.to_string(),
        ),
        None => (cur_backoff_ms, policy.max_attempts, truncate(err, 120)),
    };
    (attempt <= max_eff).then_some((sleep_ms, max_eff, reason))
}

fn truncate(s: &str, max: usize) -> String {
    if s.len() <= max {
        return s.to_string();
    }
    let mut end = max;
    while end > 0 && !s.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &s[..end])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn retry_backoff_by_error_class() {
        let policy = otto_core::workflows::RetryPolicy {
            max_attempts: 2,
            backoff_ms: 2000,
            factor: 2.0,
        };
        // 529 / overload / fd exhaustion: ≥ 20s and a 4-attempt floor.
        for (err, label) in [
            (
                "agent error: API Error 529 Overloaded",
                "provider overloaded: 529",
            ),
            ("upstream overloaded, try later", "provider overloaded"),
            ("Rate limit reached for model", "rate limit"),
            ("os error 24: Too many open files", "fd exhaustion"),
            ("dup of fd 12 failed", "fd exhaustion"),
            ("failed to spawn claude", "spawn failure"),
        ] {
            let (sleep, max_eff, reason) =
                retry_backoff(err, &policy, 1, policy.backoff_ms, 3_000).expect("retryable");
            assert!(sleep >= 20_000, "{err}: {sleep}");
            assert_eq!(max_eff, 4, "{err}");
            assert_eq!(reason, label);
        }
        // Everything else keeps the node's own policy verbatim.
        let (sleep, max_eff, reason) =
            retry_backoff("empty prompt", &policy, 1, policy.backoff_ms, 3_000).expect("retryable");
        assert_eq!(sleep, 2000);
        assert_eq!(max_eff, 2);
        assert_eq!(reason, "empty prompt");
        // Budget spent → no retry (the classified floor still applies first).
        assert!(retry_backoff("empty prompt", &policy, 3, 2000, 0).is_none());
        assert!(retry_backoff("529 overloaded", &policy, 4, 2000, 0).is_some());
        assert!(retry_backoff("529 overloaded", &policy, 5, 2000, 0).is_none());
        // The reason is bounded so one huge provider error can't flood the log.
        let long = "x".repeat(500);
        let (_, _, reason) = retry_backoff(&long, &policy, 1, 1, 0).expect("retryable");
        assert!(reason.chars().count() <= 121, "{}", reason.chars().count());
    }
}
