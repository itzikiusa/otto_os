//! THE per-provider path table: for every agent CLI Otto launches, the state
//! it may write under the user's home, the config / extension points there
//! that no sandboxed session may write (they load into every later session,
//! sandboxed or not), and the project-level config a **read-only** session
//! must not write into its own folder (the person's next session in that
//! folder loads it — Otto pre-trusts every session cwd).
//!
//! [`crate::SandboxPolicy::for_agent`], [`crate::SandboxPolicy::deny_project_agent_config`]
//! derive their lists from [`PROVIDERS`](crate::providers::PROVIDERS) (plus the few cross-CLI entries in
//! [`SHARED`](crate::providers::SHARED)), so adding a provider means adding one entry here — and
//! otto-sessions' `every_model_family_has_a_sandbox_entry` test fails until
//! every family in its provider list has one.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// What a table path is relative to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Base {
    /// `$HOME`.
    Home,
    /// `$XDG_CONFIG_HOME` (default `~/.config`).
    XdgConfig,
    /// `$XDG_DATA_HOME` (default `~/.local/share`).
    XdgData,
    /// `$XDG_STATE_HOME` (default `~/.local/state`).
    XdgState,
}

/// Which direct children (top-level files) of a CLI state home are writable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TopFiles {
    /// Any direct child (`history.jsonl`, sqlite stores, a token refresh),
    /// minus the denied files and scripts.
    Any,
    /// Only these names (and the temp siblings an atomic rewrite of them
    /// uses): deny by default.
    Only(&'static [&'static str]),
}

/// One CLI home and the state subdirectories in it the CLI may write.
#[derive(Debug, Clone, Copy)]
pub struct StateHome {
    pub base: Base,
    pub rel: &'static str,
    /// Writable subdirectories (recursive). An allow-list: extension points
    /// (skills, plugins, hooks, rules, downloaded binaries) are never in it.
    pub dirs: &'static [&'static str],
    pub top: TopFiles,
}

/// One provider family's sandbox entry.
#[derive(Debug, Clone, Copy)]
pub struct ProviderPaths {
    /// The Otto provider families (`MODEL_FAMILIES` names) this entry covers.
    pub families: &'static [&'static str],
    pub state: &'static [StateHome],
    /// Files never writable by any sandboxed session.
    pub home_deny_literal: &'static [(Base, &'static str)],
    /// Folders never writable by any sandboxed session.
    pub home_deny_subpath: &'static [(Base, &'static str)],
    /// Project files (relative to the session cwd) a read-only session must
    /// not write.
    pub project_deny_literal: &'static [&'static str],
    pub project_deny_subpath: &'static [&'static str],
}

use Base::{Home, XdgConfig, XdgData, XdgState};

/// Entries no single provider owns: tools every CLI shells out to and files
/// several CLIs read.
pub static SHARED: ProviderPaths = ProviderPaths {
    families: &[],
    state: &[],
    home_deny_literal: &[],
    home_deny_subpath: &[
        (Home, ".config/git"),
        // gh aliases (`!shell` aliases run on the user's next `gh …`) + hosts.
        (Home, ".config/gh"),
        // Otto's own context bundles: every claude session is launched with
        // `--settings ~/.otto/context/claude/<cwd>/settings.json` (hooks).
        (Home, ".otto/context"),
    ],
    // Instruction files codex, agy, opencode, pi and Muse all obey.
    project_deny_literal: &["AGENTS.md", "AGENTS.override.md"],
    project_deny_subpath: &[],
};

/// Every agent CLI Otto knows. `home_deny_*` entries apply to EVERY sandboxed
/// session; `project_deny_*` only to read-only ones (an ordinary session
/// legitimately edits its repo's checked-in config).
pub const PROVIDERS: &[ProviderPaths] = &[
    ProviderPaths {
        families: &["claude"],
        state: &[StateHome {
            base: Home,
            rel: ".claude",
            dirs: &[
                "projects",
                "todos",
                "statsig",
                "shell-snapshots",
                "session-env",
                "ide",
                "file-history",
                "sessions",
                "logs",
                "debug",
                "plans",
                "paste-cache",
                "cache",
                "backups",
                "feedback",
                "jobs",
                "state",
                "telemetry",
                // The background-session daemon's control socket dir + key (a
                // runtime handshake, never loaded as code; readable anyway).
                "daemon",
                // NOT `downloads`: the native installer stages the next claude
                // binary there — code every later session runs.
            ],
            top: TopFiles::Any,
        }],
        // Hooks (`settings*.json`), user/project-scope `mcpServers`
        // (`~/.claude.json` — Claude Code tolerates it read-only; Otto's trust
        // pre-seeding is written by the daemon, unsandboxed) and the user
        // instruction file (under skip-permissions an instruction is code).
        home_deny_literal: &[
            (Home, ".claude.json"),
            (Home, ".claude/settings.json"),
            (Home, ".claude/settings.local.json"),
            (Home, ".claude/CLAUDE.md"),
        ],
        // Plugins (with their hooks), hook scripts, subagents, slash commands.
        home_deny_subpath: &[
            (Home, ".claude/plugins"),
            (Home, ".claude/hooks"),
            (Home, ".claude/agents"),
            (Home, ".claude/commands"),
            (Home, ".claude/skills"),
            (Home, ".claude/output-styles"),
        ],
        project_deny_literal: &[
            ".claude/settings.json",
            ".claude/settings.local.json",
            ".mcp.json",
            "CLAUDE.md",
            "CLAUDE.local.md",
            ".claude/CLAUDE.md",
        ],
        project_deny_subpath: &[
            ".claude/hooks",
            ".claude/agents",
            ".claude/commands",
            ".claude/skills",
            ".claude/output-styles",
            ".claude/plugins",
        ],
    },
    ProviderPaths {
        families: &["codex"],
        state: &[StateHome {
            base: Home,
            rel: ".codex",
            dirs: &[
                "sessions",
                "archived_sessions",
                "shell_snapshots",
                "log",
                "sqlite",
                "tmp",
                ".tmp",
                "cache",
                "ipc",
                "node_repl",
                "process_manager",
                "thread-writer-locks",
                "rollout-migrations",
                "generated_images",
                "ambient-suggestions",
                // The curated-skills listing cache (JSON metadata; installing a
                // skill still writes `skills/`, which stays denied).
                "vendor_imports",
                // NOT `packages` (codex's own standalone binaries) nor
                // `computer-use` (ships the Computer Use .app bundle): code.
            ],
            top: TopFiles::Any,
        }],
        // `notify` + MCP servers, and the user instruction files.
        home_deny_literal: &[
            (Home, ".codex/config.toml"),
            (Home, ".codex/AGENTS.md"),
            (Home, ".codex/AGENTS.override.md"),
        ],
        // Prompts / skills / plugins / exec-policy rules / its own binaries.
        home_deny_subpath: &[
            (Home, ".codex/prompts"),
            (Home, ".codex/skills"),
            (Home, ".codex/plugins"),
            (Home, ".codex/rules"),
            (Home, ".codex/packages"),
        ],
        project_deny_literal: &[],
        project_deny_subpath: &[".codex"],
    },
    ProviderPaths {
        // agy (Antigravity) keeps its home under `~/.gemini` as Gemini CLI does.
        families: &["gemini", "agy"],
        state: &[StateHome {
            base: Home,
            rel: ".gemini",
            dirs: &[
                "antigravity-cli",
                "tmp",
                "history",
                // Per-project state only — `config/` itself holds
                // `mcp_config.json`, whose `mcpServers` every gemini/agy starts.
                "config/projects",
            ],
            top: TopFiles::Any,
        }],
        home_deny_literal: &[(Home, ".gemini/settings.json"), (Home, ".gemini/GEMINI.md")],
        // Extensions declare `mcpServers` that every gemini/agy starts.
        home_deny_subpath: &[(Home, ".gemini/extensions"), (Home, ".gemini/skills")],
        project_deny_literal: &["GEMINI.md"],
        // `.agents/` holds agy's project MCP servers (`mcp_config.json`), its
        // skills, rules and workflows; `.agent/` is the older spelling.
        project_deny_subpath: &[".gemini", ".agents", ".agent"],
    },
    ProviderPaths {
        families: &["grok"],
        // Grok Build writes its conversations (`sessions/**/updates.jsonl`,
        // which the usage tailer reads) and logs; top-level files are its
        // login / state.
        state: &[StateHome {
            base: Home,
            rel: ".grok",
            dirs: &["sessions", "logs"],
            top: TopFiles::Any,
        }],
        // `config.toml` (`[mcp_servers]`, permission mode), the folder-trust
        // grants, the user instruction file.
        home_deny_literal: &[
            (Home, ".grok/config.toml"),
            (Home, ".grok/trusted_folders.toml"),
            (Home, ".grok/AGENTS.md"),
        ],
        home_deny_subpath: &[
            (Home, ".grok/skills"),
            (Home, ".grok/plugins"),
            (Home, ".grok/hooks"),
            (Home, ".grok/agents"),
            (Home, ".grok/commands"),
        ],
        project_deny_literal: &[],
        // `.grok/config.toml` is grok's project MCP config.
        project_deny_subpath: &[".grok"],
    },
    ProviderPaths {
        families: &["opencode"],
        // opencode keeps sessions / messages / snapshots under
        // `$XDG_DATA_HOME/opencode` (`auth.json` and its sqlite store at the
        // top) and small UI state under `$XDG_STATE_HOME/opencode`. NOT
        // `bin/` (the LSP servers / ripgrep it downloads and runs).
        state: &[
            StateHome {
                base: XdgData,
                rel: "opencode",
                dirs: &["storage", "log", "snapshot"],
                top: TopFiles::Any,
            },
            StateHome {
                base: XdgState,
                rel: "opencode",
                dirs: &[],
                top: TopFiles::Any,
            },
        ],
        home_deny_literal: &[],
        // The whole config home: `opencode.json` (`mcp`, plugins), agents,
        // commands, modes, plugins, tools — opencode never writes it itself.
        home_deny_subpath: &[(XdgConfig, "opencode"), (XdgData, "opencode/bin")],
        project_deny_literal: &["opencode.json", "opencode.jsonc"],
        project_deny_subpath: &[".opencode"],
    },
    ProviderPaths {
        families: &["pi"],
        // pi's sessions (`--session-id` resumes from them) and top-level
        // state (`auth.json` refreshes).
        state: &[StateHome {
            base: Home,
            rel: ".pi/agent",
            dirs: &["sessions"],
            top: TopFiles::Any,
        }],
        // Settings (packages / extensions), MCP servers, custom model
        // endpoints and the system / instruction prompts.
        home_deny_literal: &[
            (Home, ".pi/agent/settings.json"),
            (Home, ".pi/agent/mcp.json"),
            (Home, ".pi/agent/models.json"),
            (Home, ".pi/agent/AGENTS.md"),
            (Home, ".pi/agent/SYSTEM.md"),
            (Home, ".pi/agent/APPEND_SYSTEM.md"),
        ],
        home_deny_subpath: &[
            (Home, ".pi/agent/extensions"),
            (Home, ".pi/agent/skills"),
            (Home, ".pi/agent/prompts"),
        ],
        project_deny_literal: &[],
        // `.pi/` holds the project MCP file, settings, extensions, skills.
        project_deny_subpath: &[".pi"],
    },
    ProviderPaths {
        families: &["muse"],
        state: &[
            // The config home is deny-by-default: only a login's token
            // refresh (`auth.json`) is written there.
            StateHome {
                base: XdgConfig,
                rel: "muse",
                dirs: &[],
                top: TopFiles::Only(&["auth.json"]),
            },
            // Session logs, index and runtime sockets — NOT `plugins/` or
            // `skills/` (code every later session loads).
            StateHome {
                base: XdgData,
                rel: "muse",
                dirs: &["sessions", "session-server", "runtime", "local-tracing"],
                top: TopFiles::Any,
            },
        ],
        // Its settings (`mcpServers`, permission profiles, hooks) and rules.
        home_deny_literal: &[
            (XdgConfig, "muse/settings.json"),
            (XdgConfig, "muse/AGENTS.md"),
        ],
        home_deny_subpath: &[
            (XdgConfig, "muse/skills"),
            (XdgConfig, "muse/plugins"),
            (XdgConfig, "muse/hooks"),
            (XdgConfig, "muse/rules"),
            (XdgData, "muse/plugins"),
            (XdgData, "muse/skills"),
        ],
        project_deny_literal: &[],
        // `--trust-workspace` loads the folder's skills and rules.
        project_deny_subpath: &[".muse"],
    },
];

/// True when `family` (a `MODEL_FAMILIES` name) has a sandbox entry.
pub fn covers(family: &str) -> bool {
    PROVIDERS.iter().any(|p| p.families.contains(&family))
}

/// The base directories the table resolves against, from the daemon's own
/// environment (the same `$XDG_CONFIG_HOME` the Skills Lab honours).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bases {
    pub home: PathBuf,
    pub config: PathBuf,
    pub data: PathBuf,
    pub state: PathBuf,
}

impl Bases {
    pub fn from_env(home: &Path) -> Self {
        Self::with_env(home, std::env::var_os)
    }

    /// `$XDG_*` when set to an absolute path (a relative one is invalid per
    /// the spec and ignored), else the `$HOME` default.
    pub fn with_env(home: &Path, env: impl Fn(&'static str) -> Option<OsString>) -> Self {
        let pick = |key: &'static str, default: &str| {
            env(key)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(default))
        };
        Self {
            home: home.to_path_buf(),
            config: pick("XDG_CONFIG_HOME", ".config"),
            data: pick("XDG_DATA_HOME", ".local/share"),
            state: pick("XDG_STATE_HOME", ".local/state"),
        }
    }

    /// The defaults alone (what a CLI started without the `$XDG_*` vars uses).
    pub fn defaults(home: &Path) -> Self {
        Self::with_env(home, |_| None)
    }

    pub fn resolve(&self, base: Base, rel: &str) -> PathBuf {
        match base {
            Home => self.home.join(rel),
            XdgConfig => self.config.join(rel),
            XdgData => self.data.join(rel),
            XdgState => self.state.join(rel),
        }
    }
}

/// Every entry: the shared one first, then each provider.
pub fn all() -> impl Iterator<Item = &'static ProviderPaths> {
    std::iter::once(&SHARED).chain(PROVIDERS.iter())
}

/// Paths of `entries` under `bases`, and — for an XDG-relative entry whose
/// variable moved it — under the `$HOME` default too, so a CLI the person
/// starts without the variable is protected as well.
pub(crate) fn deny_paths(
    entries: &[(Base, &'static str)],
    bases: &Bases,
    defaults: &Bases,
) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for (base, rel) in entries {
        for p in [bases.resolve(*base, rel), defaults.resolve(*base, rel)] {
            if !out.contains(&p) {
                out.push(p);
            }
        }
    }
    out
}
