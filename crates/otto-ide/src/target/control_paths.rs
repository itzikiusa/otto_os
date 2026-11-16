//! Control paths: files the IDE must not write for a caller who is not the
//! machine owner, because something OTHER than the editor later executes them
//! as the OS user, unsandboxed and without a review step.
//!
//! - git's own directory (`.git/config` `core.sshCommand` / `filter.*` /
//!   `credential.helper` run on Otto's next fetch or status; `.git/hooks/*` on
//!   the owner's next commit) and `.gitattributes` / `.gitmodules` (they wire
//!   those filters and submodule URLs into every checkout);
//! - the project agent config the owner's next unsandboxed agent CLI in that
//!   folder obeys — the SAME per-provider list otto-sandbox write-denies for
//!   read-only sessions (`project_deny_literal` / `project_deny_subpath` of
//!   [`otto_sandbox::providers::all`]), so the sandbox and the IDE never
//!   drift apart.
//!
//! Matching runs on the RESOLVED (canonical, symlinks followed) path, without
//! case (APFS is case-insensitive), over every component — so a nested repo
//! (`root/sub/.git/hooks`) or a committed `tools -> .git` symlink is caught.
//!
//! Who it applies to: every non-root caller, and every collab guest even in a
//! root host's room ([`guest_control_path_denial`] adds what only a guest is
//! refused). Root writing these through its own IDE session is ordinary work
//! (editing `.gitattributes`, a `CLAUDE.md`) and stays allowed.

use std::path::{Component, Path};

use otto_core::{Error, Result};

/// What only a collab guest is refused on top of [`control_path_denial`]:
/// instruction files and agent homes (also when a future sandbox list drops
/// them), and gitignored executables or launch configs the HOST's next Run,
/// Debug or shell executes — none of them shows up in the diff the host
/// reviews.
const GUEST_DENY_LITERAL: &[&str] = &[
    "CLAUDE.md",
    "CLAUDE.local.md",
    "AGENTS.md",
    "AGENTS.override.md",
    "GEMINI.md",
    ".mcp.json",
    ".envrc",
    ".vscode/launch.json",
    ".vscode/tasks.json",
    "gradlew",
    "gradlew.bat",
    "mvnw",
    "mvnw.cmd",
];
const GUEST_DENY_SUBPATH: &[&str] = &[
    ".claude",
    ".agents",
    ".codex",
    ".gemini",
    ".husky",
    ".venv/bin",
    "venv/bin",
    "node_modules/.bin",
    ".mvn/wrapper",
    "gradle/wrapper",
    ".idea/runConfigurations",
];

/// Lower-cased normal components of `p` (the root `/` and `.`/`..` dropped —
/// callers pass canonical paths, which have none of the latter).
fn lower_components(p: &Path) -> Vec<String> {
    p.components()
        .filter_map(|c| match c {
            Component::Normal(n) => Some(n.to_string_lossy().to_ascii_lowercase()),
            _ => None,
        })
        .collect()
}

fn pattern(rel: &str) -> Vec<String> {
    rel.split('/')
        .filter(|s| !s.is_empty())
        .map(str::to_ascii_lowercase)
        .collect()
}

/// `comps` names `literal` (its trailing components equal the pattern).
fn ends_with_literal(comps: &[String], literal: &str) -> bool {
    let pat = pattern(literal);
    !pat.is_empty() && comps.ends_with(&pat)
}

/// `comps` is `subpath` or lies under it, anywhere along the path.
fn within_subpath(comps: &[String], subpath: &str) -> bool {
    let pat = pattern(subpath);
    !pat.is_empty() && comps.windows(pat.len()).any(|w| w == pat.as_slice())
}

/// A directory git would use as its `$GIT_DIR` (bare repo, `--separate-git-dir`
/// target, submodule store): it has `HEAD`, `objects/` and `refs/`. Catches a
/// git dir that is not named `.git`, the way otto-git's `confined_worktree_file`
/// resolves `git rev-parse --git-dir` — without spawning git per save.
fn looks_like_git_dir(dir: &Path) -> bool {
    dir.join("HEAD").is_file() && dir.join("objects").is_dir() && dir.join("refs").is_dir()
}

/// Why `canonical` may not be written by a non-root caller, or `None`.
pub fn control_path_denial(canonical: &Path) -> Option<String> {
    control_path_denial_at(canonical, true)
}

/// [`control_path_denial`] for a path that may live on ANOTHER host (`local`
/// = false): the git-dir-by-shape probe stats the local disk, so it is
/// skipped for remote paths.
pub fn control_path_denial_at(canonical: &Path, local: bool) -> Option<String> {
    let comps = lower_components(canonical);
    let shown = canonical.display();
    if comps.iter().any(|c| c == ".git") {
        return Some(format!(
            "{shown} is inside a git directory; only the root user can change git's own files"
        ));
    }
    if let Some(name) = comps.last() {
        if name == ".gitattributes" || name == ".gitmodules" {
            return Some(format!(
                "{shown} configures git filters or submodules; only the root user can change it"
            ));
        }
    }
    if local && canonical.is_absolute() && canonical.ancestors().skip(1).any(looks_like_git_dir) {
        return Some(format!(
            "{shown} is inside a git directory; only the root user can change git's own files"
        ));
    }
    let agent_cfg = otto_sandbox::providers::all().any(|p| {
        p.project_deny_literal
            .iter()
            .any(|l| ends_with_literal(&comps, l))
            || p.project_deny_subpath
                .iter()
                .any(|s| within_subpath(&comps, s))
    });
    agent_cfg.then(|| {
        format!(
            "{shown} is agent configuration the owner's next agent session runs; \
             only the root user can change it"
        )
    })
}

/// [`control_path_denial`] plus what only an invited collab guest is refused.
pub fn guest_control_path_denial(canonical: &Path) -> Option<String> {
    guest_control_path_denial_at(canonical, true)
}

/// [`guest_control_path_denial`] for a local or remote path (see
/// [`control_path_denial_at`]).
pub fn guest_control_path_denial_at(canonical: &Path, local: bool) -> Option<String> {
    if let Some(why) = control_path_denial_at(canonical, local) {
        return Some(why);
    }
    let comps = lower_components(canonical);
    let hit = GUEST_DENY_LITERAL
        .iter()
        .any(|l| ends_with_literal(&comps, l))
        || GUEST_DENY_SUBPATH.iter().any(|s| within_subpath(&comps, s));
    hit.then(|| {
        format!(
            "{} is run or obeyed by the host's tools; guests cannot save it",
            canonical.display()
        )
    })
}

/// The write-side check every local IDE write funnels through: secrets for a
/// non-root caller are refused by `enforce_non_root_path`; this adds the
/// control paths. Root is unaffected.
pub(crate) fn enforce_control_path(canonical: &Path, is_root_user: bool) -> Result<()> {
    if is_root_user {
        return Ok(());
    }
    match control_path_denial(canonical) {
        Some(why) => Err(Error::Forbidden(why)),
        None => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn git_dir_and_git_wiring_files_are_control_paths() {
        for p in [
            "/w/.git/config",
            "/w/.git/hooks/pre-commit",
            "/w/.GIT/HOOKS/post-checkout",
            "/w/sub/.git/config",
            "/w/.git",
            "/w/.gitattributes",
            "/w/sub/.GitModules",
        ] {
            assert!(control_path_denial(Path::new(p)).is_some(), "{p}");
        }
        for p in ["/w/src/main.go", "/w/.github/workflows/ci.yml", "/w/git.md"] {
            assert!(control_path_denial(Path::new(p)).is_none(), "{p}");
        }
    }

    #[test]
    fn sandbox_agent_config_list_is_shared() {
        for provider in otto_sandbox::providers::all() {
            for l in provider.project_deny_literal {
                let p = PathBuf::from("/w/repo").join(l);
                assert!(control_path_denial(&p).is_some(), "{}", p.display());
            }
            for s in provider.project_deny_subpath {
                let p = PathBuf::from("/w/repo").join(s).join("x.json");
                assert!(control_path_denial(&p).is_some(), "{}", p.display());
            }
        }
        assert!(control_path_denial(Path::new("/w/repo/.Claude/Settings.json")).is_some());
    }

    #[test]
    fn a_git_dir_not_named_dot_git_is_found_by_shape() {
        let d = tempfile::tempdir().unwrap();
        let bare = d.path().join("store.git");
        std::fs::create_dir_all(bare.join("objects")).unwrap();
        std::fs::create_dir_all(bare.join("refs")).unwrap();
        std::fs::write(bare.join("HEAD"), "ref: refs/heads/main\n").unwrap();
        assert!(control_path_denial(&bare.join("config")).is_some());
        assert!(control_path_denial(&bare.join("hooks/pre-commit")).is_some());
        assert!(control_path_denial(&d.path().join("src/lib.rs")).is_none());
        // A REMOTE path is never judged by the local disk's shape.
        assert!(control_path_denial_at(&bare.join("config"), false).is_none());
        assert!(control_path_denial_at(Path::new("/srv/r/.git/config"), false).is_some());
    }

    #[test]
    fn guests_are_also_refused_instruction_files_and_host_executables() {
        for p in [
            "/w/CLAUDE.local.md",
            "/w/AGENTS.md",
            "/w/.agents/notes.md",
            "/w/.venv/bin/python",
            "/w/web/node_modules/.bin/vite",
            "/w/gradlew",
            "/w/.vscode/launch.json",
            "/w/.envrc",
        ] {
            assert!(guest_control_path_denial(Path::new(p)).is_some(), "{p}");
        }
        assert!(guest_control_path_denial(Path::new("/w/src/app.py")).is_none());
        assert!(guest_control_path_denial(Path::new("/w/.venv/lib/site.py")).is_none());
    }

    #[test]
    fn root_is_not_restricted() {
        assert!(enforce_control_path(Path::new("/w/.git/config"), true).is_ok());
        assert!(enforce_control_path(Path::new("/w/.git/config"), false).is_err());
    }
}
