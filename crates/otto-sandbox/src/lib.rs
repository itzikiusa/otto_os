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

/// Design Hall working copies: `<data>/design/<artifact>/work/**` is where a
/// design-assist agent edits the artifact in place, so it is re-opened after
/// the data-dir deny — per artifact, by pattern (`[^/]+` = one path segment).
/// Nothing else under `design/` is: the content-addressed blob store
/// (`design/blobs/**`, every version's bytes) stays write-denied even if it
/// were shaped like a working copy (an explicit deny follows the allow), and
/// so do the variant scratch dirs (`design/<artifact>/variants/**`).
const DESIGN_WORK_DIR: &str = "design";
const DESIGN_BLOBS_DIR: &str = "design/blobs";

/// Files/dirs of Otto's data dir an agent must not even READ: plaintext
/// secrets, the state DB (sessions, roles, tokens) incl. WAL/SHM/journal and
/// the `otto.db.*.bak` copies, the daemon's TLS key, and kubeconfigs. `prefix`
/// entries match every path starting with that string.
const DATA_DIR_DENY_READ_LITERAL: &[&str] = &["secrets.json"];
const DATA_DIR_DENY_READ_PREFIX: &[&str] = &["otto.db", "state.db"];
const DATA_DIR_DENY_READ_SUBPATH: &[&str] = &["tls", "kube", "logs"];

/// Where named provider accounts keep their CLI homes (`<data>/provider-accounts/<id>`,
/// used as `CLAUDE_CONFIG_DIR` / `CODEX_HOME`). Read-denied as a whole — other
/// accounts' OAuth credentials live there — with only the session's own
/// account home (an `extra_writable` entry inside the data dir) re-opened.
const PROVIDER_ACCOUNTS_DIR: &str = "provider-accounts";

/// Files / dirs of a provider CLI's config root that make the CLI (in EVERY
/// later session, sandboxed or not) run code the agent chose: claude
/// `settings*.json` (hooks), `.claude.json` (user-scope `mcpServers`), codex
/// `config.toml` (`notify`, MCP servers), plus claude's plugins / hook scripts
/// / subagents / slash commands. Applied to a named account's home, which IS
/// such a root (`CLAUDE_CONFIG_DIR` = `CODEX_HOME` = the account home). The
/// CLI's own state there (projects/, todos/, statsig/, sessions/…) stays
/// writable.
const PROVIDER_ROOT_DENY_WRITE_LITERAL: &[&str] = &[
    "settings.json",
    "settings.local.json",
    ".claude.json",
    "config.toml",
];
const PROVIDER_ROOT_DENY_WRITE_SUBPATH: &[&str] = &["plugins", "hooks", "agents", "commands"];

/// A read-only session's own project folder: the project-scope claude config
/// (hooks in `.claude/settings*.json`, project MCP servers in `.mcp.json`,
/// subagents / slash commands) that the user's next unsandboxed `claude` in
/// that folder runs without a prompt — Otto pre-trusts every session cwd.
/// Only for `read_only` sessions: an ordinary agent session legitimately
/// edits its repo's checked-in `.claude/` files.
const PROJECT_DENY_WRITE_LITERAL: &[&str] = &[
    ".claude/settings.json",
    ".claude/settings.local.json",
    ".mcp.json",
];
const PROJECT_DENY_WRITE_SUBPATH: &[&str] =
    &[".claude/hooks", ".claude/agents", ".claude/commands"];

/// Files under `$HOME` that make UNsandboxed programs run code the agent
/// chose: claude hooks (`settings*.json`), claude's user/project-scope
/// `mcpServers` (`~/.claude.json` — every later `claude` the user runs spawns
/// them), codex `notify` (`config.toml`), gemini/agy `mcpServers`
/// (`.gemini/settings.json`) and git's XDG config (hooks path, aliases,
/// credential helpers). Write-denied even though their parent dirs stay
/// writable for the CLIs' own state. (Claude Code tolerates a read-only
/// `~/.claude.json`: it keeps running and only skips persisting its startup
/// counters; Otto's trust pre-seeding is written by the daemon, unsandboxed.)
const HOME_DENY_WRITE_LITERAL: &[&str] = &[
    ".claude.json",
    ".claude/settings.json",
    ".claude/settings.local.json",
    ".codex/config.toml",
    ".gemini/settings.json",
];
const HOME_DENY_WRITE_SUBPATH: &[&str] = &[
    ".config/git",
    // gh aliases (`!shell` aliases run on the user's next `gh …`) + hosts.
    ".config/gh",
    // Claude Code extension points that load into EVERY claude session the
    // user runs, sandboxed or not: plugins (with their hooks), hook scripts
    // referenced from settings.json, subagent and slash-command definitions.
    // The CLI's own state (projects/, todos/, statsig/, shell-snapshots/…)
    // stays writable under the `.claude` grant.
    ".claude/plugins",
    ".claude/hooks",
    ".claude/agents",
    ".claude/commands",
];

/// The Otto desktop app's (bundle id `com.otto.app`) WebKit storage under
/// `$HOME`: localStorage holds the UI's bearer token (`otto_token`), and the
/// network cache / cookies hold API responses. Neither readable nor writable
/// (a poisoned cache entry would load into the app's webview).
const HOME_DENY_ALL_SUBPATH: &[&str] = &[
    "Library/WebKit/com.otto.app",
    "Library/Application Support/com.otto.app",
    "Library/Caches/com.otto.app",
    "Library/HTTPStorages/com.otto.app",
];

/// Files of a repo's git dir that make the DAEMON's (unsandboxed) git run
/// code: `config` (fsmonitor, sshCommand, credential helpers, filters, hooks
/// path, aliases), `config.worktree`, `commondir` (re-points the whole repo at
/// another config) and `hooks/` — and, in each linked worktree's admin dir,
/// `gitdir` (re-points the worktree at an agent-built repo whose config the
/// daemon's worktree probe would then run). The git dir itself is literal-denied too, so
/// it can't be renamed away and replaced by one the agent wrote. Objects,
/// refs, the index, logs — what `git commit`/`branch`/`stash` write — stay
/// writable. (An agent's `git push -u` still pushes; only the upstream
/// tracking line it would add to `config` is refused.)
const GIT_DIR_DENY_WRITE_LITERAL: &[&str] = &["config", "config.worktree", "commondir"];
const GIT_DIR_DENY_WRITE_SUBPATH: &[&str] = &["hooks"];
const GIT_WORKTREE_ADMIN_DENY_WRITE_LITERAL: &[&str] = &["gitdir"];

/// Programs that hand work to launchd (which runs it outside the sandbox).
/// `launchctl` talks to launchd over the bootstrap port, which mach-lookup
/// rules don't cover, so its exec is denied instead.
const AGENT_DENY_EXEC: &[&str] = &["/bin/launchctl"];

impl SandboxPolicy {
    /// Build the default policy for an **agent** session: confine writes to the
    /// workspace `cwd`, the resolved git dir(s) in `extra_writable` (so commits
    /// in a worktree still work), the agent CLIs' own config/cache dirs under
    /// `home`, the agent work areas of Otto's `data_dir`
    /// ([`AGENT_DATA_SUBDIRS`] + the Design Hall working copies
    /// `design/<artifact>/work/**`) and the system temp dirs. Reads stay global.
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
        // these the CLIs can't resume. (`~/.claude.json` is deliberately NOT a
        // root — see [`HOME_DENY_WRITE_LITERAL`].)
        for rel in [
            ".claude",
            ".codex",
            ".gemini",
            // Not the whole `~/.config`: it holds code-loading configs of
            // other tools (direnv, fish, nvim, zed tasks…). Only node CLIs'
            // update-notifier state.
            ".config/configstore",
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
        if home_set {
            let hidden: Vec<String> = HOME_DENY_ALL_SUBPATH
                .iter()
                .map(|rel| filter("subpath", &canonicalize_lenient(&home.join(rel))))
                .collect();
            trailing.push(format!(
                "(deny file-read* file-write* {})",
                hidden.join(" ")
            ));
        }
        if data_dir_set {
            // 1. Otto's data dir is read-only for the agent…
            trailing.push(format!(
                "(deny file-write* {})",
                filter("subpath", &data_dir)
            ));
            // 2. …except its agent work areas and the caller's in-data-dir
            //    extras (e.g. this session's provider-account home).
            let mut open: Vec<PathBuf> = AGENT_DATA_SUBDIRS
                .iter()
                .map(|d| data_dir.join(d))
                .collect();
            open.extend(reallow.iter().cloned());
            let mut open: Vec<String> = open.iter().map(|p| filter("subpath", p)).collect();
            open.extend(design_work_filters(&data_dir));
            trailing.push(format!("(allow file-write* {})", open.join(" ")));
            //    …but never the design blob store, whatever a pattern matched.
            trailing.push(format!(
                "(deny file-write* {})",
                filter("subpath", &data_dir.join(DESIGN_BLOBS_DIR))
            ));
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
            //    Other provider accounts' homes (their OAuth credentials) too.
            hidden.push(filter("subpath", &data_dir.join(PROVIDER_ACCOUNTS_DIR)));
            trailing.push(format!("(deny file-read* {})", hidden.join(" ")));
            let accounts = data_dir.join(PROVIDER_ACCOUNTS_DIR);
            let own: Vec<&PathBuf> = reallow
                .iter()
                .filter(|p| p.starts_with(&accounts))
                .collect();
            if !own.is_empty() {
                // 4. …but the session's own account home stays readable, and
                //    the configs in it that make the CLI run code stay
                //    write-denied (the `$HOME` carve-outs, re-rooted).
                let read: Vec<String> = own.iter().map(|p| filter("subpath", p)).collect();
                trailing.push(format!("(allow file-read* {})", read.join(" ")));
                //    `stat` of the accounts dir itself (`mkdir -p` into the own
                //    home walks it); listing it stays denied.
                trailing.push(format!(
                    "(allow file-read-metadata {})",
                    filter("literal", &accounts)
                ));
                let filters: Vec<String> = own
                    .iter()
                    .flat_map(|root| provider_root_filters(root))
                    .collect();
                trailing.push(format!("(deny file-write* {})", filters.join(" ")));
            }
        }
        // The repo's git dir(s): writable for commits, but never the files
        // that pick programs for the daemon's own git to run. AFTER the
        // data-dir block: a session repo under a re-opened work area
        // (`workflow-runs/…`, `otto-runs/…`) must not get its config back.
        let mut git_dirs: Vec<PathBuf> = Vec::new();
        let dot_git = canonicalize_lenient(&cwd.join(".git"));
        if dot_git.exists() {
            git_dirs.push(dot_git);
        }
        for e in extra_writable {
            let e = canonicalize_lenient(e);
            // Only a git dir that exists now: a session that will `git init`
            // its own repo must be able to create `.git` (git init writes
            // `config` and `hooks/`). Residual: a repo the agent creates
            // in-session has agent-written config/hooks until its next spawn
            // or resume, which re-resolves the git dir and denies them; the
            // daemon's own git neutralises hooks/fsmonitor for non-hook verbs
            // (otto-git `GIT_CONFIG_PARAMETERS`) in the meantime.
            if e.exists()
                && (e.file_name().is_some_and(|n| n == ".git") || e.join("HEAD").is_file())
            {
                git_dirs.push(e);
            }
        }
        git_dirs.sort();
        git_dirs.dedup();
        for g in &git_dirs {
            trailing.push(format!(
                "(deny file-write* {})",
                git_dir_filters(g).join(" ")
            ));
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

    /// Harden an agent policy for a **read-only** session (`meta.read_only`,
    /// typically processing untrusted mail / chat input): its own project
    /// folder stays writable, but not the project-scope claude config there
    /// ([`PROJECT_DENY_WRITE_LITERAL`] / [`PROJECT_DENY_WRITE_SUBPATH`]) that
    /// a later, unconfined `claude` in that folder would load. Appended last,
    /// so it wins over the cwd grant.
    pub fn deny_project_agent_config(mut self, cwd: &Path) -> Self {
        let filters: Vec<String> = PROJECT_DENY_WRITE_LITERAL
            .iter()
            .map(|rel| filter("literal", &canonicalize_lenient(&cwd.join(rel))))
            .chain(
                PROJECT_DENY_WRITE_SUBPATH
                    .iter()
                    .map(|rel| filter("subpath", &canonicalize_lenient(&cwd.join(rel)))),
            )
            .collect();
        self.trailing_rules
            .push(format!("(deny file-write* {})", filters.join(" ")));
        self
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

/// Canonicalize a path the way Seatbelt sees it. A path that doesn't exist
/// yet is resolved through its deepest EXISTING ancestor and the missing tail
/// re-appended — otherwise a deny for a not-yet-created `~/.claude/hooks`
/// would name the unresolved path while its parent grant (`~/.claude`, often
/// a dotfiles symlink) is resolved, and the deny would never match. On macOS
/// this also resolves the `/tmp`→`/private/tmp` and `/var`→`/private/var`
/// symlinks the sandbox sees.
fn canonicalize_lenient(p: &Path) -> PathBuf {
    if let Ok(c) = std::fs::canonicalize(p) {
        return c;
    }
    let mut tail: Vec<&std::ffi::OsStr> = Vec::new();
    let mut cur = p;
    while let (Some(parent), Some(name)) = (cur.parent(), cur.file_name()) {
        tail.push(name);
        if let Ok(mut c) = std::fs::canonicalize(parent) {
            for n in tail.iter().rev() {
                c.push(n);
            }
            return c;
        }
        cur = parent;
    }
    p.to_path_buf()
}

/// Write-deny filters for one provider CLI config root (a named account's
/// home): [`PROVIDER_ROOT_DENY_WRITE_LITERAL`] / [`PROVIDER_ROOT_DENY_WRITE_SUBPATH`].
fn provider_root_filters(root: &Path) -> Vec<String> {
    PROVIDER_ROOT_DENY_WRITE_LITERAL
        .iter()
        .map(|f| filter("literal", &canonicalize_lenient(&root.join(f))))
        .chain(
            PROVIDER_ROOT_DENY_WRITE_SUBPATH
                .iter()
                .map(|f| filter("subpath", &canonicalize_lenient(&root.join(f)))),
        )
        .collect()
}

/// Escape a path for inclusion in an SBPL string literal.
fn escape(p: &Path) -> String {
    escape_str(&p.to_string_lossy())
}

fn escape_str(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Regex filters matching `<data_dir>/design/<one segment>/work` and
/// everything below it. Empty (fail closed: no grant) when the path can't be
/// embedded in an SBPL `#"…"` regex literal (non-UTF-8 or a `"`).
fn design_work_filters(data_dir: &Path) -> Vec<String> {
    let Some(d) = data_dir.to_str().filter(|d| !d.contains('"')) else {
        return Vec::new();
    };
    let base = format!("^{}/{DESIGN_WORK_DIR}/[^/]+/work", regex_escape(d));
    vec![
        format!("(regex #\"{base}$\")"),
        format!("(regex #\"{base}/\")"),
    ]
}

/// Write-deny filters for one git dir `g` ([`GIT_DIR_DENY_WRITE_LITERAL`] /
/// [`GIT_DIR_DENY_WRITE_SUBPATH`]), applied to `g` itself, to each linked
/// worktree's admin dir (`g/worktrees/<name>/`) and to each submodule's git
/// dir (`g/modules/**`). When `g` is a worktree's `.git` FILE the literal
/// keeps its `gitdir:` pointer from being rewritten.
fn git_dir_filters(g: &Path) -> Vec<String> {
    let mut out = vec![filter("literal", g)];
    out.extend(
        GIT_DIR_DENY_WRITE_LITERAL
            .iter()
            .map(|f| filter("literal", &g.join(f))),
    );
    out.extend(
        GIT_DIR_DENY_WRITE_SUBPATH
            .iter()
            .map(|f| filter("subpath", &g.join(f))),
    );
    // Nested admin dirs, by pattern. Skipped (the literals above still apply)
    // when the path can't be embedded in a regex literal.
    if let Some(d) = g.to_str().filter(|d| !d.contains('"')) {
        let d = regex_escape(d);
        let files = GIT_DIR_DENY_WRITE_LITERAL
            .iter()
            .map(|f| regex_escape(f))
            .collect::<Vec<_>>()
            .join("|");
        out.push(format!(
            "(regex #\"^{d}/(worktrees/[^/]+|modules/.+)/({files})$\")"
        ));
        out.push(format!(
            "(regex #\"^{d}/(worktrees/[^/]+|modules/.+)/hooks(/|$)\")"
        ));
        let admin = GIT_WORKTREE_ADMIN_DENY_WRITE_LITERAL
            .iter()
            .map(|f| regex_escape(f))
            .collect::<Vec<_>>()
            .join("|");
        out.push(format!("(regex #\"^{d}/worktrees/[^/]+/({admin})$\")"));
    }
    out
}

/// Escape POSIX-regex metacharacters so a literal path matches itself.
fn regex_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if matches!(
            c,
            '\\' | '.' | '+' | '*' | '?' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '^' | '$'
        ) {
            out.push('\\');
        }
        out.push(c);
    }
    out
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
            Path::new("/nonexistent-otto-home/u"),
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
        let own_home = at(
            &sbpl,
            &format!("(subpath \"{data}/provider-accounts/acct1\")"),
        );
        assert!(
            deny < reopen && deny < own_home,
            "re-allows must follow the deny"
        );
        // Never re-opened: the daemon binary, the DB, the secrets.
        assert!(!sbpl.contains(&format!("(subpath \"{data}/bin\")")));
        assert!(!sbpl.contains(&format!(
            "(allow file-write* (subpath \"{data}/provider-accounts\")"
        )));
        // Not readable at all.
        let hidden = at(
            &sbpl,
            &format!("(deny file-read* (literal \"{data}/secrets.json\")"),
        );
        assert!(hidden > deny);
        assert!(sbpl.contains(&format!("(literal \"{data}/secrets.json\")")));
        assert!(sbpl.contains(&format!("(prefix \"{data}/otto.db\")")));
        assert!(sbpl.contains(&format!("(subpath \"{data}/tls\")")));
        // The carve-outs come after every grant, network included.
        assert!(deny > at(&sbpl, "(allow network-outbound)"));
    }

    /// Design-assist turns edit `<data>/design/<artifact>/work/**` in place:
    /// that (and only that) is re-opened, by a per-artifact pattern, and the
    /// blob store is denied again after it.
    #[test]
    fn for_agent_reopens_design_working_copies_but_not_blobs() {
        let data = "/nonexistent-otto-test/Application Support/Otto.v2";
        let sbpl = agent_policy(data).to_sbpl();
        let deny = at(&sbpl, &format!("(deny file-write* (subpath \"{data}\"))"));
        let esc = r"/nonexistent-otto-test/Application Support/Otto\.v2";
        let work = at(&sbpl, &format!("(regex #\"^{esc}/design/[^/]+/work/\")"));
        assert!(sbpl.contains(&format!("(regex #\"^{esc}/design/[^/]+/work$\")")));
        let blobs = at(
            &sbpl,
            &format!("(deny file-write* (subpath \"{data}/design/blobs\"))"),
        );
        assert!(
            deny < work && work < blobs,
            "deny data → allow work → deny blobs"
        );
        // No blanket grant of the design tree.
        assert!(!sbpl.contains(&format!("(subpath \"{data}/design\")")));
        assert_eq!(regex_escape("a.b(c)*"), r"a\.b\(c\)\*");
        // A path that can't be embedded in a regex literal gets no grant.
        assert!(design_work_filters(Path::new("/x\"y")).is_empty());
    }

    #[test]
    fn for_agent_mach_lookup_is_an_allow_list() {
        let sbpl = agent_policy("/nonexistent-otto-test/Otto").to_sbpl();
        assert!(
            !sbpl.contains("(allow mach-lookup)\n"),
            "blanket mach-lookup"
        );
        assert!(sbpl.contains("(global-name \"com.apple.trustd.agent\")"));
        assert!(sbpl.contains("(global-name \"com.apple.SecurityServer\")"));
        for escape_hatch in [
            "com.apple.coreservices.launchservicesd",
            "com.apple.coreservices.appleevents",
            "com.apple.lsd.mapdb",
        ] {
            assert!(
                !sbpl.contains(escape_hatch),
                "{escape_hatch} must not be reachable"
            );
        }
        assert!(sbpl.contains("(deny process-exec (literal \"/bin/launchctl\"))"));
    }

    #[test]
    fn for_agent_protects_configs_that_run_unsandboxed_code() {
        let sbpl = agent_policy("/nonexistent-otto-test/Otto").to_sbpl();
        let grant = at(
            &sbpl,
            "(allow file-write* (subpath \"/nonexistent-otto-home/u/.claude\"))",
        );
        let deny = at(
            &sbpl,
            "(literal \"/nonexistent-otto-home/u/.claude/settings.json\")",
        );
        assert!(
            deny > grant,
            "the settings deny must override the .claude grant"
        );
        assert!(sbpl.contains("(literal \"/nonexistent-otto-home/u/.codex/config.toml\")"));
        assert!(sbpl.contains("(subpath \"/nonexistent-otto-home/u/.config/git\")"));
    }

    /// A sandboxed agent must not plant code the user's OTHER (unsandboxed)
    /// tools run: claude plugins/hooks/agents/commands and gh aliases.
    #[test]
    fn for_agent_denies_claude_extension_points_and_gh_config() {
        let sbpl = agent_policy("/nonexistent-otto-test/Otto").to_sbpl();
        let grant = at(
            &sbpl,
            "(allow file-write* (subpath \"/nonexistent-otto-home/u/.claude\"))",
        );
        for rel in [
            ".claude/plugins",
            ".claude/hooks",
            ".claude/agents",
            ".claude/commands",
            ".config/gh",
        ] {
            let deny = at(
                &sbpl,
                &format!("(subpath \"/nonexistent-otto-home/u/{rel}\")"),
            );
            assert!(deny > grant, "{rel} deny must follow the grant");
        }
        // The CLI's own state stays writable (no deny names it).
        for rel in [".claude/projects", ".claude/todos", ".claude/statsig"] {
            assert!(
                !sbpl.contains(&format!("/nonexistent-otto-home/u/{rel}")),
                "{rel}"
            );
        }
    }

    /// The UI token lives in the app's WebKit localStorage: not readable.
    #[test]
    fn for_agent_hides_the_desktop_app_webkit_storage() {
        let sbpl = agent_policy("/nonexistent-otto-test/Otto").to_sbpl();
        let read_all = at(&sbpl, "(allow file-read*)");
        let deny = at(&sbpl, "(deny file-read* file-write* ");
        assert!(deny > read_all, "the deny must follow the global read");
        for rel in [
            "Library/WebKit/com.otto.app",
            "Library/Application Support/com.otto.app",
            "Library/Caches/com.otto.app",
        ] {
            assert!(
                sbpl[deny..].contains(&format!("(subpath \"/nonexistent-otto-home/u/{rel}\")")),
                "{rel} must be hidden"
            );
        }
        // …and that deny follows the `Library/Caches` write grant too.
        assert!(
            deny > at(
                &sbpl,
                "(subpath \"/nonexistent-otto-home/u/Library/Caches\")"
            )
        );
    }

    /// The daemon's unsandboxed git reads the repo's `.git/config` and runs
    /// its hooks: the agent may commit (objects/refs/index) but not edit
    /// config, commondir or hooks — nor swap the git dir out wholesale.
    #[test]
    fn for_agent_write_denies_git_config_and_hooks() {
        let tmp = std::env::temp_dir().join(format!("otto-sbx-git-{}", std::process::id()));
        let cwd = tmp.join("project");
        let git = cwd.join(".git");
        std::fs::create_dir_all(&git).unwrap();
        std::fs::write(git.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        let cwd = canonicalize_lenient(&cwd);
        let git = canonicalize_lenient(&git);
        let pol = SandboxPolicy::for_agent(
            &cwd,
            Path::new("/nonexistent-otto-home/u"),
            Path::new("/nonexistent-otto-test/Otto"),
            std::slice::from_ref(&git),
            NetworkPolicy::Full,
        );
        let sbpl = pol.to_sbpl();
        let g = git.display().to_string();
        let grant = at(
            &sbpl,
            &format!("(allow file-write* (subpath \"{}\"))", cwd.display()),
        );
        let deny = at(&sbpl, &format!("(deny file-write* (literal \"{g}\")"));
        assert!(deny > grant, "the git-dir deny must follow the cwd grant");
        let rule = &sbpl[deny..sbpl[deny..].find('\n').unwrap() + deny];
        for f in [
            format!("(literal \"{g}/config\")"),
            format!("(literal \"{g}/config.worktree\")"),
            format!("(literal \"{g}/commondir\")"),
            format!("(subpath \"{g}/hooks\")"),
        ] {
            assert!(rule.contains(&f), "missing {f} in {rule}");
        }
        assert!(
            rule.contains("/(worktrees/[^/]+|modules/.+)/(config|config\\.worktree|commondir)$")
        );
        // Commits still work: objects/refs/index are not named.
        for f in ["objects", "refs", "index", "logs"] {
            assert!(
                !rule.contains(&format!("{g}/{f}")),
                "{f} must stay writable"
            );
        }
        // Exactly one rule for the git dir even though it is both cwd/.git and
        // an extra.
        assert_eq!(
            sbpl.matches(&format!("(literal \"{g}/config\")")).count(),
            1
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// S11-04: `~/.claude.json` (user-scope `mcpServers`) and gemini's
    /// settings are write-denied; `~/.config` is no longer granted wholesale.
    #[test]
    fn for_agent_denies_mcp_configs_and_narrows_dot_config() {
        let pol = agent_policy("/nonexistent-otto-test/Otto");
        assert!(!pol
            .writable_roots
            .iter()
            .any(|r| r.ends_with(".claude.json")));
        assert!(!pol
            .writable_roots
            .iter()
            .any(|r| r == Path::new("/nonexistent-otto-home/u/.config")));
        assert!(pol
            .writable_roots
            .iter()
            .any(|r| r == Path::new("/nonexistent-otto-home/u/.config/configstore")));
        let sbpl = pol.to_sbpl();
        assert!(sbpl.contains("(literal \"/nonexistent-otto-home/u/.claude.json\")"));
        assert!(sbpl.contains("(literal \"/nonexistent-otto-home/u/.gemini/settings.json\")"));
    }

    /// S1-03(b) / S11-09: the session's own account home gets the config
    /// carve-outs re-rooted; every account home is read-denied, then the own
    /// one re-opened; logs are hidden.
    #[test]
    fn for_agent_account_home_carve_outs_and_read_denies() {
        let data = "/nonexistent-otto-test/Otto";
        let sbpl = agent_policy(data).to_sbpl();
        let acct = format!("{data}/provider-accounts/acct1");
        let reopen_write = at(&sbpl, &format!("(subpath \"{acct}\")"));
        let deny_read = at(&sbpl, &format!("(subpath \"{data}/provider-accounts\"))"));
        let reopen_read = at(&sbpl, &format!("(allow file-read* (subpath \"{acct}\"))"));
        let deny_cfg = at(&sbpl, &format!("(literal \"{acct}/settings.json\")"));
        assert!(reopen_write < deny_read && deny_read < reopen_read && reopen_read < deny_cfg);
        for f in [
            format!("(literal \"{acct}/.claude.json\")"),
            format!("(literal \"{acct}/config.toml\")"),
            format!("(subpath \"{acct}/hooks\")"),
            format!("(subpath \"{acct}/plugins\")"),
        ] {
            assert!(sbpl.contains(&f), "missing {f}");
        }
        assert!(sbpl.contains(&format!("(subpath \"{data}/logs\")")));
    }

    /// A session repo under a re-opened work area keeps its git carve-outs:
    /// they now come after the data-dir re-allow.
    #[test]
    fn for_agent_git_denies_follow_the_work_area_reallow() {
        let tmp = std::env::temp_dir().join(format!("otto-sbx-wr-{}", std::process::id()));
        let data = tmp.join("Otto");
        let cwd = data.join("workflow-runs/r1");
        std::fs::create_dir_all(cwd.join(".git")).unwrap();
        std::fs::write(cwd.join(".git/HEAD"), "ref: refs/heads/main\n").unwrap();
        let pol = SandboxPolicy::for_agent(
            &cwd,
            Path::new("/nonexistent-otto-home/u"),
            &data,
            &[cwd.join(".git")],
            NetworkPolicy::Full,
        );
        let sbpl = pol.to_sbpl();
        let data = canonicalize_lenient(&data);
        let reallow = at(
            &sbpl,
            &format!("(subpath \"{}/workflow-runs\")", data.display()),
        );
        let git_cfg = at(
            &sbpl,
            &format!(
                "(literal \"{}/workflow-runs/r1/.git/config\")",
                data.display()
            ),
        );
        assert!(git_cfg > reallow);
        assert!(sbpl.contains("/worktrees/[^/]+/(gitdir)$"));
        let _ = std::fs::remove_dir_all(&tmp);
    }

    /// S1-06: a missing path resolves through its deepest existing ancestor.
    #[cfg(unix)]
    #[test]
    fn canonicalize_lenient_resolves_existing_ancestors() {
        let tmp = std::env::temp_dir().join(format!("otto-sbx-canon-{}", std::process::id()));
        let real = tmp.join("real");
        std::fs::create_dir_all(&real).unwrap();
        let link = tmp.join("link");
        let _ = std::fs::remove_file(&link);
        std::os::unix::fs::symlink(&real, &link).unwrap();
        let got = canonicalize_lenient(&link.join("hooks/x.sh"));
        assert_eq!(
            got,
            std::fs::canonicalize(&real).unwrap().join("hooks/x.sh")
        );
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn deny_project_agent_config_appends_last() {
        let pol = agent_policy("/nonexistent-otto-test/Otto")
            .deny_project_agent_config(Path::new("/work/project"));
        let last = pol.trailing_rules.last().unwrap();
        assert!(last.starts_with("(deny file-write* "));
        assert!(last.contains("(literal \"/work/project/.claude/settings.json\")"));
        assert!(last.contains("(literal \"/work/project/.mcp.json\")"));
        assert!(last.contains("(subpath \"/work/project/.claude/hooks\")"));
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
        let home = PathBuf::from("/nonexistent-otto-home/u");
        let data = PathBuf::from("/nonexistent-otto-home/u/.otto/data");
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
