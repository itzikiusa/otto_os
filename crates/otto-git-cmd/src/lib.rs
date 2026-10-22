//! The hardened `git` command builder: every `git` the daemon spawns outside
//! `otto_git::LocalGit` starts here. A leaf crate (tokio `process` only) so a
//! crate that merely runs git does not depend on otto-git.

use std::process::Stdio;
use tokio::process::Command;

/// `GIT_CONFIG_PARAMETERS` every daemon git runs with (otto-git's `LocalGit`
/// explains each key next to its hook-verb variant):
/// - `core.fsmonitor=false` — repo config can't name a monitor program;
/// - `core.hooksPath=/dev/null` — no repo hook runs.
pub const HARDENED_GIT_CONFIG: &str = "'core.fsmonitor=false' 'core.hooksPath=/dev/null'";

/// A `git` command for code OUTSIDE `otto_git::LocalGit` (rev-parse probes,
/// goal-loop evidence, skill-eval scratch repos, plugin clones…) with the same
/// hardening every daemon git gets: repo config can't pick a program for it
/// to run ([`HARDENED_GIT_CONFIG`]: fsmonitor off, hooks off), no pager, no
/// credential prompt, stdin closed, killed when dropped. Prefer `LocalGit`
/// (output cap + timeouts) where it fits; spawning a bare
/// `Command::new("git")` outside otto-git / otto-git-cmd is refused by a guard
/// test (`otto-server` `no_bare_git_spawns`).
pub fn hardened_command() -> Command {
    let mut cmd = Command::new("git");
    harden(cmd.as_std_mut());
    cmd.kill_on_drop(true);
    cmd
}

/// [`hardened_command`] for synchronous callers.
pub fn hardened_std_command() -> std::process::Command {
    let mut cmd = std::process::Command::new("git");
    harden(&mut cmd);
    cmd
}

/// Apply the hardening env (+ closed stdin) to an existing command.
pub fn harden(cmd: &mut std::process::Command) {
    cmd.env("GIT_TERMINAL_PROMPT", "0")
        .env("GIT_CONFIG_PARAMETERS", HARDENED_GIT_CONFIG)
        .env("GIT_PAGER", "cat")
        .stdin(Stdio::null());
    #[cfg(test)]
    test_hermetic(cmd);
}

/// Unit tests run git against throwaway fixtures: the developer's own
/// `~/.gitconfig` / system config (a `url.*.insteadOf` rewrite, a signing
/// key, a default branch) must not leak in — CI has none, so a laptop with,
/// say, a Bitbucket `insteadOf` failed tests CI passes. Callers apply it from
/// their own `#[cfg(test)]` code (a `cfg(test)` here would only cover THIS
/// crate's tests); production spawns keep the user's config.
pub fn test_hermetic(cmd: &mut std::process::Command) {
    let null = if cfg!(windows) { "NUL" } else { "/dev/null" };
    cmd.env("GIT_CONFIG_GLOBAL", null)
        .env("GIT_CONFIG_NOSYSTEM", "1");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env_of(cmd: &std::process::Command, key: &str) -> Option<String> {
        cmd.get_envs()
            .find(|(k, _)| *k == key)
            .and_then(|(_, v)| v.map(|v| v.to_string_lossy().into_owned()))
    }

    #[test]
    fn both_builders_carry_the_hardened_env() {
        let tokio_cmd = hardened_command();
        let std_cmd = hardened_std_command();
        for cmd in [tokio_cmd.as_std(), &std_cmd] {
            assert_eq!(
                env_of(cmd, "GIT_CONFIG_PARAMETERS").as_deref(),
                Some(HARDENED_GIT_CONFIG)
            );
            assert_eq!(env_of(cmd, "GIT_TERMINAL_PROMPT").as_deref(), Some("0"));
            assert_eq!(env_of(cmd, "GIT_PAGER").as_deref(), Some("cat"));
        }
    }

    #[test]
    fn hermetic_drops_user_and_system_config() {
        let mut cmd = std::process::Command::new("git");
        test_hermetic(&mut cmd);
        assert_eq!(env_of(&cmd, "GIT_CONFIG_NOSYSTEM").as_deref(), Some("1"));
        assert!(env_of(&cmd, "GIT_CONFIG_GLOBAL").is_some());
    }
}
