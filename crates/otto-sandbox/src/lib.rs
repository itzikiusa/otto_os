//! OS-level process confinement for spawned agent/shell sessions.
//!
//! On macOS this generates an Apple **Seatbelt** profile (SBPL) and wraps the
//! command in `/usr/bin/sandbox-exec`, the same primitive Claude Code and the
//! Codex CLI use. The posture mirrors Anthropic's `sandbox-runtime`:
//!
//! - **read** is allowed everywhere by default (minus explicit `deny_read`
//!   carveouts), so the agent can read system libraries, the repo and its
//!   out-of-tree context bundle;
//! - **write** is denied everywhere by default and only re-allowed for an
//!   explicit set of `writable_roots` (the workspace + the resolved git dir so
//!   commits still work + the agent CLIs' own config/cache dirs + temp);
//! - **network** is policy-controlled (`Full` keeps agents able to reach their
//!   model API; `LoopbackOnly`/`None` are for stricter, non-model shells).
//!
//! The crate is pure: it only *builds* the profile and *rewrites* the command.
//! The caller (otto-sessions) spawns the rewritten command. On non-macOS this
//! degrades to a no-op so the workspace still builds and lints cleanly.

use std::path::{Path, PathBuf};

/// Outbound network posture for a sandboxed process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetworkPolicy {
    /// No network at all (default Seatbelt deny).
    None,
    /// Only loopback (127.0.0.1 / localhost) — reaches Otto's own daemon but no
    /// external host.
    LoopbackOnly,
    /// Unrestricted network. Filesystem write-confinement is still enforced.
    /// This is the only mode in which an agent CLI can reach its model API.
    Full,
}

/// A filesystem + network confinement policy for one spawned process.
#[derive(Debug, Clone)]
pub struct SandboxPolicy {
    /// Directories (recursive) the process may write to. Everything else is
    /// read-only. Should always include the workspace cwd.
    pub writable_roots: Vec<PathBuf>,
    /// Paths whose *reads* are denied even though read is otherwise global
    /// (e.g. secret stores). Empty by default.
    pub deny_read: Vec<PathBuf>,
    /// Outbound network posture.
    pub network: NetworkPolicy,
    /// Mach services the process may look up. `None` = any (the historical
    /// blanket `(allow mach-lookup)`, kept for daemon-spawned tools); `Some` =
    /// only these global names. Mach lookup is how a process reaches
    /// LaunchServices / AppleEvents / other agents that act OUTSIDE its
    /// sandbox, so an agent gets [`AGENT_MACH_SERVICES`] only.
    pub mach_services: Option<Vec<String>>,
    /// Raw SBPL rules appended after everything else. Seatbelt is last-match,
    /// so these carve exceptions out of the grants above — e.g. Otto's own
    /// data dir is write-denied even when it sits under a writable root, with
    /// its agent work areas re-allowed after that deny. Built by the
    /// constructors from escaped paths; empty by default.
    pub trailing_rules: Vec<String>,
}

/// Mach services an agent CLI (claude / codex / agy under node or a native
/// binary) and its usual toolchain (git, gh, curl, python, npm) need. Chosen
/// from the services Codex's and Anthropic's Seatbelt profiles allow, then
/// verified with `sandbox-exec` probes: node DNS + `fetch` (TLS), curl, git
/// over HTTPS, `security find-generic-password` (claude's OAuth lookup), `gh
/// auth status` (keyring), `claude`/`codex --version`, python getpass /
/// tempdir / DNS all behave exactly as with a blanket allow — while
/// LaunchServices resolution (`open -a …`, `path to application`) fails.
///
/// Deliberately absent: LaunchServices (`com.apple.coreservices.launchservicesd`,
/// `com.apple.lsd.*`), AppleEvents (`com.apple.coreservices.appleevents`) and
/// every app/agent service — the ways a sandboxed process asks something
/// UNsandboxed to run code for it (`open -a Terminal x.command`).
pub const AGENT_MACH_SERVICES: &[&str] = &[
    // User/group lookups (getpwuid, getgrouplist) — every CLI.
    "com.apple.system.opendirectoryd.libinfo",
    "com.apple.system.opendirectoryd.membership",
    "com.apple.system.DirectoryService.libinfo_v1",
    // Logging + notify(3) (timezone changes, etc.).
    "com.apple.system.logger",
    "com.apple.system.notification_center",
    "com.apple.logd",
    "com.apple.diagnosticd",
    // confstr(_CS_DARWIN_USER_TEMP_DIR/CACHE_DIR) — tmpdir resolution.
    "com.apple.bsd.dirhelper",
    "com.apple.PowerManagement.control",
    // CFPreferences reads.
    "com.apple.cfprefsd.daemon",
    "com.apple.cfprefsd.agent",
    // File watchers (fsevents) used by agent CLIs and dev servers.
    "com.apple.FSEvents",
    // Network configuration + DNS.
    "com.apple.SystemConfiguration.configd",
    "com.apple.SystemConfiguration.DNSConfiguration",
    "com.apple.dnssd.service",
    "com.apple.networkd",
    // TLS trust evaluation + the keychain (claude's OAuth token, git/gh
    // credential helpers).
    "com.apple.trustd",
    "com.apple.trustd.agent",
    "com.apple.ocspd",
    "com.apple.SecurityServer",
    "com.apple.securityd.xpc",
];

/// Subdirectories of Otto's data dir that ARE agent work areas (session cwds
/// the engines create, and the workflow step-handoff dir agents are told to
/// write into). Everything else there — `bin/ottod` (launchd re-executes it),
/// `otto.db*`, `secrets.json`, `tls/`, provider homes of other accounts —
/// stays write-denied. Extend this when an engine starts a session with a cwd
/// (or a handoff dir) under the data dir.
pub const AGENT_DATA_SUBDIRS: &[&str] = &[
    "workflow-runs",
    "workflow-context",
    "scheduled",
    "personal",
    "goal-loops",
    "otto-runs",
    "swarm",
    "insights",
    "db_assist",
    "canvas",
    "browser_summarize",
];

/// Files/dirs of Otto's data dir an agent must not even READ: plaintext
/// secrets, the state DB (sessions, roles, tokens) incl. WAL/SHM/journal and
/// the `otto.db.*.bak` copies, the daemon's TLS key, and kubeconfigs. `prefix`
/// entries match every path starting with that string.
const DATA_DIR_DENY_READ_LITERAL: &[&str] = &["secrets.json"];
const DATA_DIR_DENY_READ_PREFIX: &[&str] = &["otto.db", "state.db"];
const DATA_DIR_DENY_READ_SUBPATH: &[&str] = &["tls", "kube"];

/// Files under `$HOME` that make UNsandboxed programs run code the agent
/// chose: claude hooks (`settings*.json`), codex `notify` (`config.toml`) and
/// git's XDG config (hooks path, aliases, credential helpers). Write-denied
/// even though their parent dirs stay writable for the CLIs' own state.
const HOME_DENY_WRITE_LITERAL: &[&str] = &[
    ".claude/settings.json",
    ".claude/settings.local.json",
    ".codex/config.toml",
];
const HOME_DENY_WRITE_SUBPATH: &[&str] = &[".config/git"];

/// Programs that hand work to launchd (which runs it outside the sandbox).
/// `launchctl` talks to launchd over the bootstrap port, which mach-lookup
/// rules don't cover, so its exec is denied instead.
const AGENT_DENY_EXEC: &[&str] = &["/bin/launchctl"];

impl SandboxPolicy {
    /// Build the default policy for an **agent** session: confine writes to the
    /// workspace `cwd`, the resolved git dir(s) in `extra_writable` (so commits
    /// in a worktree still work), the agent CLIs' own config/cache dirs under
    /// `home`, the agent work areas of Otto's `data_dir`
    /// ([`AGENT_DATA_SUBDIRS`]) and the system temp dirs. Reads stay global.
    ///
    /// Otto's `data_dir` itself is write-denied (and its secrets / state DB /
    /// TLS key read-denied) even when it lies under a writable root: a
    /// "confined" agent could otherwise replace `bin/ottod` (launchd re-runs
    /// it unsandboxed), edit `otto.db` (roles, tokens, the sandbox setting
    /// itself) or read `secrets.json`. An `extra_writable` entry INSIDE
    /// `data_dir` (e.g. the session's own provider-account home) is re-allowed
    /// after that deny. Mach lookups are limited to [`AGENT_MACH_SERVICES`].
    pub fn for_agent(
        cwd: &Path,
        home: &Path,
        data_dir: &Path,
        extra_writable: &[PathBuf],
        network: NetworkPolicy,
    ) -> Self {
        let data_dir = canonicalize_lenient(data_dir);
        let data_dir_set = !data_dir.as_os_str().is_empty();
        let mut roots: Vec<PathBuf> = Vec::new();
        let mut reallow: Vec<PathBuf> = Vec::new();
        let mut push = |p: PathBuf| {
            if !p.as_os_str().is_empty() {
                roots.push(canonicalize_lenient(&p));
            }
        };

        push(cwd.to_path_buf());
        for e in extra_writable {
            let e = canonicalize_lenient(e);
            if data_dir_set && e.starts_with(&data_dir) {
                reallow.push(e);
            } else {
                push(e);
            }
        }

        // Temp dirs the toolchain (node, git, build tools) scribbles into.
        push(std::env::temp_dir());
        push(PathBuf::from("/tmp"));
        push(PathBuf::from("/private/tmp"));
        push(PathBuf::from("/private/var/folders"));

        // The agent CLIs persist transcripts / session ids / caches here; without
        // these the CLIs can't resume and Otto's pre-trust writes fail.
        for rel in [
            ".claude",
            ".claude.json",
            ".codex",
            ".gemini",
            ".config",
            ".cache",
            ".npm",
            ".otto",
            "Library/Caches",
        ] {
            push(home.join(rel));
        }

        // De-duplicate.
        roots.sort();
        roots.dedup();

        // Carve-outs, in order (Seatbelt: last match wins).
        let mut trailing: Vec<String> = Vec::new();
        let home_set = !home.as_os_str().is_empty();
        if home_set {
            let mut filters: Vec<String> = HOME_DENY_WRITE_LITERAL
                .iter()
                .map(|rel| filter("literal", &canonicalize_lenient(&home.join(rel))))
                .collect();
            filters.extend(
                HOME_DENY_WRITE_SUBPATH
                    .iter()
                    .map(|rel| filter("subpath", &canonicalize_lenient(&home.join(rel)))),
            );
            trailing.push(format!("(deny file-write* {})", filters.join(" ")));
        }
        if data_dir_set {
            // 1. Otto's data dir is read-only for the agent…
            trailing.push(format!("(deny file-write* {})", filter("subpath", &data_dir)));
            // 2. …except its agent work areas and the caller's in-data-dir
            //    extras (e.g. this session's provider-account home).
            let mut open: Vec<PathBuf> = AGENT_DATA_SUBDIRS
                .iter()
                .map(|d| data_dir.join(d))
                .collect();
            open.extend(reallow);
            let open: Vec<String> = open.iter().map(|p| filter("subpath", p)).collect();
            trailing.push(format!("(allow file-write* {})", open.join(" ")));
            // 3. Secrets / state DB / TLS key / kubeconfigs are not even readable.
            let mut hidden: Vec<String> = Vec::new();
            hidden.extend(
                DATA_DIR_DENY_READ_LITERAL
                    .iter()
                    .map(|f| filter("literal", &data_dir.join(f))),
            );
            hidden.extend(
                DATA_DIR_DENY_READ_PREFIX
                    .iter()
                    .map(|f| filter("prefix", &data_dir.join(f))),
            );
            hidden.extend(
                DATA_DIR_DENY_READ_SUBPATH
                    .iter()
                    .map(|f| filter("subpath", &data_dir.join(f))),
            );
            trailing.push(format!("(deny file-read* {})", hidden.join(" ")));
        }
        let exec: Vec<String> = AGENT_DENY_EXEC
            .iter()
            .map(|p| filter("literal", Path::new(p)))
            .collect();
        trailing.push(format!("(deny process-exec {})", exec.join(" ")));

        Self {
            writable_roots: roots,
            deny_read: Vec::new(),
            network,
            mach_services: Some(AGENT_MACH_SERVICES.iter().map(|s| s.to_string()).collect()),
            trailing_rules: trailing,
        }
    }

    /// Build the policy for a **daemon-spawned tool** (today: a headless Blender
    /// render of a daemon-generated script — see `otto-server::design_blender`).
    /// Much tighter than `for_agent`: writes are confined to the job's `out_dir`,
    /// the system temp dirs and the tool's own cache/config dirs under `$HOME`
    /// (Blender writes its `userpref`/cache on first launch); **no network** —
    /// the tool has nothing to fetch. Reads stay global so the binary, its
    /// bundled Python and system libraries load.
    pub fn for_tool(out_dir: &Path) -> Self {
        let mut roots: Vec<PathBuf> = Vec::new();
        let mut push = |p: PathBuf| {
            if !p.as_os_str().is_empty() {
                roots.push(canonicalize_lenient(&p));
            }
        };

        push(out_dir.to_path_buf());
        push(std::env::temp_dir());
        push(PathBuf::from("/tmp"));
        push(PathBuf::from("/private/tmp"));
        push(PathBuf::from("/private/var/folders"));

        if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
            for rel in [
                // Blender: prefs/scripts/cache (macOS + XDG layouts).
                "Library/Application Support/Blender",
                "Library/Caches",
                ".config/blender",
                ".cache",
            ] {
                push(home.join(rel));
            }
        }

        roots.sort();
        roots.dedup();

        Self {
            writable_roots: roots,
            deny_read: Vec::new(),
            network: NetworkPolicy::None,
            // A headless Blender needs GPU/font/etc. services an allow-list
            // would have to track; it runs a daemon-generated script, not an
            // agent, so it keeps the historical blanket mach-lookup.
            mach_services: None,
            trailing_rules: Vec::new(),
        }
    }

    /// Render the macOS Seatbelt (SBPL) profile for this policy.
    pub fn to_sbpl(&self) -> String {
        let mut p = String::new();
        p.push_str("(version 1)\n");
        p.push_str("(deny default)\n");
        // Let the program (and the subprocesses agents spawn) actually run.
        p.push_str("(allow process-exec)\n");
        p.push_str("(allow process-fork)\n");
        p.push_str("(allow signal (target self))\n");
        p.push_str("(allow sysctl-read)\n");
        match &self.mach_services {
            None => p.push_str("(allow mach-lookup)\n"),
            Some(names) if names.is_empty() => {}
            Some(names) => {
                p.push_str("(allow mach-lookup");
                for n in names {
                    p.push_str(&format!(" (global-name \"{}\")", escape_str(n)));
                }
                p.push_str(")\n");
            }
        }
        p.push_str("(allow ipc-posix-shm)\n");
        p.push_str("(allow system-socket)\n");
        // Read everywhere; the PTY + devices need ioctl + /dev writes.
        p.push_str("(allow file-read*)\n");
        p.push_str("(allow file-ioctl)\n");
        p.push_str("(allow file-write* (subpath \"/dev\"))\n");

        // Secret-read carveouts win because Seatbelt is last-match.
        for d in &self.deny_read {
            p.push_str(&format!("(deny file-read* (subpath \"{}\"))\n", escape(d)));
        }

        // Writable roots (everything else stays read-only).
        for r in &self.writable_roots {
            p.push_str(&format!(
                "(allow file-write* (subpath \"{}\"))\n",
                escape(r)
            ));
        }

        // Network.
        match self.network {
            NetworkPolicy::None => {}
            NetworkPolicy::LoopbackOnly => {
                p.push_str("(allow network-outbound (remote ip \"localhost:*\"))\n");
                p.push_str("(allow network-bind (local ip \"localhost:*\"))\n");
            }
            NetworkPolicy::Full => {
                p.push_str("(allow network-outbound)\n");
                p.push_str("(allow network-inbound)\n");
                p.push_str("(allow network-bind)\n");
            }
        }

        // Carve-outs last, so they win over every grant above.
        for rule in &self.trailing_rules {
            p.push_str(rule);
            p.push('\n');
        }
        p
    }

    /// Rewrite `(program, args)` to run under `/usr/bin/sandbox-exec` with this
    /// policy's profile passed inline (`-p`). The original program becomes the
    /// command sandbox-exec runs.
    pub fn wrap(&self, program: &str, args: &[String]) -> (String, Vec<String>) {
        let mut wrapped = vec!["-p".to_string(), self.to_sbpl(), program.to_string()];
        wrapped.extend(args.iter().cloned());
        ("/usr/bin/sandbox-exec".to_string(), wrapped)
    }
}

/// True when OS-level sandboxing is available on this host (macOS with
/// `sandbox-exec` present).
pub fn is_supported() -> bool {
    cfg!(target_os = "macos") && Path::new("/usr/bin/sandbox-exec").exists()
}

/// Canonicalize a path, falling back to the input when it doesn't exist yet
/// (a subpath rule still matches by prefix). On macOS this also resolves the
/// `/tmp`→`/private/tmp` and `/var`→`/private/var` symlinks the sandbox sees.
fn canonicalize_lenient(p: &Path) -> PathBuf {
    std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf())
}

/// Escape a path for inclusion in an SBPL string literal.
fn escape(p: &Path) -> String {
    escape_str(&p.to_string_lossy())
}

fn escape_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// One SBPL path filter, e.g. `(subpath "/x")`. `kind` is `literal`,
/// `subpath` or `prefix`.
fn filter(kind: &str, p: &Path) -> String {
    format!("({kind} \"{}\")", escape(p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(roots: &[&str], net: NetworkPolicy) -> SandboxPolicy {
        SandboxPolicy {
            writable_roots: roots.iter().map(PathBuf::from).collect(),
            deny_read: Vec::new(),
            network: net,
            mach_services: None,
            trailing_rules: Vec::new(),
        }
    }

    fn agent_policy(data: &str) -> SandboxPolicy {
        SandboxPolicy::for_agent(
            Path::new("/work/project"),
            Path::new("/home/u"),
            Path::new(data),
            &[
                PathBuf::from("/work/project/.git"),
                PathBuf::from(format!("{data}/provider-accounts/acct1")),
            ],
            NetworkPolicy::Full,
        )
    }

    /// Byte offset of `needle` in `hay` (panics with context when absent).
    fn at(hay: &str, needle: &str) -> usize {
        hay.find(needle)
            .unwrap_or_else(|| panic!("missing {needle:?} in profile:\n{hay}"))
    }

    /// The escape the report found: a "write-confined" agent could replace
    /// `bin/ottod`, edit `otto.db` and read `secrets.json`. The data dir is now
    /// write-denied AFTER every grant (last match wins), with only the agent
    /// work areas and the session's own provider home re-opened after that.
    #[test]
    fn for_agent_denies_otto_data_dir_except_work_areas() {
        let data = "/nonexistent-otto-test/Application Support/Otto";
        let pol = agent_policy(data);
        assert!(
            !pol.writable_roots.iter().any(|r| r.starts_with(data)),
            "the data dir (or anything in it) must not be a plain writable root: {:?}",
            pol.writable_roots
        );
        let sbpl = pol.to_sbpl();
        let deny = at(&sbpl, &format!("(deny file-write* (subpath \"{data}\"))"));
        let reopen = at(&sbpl, &format!("(subpath \"{data}/workflow-context\")"));
        let own_home = at(&sbpl, &format!("(subpath \"{data}/provider-accounts/acct1\")"));
        assert!(deny < reopen && deny < own_home, "re-allows must follow the deny");
        // Never re-opened: the daemon binary, the DB, the secrets.
        assert!(!sbpl.contains(&format!("(subpath \"{data}/bin\")")));
        assert!(!sbpl.contains(&format!("(subpath \"{data}/provider-accounts\")")));
        // Not readable at all.
        let hidden = at(&sbpl, "(deny file-read*");
        assert!(hidden > deny);
        assert!(sbpl.contains(&format!("(literal \"{data}/secrets.json\")")));
        assert!(sbpl.contains(&format!("(prefix \"{data}/otto.db\")")));
        assert!(sbpl.contains(&format!("(subpath \"{data}/tls\")")));
        // The carve-outs come after every grant, network included.
        assert!(deny > at(&sbpl, "(allow network-outbound)"));
    }

    #[test]
    fn for_agent_mach_lookup_is_an_allow_list() {
        let sbpl = agent_policy("/nonexistent-otto-test/Otto").to_sbpl();
        assert!(!sbpl.contains("(allow mach-lookup)\n"), "blanket mach-lookup");
        assert!(sbpl.contains("(global-name \"com.apple.trustd.agent\")"));
        assert!(sbpl.contains("(global-name \"com.apple.SecurityServer\")"));
        for escape_hatch in [
            "com.apple.coreservices.launchservicesd",
            "com.apple.coreservices.appleevents",
            "com.apple.lsd.mapdb",
        ] {
            assert!(!sbpl.contains(escape_hatch), "{escape_hatch} must not be reachable");
        }
        assert!(sbpl.contains("(deny process-exec (literal \"/bin/launchctl\"))"));
    }

    #[test]
    fn for_agent_protects_configs_that_run_unsandboxed_code() {
        let sbpl = agent_policy("/nonexistent-otto-test/Otto").to_sbpl();
        let grant = at(&sbpl, "(allow file-write* (subpath \"/home/u/.claude\"))");
        let deny = at(&sbpl, "(literal \"/home/u/.claude/settings.json\")");
        assert!(deny > grant, "the settings deny must override the .claude grant");
        assert!(sbpl.contains("(literal \"/home/u/.codex/config.toml\")"));
        assert!(sbpl.contains("(subpath \"/home/u/.config/git\")"));
    }

    #[test]
    fn for_tool_keeps_blanket_mach_lookup_and_no_carve_outs() {
        let pol = SandboxPolicy::for_tool(&std::env::temp_dir().join("otto-tool-out"));
        assert!(pol.mach_services.is_none());
        assert!(pol.trailing_rules.is_empty());
        assert!(pol.to_sbpl().contains("(allow mach-lookup)\n"));
    }

    #[test]
    fn sbpl_is_default_deny_with_global_read() {
        let sbpl = policy(&["/work"], NetworkPolicy::None).to_sbpl();
        assert!(sbpl.starts_with("(version 1)\n(deny default)"));
        assert!(sbpl.contains("(allow file-read*)"));
        assert!(sbpl.contains("(allow process-exec)"));
    }

    #[test]
    fn sbpl_allows_writes_only_to_roots() {
        let sbpl = policy(&["/work", "/repo/.git"], NetworkPolicy::None).to_sbpl();
        assert!(sbpl.contains("(allow file-write* (subpath \"/work\"))"));
        assert!(sbpl.contains("(allow file-write* (subpath \"/repo/.git\"))"));
        // No catch-all write allow.
        assert!(!sbpl.contains("(allow file-write*)\n"));
    }

    #[test]
    fn sbpl_network_modes() {
        assert!(!policy(&["/w"], NetworkPolicy::None)
            .to_sbpl()
            .contains("network-outbound"));
        assert!(policy(&["/w"], NetworkPolicy::Full)
            .to_sbpl()
            .contains("(allow network-outbound)\n"));
        let lo = policy(&["/w"], NetworkPolicy::LoopbackOnly).to_sbpl();
        assert!(lo.contains("localhost"));
        assert!(!lo.contains("(allow network-outbound)\n"));
    }

    #[test]
    fn sbpl_deny_read_carveout_is_present() {
        let mut pol = policy(&["/w"], NetworkPolicy::Full);
        pol.deny_read.push(PathBuf::from("/secret"));
        assert!(pol
            .to_sbpl()
            .contains("(deny file-read* (subpath \"/secret\"))"));
    }

    #[test]
    fn wrap_prepends_sandbox_exec() {
        let pol = policy(&["/w"], NetworkPolicy::Full);
        let (prog, args) = pol.wrap("claude", &["--foo".into(), "bar".into()]);
        assert_eq!(prog, "/usr/bin/sandbox-exec");
        assert_eq!(args[0], "-p");
        // profile, then the original command + args in order.
        assert_eq!(args[2], "claude");
        assert_eq!(args[3], "--foo");
        assert_eq!(args[4], "bar");
        assert!(args[1].contains("(deny default)"));
    }

    #[test]
    fn for_tool_confines_writes_to_out_dir_and_cuts_network() {
        let out = std::env::temp_dir().join("otto-tool-out");
        let pol = SandboxPolicy::for_tool(&out);
        assert!(pol
            .writable_roots
            .iter()
            .any(|r| r.ends_with("otto-tool-out")));
        // A tool never gets the workspace-ish agent dirs…
        assert!(!pol.writable_roots.iter().any(|r| r.ends_with(".claude")));
        // …nor any network.
        assert_eq!(pol.network, NetworkPolicy::None);
        let sbpl = pol.to_sbpl();
        assert!(!sbpl.contains("network-outbound"));
        assert!(sbpl.contains("(allow file-write* (subpath \""));
    }

    #[test]
    fn for_agent_includes_cwd_git_and_agent_dirs() {
        let cwd = PathBuf::from("/work/project");
        let home = PathBuf::from("/home/u");
        let data = PathBuf::from("/home/u/.otto/data");
        let gitdir = PathBuf::from("/work/project/.git");
        let pol = SandboxPolicy::for_agent(
            &cwd,
            &home,
            &data,
            std::slice::from_ref(&gitdir),
            NetworkPolicy::Full,
        );
        // cwd, git dir, and an agent config dir are all writable.
        assert!(pol.writable_roots.iter().any(|r| r.ends_with("project")));
        assert!(pol.writable_roots.iter().any(|r| r.ends_with(".claude")));
        // network policy is carried through.
        assert_eq!(pol.network, NetworkPolicy::Full);
    }
}
