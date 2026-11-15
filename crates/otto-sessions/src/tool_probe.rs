//! Launch-readiness probes for `/api/v1/meta` (`MetaResp.tools`): is each
//! agent CLI (and git / clickhouse) on PATH, and which version. Lives beside
//! [`crate::provider_models`]; both are cached so `/meta` — on the app's boot
//! path — stays fast.

use std::collections::BTreeMap;
use std::time::Duration;

use otto_core::api::ToolStatus;
use tokio::process::Command;
use tokio::time::timeout;

/// Cold-cache inline wait budget on `/meta` before serving fast filesystem
/// presence and letting the spawned version probe finish in the background.
const COLD_INLINE_BUDGET: Duration = Duration::from_millis(300);

/// Fast in-process executable check (`is_file` + execute bit on Unix) across
/// explicit paths or `$PATH`, avoiding a `which` subprocess per tool.
pub fn executable_in_path_or_fs(program: &str) -> bool {
    fn is_exec(p: &std::path::Path) -> bool {
        let Ok(meta) = std::fs::metadata(p) else {
            return false;
        };
        if !meta.is_file() {
            return false;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            meta.permissions().mode() & 0o111 != 0
        }
        #[cfg(not(unix))]
        {
            true
        }
    }

    let p = std::path::Path::new(program);
    if program.contains('/') {
        return is_exec(p);
    }
    let Some(path) = std::env::var_os("PATH") else {
        return false;
    };
    std::env::split_paths(&path)
        .map(|dir| dir.join(program))
        .any(|cand| is_exec(&cand))
}

fn rc_content_defines_alias_or_fn(content: &str, program: &str) -> bool {
    if program.is_empty() {
        return false;
    }
    for raw_line in content.lines() {
        let mut line = raw_line.trim_start();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix("alias ") {
            let rest = rest.trim_start();
            for prefix in [
                format!("{program}="),
                format!("'{program}'="),
                format!("\"{program}\"="),
            ] {
                if rest.starts_with(&prefix) {
                    return true;
                }
            }
        }
        if let Some(rest) = line.strip_prefix("function ") {
            line = rest.trim_start();
            if let Some(after) = line.strip_prefix(program) {
                if after.is_empty()
                    || after.starts_with(char::is_whitespace)
                    || after.starts_with('(')
                    || after.starts_with('{')
                {
                    return true;
                }
            }
            continue;
        }
        if let Some(after) = line.strip_prefix(program) {
            let after = after.trim_start();
            if after.starts_with("()") {
                return true;
            }
        }
    }
    false
}

/// Fast (<1 ms) scan of standard shell rc files for an `alias <program>=…` or
/// `<program>()` shell function definition so custom providers like
/// `claudework -> claude-work` resolve immediately during cold `/meta` probes
/// without waiting on an interactive login shell boot.
pub fn rc_defines_alias_or_fn(program: &str) -> bool {
    if program.is_empty() || program.contains('/') {
        return false;
    }
    let mut candidates = Vec::with_capacity(11);
    if let Some(zdot) = std::env::var_os("ZDOTDIR").filter(|v| !v.is_empty()) {
        let zdir = std::path::PathBuf::from(zdot);
        for name in [".zshrc", ".zshenv", ".zprofile"] {
            candidates.push(zdir.join(name));
        }
    }
    if let Some(home) = std::env::var_os("HOME").filter(|v| !v.is_empty()) {
        let hdir = std::path::PathBuf::from(home);
        for name in [
            ".zshrc",
            ".zshenv",
            ".zprofile",
            ".bashrc",
            ".bash_profile",
            ".bash_aliases",
            ".aliases",
            ".profile",
        ] {
            let p = hdir.join(name);
            if !candidates.contains(&p) {
                candidates.push(p);
            }
        }
    }
    for path in candidates {
        let Ok(file) = std::fs::File::open(&path) else {
            continue;
        };
        use std::io::Read;
        let mut buf = String::new();
        if file.take(256 * 1024).read_to_string(&mut buf).is_ok()
            && rc_content_defines_alias_or_fn(&buf, program)
        {
            return true;
        }
    }
    false
}

fn fast_tool_statuses(specs: &BTreeMap<String, String>) -> Vec<ToolStatus> {
    specs
        .iter()
        .filter_map(|(name, program)| {
            let builtin_name = DETECTED_TOOLS.contains(&name.as_str()) && program == name;
            if executable_in_path_or_fs(program)
                || (!builtin_name && !program.contains('/') && rc_defines_alias_or_fn(program))
            {
                Some(ToolStatus {
                    name: name.clone(),
                    found: true,
                    version: None,
                })
            } else if !builtin_name && !program.contains('/') {
                // Bare custom command may be defined in a sourced shell plugin:
                // omit from the provisional vector so pickers keep it enabled
                // (`checked: false`) until `run_tool_probe` finishes.
                None
            } else {
                Some(ToolStatus {
                    name: name.clone(),
                    found: false,
                    version: None,
                })
            }
        })
        .collect()
}
/// Tools probed by name on every `/meta`.
pub const DETECTED_TOOLS: [&str; 5] = ["claude", "codex", "agy", "git", "clickhouse"];

/// The `name → program` map to probe: [`DETECTED_TOOLS`] by name, overridden
/// by each provider's configured executable (`None` = no executable).
pub fn tool_specs<'a>(
    providers: impl IntoIterator<Item = (&'a str, Option<String>)>,
) -> BTreeMap<String, String> {
    let mut specs: BTreeMap<String, String> = DETECTED_TOOLS
        .iter()
        .map(|name| (name.to_string(), name.to_string()))
        .collect();
    for (name, program) in providers {
        if let Some(program) = program {
            configure_provider_probe(&mut specs, name, program);
        }
    }
    specs
}

/// How long a `/meta` tool probe is reused (S8-04).
const TOOLS_TTL: Duration = Duration::from_secs(60);

/// Past [`TOOLS_TTL`] a probe is still SERVED (and refreshed in the
/// background) for this long. `/meta` is on the app's boot path and a cold
/// probe forks `which` + `<tool> --version` for Node-based CLIs (a day of
/// real telemetry: both app launches waited 1.4 s and 2.7 s for `/meta`).
const TOOLS_MAX_STALE: Duration = Duration::from_secs(24 * 3600);

/// The last tool probe: when, for which `name → program` specs, and the result.
type ToolsCache = Option<(
    std::time::Instant,
    BTreeMap<String, String>,
    Vec<ToolStatus>,
)>;

static TOOLS_CACHE: tokio::sync::Mutex<ToolsCache> = tokio::sync::Mutex::const_new(None);
/// One background refresh at a time.
static TOOLS_REFRESHING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// What a cached probe of `age` may be used for.
#[derive(Debug, PartialEq, Eq)]
enum Freshness {
    Fresh,
    /// Serve it, refresh behind the response.
    Stale,
    /// Probe inline.
    Expired,
}

/// A probe that found no agent CLI is never served stale: the first-run
/// coach's "Re-check" after installing one must see it on that click.
fn freshness(age: Duration, tools: &[ToolStatus]) -> Freshness {
    let has_agent = tools
        .iter()
        .any(|t| t.found && t.name != "git" && t.name != "clickhouse");
    if age < TOOLS_TTL {
        Freshness::Fresh
    } else if age < TOOLS_MAX_STALE && has_agent {
        Freshness::Stale
    } else {
        Freshness::Expired
    }
}

async fn run_tool_probe(specs: &BTreeMap<String, String>) -> Vec<ToolStatus> {
    #[cfg(test)]
    PROBE_RUNS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    // Concurrently, results in spec order.
    let probes: Vec<_> = specs
        .iter()
        .map(|(name, program)| {
            let (name, program) = (name.clone(), program.clone());
            tokio::spawn(async move { detect_tool(&name, &program).await })
        })
        .collect();
    let mut tools = Vec::with_capacity(probes.len());
    for (probe, name) in probes.into_iter().zip(specs.keys()) {
        tools.push(probe.await.unwrap_or_else(|_| ToolStatus {
            name: name.clone(),
            found: false,
            version: None,
        }));
    }
    tools
}

/// Probe every tool at most once per [`TOOLS_TTL`] (S8-04). The async mutex is
/// held across an inline probe, so concurrent callers single-flight onto one
/// run instead of each forking `which` + `<tool> --version` (claude/codex are
/// Node boots). A changed spec map — a provider added or its command
/// overridden — misses the cache, so a `providers` settings change is picked
/// up at once. A stale probe of the same specs is served while one background
/// refresh replaces it (see [`freshness`]).
pub async fn probe_tools_cached(specs: BTreeMap<String, String>) -> Vec<ToolStatus> {
    use std::sync::atomic::Ordering;
    let mut slot = TOOLS_CACHE.lock().await;
    if let Some((at, cached_specs, tools)) = slot.as_ref() {
        if *cached_specs == specs {
            match freshness(at.elapsed(), tools) {
                Freshness::Fresh => return tools.clone(),
                Freshness::Stale => {
                    if !TOOLS_REFRESHING.swap(true, Ordering::AcqRel) {
                        tokio::spawn(async move {
                            // Cleared even if the probe panics.
                            struct Done;
                            impl Drop for Done {
                                fn drop(&mut self) {
                                    TOOLS_REFRESHING.store(false, Ordering::Release);
                                }
                            }
                            let _done = Done;
                            let tools = run_tool_probe(&specs).await;
                            *TOOLS_CACHE.lock().await =
                                Some((std::time::Instant::now(), specs, tools));
                        });
                    }
                    return tools.clone();
                }
                Freshness::Expired => {}
            }
        }
    }
    if TOOLS_REFRESHING.load(Ordering::Acquire) {
        return slot
            .as_ref()
            .filter(|(_, s, _)| *s == specs)
            .map(|(_, _, t)| t.clone())
            .unwrap_or_else(|| fast_tool_statuses(&specs));
    }
    let specs_bg = specs.clone();
    let (tx, rx) = tokio::sync::oneshot::channel::<Vec<ToolStatus>>();
    TOOLS_REFRESHING.store(true, Ordering::Release);
    tokio::spawn(async move {
        struct Done;
        impl Drop for Done {
            fn drop(&mut self) {
                TOOLS_REFRESHING.store(false, Ordering::Release);
            }
        }
        let _done = Done;
        let tools = run_tool_probe(&specs_bg).await;
        let _ = tx.send(tools.clone());
        *TOOLS_CACHE.lock().await = Some((std::time::Instant::now(), specs_bg, tools));
    });
    match timeout(COLD_INLINE_BUDGET, rx).await {
        Ok(Ok(tools)) => {
            *slot = Some((std::time::Instant::now(), specs, tools.clone()));
            tools
        }
        _ => fast_tool_statuses(&specs),
    }
}

/// Count of real probe passes run (test hook for the single-flight cache).
#[cfg(test)]
static PROBE_RUNS: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// A template cannot be checked until a session supplies its cwd/id. Omit its
/// row so callers report unchecked rather than disabling a valid provider.
fn configure_provider_probe(tools: &mut BTreeMap<String, String>, name: &str, program: String) {
    if program.contains("{cwd}") || program.contains("{sid}") {
        tools.remove(name);
    } else {
        tools.insert(name.to_string(), program);
    }
}

/// Probe one external tool: `which <name>` for presence, then
/// `<name> --version` (2s timeout) for the version string.
async fn detect_tool(name: &str, program: &str) -> ToolStatus {
    let found = executable_in_path_or_fs(program);
    // A configured command that isn't on PATH may be a shell alias/function
    // (`alias claude-work='CLAUDE_CONFIG_DIR=… claude'`); spawn runs those via
    // the login shell (`otto_sessions::providers::shell_wrapped`), so check
    // standard rc files first (<1 ms) and fall back to that login shell before
    // reporting it missing. Plain built-in names skip this.
    let builtin_name = DETECTED_TOOLS.contains(&name) && program == name;
    let found = found
        || (!builtin_name
            && !program.contains('/')
            && (rc_defines_alias_or_fn(program) || shell_knows(program).await));

    let mut version = None;
    if found && DETECTED_TOOLS.contains(&name) {
        if let Ok(Ok(out)) = timeout(
            Duration::from_secs(2),
            Command::new(program)
                .arg("--version")
                .kill_on_drop(true)
                .output(),
        )
        .await
        {
            if out.status.success() {
                version = String::from_utf8_lossy(&out.stdout)
                    .lines()
                    .next()
                    .map(|l| l.trim().to_string())
                    .filter(|l| !l.is_empty());
            }
        }
    }

    ToolStatus {
        name: name.to_string(),
        found,
        version,
    }
}

/// Whether the user's interactive login shell resolves `program` (alias,
/// function or executable). The name travels as `$1`, never spliced into the
/// script. 15s cap: interactive rc plugins can take several seconds under
/// post-deploy boot load, and the result is cached for [`TOOLS_TTL`].
async fn shell_knows(program: &str) -> bool {
    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| "/bin/zsh".to_string());
    let mut cmd = Command::new(shell);
    cmd.args([
        "-ilc",
        r#"command -v "$1" >/dev/null"#,
        "otto-probe",
        program,
    ])
    .stdin(std::process::Stdio::null())
    .kill_on_drop(true);
    matches!(
        timeout(Duration::from_secs(15), cmd.output()).await,
        Ok(Ok(out)) if out.status.success()
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn join_all_probes(specs: &BTreeMap<String, String>) -> Vec<Vec<ToolStatus>> {
        let calls: Vec<_> = (0..50)
            .map(|_| tokio::spawn(probe_tools_cached(specs.clone())))
            .collect();
        let mut out = Vec::new();
        for call in calls {
            out.push(call.await.unwrap());
        }
        out
    }

    #[test]
    fn context_dependent_provider_programs_remain_unchecked_including_builtin_overrides() {
        let mut probes = BTreeMap::from([("claude".to_string(), "claude".to_string())]);
        configure_provider_probe(&mut probes, "claude", "{cwd}/.venv/bin/agent".into());
        configure_provider_probe(&mut probes, "custom-agent", "/tmp/{sid}/agent".into());
        assert!(
            probes.is_empty(),
            "templates must not become false missing-executable reports"
        );
        configure_provider_probe(&mut probes, "custom-agent", "/bin/sh".into());
        assert_eq!(probes["custom-agent"], "/bin/sh");
    }

    #[test]
    fn rc_alias_and_function_definitions_are_detected_and_provisional_probe_never_disables_custom_bare_cmd(
    ) {
        let rc = r#"
            # alias commented-out='claude'
            alias claude-work='CLAUDE_CONFIG_DIR="$HOME/.claude-work" claude'
            alias "claude-alt"="claude --profile alt"
            my-wrapper() {
              claude "$@"
            }
            function other-wrapper {
              codex "$@"
            }
        "#;
        assert!(rc_content_defines_alias_or_fn(rc, "claude-work"));
        assert!(rc_content_defines_alias_or_fn(rc, "claude-alt"));
        assert!(rc_content_defines_alias_or_fn(rc, "my-wrapper"));
        assert!(rc_content_defines_alias_or_fn(rc, "other-wrapper"));
        assert!(!rc_content_defines_alias_or_fn(rc, "commented-out"));
        assert!(!rc_content_defines_alias_or_fn(rc, "claude-work-extra"));

        let specs = BTreeMap::from([
            ("sh-ready".to_string(), "/bin/sh".to_string()),
            (
                "custom-sourced-alias".to_string(),
                "otto-nonexistent-sourced-alias-xyz".to_string(),
            ),
            (
                "missing-abs".to_string(),
                "/nonexistent/otto-missing-bin".to_string(),
            ),
        ]);
        let fast = fast_tool_statuses(&specs);
        assert!(
            fast.iter().any(|t| t.name == "sh-ready" && t.found),
            "path executable reported ready"
        );
        assert!(
            !fast.iter().any(|t| t.name == "custom-sourced-alias"),
            "bare custom command omitted from provisional list instead of false-negative found:false"
        );
        assert!(
            fast.iter().any(|t| t.name == "missing-abs" && !t.found),
            "explicit path missing executable still reported false"
        );
    }

    /// S8-04: 50 concurrent `/meta` probes run the tool detection ONCE per
    /// TTL (single-flight + cache); a changed spec map misses the cache.
    /// Both cache tests share the process-wide probe cache.
    static CACHE_TESTS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[test]
    fn stale_probes_are_served_only_while_an_agent_cli_was_found() {
        let tool = |name: &str, found| ToolStatus {
            name: name.into(),
            found,
            version: None,
        };
        let agent = [tool("git", true), tool("claude", true)];
        let none = [
            tool("git", true),
            tool("clickhouse", true),
            tool("claude", false),
        ];
        let min = Duration::from_secs(60);
        assert_eq!(freshness(Duration::from_secs(5), &none), Freshness::Fresh);
        assert_eq!(freshness(min * 10, &agent), Freshness::Stale);
        assert_eq!(freshness(min * 10, &none), Freshness::Expired);
        assert_eq!(freshness(TOOLS_MAX_STALE, &agent), Freshness::Expired);
    }

    /// A stale probe answers at once; the refresh runs behind the response.
    #[tokio::test]
    async fn stale_probe_is_served_and_refreshed_in_the_background() {
        use std::sync::atomic::Ordering;
        let _serial = CACHE_TESTS.lock().await;
        let specs = BTreeMap::from([("stale-agent".to_string(), "/bin/sh".to_string())]);
        let old = vec![ToolStatus {
            name: "stale-agent".into(),
            found: true,
            version: Some("old".into()),
        }];
        let aged = std::time::Instant::now()
            .checked_sub(Duration::from_secs(600))
            .unwrap();
        *TOOLS_CACHE.lock().await = Some((aged, specs.clone(), old.clone()));
        let before = PROBE_RUNS.load(Ordering::Relaxed);
        let served = probe_tools_cached(specs.clone()).await;
        assert_eq!(served[0].version.as_deref(), Some("old"), "served stale");
        for _ in 0..200 {
            if !TOOLS_REFRESHING.load(Ordering::Acquire) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        assert_eq!(PROBE_RUNS.load(Ordering::Relaxed) - before, 1);
        let refreshed = probe_tools_cached(specs).await;
        assert_eq!(refreshed[0].version, None, "the refresh replaced it");
        assert_eq!(PROBE_RUNS.load(Ordering::Relaxed) - before, 1, "now fresh");
    }

    #[tokio::test]
    async fn tool_probe_is_single_flight_and_cached() {
        use std::sync::atomic::Ordering;
        let _serial = CACHE_TESTS.lock().await;
        let specs = BTreeMap::from([("probe-cache-test".to_string(), "/bin/sh".to_string())]);
        let before = PROBE_RUNS.load(Ordering::Relaxed);
        let all = join_all_probes(&specs).await;
        assert!(all.iter().all(|t| t.len() == 1 && t[0].found));
        assert_eq!(
            PROBE_RUNS.load(Ordering::Relaxed) - before,
            1,
            "concurrent callers share one probe"
        );
        let mut changed = specs.clone();
        changed.insert("probe-cache-test-2".into(), "/bin/sh".into());
        assert_eq!(probe_tools_cached(changed).await.len(), 2);
        assert_eq!(PROBE_RUNS.load(Ordering::Relaxed) - before, 2);
    }

    #[tokio::test]
    async fn custom_provider_detection_uses_its_configured_executable() {
        let ready = detect_tool("custom-agent", "/bin/sh").await;
        assert_eq!(ready.name, "custom-agent");
        assert!(ready.found);
        assert!(
            ready.version.is_none(),
            "custom executables are never run just to probe a version"
        );
        assert!(
            !detect_tool("custom-agent", "/nonexistent/otto-custom-agent")
                .await
                .found
        );
    }
}
