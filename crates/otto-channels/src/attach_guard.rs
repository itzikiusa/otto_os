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
    tmp_roots: Vec<PathBuf>,
    denied: Vec<PathBuf>,
}

/// Filename/dir prefixes of Otto's own inbound channel downloads in global temp
/// dirs (`$TMPDIR` / `/tmp`). Refused so a prompt-injected agent in one
/// workspace cannot exfiltrate another workspace's downloaded Slack/Telegram
/// attachments or thread transcripts via `⟦otto-file⟧`.
const TEMP_CHANNEL_PREFIXES: &[&str] = &["otto-slack-", "otto-telegram-"];

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
        let mut canonical_tmp_roots: Vec<PathBuf> = Vec::new();
        for t in tmp_roots {
            let t = canon_or_raw(t);
            if t.is_absolute() && t.parent().is_some() {
                roots.push(t.clone());
                canonical_tmp_roots.push(t);
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
        Self {
            roots,
            tmp_roots: canonical_tmp_roots,
            denied,
        }
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
        // Global `/tmp` and `$TMPDIR` hold inbound channel downloads from ALL
        // workspaces (`otto-slack-*`, `otto-telegram-*`). Refuse them here so
        // one workspace's session cannot attach another workspace's files.
        if self.tmp_roots.iter().any(|t| {
            canon.strip_prefix(t).ok().is_some_and(|rel| {
                rel.components().any(|c| {
                    let lower = c.as_os_str().to_string_lossy().to_ascii_lowercase();
                    TEMP_CHANNEL_PREFIXES.iter().any(|p| lower.starts_with(p))
                })
            })
        }) {
            return Err("channel temp downloads cannot be attached");
        }
        if !self.roots.iter().any(|r| canon.starts_with(r)) {
            return Err("the path is outside the session's working directory and /tmp");
        }
        Ok(())
    }

    /// Resolve `raw` (absolute, or relative to the session cwd `cwd`), follow
    /// every symlink, and vet the result. Returns the canonical path + size.
    pub fn vet(&self, raw: &str, cwd: &str) -> Result<(PathBuf, u64), String> {
        let (canon, meta) = self.vet_meta(raw, cwd)?;
        Ok((canon, meta.len()))
    }

    /// Vet `raw` and open the resolved canonical path without following a
    /// swapped-in symlink (`O_NOFOLLOW | O_NONBLOCK` on Unix), then re-verify
    /// via `fstat` that the open descriptor is still the exact same regular,
    /// single-link file within [`MAX_ATTACHMENT_BYTES`].
    pub fn open_vetted(&self, raw: &str, cwd: &str) -> Result<(PathBuf, std::fs::File), String> {
        let (canon, pre_meta) = self.vet_meta(raw, cwd)?;
        #[cfg(unix)]
        let expected_id = {
            use std::os::unix::fs::MetadataExt;
            Some((pre_meta.dev(), pre_meta.ino()))
        };
        #[cfg(not(unix))]
        let expected_id: Option<(u64, u64)> = None;
        let file = open_canonical_nofollow(&canon, expected_id)?;
        Ok((canon, file))
    }

    fn vet_meta(&self, raw: &str, cwd: &str) -> Result<(PathBuf, std::fs::Metadata), String> {
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
        validate_regular_meta(&meta)?;
        Ok((canon, meta))
    }
}

fn validate_regular_meta(meta: &std::fs::Metadata) -> Result<(), String> {
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
    Ok(())
}

/// Open a vetted canonical path with `O_NOFOLLOW | O_NONBLOCK` (Unix) and
/// re-check `fstat` metadata (regular file, `nlink <= 1`, size cap, and same
/// `(dev, ino)` as the pre-open `stat`) to close the vet→open TOCTOU window.
fn open_canonical_nofollow(
    canon: &Path,
    expected_dev_ino: Option<(u64, u64)>,
) -> Result<std::fs::File, String> {
    let mut opts = std::fs::OpenOptions::new();
    opts.read(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.custom_flags(
            (rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32,
        );
    }
    let file = opts
        .open(canon)
        .map_err(|e| format!("cannot open vetted file: {e}"))?;
    let fd_meta = file
        .metadata()
        .map_err(|e| format!("cannot fstat vetted file: {e}"))?;
    validate_regular_meta(&fd_meta)?;
    #[cfg(unix)]
    if let Some((dev, ino)) = expected_dev_ino {
        use std::os::unix::fs::MetadataExt;
        if fd_meta.dev() != dev || fd_meta.ino() != ino {
            return Err("the file changed between vetting and opening".into());
        }
    }
    #[cfg(not(unix))]
    let _ = expected_dev_ino;
    Ok(file)
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

    #[test]
    fn refuses_channel_temp_downloads_in_global_tmp() {
        let p = policy("/nonexistent-home/u/code/repo");
        // Ordinary temp reports remain allowed.
        assert!(p
            .check_canonical(Path::new("/nonexistent-tmp/investigation.md"))
            .is_ok());
        // Other workspaces' Slack/Telegram downloads and transcripts in global
        // temp must be rejected.
        for bad in [
            "/nonexistent-tmp/otto-slack-F123-payroll.pdf",
            "/nonexistent-tmp/otto-slack-thread-C1-123.md",
            "/nonexistent-tmp/otto-telegram-999-photo.jpg",
            "/nonexistent-tmp/OTTO-SLACK-UPPER.txt",
            "/nonexistent-tmp/otto-slack-dir/nested.txt",
        ] {
            assert!(p.check_canonical(Path::new(bad)).is_err(), "{bad}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn open_vetted_and_nofollow_block_symlink_and_inode_swap() {
        use std::io::Read as _;
        let base =
            std::env::temp_dir().join(format!("otto-attach-nofollow-{}", otto_core::new_id()));
        let cwd = base.join("repo");
        let tmp = base.join("tmp");
        std::fs::create_dir_all(&cwd).unwrap();
        std::fs::create_dir_all(&tmp).unwrap();

        let ok_file = cwd.join("report.md");
        std::fs::write(&ok_file, b"# ok").unwrap();
        let other_tmp_dl = tmp.join("otto-slack-secret.txt");
        std::fs::write(&other_tmp_dl, b"other-workspace").unwrap();
        let sym = cwd.join("swapped.md");
        std::os::unix::fs::symlink(&ok_file, &sym).unwrap();
        let sym_to_tmp_dl = cwd.join("leak.md");
        std::os::unix::fs::symlink(&other_tmp_dl, &sym_to_tmp_dl).unwrap();

        let p = AttachmentPolicy::build(
            cwd.to_str().unwrap(),
            None,
            None,
            std::slice::from_ref(&tmp),
        );
        let cwd_s = cwd.to_str().unwrap();

        // Normal file opens and reads cleanly.
        let (canon, mut f) = p.open_vetted("report.md", cwd_s).unwrap();
        assert_eq!(canon, ok_file.canonicalize().unwrap());
        let mut s = String::new();
        f.read_to_string(&mut s).unwrap();
        assert_eq!(s, "# ok");

        // Symlink to another workspace's temp download is rejected.
        assert!(p.open_vetted("leak.md", cwd_s).is_err());
        assert!(p
            .open_vetted(other_tmp_dl.to_str().unwrap(), cwd_s)
            .is_err());

        // Direct O_NOFOLLOW open rejects a symlink swapped into the canonical path.
        assert!(open_canonical_nofollow(&sym, None).is_err());

        // Wrong expected (dev, ino) rejects a file swapped between vet and open.
        assert!(open_canonical_nofollow(&canon, Some((0, 0))).is_err());

        let _ = std::fs::remove_dir_all(&base);
    }
}
