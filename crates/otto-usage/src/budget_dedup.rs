//! Budget-exceeded de-duplication logic extracted from the monitor so it can be
//! unit-tested without a running daemon context.
//!
//! The sampler in `otto-server::monitor::spawn_budget_sampler` uses
//! [`BudgetDedup`] to ensure that a crossing emits exactly one "exceeded"
//! event per window (and exactly one "recovered" event when spend drops back),
//! rather than re-firing on every metrics tick.

use std::collections::HashSet;

/// The result of one budget-check for a single `(scope, key)`.
#[derive(Debug, PartialEq, Eq)]
pub enum BudgetSignal {
    /// First time this `(scope, key)` has been seen above its cap — emit once.
    Exceeded,
    /// Was alerted above the cap but has now dropped back — emit once.
    Recovered,
    /// No state change (still below cap, or still above cap but already alerted).
    NoChange,
}

/// In-memory de-duplication for budget crossing events.
///
/// Keeps the set of `"scope:key"` identifiers that have already emitted an
/// "exceeded" signal. Call [`apply`](BudgetDedup::apply) for each budget row
/// on every sample tick; emit an event when it returns `Exceeded` or
/// `Recovered`.
///
/// A daemon restart resets the set, which is harmless — re-alerting after a
/// restart is a minor UX inconvenience, not a correctness problem.
#[derive(Default)]
pub struct BudgetDedup {
    alerted: HashSet<String>,
    /// What the last completed check saw (see [`BudgetCheckStamp`]).
    checked: Option<BudgetCheckStamp>,
}

/// Everything a budget check's outcome depends on besides the stored spend:
/// the usage generation (bumped per usage-event write,
/// [`crate::UsageEngine::usage_generation`]), the budget config, and the
/// calendar day (the `today() - N` window rolls daily, so a capped key can
/// recover with no new usage). An unchanged stamp means the previous result
/// still holds, so the sampler skips the spend scan — which would otherwise
/// wake an idle-stopped ClickHouse on every metrics tick (perf3 N1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BudgetCheckStamp {
    usage_gen: u64,
    config: String,
    /// Local and UTC dates — the window rolls on the server's `today()`.
    days: (chrono::NaiveDate, chrono::NaiveDate),
}

impl BudgetCheckStamp {
    /// Stamp for a check now. `config` is any stable rendering of the budget
    /// config (its JSON), so an edited cap or window re-checks immediately.
    pub fn now(usage_gen: u64, config: String) -> Self {
        Self::at(usage_gen, config, chrono::Utc::now())
    }

    fn at(usage_gen: u64, config: String, now: chrono::DateTime<chrono::Utc>) -> Self {
        Self {
            usage_gen,
            config,
            days: (
                now.with_timezone(&chrono::Local).date_naive(),
                now.date_naive(),
            ),
        }
    }
}

impl BudgetDedup {
    pub fn new() -> Self {
        Self::default()
    }

    /// Apply one budget row's `exceeded` flag.
    ///
    /// - `exceeded = true, not in set` → insert + return `Exceeded`.
    /// - `exceeded = true, already in set` → return `NoChange`.
    /// - `exceeded = false, in set` → remove + return `Recovered`.
    /// - `exceeded = false, not in set` → return `NoChange`.
    pub fn apply(&mut self, scope: &str, key: &str, exceeded: bool) -> BudgetSignal {
        let entry = format!("{scope}:{key}");
        if exceeded {
            if self.alerted.insert(entry) {
                BudgetSignal::Exceeded
            } else {
                BudgetSignal::NoChange
            }
        } else if self.alerted.remove(&entry) {
            BudgetSignal::Recovered
        } else {
            BudgetSignal::NoChange
        }
    }

    /// True if the given `(scope, key)` pair is currently alerted.
    #[cfg(test)]
    pub fn is_alerted(&self, scope: &str, key: &str) -> bool {
        self.alerted.contains(&format!("{scope}:{key}"))
    }

    /// Clear all alerts (called when enforcement is turned off mid-session).
    pub fn clear(&mut self) {
        self.alerted.clear();
        self.checked = None;
    }

    /// True when a check with `stamp` would only repeat the last one: no
    /// usage written, same config, same day. Take the stamp BEFORE the
    /// check, so usage landing mid-check re-checks on the next tick.
    pub fn unchanged_since_last_check(&self, stamp: &BudgetCheckStamp) -> bool {
        self.checked.as_ref() == Some(stamp)
    }

    /// Record that a check with `stamp` completed.
    pub fn mark_checked(&mut self, stamp: BudgetCheckStamp) {
        self.checked = Some(stamp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// perf3 N1: the sampler re-checks only when usage was written, the
    /// config changed, or the day rolled — otherwise every metrics tick ran
    /// a spend scan that woke an idle-stopped ClickHouse.
    #[test]
    fn budget_check_skips_until_usage_config_or_day_changes() {
        let t0 = chrono::Utc::now();
        let mut dd = BudgetDedup::new();
        let first = BudgetCheckStamp::at(3, "{cap:10}".into(), t0);
        assert!(!dd.unchanged_since_last_check(&first), "first check runs");
        dd.mark_checked(first.clone());
        let tick = BudgetCheckStamp::at(3, "{cap:10}".into(), t0 + chrono::Duration::seconds(60));
        assert!(dd.unchanged_since_last_check(&tick), "idle tick skips");
        assert!(!dd.unchanged_since_last_check(&BudgetCheckStamp::at(4, "{cap:10}".into(), t0)));
        assert!(!dd.unchanged_since_last_check(&BudgetCheckStamp::at(3, "{cap:5}".into(), t0)));
        let tomorrow = BudgetCheckStamp::at(3, "{cap:10}".into(), t0 + chrono::Duration::days(1));
        assert!(!dd.unchanged_since_last_check(&tomorrow), "window rolled");
        dd.clear();
        assert!(
            !dd.unchanged_since_last_check(&first),
            "re-enable re-checks"
        );
    }

    #[test]
    fn first_crossing_emits_exceeded_once() {
        let mut dd = BudgetDedup::new();
        // First tick above cap → Exceeded.
        assert_eq!(dd.apply("provider", "claude", true), BudgetSignal::Exceeded);
        // Second + third tick still above cap → no repeat.
        assert_eq!(dd.apply("provider", "claude", true), BudgetSignal::NoChange);
        assert_eq!(dd.apply("provider", "claude", true), BudgetSignal::NoChange);
        assert!(dd.is_alerted("provider", "claude"));
    }

    #[test]
    fn recovery_emits_once_then_stops() {
        let mut dd = BudgetDedup::new();
        // Cross the cap.
        assert_eq!(dd.apply("workspace", "ws1", true), BudgetSignal::Exceeded);
        // Drop back below cap → single Recovered.
        assert_eq!(dd.apply("workspace", "ws1", false), BudgetSignal::Recovered);
        // Stay below cap → no repeat.
        assert_eq!(dd.apply("workspace", "ws1", false), BudgetSignal::NoChange);
        assert!(!dd.is_alerted("workspace", "ws1"));
    }

    #[test]
    fn re_crossing_after_recovery_emits_again() {
        let mut dd = BudgetDedup::new();
        dd.apply("provider", "codex", true); // exceeded
        dd.apply("provider", "codex", false); // recovered
                                              // Re-cross — should emit Exceeded again (set was cleared on recovery).
        assert_eq!(dd.apply("provider", "codex", true), BudgetSignal::Exceeded);
    }

    #[test]
    fn independent_keys_do_not_interfere() {
        let mut dd = BudgetDedup::new();
        assert_eq!(dd.apply("provider", "claude", true), BudgetSignal::Exceeded);
        // Different key — independent state.
        assert_eq!(dd.apply("provider", "codex", true), BudgetSignal::Exceeded);
        // First key — already alerted.
        assert_eq!(dd.apply("provider", "claude", true), BudgetSignal::NoChange);
        // Workspace scope is a separate namespace.
        assert_eq!(dd.apply("workspace", "ws1", true), BudgetSignal::Exceeded);
    }

    #[test]
    fn no_crossing_never_emits() {
        let mut dd = BudgetDedup::new();
        for _ in 0..5 {
            assert_eq!(
                dd.apply("provider", "claude", false),
                BudgetSignal::NoChange
            );
        }
    }

    #[test]
    fn clear_resets_all_alerts() {
        let mut dd = BudgetDedup::new();
        dd.apply("provider", "claude", true);
        dd.apply("workspace", "ws1", true);
        dd.clear();
        assert!(!dd.is_alerted("provider", "claude"));
        assert!(!dd.is_alerted("workspace", "ws1"));
        // After clear, a previously-exceeded key is eligible for Exceeded again.
        assert_eq!(dd.apply("provider", "claude", true), BudgetSignal::Exceeded);
    }
}
