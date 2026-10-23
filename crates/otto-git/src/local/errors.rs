//! How a failed `git` invocation maps to an [`Error`]: a local refusal is a
//! 409 the user can act on, a rejected push a 409 with a next step, anything
//! else an `Upstream` 502 with an auth hint when credentials failed. Split out
//! of `local.rs`; re-exported there so `crate::local::upstream_err` & co. keep
//! resolving.

use crate::output_noise::is_noise_line;
use otto_core::{Error, Result};

/// `Ok` when a failed `git diff HEAD` only means HEAD is unborn (no commits
/// yet) — the one failure the `Working` target falls back from. Anything else
/// (corrupt index, missing object) is the error, never an empty diff.
pub(crate) fn unborn_or_err(stderr: &str, stdout: &[u8], code: Option<i32>) -> Result<()> {
    // No path: "ambiguous argument 'HEAD': unknown revision"; with `-- <path>`:
    // "bad revision 'HEAD'". A HEAD naming a MISSING object ("bad object
    // HEAD") is corruption, not an empty repository.
    const UNBORN: [&str; 3] = [
        "ambiguous argument 'HEAD'",
        "unknown revision",
        "bad revision 'HEAD'",
    ];
    if UNBORN.iter().any(|m| stderr.contains(m)) {
        return Ok(());
    }
    Err(upstream_err(stderr, &String::from_utf8_lossy(stdout), code))
}

/// True when git's own message says the LOCAL repository refused the operation
/// — a dirty tree, an unconfigured upstream, divergent branches, an unfinished
/// merge. Nothing is wrong with the remote (git usually never even dialled it),
/// so these must NOT surface as `Error::Upstream`/502: the UI reports a 502 as
/// "the git provider is unavailable" and raises a global outage banner, which
/// for "please commit your changes before you merge" is simply untrue.
///
/// Matched on git's stable porcelain wording; anything unrecognised keeps the
/// conservative 502 (a genuine network/auth failure looks like nothing here).
pub(crate) fn local_refusal(msg: &str) -> bool {
    const MARKERS: [&str; 29] = [
        "local changes to the following files would be overwritten",
        "would be overwritten by",
        "please commit your changes or stash them",
        "you have unstaged changes",
        "cannot pull with rebase",
        "need to specify how to reconcile divergent branches",
        "there is no tracking information for the current branch",
        "you have not concluded your merge",
        "fix conflicts and run",
        "refusing to merge unrelated histories",
        "not something we can merge",
        "no such ref was fetched",
        // Unresolved index / mid-operation refusals (merge onto a conflicted
        // index, a second cherry-pick, …).
        "you need to resolve your current index first",
        "you have unmerged files",
        "unmerged files",
        "cherry-pick or revert is already in progress",
        "a rebase is in progress",
        "no rebase in progress",
        // A bad/unknown ref is the caller's input, not a provider outage.
        "did not match any file",
        // `checkout <name> --` of a name that is no revision.
        "invalid reference",
        // Nothing to do — e.g. `commit` with an empty index.
        "nothing to commit",
        // A concurrent git process holds the index lock — transient, retryable.
        "index.lock",
        "another git process seems to be running",
        // `pull --ff-only` on a diverged branch — the caller's choice of mode,
        // not an outage.
        "not possible to fast-forward",
        // Push/pull on a detached HEAD, or pushing an unborn branch (no
        // commit yet) — nothing was sent anywhere.
        "you are not currently on a branch",
        "src refspec",
        // Unborn branch (no first commit) — stash/log have nothing to act on.
        "you do not have the initial commit yet",
        "does not have any commits yet",
        // A conflicted path handed to restore/unstage — resolve it first.
        "is unmerged",
    ];
    let lc = msg.to_ascii_lowercase();
    MARKERS.iter().any(|m| lc.contains(m))
}

/// Stable prefix of the 409 a non-fast-forward push rejection maps to. The UI
/// matches on it to offer "Pull" / "Force push with lease…" instead of a dead
/// end — keep it in sync with `ui/src/modules/git/push-errors.ts`.
pub const PUSH_REJECTED: &str = "push rejected: the remote branch has commits yours doesn't";
/// Stable prefix of the 409 a refused force-with-lease maps to.
pub const PUSH_LEASE_REFUSED: &str = "force push refused: the remote branch moved";

/// Map a failed `git push` to an error the caller can act on. A rejection
/// because the remote has commits we don't (`fetch first` / `non-fast-forward`)
/// or a refused lease (`stale info` / `remote ref updated since checkout`) is
/// a LOCAL-state 409 — nothing is wrong with the provider, and calling it a 502
/// told the user the remote was down when the fix is "pull first". Everything
/// else (auth, network, hook/protected-branch refusals) goes through
/// [`upstream_err`].
pub(crate) fn push_err(stderr: &str, stdout: &str, code: Option<i32>) -> Error {
    let rejected = stderr
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with("! [rejected]"));
    if let Some(line) = rejected {
        if line.contains("(stale info)") || line.contains("(remote ref updated since checkout)") {
            return Error::Conflict(format!(
                "{PUSH_LEASE_REFUSED} since you last fetched and integrated it — fetch, review \
                 the new commits, then try again ({line})"
            ));
        }
        if line.contains("(fetch first)") || line.contains("(non-fast-forward)") {
            return Error::Conflict(format!(
                "{PUSH_REJECTED} — pull to bring them in, then push again ({line})"
            ));
        }
    }
    upstream_err(stderr, stdout, code)
}

/// A one-line next step for a remote op that failed on CREDENTIALS, appended
/// to the error so "fatal: Authentication failed for …" says where to fix it.
/// `None` for anything that isn't recognisably an auth failure.
fn auth_hint(full_lc: &str) -> Option<&'static str> {
    const HTTPS: [&str; 5] = [
        "authentication failed",
        "could not read username",
        "could not read password",
        "the requested url returned error: 401",
        "the requested url returned error: 403",
    ];
    if full_lc.contains("permission denied (publickey")
        || full_lc.contains("host key verification failed")
    {
        return Some(
            "SSH authentication failed — check that your SSH agent has a key this remote accepts \
             (`ssh-add -l`) and that the host is in known_hosts",
        );
    }
    if HTTPS.iter().any(|m| full_lc.contains(m)) {
        return Some(
            "authentication failed — check the repository's git account token (Settings → Git \
             Accounts: Test, expiry and write scope)",
        );
    }
    None
}

/// A `git fetch` failure that only means "the host doesn't publish this
/// ref" (try the next one) — as opposed to auth / network / repo errors.
pub(crate) fn is_missing_remote_ref(stderr: &str) -> bool {
    let lc = stderr.to_ascii_lowercase();
    lc.contains("couldn't find remote ref") || lc.contains("could not find remote ref")
}

pub(crate) fn upstream_err(stderr: &str, stdout: &str, code: Option<i32>) -> Error {
    // Among the meaningful (non-noise) lines, prefer one that actually names the
    // failure — git scatters the real reason ("! [remote rejected] …", "error:
    // failed to push …") after benign chatter like "To <url>".
    let meaningful: Vec<&str> = stderr
        .lines()
        .chain(stdout.lines())
        .map(str::trim)
        .filter(|&l| !is_noise_line(l))
        .collect();
    let pick = meaningful
        .iter()
        .copied()
        .find(|l| {
            let lc = l.to_ascii_lowercase();
            lc.contains("rejected")
                || lc.starts_with("error:")
                || lc.starts_with("fatal:")
                || lc.starts_with("remote:")
        })
        .or_else(|| meaningful.first().copied())
        .unwrap_or("git failed with no output");
    // Classify BEFORE wrapping: a local refusal is a 409 the user can act on
    // ("stash your changes"), not a 502 that accuses the remote of being down.
    // Scan every meaningful line, not just `pick` — git prints the reason
    // ("Please commit your changes…") on a different line from the "error:" it
    // leads with.
    let full = meaningful.join("\n");
    if local_refusal(&full) {
        return Error::Conflict(pick.to_string());
    }
    let hint = auth_hint(&full.to_ascii_lowercase())
        .map(|h| format!(" — {h}"))
        .unwrap_or_default();
    Error::Upstream(format!(
        "git exited {}: {}{hint}",
        code.map_or_else(|| "?".to_string(), |c| c.to_string()),
        pick
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_an_unborn_head_falls_back() {
        let unborn =
            "fatal: ambiguous argument 'HEAD': unknown revision or path not in the working tree.";
        assert!(unborn_or_err(unborn, b"", Some(128)).is_ok());
        assert!(unborn_or_err("fatal: bad revision 'HEAD'", b"", Some(128)).is_ok());
        for broken in ["fatal: bad object HEAD", "fatal: index file corrupt"] {
            let err = unborn_or_err(broken, b"", Some(128)).unwrap_err();
            assert!(
                err.to_string()
                    .contains(broken.trim_start_matches("fatal: ")),
                "{err}"
            );
        }
    }
}
