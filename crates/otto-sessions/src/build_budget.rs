//! Per-session build budget: parallelism hints in the agent's environment so
//! the `cargo` / `go test` / `make` an agent (or one of its CLI's own helper
//! sub-agents — they inherit the env) runs no longer assumes it owns every
//! core. Six reviewers each running `cargo test` with 8 jobs on an 8-core
//! machine is a load of 48 before anything else happens.
//!
//! Hints only: a value the user already has — in the daemon's environment or
//! in the session's own env (account, network profile, context hook) — is
//! never replaced, and the user's shell profile (which the CLIs' shell tools
//! source) still wins over anything set here.
//!
//! Setting `agent_build_parallelism`: absent / `0` = auto ([`auto_budget`]),
//! a number = that many workers, `"off"` = set nothing.

use serde_json::Value;

/// Setting key (see the module docs).
pub const BUILD_PARALLELISM_SETTING: &str = "agent_build_parallelism";

/// Variables set to the plain worker count, each read by the named tool:
/// - `CARGO_BUILD_JOBS` — cargo's `build.jobs` (cargo reference, "Environment
///   variables": every config key has a `CARGO_*` env form).
/// - `RUST_TEST_THREADS` — libtest's test-thread count (`cargo test`).
/// - `NEXTEST_TEST_THREADS` — cargo-nextest's `--test-threads`.
/// - `GOMAXPROCS` — the Go runtime's OS-thread cap (`go` tool, `go test`
///   binaries, gopls).
/// - `CMAKE_BUILD_PARALLEL_LEVEL` — `cmake --build` (CMake ≥ 3.12; also the
///   `cmake` crate behind `rdkafka-sys`).
///
/// Deliberately NOT set: `npm_config_jobs` (npm ≥ 7 has no `jobs` config),
/// `JEST_WORKERS` (Jest reads only `--maxWorkers` / its config),
/// `VITEST_MAX_THREADS` (dropped with Vitest's pool rewrite), and
/// `UV_THREADPOOL_SIZE` (libuv I/O threads, not CPU parallelism).
pub const PLAIN_COUNT_VARS: [&str; 5] = [
    "CARGO_BUILD_JOBS",
    "RUST_TEST_THREADS",
    "NEXTEST_TEST_THREADS",
    "GOMAXPROCS",
    "CMAKE_BUILD_PARALLEL_LEVEL",
];

/// What the setting asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BudgetMode {
    /// Derive from cores and live agents ([`auto_budget`]).
    Auto,
    /// A fixed worker count (≥ 1).
    Fixed(usize),
    /// Set no variable at all.
    Off,
}

/// Parse the `agent_build_parallelism` value. Unknown shapes fall back to
/// auto, so a typo never silently removes the budget.
pub fn budget_mode(value: Option<&Value>) -> BudgetMode {
    match value {
        Some(Value::String(s)) if s.trim().eq_ignore_ascii_case("off") => BudgetMode::Off,
        Some(Value::String(s)) => match s.trim().parse::<usize>() {
            Ok(n) if n > 0 => BudgetMode::Fixed(n),
            _ => BudgetMode::Auto,
        },
        Some(Value::Bool(false)) => BudgetMode::Off,
        Some(v) => match v.as_u64() {
            Some(n) if n > 0 => BudgetMode::Fixed(n as usize),
            _ => BudgetMode::Auto,
        },
        None => BudgetMode::Auto,
    }
}

/// Auto budget: the machine's cores shared by the live agent sessions
/// (including the one being spawned), never below 2 workers (a build with
/// one job is slower than it is kind) and never above `cores - 1` (one core
/// stays for the desktop app and the daemon). The share is taken at spawn
/// time; a session keeps it for its life.
pub fn auto_budget(cores: usize, live_agents: usize) -> usize {
    let cores = cores.max(1);
    let share = cores / live_agents.max(1);
    share.clamp(2, cores.saturating_sub(1).max(2))
}

/// The worker count for `mode`, or `None` for [`BudgetMode::Off`].
pub fn budget_for(mode: BudgetMode, cores: usize, live_agents: usize) -> Option<usize> {
    match mode {
        BudgetMode::Off => None,
        BudgetMode::Fixed(n) => Some(n.max(1)),
        BudgetMode::Auto => Some(auto_budget(cores, live_agents)),
    }
}

/// The env entries to ADD for a budget of `n` workers. `existing` answers
/// "is this variable already set, and to what" — the daemon's environment
/// merged with the session's own `spec.env`. A set variable is left alone;
/// `GOFLAGS` / `MAKEFLAGS` get `-p=n` / `-jn` appended to an existing value
/// unless it already sets parallelism (or, for make, a jobserver is in play).
pub fn budget_env(n: usize, existing: impl Fn(&str) -> Option<String>) -> Vec<(String, String)> {
    let mut out = Vec::new();
    for key in PLAIN_COUNT_VARS {
        if existing(key).is_none() {
            out.push((key.to_string(), n.to_string()));
        }
    }
    // `go build` / `go test` package parallelism (`-p`), read from GOFLAGS.
    match existing("GOFLAGS") {
        None => out.push(("GOFLAGS".into(), format!("-p={n}"))),
        Some(v) if !go_sets_parallelism(&v) => {
            out.push(("GOFLAGS".into(), join_flags(&v, &format!("-p={n}"))))
        }
        Some(_) => {}
    }
    // GNU make reads MAKEFLAGS on start-up.
    match existing("MAKEFLAGS") {
        None => out.push(("MAKEFLAGS".into(), format!("-j{n}"))),
        Some(v) if !make_sets_jobs(&v) => {
            out.push(("MAKEFLAGS".into(), join_flags(&v, &format!("-j{n}"))))
        }
        Some(_) => {}
    }
    out
}

fn join_flags(existing: &str, add: &str) -> String {
    let existing = existing.trim();
    if existing.is_empty() {
        add.to_string()
    } else {
        format!("{existing} {add}")
    }
}

/// `-p=4`, `-p 4` or `--p=4` already in GOFLAGS.
fn go_sets_parallelism(flags: &str) -> bool {
    flags.split_whitespace().any(|t| {
        let t = t.trim_start_matches('-');
        t == "p" || t.starts_with("p=")
    })
}

/// `-j`, `-j4`, `--jobs…`, a single-letter cluster holding `j` (`-kj`, or
/// make's own leading `kj` word) or a jobserver hand-off already in MAKEFLAGS.
fn make_sets_jobs(flags: &str) -> bool {
    flags.split_whitespace().enumerate().any(|(i, t)| {
        if t.starts_with("--jobs") || t.contains("jobserver") {
            return true;
        }
        let cluster = match t.strip_prefix('-') {
            Some(rest) if !rest.starts_with('-') => rest,
            // make writes its single-letter flags as a bare first word.
            None if i == 0 && !t.contains('=') => t,
            _ => return false,
        };
        cluster
            .chars()
            .take_while(|c| c.is_ascii_alphabetic())
            .any(|c| c == 'j')
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::collections::HashMap;

    fn env_of(pairs: &[(&str, &str)]) -> impl Fn(&str) -> Option<String> {
        let map: HashMap<String, String> = pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect();
        move |k: &str| map.get(k).cloned()
    }

    fn get<'a>(out: &'a [(String, String)], key: &str) -> Option<&'a str> {
        out.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }

    #[test]
    fn auto_budget_shares_the_cores_between_live_agents() {
        // Alone on 8 cores: all but one.
        assert_eq!(auto_budget(8, 1), 7);
        assert_eq!(auto_budget(8, 2), 4);
        assert_eq!(auto_budget(8, 4), 2);
        // Never below 2, however crowded.
        assert_eq!(auto_budget(8, 12), 2);
        assert_eq!(auto_budget(8, 0), 7);
        // Tiny machines still get 2.
        assert_eq!(auto_budget(1, 1), 2);
        assert_eq!(auto_budget(2, 1), 2);
        assert_eq!(auto_budget(16, 3), 5);
    }

    #[test]
    fn the_setting_parses_auto_fixed_and_off() {
        assert_eq!(budget_mode(None), BudgetMode::Auto);
        assert_eq!(budget_mode(Some(&json!(0))), BudgetMode::Auto);
        assert_eq!(budget_mode(Some(&json!(3))), BudgetMode::Fixed(3));
        assert_eq!(budget_mode(Some(&json!("6"))), BudgetMode::Fixed(6));
        assert_eq!(budget_mode(Some(&json!("off"))), BudgetMode::Off);
        assert_eq!(budget_mode(Some(&json!("OFF "))), BudgetMode::Off);
        assert_eq!(budget_mode(Some(&json!(false))), BudgetMode::Off);
        assert_eq!(budget_mode(Some(&json!("lots"))), BudgetMode::Auto);
        assert_eq!(budget_mode(Some(&json!(-2))), BudgetMode::Auto);
        assert_eq!(budget_for(BudgetMode::Off, 8, 1), None);
        assert_eq!(budget_for(BudgetMode::Fixed(3), 8, 1), Some(3));
        assert_eq!(budget_for(BudgetMode::Auto, 8, 4), Some(2));
    }

    #[test]
    fn an_empty_environment_gets_every_hint() {
        let out = budget_env(3, env_of(&[]));
        for key in PLAIN_COUNT_VARS {
            assert_eq!(get(&out, key), Some("3"), "{key}");
        }
        assert_eq!(get(&out, "GOFLAGS"), Some("-p=3"));
        assert_eq!(get(&out, "MAKEFLAGS"), Some("-j3"));
    }

    #[test]
    fn a_value_the_user_already_set_is_never_clobbered() {
        let out = budget_env(
            2,
            env_of(&[
                ("CARGO_BUILD_JOBS", "16"),
                ("GOMAXPROCS", "1"),
                ("GOFLAGS", "-p=8 -mod=mod"),
                ("MAKEFLAGS", "-j12"),
            ]),
        );
        assert_eq!(get(&out, "CARGO_BUILD_JOBS"), None);
        assert_eq!(get(&out, "GOMAXPROCS"), None);
        assert_eq!(get(&out, "GOFLAGS"), None, "already sets -p");
        assert_eq!(get(&out, "MAKEFLAGS"), None, "already sets -j");
        assert_eq!(get(&out, "RUST_TEST_THREADS"), Some("2"));
    }

    #[test]
    fn goflags_and_makeflags_are_appended_to_not_replaced() {
        let out = budget_env(
            4,
            env_of(&[
                ("GOFLAGS", "-mod=vendor"),
                ("MAKEFLAGS", "--no-print-directory"),
            ]),
        );
        assert_eq!(get(&out, "GOFLAGS"), Some("-mod=vendor -p=4"));
        assert_eq!(get(&out, "MAKEFLAGS"), Some("--no-print-directory -j4"));
        // An empty value is treated as "set, nothing in it".
        let out = budget_env(4, env_of(&[("GOFLAGS", " ")]));
        assert_eq!(get(&out, "GOFLAGS"), Some("-p=4"));
    }

    #[test]
    fn existing_parallelism_flags_are_recognised() {
        assert!(go_sets_parallelism("-p=2"));
        assert!(go_sets_parallelism("-mod=mod -p 2"));
        assert!(go_sets_parallelism("--p=2"));
        assert!(!go_sets_parallelism("-mod=mod -race"));
        assert!(!go_sets_parallelism("-pgo=auto"));
        assert!(make_sets_jobs("-j"));
        assert!(make_sets_jobs("-j8"));
        assert!(make_sets_jobs("-kj4"));
        assert!(make_sets_jobs("kj --jobserver-auth=3,4"));
        assert!(make_sets_jobs("--jobs=4"));
        assert!(!make_sets_jobs("-k"));
        assert!(!make_sets_jobs("--no-print-directory"));
        assert!(!make_sets_jobs("w"));
    }
}
