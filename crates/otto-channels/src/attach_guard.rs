//! Confinement for `⟦otto-file⟧` attachments.
//!
//! An agent's final reply can ask Otto to upload a local file to the chat
//! (`⟦otto-file⟧/abs/path⟦/otto-file⟧`). The *daemon* performs that upload, so
//! without a guard a prompt-injected reply (a Jira ticket, PR body, web page or
//! quoted chat text telling the agent to "attach" a path) could make Otto post
//! `~/Library/Application Support/Otto/secrets.json`, `~/.ssh/id_*` or
//! `~/.aws/credentials` to a shared channel — bypassing the agent's own
//! network-deny sandbox because the egress is Otto's, not the agent's.
//!
//! Policy (checked against the CANONICAL path, so symlinks can't escape):
//! 1. the file must live under an allowed root — the session's working
//!    directory (unless that is `/`, `$HOME` or an ancestor of either `$HOME` or
//!    Otto's data dir), `/tmp`, the daemon's `$TMPDIR`, or one of Otto's own
//!    generated-artifact dirs under the data dir;
//! 2. it must not sit under a protected location (`~/.ssh`, `~/.aws`, the
//!    keychains, Otto's secrets/DB/bin/credential dirs, …), whatever the root;
//! 3. its file name must not look like a credential (`.env`, `*.pem`, `id_rsa`…)
//!    and it must not be inside a `.git` directory;
//! 4. it must be a regular, single-link file no larger than
//!    [`MAX_ATTACHMENT_BYTES`].

use std::path::{Path, PathBuf};

/// Hard cap on one attachment — also bounds the in-memory buffer the upload
/// needs (the whole file is read before it is posted).
pub const MAX_ATTACHMENT_BYTES: u64 = 20 * 1024 * 1024;

/// Sub-dirs of Otto's data dir that hold user-facing generated artifacts
/// (reports, snips, run outputs) and so may be attached.
const DATA_ARTIFACT_DIRS: &[&str] = &["insights", "snips", "workflow-runs", "otto-runs", "canvas"];

/// Entries of Otto's data dir that must never leave the machine.
const DATA_PROTECTED: &[&str] = &[
    "secrets.json",
    "otto.db",
    "otto.db-wal",
    "otto.db-shm",
    "bin",
    "provider-accounts",
    "kube",
    "tls",
    "server",
    "clickhouse",
];

/// Home-relative locations that hold credentials.
const HOME_PROTECTED: &[&str] = &[
    ".ssh",
    ".aws",
    ".gnupg",
    ".kube",
    ".docker",
    ".netrc",
    ".pgpass",
    ".npmrc",
    ".pypirc",
    ".git-credentials",
    ".config/gh",
    ".config/gcloud",
    ".azure",
    ".claude",
    ".codex",
    "Library/Keychains",
    "Library/Cookies",
];

/// The resolved allow/deny sets for one session's attachments.
#[derive(Debug, Clone)]
pub struct AttachmentPolicy {
    roots: Vec<PathBuf>,
    denied: Vec<PathBuf>,
}

/// Canonicalize when the path exists (so `/tmp` → `/private/tmp` on macOS and
/// symlinked homes compare correctly); otherwise keep it verbatim.
fn canon_or_raw(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| p.to_path_buf())
}

/// The daemon's data dir, resolved exactly like `ottod::config::Config::load`.
pub fn otto_data_dir(home: Option<&Path>) -> Option<PathBuf> {
    match std::env::var_os("OTTO_DATA_DIR") {
        Some(d) => Some(PathBuf::from(d)),
        None => home.map(|h| h.join("Library/Application Support/Otto")),
    }
}

impl AttachmentPolicy {
    /// Build the policy for a session whose working directory is `cwd`.
    /// `tmp_roots` are the scratch dirs agents are told to write reports into
    /// (production: `/tmp` + `$TMPDIR`, see [`AttachmentPolicy::for_session`]).
    pub fn build(
        cwd: &str,
        home: Option<&Path>,
        data_dir: Option<&Path>,
        tmp_roots: &[PathBuf],
    ) -> Self {
        let home = home.map(canon_or_raw);
        let data_dir = data_dir.map(canon_or_raw);
        let mut roots: Vec<PathBuf> = Vec::new();

        let cwd = cwd.trim();
        if !cwd.is_empty() {
            let c = canon_or_raw(Path::new(cwd));
            // A session rooted at `/`, `$HOME` (or any ancestor of it / of the
            // data dir) would make every personal file attachable — refuse it
            // as a root; such sessions attach via /tmp instead.
            let too_broad = c.parent().is_none()
                || home.as_deref().is_some_and(|h| h.starts_with(&c))
                || data_dir.as_deref().is_some_and(|d| d.starts_with(&c));
            if c.is_absolute() && !too_broad {
                roots.push(c);
            }
        }
        for t in tmp_roots {
            let t = canon_or_raw(t);
            if t.is_absolute() && t.parent().is_some() {
                roots.push(t);
            }
        }
        let mut denied: Vec<PathBuf> = Vec::new();
        if let Some(d) = &data_dir {
            roots.extend(DATA_ARTIFACT_DIRS.iter().map(|s| d.join(s)));
            denied.extend(DATA_PROTECTED.iter().map(|s| d.join(s)));
        }
        if let Some(h) = &home {
            denied.extend(HOME_PROTECTED.iter().map(|s| h.join(s)));
            // The default data dir is protected even when `$OTTO_DATA_DIR`
            // points elsewhere (a dev/e2e daemon must not leak the real one).
            let default_data = h.join("Library/Application Support/Otto");
            denied.extend(DATA_PROTECTED.iter().map(|s| default_data.join(s)));
        }
        Self { roots, denied }
    }

    /// The production policy: `$HOME`, Otto's data dir, `/tmp` and `$TMPDIR`.
    pub fn for_session(cwd: &str) -> Self {
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let data_dir = otto_data_dir(home.as_deref());
        let tmp = [PathBuf::from("/tmp"), std::env::temp_dir()];
        Self::build(cwd, home.as_deref(), data_dir.as_deref(), &tmp)
    }

    /// Verdict for an already-CANONICAL path (pure; no filesystem access).
    pub fn check_canonical(&self, canon: &Path) -> Result<(), &'static str> {
        if sensitive_file_name(canon) {
            return Err("the file name looks like a credential");
        }
        if self.denied.iter().any(|d| canon.starts_with(d)) {
            return Err("the path is a protected location");
        }
        // Git internals carry remote URLs with embedded tokens and credential
        // helpers' config — never a legitimate report attachment.
        if canon.components().any(|c| c.as_os_str() == ".git") {
            return Err("the path is inside a .git directory");
        }
        if !self.roots.iter().any(|r| canon.starts_with(r)) {
            return Err("the path is outside the session's working directory and /tmp");
        }
        Ok(())
    }

    /// Resolve `raw` (absolute, or relative to the session cwd `cwd`), follow
    /// every symlink, and vet the result. Returns the canonical path + size.
    pub fn vet(&self, raw: &str, cwd: &str) -> Result<(PathBuf, u64), String> {
        let p = Path::new(raw);
        let joined = if p.is_absolute() {
            p.to_path_buf()
        } else {
            Path::new(cwd).join(p)
        };
        let canon = joined
            .canonicalize()
            .map_err(|e| format!("cannot resolve path: {e}"))?;
        // The name the agent typed matters too: a symlink named `id_rsa`
        // pointing at an innocuous file is still refused.
        if sensitive_file_name(&joined) {
            return Err("the file name looks like a credential".into());
        }
        self.check_canonical(&canon).map_err(str::to_string)?;
        let meta = std::fs::metadata(&canon).map_err(|e| format!("cannot stat: {e}"))?;
        if !meta.is_file() {
            return Err("not a regular file".into());
        }
        // A hard link is a second name for a file that may live anywhere on the
        // volume (e.g. `ln ~/.ssh/id_ed25519 /tmp/x`) — canonicalize can't see
        // through it, so refuse multiply-linked files outright.
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            if meta.nlink() > 1 {
                return Err("the file has multiple hard links".into());
            }
        }
        if meta.len() > MAX_ATTACHMENT_BYTES {
            return Err(format!(
                "the file is {} bytes (limit {MAX_ATTACHMENT_BYTES})",
                meta.len()
            ));
        }
        Ok((canon, meta.len()))
    }
}

/// File names that are credentials by convention, whatever directory they sit in.
pub fn sensitive_file_name(p: &Path) -> bool {
    let Some(name) = p.file_name().and_then(|n| n.to_str()) else {
        return false;
    };
    let lower = name.to_ascii_lowercase();
    const EXACT: &[&str] = &[
        "secrets.json",
        "credentials",
        "credentials.json",
        ".netrc",
        ".pgpass",
        ".npmrc",
        ".pypirc",
        ".git-credentials",
        "otto.db",
        "id_rsa",
        "id_dsa",
        "id_ecdsa",
        "id_ed25519",
    ];
    if EXACT.contains(&lower.as_str()) {
        return true;
    }
    if lower == ".env" || lower.starts_with(".env.") {
        return true;
    }
    if lower.starts_with("otto.db-") {
        return true;
    }
    [".pem", ".key", ".p12", ".pfx", ".keystore", ".jks"]
        .iter()
        .any(|ext| lower.ends_with(ext))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn policy(cwd: &str) -> AttachmentPolicy {
        AttachmentPolicy::build(
            cwd,
            Some(Path::new("/nonexistent-home/u")),
            Some(Path::new(
                "/nonexistent-home/u/Library/Application Support/Otto",
            )),
            &[PathBuf::from("/nonexistent-tmp")],
        )
    }

    #[test]
    fn allows_files_under_cwd_tmp_and_artifact_dirs() {
        let p = policy("/nonexistent-home/u/code/repo");
        assert!(p
            .check_canonical(Path::new("/nonexistent-home/u/code/repo/report.md"))
            .is_ok());
        assert!(p
            .check_canonical(Path::new("/nonexistent-tmp/investigation.md"))
            .is_ok());
        assert!(p
            .check_canonical(Path::new(
                "/nonexistent-home/u/Library/Application Support/Otto/insights/r.html"
            ))
            .is_ok());
    }

    #[test]
    fn refuses_secrets_and_paths_outside_roots() {
        let p = policy("/nonexistent-home/u/code/repo");
        for bad in [
            "/nonexistent-home/u/Library/Application Support/Otto/secrets.json",
            "/nonexistent-home/u/Library/Application Support/Otto/otto.db",
            "/nonexistent-home/u/.ssh/id_ed25519",
            "/nonexistent-home/u/.aws/credentials",
            "/nonexistent-home/u/notes.txt",
            "/etc/passwd",
            "/nonexistent-home/u/code/repo/.env",
            "/nonexistent-home/u/code/repo/deploy.pem",
            "/nonexistent-home/u/code/repo/.git/config",
        ] {
            assert!(p.check_canonical(Path::new(bad)).is_err(), "{bad}");
        }
    }

    #[test]
    fn home_or_root_cwd_is_not_a_root() {
        for cwd in ["/nonexistent-home/u", "/nonexistent-home", "/"] {
            let p = policy(cwd);
            assert!(
                p.check_canonical(Path::new("/nonexistent-home/u/notes.txt"))
                    .is_err(),
                "cwd {cwd} must not open up $HOME"
            );
        }
        // …and the data dir's own parent chain is not a root either.
        let p = policy("/nonexistent-home/u/Library/Application Support");
        assert!(p
            .check_canonical(Path::new(
                "/nonexistent-home/u/Library/Application Support/Otto/personal/x.md"
            ))
            .is_err());
    }

    #[test]
    fn protected_dirs_win_even_inside_a_root() {
        // A (weird) session whose cwd is Otto's data dir itself is refused as
        // a root; one inside it still can't reach the protected entries.
        let p = policy("/nonexistent-home/u/Library/Application Support/Otto/personal/a1");
        assert!(p
            .check_canonical(Path::new(
                "/nonexistent-home/u/Library/Application Support/Otto/personal/a1/out.md"
            ))
            .is_ok());
        let p = policy("/nonexistent-home/u/Library/Application Support/Otto");
        assert!(p
            .check_canonical(Path::new(
                "/nonexistent-home/u/Library/Application Support/Otto/bin/aws"
            ))
            .is_err());
    }

    #[test]
    fn sensitive_names() {
        for n in [
            "id_rsa",
            ".env",
            ".env.local",
            "a.PEM",
            "x.p12",
            "secrets.json",
            "otto.db-wal",
        ] {
            assert!(sensitive_file_name(Path::new(n)), "{n}");
        }
        for n in ["report.md", "id_rsa.pub", "environment.md", "keys.txt"] {
            assert!(!sensitive_file_name(Path::new(n)), "{n}");
        }
    }

    /// End-to-end through the filesystem: a symlink inside the session cwd
    /// that points outside it is refused (canonicalization sees through it),
    /// a plain file is accepted, and an oversized/hard-linked one is refused.
    #[cfg(unix)]
    #[test]
    fn vet_follows_symlinks_and_checks_links() {
        let base = std::env::temp_dir().join(format!(
            "otto-attach-guard-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let cwd = base.join("repo");
        let outside = base.join("outside");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let secret = outside.join("token.txt");
        std::fs::write(&secret, b"s3cret").unwrap();
        let ok_file = cwd.join("report.md");
        std::fs::write(&ok_file, b"# report").unwrap();
        std::os::unix::fs::symlink(&secret, cwd.join("innocent.md")).unwrap();
        std::fs::hard_link(&secret, cwd.join("hard.md")).unwrap();

        // No tmp roots: only the cwd is allowed.
        let p = AttachmentPolicy::build(cwd.to_str().unwrap(), None, None, &[]);
        let cwd_s = cwd.to_str().unwrap();
        assert!(p.vet(ok_file.to_str().unwrap(), cwd_s).is_ok());
        assert!(p.vet("report.md", cwd_s).is_ok(), "relative to cwd");
        assert!(p
            .vet(cwd.join("innocent.md").to_str().unwrap(), cwd_s)
            .is_err());
        assert!(p.vet(cwd.join("hard.md").to_str().unwrap(), cwd_s).is_err());
        assert!(p.vet(secret.to_str().unwrap(), cwd_s).is_err());
        assert!(p
            .vet(cwd.join("missing.md").to_str().unwrap(), cwd_s)
            .is_err());
        assert!(p.vet("../outside/token.txt", cwd_s).is_err());

        let _ = std::fs::remove_dir_all(&base);
    }
}
