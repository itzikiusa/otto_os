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
}

impl SandboxPolicy {
    /// Build the default policy for an **agent** session: confine writes to the
    /// workspace `cwd`, the resolved git dir(s) in `extra_writable` (so commits
    /// in a worktree still work), the agent CLIs' own config/cache dirs under
    /// `home`, the Otto `data_dir`, and the system temp dirs. Reads stay global.
    pub fn for_agent(
        cwd: &Path,
        home: &Path,
        data_dir: &Path,
        extra_writable: &[PathBuf],
        network: NetworkPolicy,
    ) -> Self {
        let mut roots: Vec<PathBuf> = Vec::new();
        let mut push = |p: PathBuf| {
            if !p.as_os_str().is_empty() {
                roots.push(canonicalize_lenient(&p));
            }
        };

        push(cwd.to_path_buf());
        for e in extra_writable {
            push(e.clone());
        }
        push(data_dir.to_path_buf());

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

        Self {
            writable_roots: roots,
            deny_read: Vec::new(),
            network,
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
        p.push_str("(allow mach-lookup)\n");
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
            p.push_str(&format!("(allow file-write* (subpath \"{}\"))\n", escape(r)));
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
    p.to_string_lossy().replace('\\', "\\\\").replace('"', "\\\"")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(roots: &[&str], net: NetworkPolicy) -> SandboxPolicy {
        SandboxPolicy {
            writable_roots: roots.iter().map(PathBuf::from).collect(),
            deny_read: Vec::new(),
            network: net,
        }
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
    fn for_agent_includes_cwd_git_and_agent_dirs() {
        let cwd = PathBuf::from("/work/project");
        let home = PathBuf::from("/home/u");
        let data = PathBuf::from("/home/u/.otto/data");
        let gitdir = PathBuf::from("/work/project/.git");
        let pol =
            SandboxPolicy::for_agent(&cwd, &home, &data, std::slice::from_ref(&gitdir), NetworkPolicy::Full);
        // cwd, git dir, and an agent config dir are all writable.
        assert!(pol.writable_roots.iter().any(|r| r.ends_with("project")));
        assert!(pol
            .writable_roots
            .iter()
            .any(|r| r.ends_with(".claude")));
        // network policy is carried through.
        assert_eq!(pol.network, NetworkPolicy::Full);
    }
}
