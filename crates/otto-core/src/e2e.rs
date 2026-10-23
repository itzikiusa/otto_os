//! The one "is this daemon running inside the isolated e2e harness?" predicate.
//!
//! `ui/e2e` boots a throwaway daemon with `OTTO_E2E=1` (on a temp data dir +
//! port). Code that would otherwise read the real home (transcripts, trust
//! files, provider configs) asks this instead of re-reading the env var, so
//! every crate agrees on what counts as "set".

/// True when `OTTO_E2E` is `1` or `true` (case-insensitive, trimmed).
pub fn isolated() -> bool {
    std::env::var("OTTO_E2E").is_ok_and(|v| is_truthy(&v))
}

fn is_truthy(v: &str) -> bool {
    let v = v.trim();
    v == "1" || v.eq_ignore_ascii_case("true")
}

#[cfg(test)]
mod tests {
    use super::is_truthy;

    #[test]
    fn only_one_and_true_count_as_isolated() {
        assert!(is_truthy("1"));
        assert!(is_truthy("true"));
        assert!(is_truthy(" TRUE "));
        assert!(!is_truthy(""));
        assert!(!is_truthy("0"));
        assert!(!is_truthy("false"));
        assert!(!is_truthy("yes"));
    }
}
