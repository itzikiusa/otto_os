//! Where agent transcripts live (architecture-6-01): the ONE answer the usage
//! tailer, the History index, per-session resolution, improvement evidence and
//! the session manager share, so a new provider root (another Claude config
//! dir, a subscription account kind) is added here once instead of in five
//! places that already disagreed.
//!
//! Precedence, for both [`TranscriptRoots::discover`] (every root the daemon
//! scans) and [`TranscriptRoots::for_session`] (the roots one session's CLI
//! wrote to):
//!   1. `OTTO_TRANSCRIPT_ROOTS=<claude projects>:<codex sessions>` — exactly
//!      those two directories;
//!   2. the isolated e2e daemon ([`isolated`]) — the per-daemon fixture dirs
//!      `<data_dir>/e2e-transcripts/{claude,codex}`; the real home is never
//!      read;
//!   3. the defaults — `~/.claude/projects`, every `~/.claude-*/projects`,
//!      `~/.config/claude/projects`, `$CODEX_HOME/sessions` (else
//!      `~/.codex/sessions`) — plus, for `discover`, each subscription
//!      account's `<data_dir>/provider-accounts/<id>/{projects,sessions}`; for
//!      `for_session`, the session's own account home replaces the defaults.
//!
//! Everything here is blocking filesystem work (a few `read_dir`s): async
//! callers run it off the runtime.

use std::path::{Path, PathBuf};

/// The one `OTTO_E2E` predicate (re-exported so transcript consumers need no
/// direct otto-core import for it).
pub use otto_core::e2e::isolated;

/// `OTTO_TRANSCRIPT_ROOTS` — an explicit `<claude>:<codex>` pair.
pub const ROOTS_ENV: &str = "OTTO_TRANSCRIPT_ROOTS";

/// The isolated daemon's fixture tree (`claude/`, `codex/` beneath it). The
/// usage tailer also uses it as the "home" for its native (Grok/Antigravity)
/// roots, so an e2e daemon has one fixture location, not two.
pub fn isolated_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("e2e-transcripts")
}

/// Why a root is in the set — the usage tailer skips pre-existing history
/// only under the roots it always read ([`RootKind::Default`]); roots added
/// later backfill at their true timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RootKind {
    /// `~/.claude/projects`, the Codex sessions root, an explicit override or
    /// the isolated fixture dir.
    Default,
    /// Another local Claude config dir (`~/.claude-*`, `~/.config/claude`).
    Extra,
    /// A subscription account home under `<data_dir>/provider-accounts`.
    Account,
}

/// One transcript directory.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Root {
    pub dir: PathBuf,
    /// The provider label usage rows carry (`claude`, `claude-work` for
    /// `~/.claude-work`, `codex`).
    pub label: String,
    pub kind: RootKind,
}

impl Root {
    fn new(dir: PathBuf, label: &str, kind: RootKind) -> Self {
        Self {
            dir,
            label: label.to_string(),
            kind,
        }
    }
}

/// Claude project roots and Codex sessions roots, in precedence order (the
/// first of each is the "primary" one).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TranscriptRoots {
    pub claude: Vec<Root>,
    pub codex: Vec<Root>,
}

/// The process-environment inputs to root discovery, captured once so tests
/// (and the usage tailer, which must not read the environment in isolated
/// test daemons) can inject them.
#[derive(Debug, Clone, Default)]
pub struct RootsEnv {
    /// Raw `OTTO_TRANSCRIPT_ROOTS`.
    pub explicit: Option<String>,
    /// [`isolated`].
    pub isolated: bool,
    /// `$CODEX_HOME` (non-empty).
    pub codex_home: Option<PathBuf>,
}

impl RootsEnv {
    pub fn from_process() -> Self {
        Self {
            explicit: std::env::var(ROOTS_ENV)
                .ok()
                .filter(|v| !v.trim().is_empty()),
            isolated: isolated(),
            codex_home: std::env::var_os("CODEX_HOME")
                .filter(|v| !v.is_empty())
                .map(PathBuf::from),
        }
    }

    /// The explicit pair or the isolated fixture dirs, when either applies.
    fn overridden(&self, data_dir: Option<&Path>) -> Option<TranscriptRoots> {
        if let Some(raw) = &self.explicit {
            let mut parts = raw.split(':').map(str::trim).filter(|p| !p.is_empty());
            if let (Some(c), Some(x)) = (parts.next(), parts.next()) {
                return Some(TranscriptRoots::pair(PathBuf::from(c), PathBuf::from(x)));
            }
            tracing::warn!("{ROOTS_ENV} must be `<claude root>:<codex root>`; ignoring");
        }
        if self.isolated {
            // No data dir to anchor the fixture on: an empty set, never the
            // real home.
            let Some(data_dir) = data_dir else {
                return Some(TranscriptRoots::default());
            };
            let base = isolated_dir(data_dir);
            let _ = std::fs::create_dir_all(base.join("claude"));
            let _ = std::fs::create_dir_all(base.join("codex"));
            return Some(TranscriptRoots::pair(
                base.join("claude"),
                base.join("codex"),
            ));
        }
        None
    }

    /// The local defaults under `home` (no account homes). `config_dirs`:
    /// also list every `~/.claude-*/projects` (one `read_dir` of the home) —
    /// the scan wants them; one session's CLI wrote to the default dir.
    fn defaults(&self, home: &Path, config_dirs: bool) -> TranscriptRoots {
        let mut claude = vec![Root::new(
            home.join(".claude").join("projects"),
            "claude",
            RootKind::Default,
        )];
        let listed = if config_dirs {
            read_subdirs(home)
        } else {
            Vec::new()
        };
        let mut extra: Vec<Root> = listed
            .into_iter()
            .filter_map(|dir| {
                let name = dir.file_name()?.to_str()?.to_string();
                let projects = dir.join("projects");
                (name.starts_with(".claude-") && projects.is_dir())
                    .then(|| Root::new(projects, name.trim_start_matches('.'), RootKind::Extra))
            })
            .collect();
        extra.sort_by(|a, b| a.dir.cmp(&b.dir));
        claude.extend(extra);
        claude.push(Root::new(
            home.join(".config").join("claude").join("projects"),
            "claude",
            RootKind::Extra,
        ));
        TranscriptRoots {
            claude,
            codex: vec![Root::new(
                self.default_codex_root(home),
                "codex",
                RootKind::Default,
            )],
        }
    }

    /// `$CODEX_HOME/sessions`, else `<home>/.codex/sessions` — where a Codex
    /// CLI started without a subscription account writes rollouts.
    pub fn default_codex_root(&self, home: &Path) -> PathBuf {
        self.codex_home
            .clone()
            .unwrap_or_else(|| home.join(".codex"))
            .join("sessions")
    }

    /// [`TranscriptRoots::discover`] against these inputs.
    pub fn discover(&self, home: &Path, data_dir: &Path) -> TranscriptRoots {
        if let Some(roots) = self.overridden(Some(data_dir)) {
            return roots;
        }
        let mut roots = self.defaults(home, true);
        let mut accounts = read_subdirs(&data_dir.join("provider-accounts"));
        accounts.sort();
        for account in accounts {
            let (projects, sessions) = (account.join("projects"), account.join("sessions"));
            if projects.is_dir() {
                roots
                    .claude
                    .push(Root::new(projects, "claude", RootKind::Account));
            }
            if sessions.is_dir() {
                roots
                    .codex
                    .push(Root::new(sessions, "codex", RootKind::Account));
            }
        }
        roots.dedup();
        roots
    }

    /// [`TranscriptRoots::for_session`] against these inputs. `accounts_root`
    /// is `<data_dir>/provider-accounts` (None: the caller has no accounts, so
    /// an `account_id` resolves to nothing rather than to the defaults).
    pub fn for_session(
        &self,
        meta: &serde_json::Value,
        home: &Path,
        accounts_root: Option<&Path>,
        data_dir: Option<&Path>,
    ) -> TranscriptRoots {
        if let Some(roots) = self.overridden(data_dir) {
            return roots;
        }
        match meta.get("account_id").and_then(|v| v.as_str()) {
            Some(id) => match (accounts_root, otto_core::paths::safe_component(id)) {
                (Some(root), Some(id)) => {
                    let home = root.join(id);
                    TranscriptRoots {
                        claude: vec![Root::new(
                            home.join("projects"),
                            "claude",
                            RootKind::Account,
                        )],
                        codex: vec![Root::new(home.join("sessions"), "codex", RootKind::Account)],
                    }
                }
                // An invalid id never falls back to another tree.
                _ => TranscriptRoots::default(),
            },
            None => self.defaults(home, false),
        }
    }
}

impl TranscriptRoots {
    /// Every root the daemon scans (usage tailer, History index), from the
    /// process environment. See the module docs for the precedence.
    pub fn discover(home: &Path, data_dir: &Path) -> Self {
        RootsEnv::from_process().discover(home, data_dir)
    }

    /// The roots one session's CLI wrote to: its subscription account home
    /// (`meta.account_id`), else the local defaults (`~/.claude/projects`,
    /// `~/.config/claude/projects`, the Codex sessions root — other accounts
    /// and `~/.claude-*` dirs are never searched; no directory listing), from
    /// the process environment and `$HOME`.
    pub fn for_session(meta: &serde_json::Value, data_dir: &Path) -> Self {
        RootsEnv::from_process().for_session(
            meta,
            &home_dir(),
            Some(&data_dir.join("provider-accounts")),
            Some(data_dir),
        )
    }

    fn pair(claude: PathBuf, codex: PathBuf) -> Self {
        Self {
            claude: vec![Root::new(claude, "claude", RootKind::Default)],
            codex: vec![Root::new(codex, "codex", RootKind::Default)],
        }
    }

    fn dedup(&mut self) {
        for list in [&mut self.claude, &mut self.codex] {
            let mut seen = std::collections::HashSet::new();
            list.retain(|r| seen.insert(r.dir.clone()));
        }
    }

    pub fn claude_dirs(&self) -> impl Iterator<Item = &Path> {
        self.claude.iter().map(|r| r.dir.as_path())
    }

    pub fn codex_dirs(&self) -> impl Iterator<Item = &Path> {
        self.codex.iter().map(|r| r.dir.as_path())
    }

    /// Every directory, Claude roots first.
    pub fn all_dirs(&self) -> impl Iterator<Item = &Path> {
        self.claude_dirs().chain(self.codex_dirs())
    }

    /// The Claude root `file` lives under, if any.
    pub fn claude_root_of(&self, file: &Path) -> Option<&Root> {
        self.claude.iter().find(|r| file.starts_with(&r.dir))
    }
}

/// `$HOME` (the root of `~/.claude`), falling back to `/` when unset.
pub fn home_dir() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// Immediate subdirectories of `dir` (empty on any error).
#[allow(clippy::disallowed_methods)] // sync helper: root discovery is blocking by contract — async callers offload it
fn read_subdirs(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()))
        .map(|e| e.path())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rollout(root: &Path) -> PathBuf {
        let day = root.join("2026/10/23");
        std::fs::create_dir_all(&day).unwrap();
        let file = day.join("rollout-2026-10-23T10-00-00-abc.jsonl");
        std::fs::write(&file, b"{}\n").unwrap();
        file
    }

    fn holds(roots: &[Root], file: &Path) -> bool {
        roots.iter().any(|r| file.starts_with(&r.dir))
    }

    #[test]
    fn codex_rollouts_under_an_account_and_codex_home_are_both_found() {
        let tmp = tempfile::tempdir().unwrap();
        let (home, data) = (tmp.path().join("home"), tmp.path().join("data"));
        let codex_home = tmp.path().join("custom-codex");
        let in_account = rollout(&data.join("provider-accounts/acct-1/sessions"));
        let in_codex_home = rollout(&codex_home.join("sessions"));
        let env = RootsEnv {
            codex_home: Some(codex_home.clone()),
            ..RootsEnv::default()
        };

        let all = env.discover(&home, &data);
        assert!(holds(&all.codex, &in_account), "{all:?}");
        assert!(holds(&all.codex, &in_codex_home), "{all:?}");
        assert_eq!(all.codex[0].kind, RootKind::Default);
        assert!(all.codex.iter().any(|r| r.kind == RootKind::Account));

        let accounts = data.join("provider-accounts");
        let mine = env.for_session(
            &serde_json::json!({"account_id": "acct-1"}),
            &home,
            Some(&accounts),
            Some(&data),
        );
        assert!(holds(&mine.codex, &in_account));
        assert!(!holds(&mine.codex, &in_codex_home), "another tree leaked");
        let local = env.for_session(&serde_json::json!({}), &home, Some(&accounts), Some(&data));
        assert!(holds(&local.codex, &in_codex_home));
        assert!(
            !holds(&local.codex, &in_account),
            "other accounts are not searched"
        );
        // A traversal id resolves to nothing, never to the defaults.
        let bad = env.for_session(
            &serde_json::json!({"account_id": "../x"}),
            &home,
            Some(&accounts),
            Some(&data),
        );
        assert_eq!(bad, TranscriptRoots::default());
    }

    #[test]
    fn claude_config_dirs_and_accounts_are_labelled() {
        let tmp = tempfile::tempdir().unwrap();
        let (home, data) = (tmp.path().join("home"), tmp.path().join("data"));
        std::fs::create_dir_all(home.join(".claude-work/projects")).unwrap();
        std::fs::create_dir_all(home.join(".claude-empty")).unwrap();
        std::fs::create_dir_all(data.join("provider-accounts/a/projects")).unwrap();
        let roots = RootsEnv::default().discover(&home, &data);
        let labels: Vec<_> = roots
            .claude
            .iter()
            .map(|r| (r.label.as_str(), r.kind))
            .collect();
        assert_eq!(
            labels,
            vec![
                ("claude", RootKind::Default),
                ("claude-work", RootKind::Extra),
                ("claude", RootKind::Extra),
                ("claude", RootKind::Account),
            ]
        );
        assert_eq!(roots.claude[0].dir, home.join(".claude/projects"));
        assert_eq!(roots.codex[0].dir, home.join(".codex/sessions"));
    }

    #[test]
    fn the_explicit_pair_wins_over_isolation_and_accounts() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path().join("data");
        std::fs::create_dir_all(data.join("provider-accounts/a/sessions")).unwrap();
        let env = RootsEnv {
            explicit: Some("/a/claude:/b/codex".into()),
            isolated: true,
            codex_home: Some("/c".into()),
        };
        let expect = TranscriptRoots::pair("/a/claude".into(), "/b/codex".into());
        assert_eq!(env.discover(tmp.path(), &data), expect);
        let meta = serde_json::json!({"account_id": "a"});
        assert_eq!(
            env.for_session(&meta, tmp.path(), None, Some(&data)),
            expect
        );
    }

    #[test]
    fn isolated_mode_never_reads_the_real_home() {
        let tmp = tempfile::tempdir().unwrap();
        let (home, data) = (tmp.path().join("home"), tmp.path().join("data"));
        std::fs::create_dir_all(home.join(".claude/projects/-p")).unwrap();
        std::fs::create_dir_all(home.join(".claude-work/projects")).unwrap();
        std::fs::create_dir_all(data.join("provider-accounts/a/projects")).unwrap();
        let env = RootsEnv {
            isolated: true,
            codex_home: Some(home.join(".codex")),
            ..RootsEnv::default()
        };
        let fixture = isolated_dir(&data);
        for roots in [
            env.discover(&home, &data),
            env.for_session(&serde_json::json!({}), &home, None, Some(&data)),
            env.for_session(
                &serde_json::json!({"account_id": "a"}),
                &home,
                None,
                Some(&data),
            ),
        ] {
            assert!(
                roots.all_dirs().all(|d| d.starts_with(&fixture)),
                "{roots:?}"
            );
            assert_eq!(roots.claude.len(), 1);
            assert_eq!(roots.codex.len(), 1);
        }
        assert!(fixture.join("claude").is_dir() && fixture.join("codex").is_dir());
        // No data dir to anchor on: nothing, still not the home.
        let none = env.for_session(&serde_json::json!({}), &home, None, None);
        assert_eq!(none, TranscriptRoots::default());
    }
}
