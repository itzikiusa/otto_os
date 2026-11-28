//! Kotest runs: one spec, container or test through Gradle or Maven, addressed with Kotest's
//! own `pkg.Spec.a -- b -- c` pattern, and the check that the chosen test is what ran.

use std::collections::{BTreeMap, HashSet};
use std::sync::Mutex;

use crate::gutter::kotest::kotest_tree;
use crate::types::{IdeRunConfig, IdeTargetKind, IdeTestCaseResult, IdeTestCaseStatus};

/// `pkg.Spec`, or `pkg.Spec.a -- b -- c` for a container or test inside it.
pub(crate) fn kotest_pattern(cfg: &IdeRunConfig) -> String {
    let fqcn = cfg.test_filter.as_deref().unwrap_or_default().trim();
    if cfg.test_path.is_empty() {
        fqcn.to_string()
    } else {
        format!("{fqcn}.{}", cfg.test_path.join(" -- "))
    }
}

/// The selection for Kotest's engine (Maven's `-Dtest` only picks the class; Gradle also gets
/// it as `--tests`) and the override that runs a disabled test the person clicked.
pub(crate) fn kotest_env(cfg: &IdeRunConfig) -> [(String, String); 2] {
    [
        ("KOTEST_INCLUDE_PATTERN".into(), kotest_pattern(cfg)),
        ("KOTEST_TEST_ENABLED_OVERRIDE".into(), "true".into()),
    ]
}

/// `test --tests <pattern> --rerun`: the pattern is a task input, the environment is not, so
/// `--rerun` keeps an unchanged build from skipping the tests.
pub(crate) fn kotest_gradle_args(cfg: &IdeRunConfig, mut args: Vec<String>) -> Vec<String> {
    if args.is_empty() {
        args.push("test".into());
    }
    args.extend(["--tests".into(), kotest_pattern(cfg), "--rerun".into()]);
    args
}

/// The spec's simple class name, Surefire's `-Dtest` (its default includes skip `*Spec`).
pub(crate) fn kotest_maven_filter(cfg: &IdeRunConfig) -> String {
    let fqcn = cfg.test_filter.as_deref().unwrap_or_default().trim();
    fqcn.rsplit('.').next().unwrap_or(fqcn).to_string()
}

/// Modules where Kotest ran the siblings of a chosen test: their gutter offers spec items
/// only, until the daemon restarts.
static SPEC_ONLY_MODULES: Mutex<Option<HashSet<String>>> = Mutex::new(None);

pub(crate) fn spec_only_module(module: &str) -> bool {
    SPEC_ONLY_MODULES
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .as_ref()
        .is_some_and(|s| s.contains(module))
}

/// Whether a reported case is the test at `path`: Surefire names it `a/b/c`, Gradle by its
/// display name (`c`, `It: c`, `Then: c`, `should c`, …).
fn names_test(case: &str, path: &[String]) -> bool {
    const PREFIXES: [&str; 6] = [
        "It: ",
        "Then: ",
        "Scenario: ",
        "Expect: ",
        "should ",
        "Test: ",
    ];
    let leaf = path.last().map_or("", String::as_str);
    let shown = PREFIXES
        .iter()
        .find_map(|p| case.strip_prefix(p))
        .unwrap_or(case);
    case == path.join("/") || case == leaf || shown == leaf
}

/// After a Kotest run of `cfg.test_path` from `module`: drop the siblings Kotest reports as
/// skipped (they did not run) and say when the selection did not hold. `src` is the spec's
/// source, which tells a chosen test from a container (whose children's names vary).
pub(crate) fn check_kotest_results(
    cfg: &IdeRunConfig,
    module: &str,
    src: Option<&str>,
    results: &mut Vec<IdeTestCaseResult>,
) -> Option<String> {
    let path = &cfg.test_path;
    if path.is_empty() {
        return None;
    }
    results.retain(|r| r.status != IdeTestCaseStatus::Skipped || names_test(&r.name, path));
    let leaf = src.is_some_and(|s| {
        kotest_tree(s)
            .iter()
            .any(|n| &n.path == path && !n.container)
    });
    let matching = results.iter().filter(|r| names_test(&r.name, path)).count();
    if results.is_empty() || (leaf && matching == 0) {
        return Some(format!(
            "Kotest ran no test named '{}'. Single-test runs need Kotest 6.1+; check the test name and version.",
            path.join(" -- ")
        ));
    }
    if leaf && results.len() > matching {
        SPEC_ONLY_MODULES
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get_or_insert_with(HashSet::new)
            .insert(module.to_string());
        return Some(format!(
            "Kotest ran other tests too ({}). This Kotest version does not support selecting nested tests.",
            results.len() - matching
        ));
    }
    None
}

/// The port Gradle's `--debug-jvm` test JVM listens on.
pub(crate) const GRADLE_DEBUG_PORT: u16 = 5005;

/// A Kotest Debug: the run's command plus the build tool's own debug switch, and the port
/// its suspended test JVM will listen on.
pub(crate) struct KotestDebugCommand {
    pub program: String,
    pub args: Vec<String>,
    pub port: u16,
    /// "Gradle", "Maven", or "Gradle or Maven" on SSH (the remote shell picks).
    pub tool: &'static str,
}

/// `maven_port` is where Surefire's forked JVM listens (`127.0.0.1` only); Gradle's is fixed.
pub(crate) fn kotest_debug_command(
    cfg: &IdeRunConfig,
    target_kind: IdeTargetKind,
    cwd: &str,
    env: &BTreeMap<String, String>,
    maven_port: u16,
) -> KotestDebugCommand {
    let mut g_args = kotest_gradle_args(cfg, Vec::new());
    g_args.push("--debug-jvm".into());
    let m_args = vec![
        "test".into(),
        format!(
            "-Dmaven.surefire.debug=-agentlib:jdwp=transport=dt_socket,server=y,suspend=y,address=127.0.0.1:{maven_port}"
        ),
    ];
    let filter = kotest_maven_filter(cfg);
    let gradle = super::local_gradle_only(target_kind, cwd);
    let (program, args) = super::jvm_test_cmd(
        cfg,
        target_kind,
        cwd,
        None,
        (g_args, None),
        (m_args, Some(&filter)),
        Vec::new(),
    );
    let program = if target_kind == IdeTargetKind::Local {
        super::resolve_program_in_env(&program, env)
            .to_string_lossy()
            .into_owned()
    } else {
        program
    };
    let (port, tool) = match (target_kind, gradle) {
        (IdeTargetKind::Local, true) => (GRADLE_DEBUG_PORT, "Gradle"),
        (IdeTargetKind::Local, false) => (maven_port, "Maven"),
        _ => (GRADLE_DEBUG_PORT, "Gradle or Maven"),
    };
    KotestDebugCommand {
        program,
        args,
        port,
        tool,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn case(name: &str, status: IdeTestCaseStatus) -> IdeTestCaseResult {
        IdeTestCaseResult {
            name: name.into(),
            suite: Some("com.acme.LoginSpec".into()),
            status,
            duration_ms: None,
            file: None,
            line: None,
            message: None,
        }
    }

    fn cfg(path: &[&str]) -> IdeRunConfig {
        IdeRunConfig {
            test_filter: Some("com.acme.LoginSpec".into()),
            test_path: path.iter().map(|s| s.to_string()).collect(),
            ..IdeRunConfig::default()
        }
    }

    const SRC: &str = r#"class LoginSpec : DescribeSpec({
    describe("a user") {
        it("sees dashboard") { }
        it("sees failing sibling") { }
    }
})"#;

    #[test]
    fn pattern_args_and_env_address_the_chosen_test() {
        let c = cfg(&["a user", "sees dashboard"]);
        assert_eq!(
            kotest_pattern(&c),
            "com.acme.LoginSpec.a user -- sees dashboard"
        );
        assert_eq!(
            kotest_gradle_args(&c, Vec::new()),
            [
                "test",
                "--tests",
                "com.acme.LoginSpec.a user -- sees dashboard",
                "--rerun"
            ]
        );
        assert_eq!(kotest_maven_filter(&c), "LoginSpec");
        assert_eq!(
            kotest_env(&c)[0].1,
            "com.acme.LoginSpec.a user -- sees dashboard"
        );
        assert_eq!(kotest_pattern(&cfg(&[])), "com.acme.LoginSpec");
    }

    #[test]
    fn a_gradle_or_maven_leaf_run_keeps_only_the_chosen_test() {
        // Gradle's XML names the leaf by its display name, Surefire by its path.
        for name in [
            "It: sees dashboard",
            "a user/sees dashboard",
            "should sees dashboard",
        ] {
            let mut r = vec![
                case(name, IdeTestCaseStatus::Passed),
                case("It: sees failing sibling", IdeTestCaseStatus::Skipped),
            ];
            let c = cfg(&["a user", "sees dashboard"]);
            assert_eq!(check_kotest_results(&c, "/m1", Some(SRC), &mut r), None);
            assert_eq!(r.len(), 1, "{name}");
        }
        assert!(!spec_only_module("/m1"));
    }

    #[test]
    fn nothing_ran_or_siblings_ran_are_reported() {
        let c = cfg(&["a user", "sees dashboard"]);
        let mut none = vec![case("It: sees dashboard", IdeTestCaseStatus::Skipped)];
        let mut none = none.drain(..).filter(|_| false).collect::<Vec<_>>();
        let msg = check_kotest_results(&c, "/m2", Some(SRC), &mut none).unwrap();
        assert!(
            msg.starts_with("Kotest ran no test named 'a user -- sees dashboard'"),
            "{msg}"
        );

        let mut all = vec![
            case("It: sees dashboard", IdeTestCaseStatus::Passed),
            case("It: sees failing sibling", IdeTestCaseStatus::Failed),
        ];
        let msg = check_kotest_results(&c, "/m3", Some(SRC), &mut all).unwrap();
        assert_eq!(
            msg,
            "Kotest ran other tests too (1). This Kotest version does not support selecting nested tests."
        );
        assert!(spec_only_module("/m3"));
    }

    #[test]
    fn a_container_run_may_run_all_its_children() {
        let c = cfg(&["a user"]);
        let mut r = vec![
            case("It: sees dashboard", IdeTestCaseStatus::Passed),
            case("It: sees failing sibling", IdeTestCaseStatus::Failed),
        ];
        assert_eq!(check_kotest_results(&c, "/m4", Some(SRC), &mut r), None);
        assert_eq!(r.len(), 2);
    }
}
