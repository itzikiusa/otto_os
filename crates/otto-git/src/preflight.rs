//! Outgoing push preflight safety gate (`E-09`).
//!
//! Scans commits about to be pushed for:
//! 1. High-confidence secrets (`otto_core::redact::redact_text`, excluding plain emails)
//! 2. Sensitive control-path edits (`.git/`, `.github/workflows/`, `.claude/`, `.ssh/`, `.env`, `*.pem`, `*.key`)
//! 3. Oversized blobs (`> 5 MiB`)
//!
//! Runs strictly on local repository state (no network I/O) and never echoes raw
//! secret values back in findings.

use std::collections::{HashMap, HashSet};
use std::path::Path;

use otto_core::redact::redact_text;
use otto_core::{Error, Result};
use serde::{Deserialize, Serialize};

use crate::local::{LocalGit, SpawnClass};

/// Blobs strictly larger than 5 MiB trigger a `large_blobs` preflight warning.
pub const LARGE_BLOB_THRESHOLD_BYTES: u64 = 5 * 1024 * 1024;
/// Maximum outgoing commits inspected in a single preflight call.
const MAX_COMMITS_SCANNED: usize = 64;
/// Maximum findings emitted per category to keep responses bounded.
const MAX_FINDINGS_PER_BUCKET: usize = 50;

#[derive(Debug, Default, Deserialize)]
pub struct PushPreflightReq {
    #[serde(default)]
    pub remote: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub force: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PreflightFinding {
    pub kind: String,
    pub path: String,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub commit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PushPreflightResp {
    pub ok: bool,
    pub commits_ahead: usize,
    pub secrets: Vec<PreflightFinding>,
    pub control_paths: Vec<PreflightFinding>,
    pub large_blobs: Vec<PreflightFinding>,
}

impl PushPreflightResp {
    pub fn empty() -> Self {
        Self {
            ok: true,
            commits_ahead: 0,
            secrets: Vec::new(),
            control_paths: Vec::new(),
            large_blobs: Vec::new(),
        }
    }
}

/// Classify a repository-relative path as a sensitive control/credential path.
/// Returns a short human-readable reason if sensitive, or `None` otherwise.
pub fn classify_control_path(path: &str) -> Option<&'static str> {
    let norm = path.replace('\\', "/");
    let norm = norm.trim_start_matches("./");
    let lower = norm.to_ascii_lowercase();
    let segments: Vec<&str> = lower.split('/').filter(|s| !s.is_empty()).collect();
    if segments.is_empty() {
        return None;
    }

    if segments.contains(&".git") || segments.last() == Some(&".gitmodules") {
        return Some("git metadata or submodule control path");
    }
    if lower.starts_with(".github/workflows/")
        || segments
            .windows(2)
            .any(|w| w[0] == ".github" && w[1] == "workflows")
    {
        return Some("GitHub Actions workflow definition");
    }
    if segments.contains(&".claude") {
        return Some("Claude agent control directory (.claude/)");
    }
    if segments.contains(&".ssh") {
        return Some("SSH configuration or key directory (.ssh/)");
    }

    let file = *segments.last()?;
    if file == ".env" || file.starts_with(".env.") {
        let is_template = matches!(
            file,
            ".env.example" | ".env.sample" | ".env.template" | ".env.dist"
        );
        if !is_template {
            return Some("environment secret file (.env)");
        }
    }
    if file.ends_with(".pem")
        || file.ends_with(".key")
        || file.ends_with(".p12")
        || file.ends_with(".pfx")
        || matches!(file, "id_rsa" | "id_ed25519" | "id_ecdsa" | "id_dsa")
    {
        return Some("private key or certificate material");
    }
    None
}

fn contains_pem_header(line: &str) -> bool {
    let mut rest = line;
    while let Some(idx) = rest.find("-----BEGIN ") {
        let after = &rest[idx + "-----BEGIN ".len()..];
        if let Some(end) = after.find("-----") {
            let label = &after[..end];
            if !label.is_empty()
                && label.bytes().all(|b| {
                    b.is_ascii_uppercase() || b.is_ascii_digit() || matches!(b, b' ' | b'-')
                })
                && (label.contains("KEY") || label.contains("CERTIFICATE") || label.contains("PGP"))
            {
                return true;
            }
        }
        rest = after;
    }
    false
}

/// Scan unified diff output (`git show --format= --unified=0`) for added lines
/// containing high-confidence secrets, never returning the raw secret value.
pub fn scan_patch_for_secrets(
    patch: &str,
    commit_short: Option<&str>,
    out: &mut Vec<PreflightFinding>,
    seen: &mut HashSet<(String, u32, String)>,
) {
    let mut current_path: Option<String> = None;
    let mut new_line: u32 = 0;

    for raw_line in patch.lines() {
        if out.len() >= MAX_FINDINGS_PER_BUCKET {
            break;
        }
        if let Some(rest) = raw_line.strip_prefix("+++ ") {
            current_path = if rest == "/dev/null" {
                None
            } else {
                Some(
                    rest.strip_prefix("b/")
                        .unwrap_or(rest)
                        .trim_matches('"')
                        .to_string(),
                )
            };
            continue;
        }
        if raw_line.starts_with("@@ ") {
            if let Some(plus_part) = raw_line.split('+').nth(1) {
                let start_str = plus_part
                    .split(|c: char| !c.is_ascii_digit())
                    .next()
                    .unwrap_or("");
                if let Ok(n) = start_str.parse::<u32>() {
                    new_line = n;
                }
            }
            continue;
        }
        if raw_line.starts_with("diff --git ")
            || raw_line.starts_with("--- ")
            || raw_line.starts_with("index ")
        {
            continue;
        }
        if let Some(added) = raw_line.strip_prefix('+') {
            let line_no = new_line;
            new_line = new_line.saturating_add(1);
            let Some(ref path) = current_path else {
                continue;
            };
            // Normalize common inline delimiters (`"`, `'`, `,`, `&`) to spaces
            // so compact JSON (`{"k":"ghp_..."}`), query strings, and function
            // args are tokenized by `redact_text` without breaking PEM headers
            // or `Bearer <tok>` / `KEY=VALUE` shapes.
            let normalized: String = added
                .chars()
                .map(|c| {
                    if matches!(c, '"' | '\'' | ',' | '&') {
                        ' '
                    } else {
                        c
                    }
                })
                .collect();
            let mut hit_kinds: Vec<String> = redact_text(&normalized)
                .hits
                .into_iter()
                .map(|h| h.kind)
                .filter(|k| k != "email")
                .collect();
            if contains_pem_header(added) {
                hit_kinds.push("private_key".into());
            }
            for kind in hit_kinds {
                if out.len() >= MAX_FINDINGS_PER_BUCKET {
                    break;
                }
                let dedupe_key = (path.clone(), line_no, kind.clone());
                if seen.insert(dedupe_key) {
                    out.push(PreflightFinding {
                        kind: "secret".into(),
                        path: path.clone(),
                        detail: format!("potential {kind} secret detected"),
                        commit: commit_short.map(str::to_string),
                        line: Some(line_no),
                        bytes: None,
                    });
                }
            }
        } else if raw_line.starts_with(' ') {
            new_line = new_line.saturating_add(1);
        }
    }
}

fn format_mib(bytes: u64) -> String {
    let mib = (bytes as f64) / (1024.0 * 1024.0);
    format!("{mib:.1} MiB (> 5 MiB)")
}

impl LocalGit {
    /// Run a local-only push preflight scan over outgoing commits.
    pub async fn push_preflight(&self, req: &PushPreflightReq) -> Result<PushPreflightResp> {
        let branch_opt = req
            .branch
            .as_deref()
            .map(str::trim)
            .filter(|b| !b.is_empty());
        if let Some(b) = branch_opt {
            Self::guard_ref(b)?;
        }
        let remote_opt = req
            .remote
            .as_deref()
            .map(str::trim)
            .filter(|r| !r.is_empty());
        if let Some(r) = remote_opt {
            Self::guard_ref(r)?;
        }

        // Determine target ref and branch name.
        let (target_ref, branch_name) = if let Some(b) = branch_opt {
            let full = format!("refs/heads/{b}");
            if self.ref_exists(&full).await {
                (full, Some(b.to_string()))
            } else if self.ref_exists(b).await {
                (b.to_string(), Some(b.to_string()))
            } else {
                return Err(Error::Invalid(format!("branch '{b}' not found")));
            }
        } else {
            let cur = self
                .current_branch()
                .await
                .unwrap_or_else(|_| "HEAD".into());
            let bname = (cur != "HEAD").then_some(cur);
            ("HEAD".to_string(), bname)
        };

        // Empty unborn repository: nothing to push.
        if !self.ref_exists(&target_ref).await {
            return Ok(PushPreflightResp::empty());
        }

        let base_ref = self
            .resolve_preflight_base(remote_opt, branch_name.as_deref(), &target_ref)
            .await;

        let range_arg = match base_ref {
            Some(ref base) => format!("{base}..{target_ref}"),
            None => target_ref.clone(),
        };

        let (count_ok, count_out, _, _) = self
            .run_raw_class(
                &["rev-list", "--count", &range_arg],
                &[],
                SpawnClass::LocalRead,
            )
            .await?;
        let commits_ahead = if count_ok {
            count_out.trim().parse::<usize>().unwrap_or(0)
        } else {
            0
        };
        if commits_ahead == 0 {
            return Ok(PushPreflightResp::empty());
        }

        let max_arg = format!("--max-count={MAX_COMMITS_SCANNED}");
        let (list_ok, list_out, _, _) = self
            .run_raw_class(
                &["rev-list", &max_arg, &range_arg],
                &[],
                SpawnClass::LocalRead,
            )
            .await?;
        if !list_ok {
            return Ok(PushPreflightResp::empty());
        }
        let commits: Vec<String> = list_out
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();

        let mut secrets = Vec::new();
        let mut secret_seen = HashSet::new();
        let mut control_paths = Vec::new();
        let mut control_seen = HashSet::new();
        let mut blob_candidates: Vec<(String, String, String)> = Vec::new(); // (oid, path, short_sha)
        let mut blob_oids_seen = HashSet::new();

        for sha in &commits {
            let short = &sha[..sha.len().min(7)];

            // 1. Inspect touched files and new blob OIDs via diff-tree --raw -z.
            let (dt_ok, dt_out, _, _) = self
                .run_raw_class(
                    &[
                        "diff-tree",
                        "--no-commit-id",
                        "--root",
                        "-r",
                        "--raw",
                        "-z",
                        sha,
                    ],
                    &[],
                    SpawnClass::LocalRead,
                )
                .await?;
            if dt_ok {
                for (status, dst_oid, path) in parse_raw_diff_tree_z(&dt_out) {
                    if status == "D" {
                        continue;
                    }
                    if control_paths.len() < MAX_FINDINGS_PER_BUCKET {
                        if let Some(reason) = classify_control_path(&path) {
                            if control_seen.insert(path.clone()) {
                                control_paths.push(PreflightFinding {
                                    kind: "control_path".into(),
                                    path: path.clone(),
                                    detail: reason.into(),
                                    commit: Some(short.to_string()),
                                    line: None,
                                    bytes: None,
                                });
                            }
                        }
                    }
                    if !dst_oid.chars().all(|c| c == '0') && blob_oids_seen.insert(dst_oid.clone())
                    {
                        blob_candidates.push((dst_oid, path, short.to_string()));
                    }
                }
            }

            // 2. Scan added diff lines for secrets.
            if secrets.len() < MAX_FINDINGS_PER_BUCKET {
                let (show_ok, patch, _, _) = self
                    .run_raw_class(
                        &[
                            "show",
                            "--format=",
                            "--unified=0",
                            "--no-color",
                            "--no-ext-diff",
                            "--no-textconv",
                            sha,
                        ],
                        &[],
                        SpawnClass::LocalRead,
                    )
                    .await?;
                if show_ok {
                    scan_patch_for_secrets(&patch, Some(short), &mut secrets, &mut secret_seen);
                }
            }
        }

        // 3. Batch-check blob sizes via `git cat-file --batch-check`.
        let mut large_blobs = Vec::new();
        if !blob_candidates.is_empty() {
            let sizes = self
                .batch_blob_sizes(blob_candidates.iter().map(|(oid, _, _)| oid.as_str()))
                .await;
            for (oid, path, short) in blob_candidates {
                if large_blobs.len() >= MAX_FINDINGS_PER_BUCKET {
                    break;
                }
                if let Some(&size) = sizes.get(&oid) {
                    if size > LARGE_BLOB_THRESHOLD_BYTES {
                        large_blobs.push(PreflightFinding {
                            kind: "large_blob".into(),
                            path,
                            detail: format_mib(size),
                            commit: Some(short),
                            line: None,
                            bytes: Some(size),
                        });
                    }
                }
            }
        }

        let ok = secrets.is_empty() && control_paths.is_empty() && large_blobs.is_empty();
        Ok(PushPreflightResp {
            ok,
            commits_ahead,
            secrets,
            control_paths,
            large_blobs,
        })
    }

    async fn ref_exists(&self, rev: &str) -> bool {
        matches!(
            self.run_raw_class(
                &["rev-parse", "--verify", "--quiet", rev],
                &[],
                SpawnClass::LocalRead,
            )
            .await,
            Ok((true, _, _, _))
        )
    }

    async fn resolve_preflight_base(
        &self,
        remote: Option<&str>,
        branch: Option<&str>,
        target_ref: &str,
    ) -> Option<String> {
        let mut candidates: Vec<String> = Vec::new();
        if let (Some(r), Some(b)) = (remote, branch) {
            candidates.push(format!("refs/remotes/{r}/{b}"));
        }
        if target_ref == "HEAD" {
            candidates.push("@{upstream}".into());
        } else if let Some(b) = branch {
            candidates.push(format!("{b}@{{upstream}}"));
        }
        let fallback_remote = remote.unwrap_or("origin");
        if let Some(b) = branch {
            candidates.push(format!("refs/remotes/{fallback_remote}/{b}"));
        }
        candidates.push(format!("refs/remotes/{fallback_remote}/HEAD"));
        candidates.push(format!("refs/remotes/{fallback_remote}/main"));
        candidates.push(format!("refs/remotes/{fallback_remote}/master"));

        for cand in candidates {
            if self.ref_exists(&cand).await {
                return Some(cand);
            }
        }
        None
    }

    async fn batch_blob_sizes<'a>(
        &self,
        oids: impl Iterator<Item = &'a str>,
    ) -> HashMap<String, u64> {
        use tokio::io::AsyncWriteExt;
        let input: String = oids.map(|o| format!("{o}\n")).collect();
        if input.is_empty() {
            return HashMap::new();
        }
        let mut cmd = crate::hardened_command();
        cmd.arg("-C")
            .arg(self.path())
            .args([
                "cat-file",
                "--batch-check=%(objectname) %(objecttype) %(objectsize)",
            ])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let Ok(mut child) = cmd.spawn() else {
            return HashMap::new();
        };
        if let Some(mut stdin) = child.stdin.take() {
            let bytes = input.into_bytes();
            tokio::spawn(async move {
                let _ = stdin.write_all(&bytes).await;
                let _ = stdin.shutdown().await;
            });
        }
        let Ok(Ok(out)) =
            tokio::time::timeout(std::time::Duration::from_secs(10), child.wait_with_output())
                .await
        else {
            return HashMap::new();
        };
        if !out.status.success() {
            return HashMap::new();
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let mut sizes = HashMap::new();
        for line in stdout.lines() {
            let mut parts = line.split_whitespace();
            let (Some(oid), Some(ty), Some(sz)) = (parts.next(), parts.next(), parts.next()) else {
                continue;
            };
            if ty == "blob" {
                if let Ok(n) = sz.parse::<u64>() {
                    sizes.insert(oid.to_string(), n);
                }
            }
        }
        sizes
    }
}

/// Parse `git diff-tree --raw -z` NUL-delimited output into `(status_code, dst_oid, path)`.
fn parse_raw_diff_tree_z(raw: &str) -> Vec<(String, String, String)> {
    let mut out = Vec::new();
    let mut parts = raw.split('\0').peekable();
    while let Some(meta) = parts.next() {
        let meta = meta.trim_start_matches('\n');
        if !meta.starts_with(':') {
            continue;
        }
        // Format: `:oldmode newmode oldoid newoid status`
        let fields: Vec<&str> = meta[1..].split_whitespace().collect();
        if fields.len() < 5 {
            continue;
        }
        let dst_oid = fields[3].to_string();
        let status = fields[4];
        let code = status.chars().next().unwrap_or('M').to_string();
        let path = if code == "R" || code == "C" {
            let _old = parts.next();
            parts.next()
        } else {
            parts.next()
        };
        if let Some(p) = path {
            if !p.is_empty() && Path::new(p).components().next().is_some() {
                out.push((code, dst_oid, p.to_string()));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn run_git(dir: &Path, args: &[&str]) {
        let status = Command::new("git")
            .arg("-C")
            .arg(dir)
            .args(args)
            .env("GIT_AUTHOR_NAME", "Test")
            .env("GIT_AUTHOR_EMAIL", "test@example.com")
            .env("GIT_COMMITTER_NAME", "Test")
            .env("GIT_COMMITTER_EMAIL", "test@example.com")
            .status()
            .expect("git");
        assert!(status.success(), "git {args:?} failed");
    }

    #[test]
    fn classify_control_path_detects_sensitive_files_and_ignores_env_examples() {
        assert!(classify_control_path(".github/workflows/release.yml").is_some());
        assert!(classify_control_path(".claude/settings.json").is_some());
        assert!(classify_control_path(".ssh/authorized_keys").is_some());
        assert!(classify_control_path(".git/config").is_some());
        assert!(classify_control_path("infra/.env").is_some());
        assert!(classify_control_path(".env.production").is_some());
        assert!(classify_control_path("certs/server.pem").is_some());
        assert!(classify_control_path("keys/tls.key").is_some());
        assert!(classify_control_path("id_ed25519").is_some());

        assert!(classify_control_path(".env.example").is_none());
        assert!(classify_control_path(".env.sample").is_none());
        assert!(classify_control_path("src/main.rs").is_none());
        assert!(classify_control_path("docs/guide.md").is_none());
    }

    #[test]
    fn scan_patch_for_secrets_detects_tokens_and_pem_without_leaking_values() {
        let fake_gh = format!("ghp_{}", "A".repeat(36));
        let patch = format!(
            "diff --git a/src/cfg.rs b/src/cfg.rs\n\
             --- a/src/cfg.rs\n\
             +++ b/src/cfg.rs\n\
             @@ -1,0 +1,4 @@\n\
             +// Contact dev@example.com\n\
             +const TOKEN: &str = \"{fake_gh}\";\n\
             +const KEY: &str = \"-----BEGIN RSA PRIVATE KEY-----\";\n\
             +const OK: &str = \"hello\";\n"
        );
        let mut findings = Vec::new();
        let mut seen = HashSet::new();
        scan_patch_for_secrets(&patch, Some("abc1234"), &mut findings, &mut seen);
        assert_eq!(findings.len(), 2, "got {findings:?}");
        assert_eq!(findings[0].path, "src/cfg.rs");
        assert_eq!(findings[0].line, Some(2));
        assert!(!findings[0].detail.contains(&fake_gh));
        assert_eq!(findings[1].line, Some(3));
        assert!(findings[1].detail.contains("private_key"));
    }

    #[tokio::test]
    async fn push_preflight_flags_secret_control_path_and_large_blob_in_outgoing_commits() {
        let td = tempfile::tempdir().unwrap();
        let bare = td.path().join("remote.git");
        let work = td.path().join("work");
        std::fs::create_dir_all(&bare).unwrap();
        std::fs::create_dir_all(&work).unwrap();
        run_git(&bare, &["init", "--bare", "-b", "main"]);
        run_git(&work, &["init", "-b", "main"]);
        run_git(&work, &["remote", "add", "origin", bare.to_str().unwrap()]);
        std::fs::write(work.join("README.md"), "# demo\n").unwrap();
        run_git(&work, &["add", "README.md"]);
        run_git(&work, &["commit", "-m", "initial"]);
        run_git(&work, &["push", "-u", "origin", "main"]);

        let git = LocalGit::new(&work);

        // Clean state: nothing ahead -> ok: true.
        let clean = git
            .push_preflight(&PushPreflightReq::default())
            .await
            .unwrap();
        assert!(clean.ok);
        assert_eq!(clean.commits_ahead, 0);

        // Add outgoing commit with a secret, a workflow file, and a >5 MiB blob.
        std::fs::create_dir_all(work.join(".github/workflows")).unwrap();
        std::fs::write(
            work.join(".github/workflows/deploy.yml"),
            "name: deploy\non: push\n",
        )
        .unwrap();
        let fake_tok = format!("ghp_{}", "B".repeat(36));
        std::fs::write(work.join("config.txt"), format!("token={fake_tok}\n")).unwrap();
        std::fs::write(
            work.join("big.bin"),
            vec![0x55u8; (LARGE_BLOB_THRESHOLD_BYTES as usize) + 1024],
        )
        .unwrap();
        run_git(&work, &["add", "."]);
        run_git(&work, &["commit", "-m", "risky changes"]);

        let resp = git
            .push_preflight(&PushPreflightReq::default())
            .await
            .unwrap();
        assert!(!resp.ok);
        assert_eq!(resp.commits_ahead, 1);
        assert_eq!(resp.secrets.len(), 1, "{resp:?}");
        assert_eq!(resp.secrets[0].path, "config.txt");
        assert!(!resp.secrets[0].detail.contains(&fake_tok));
        assert_eq!(resp.control_paths.len(), 1, "{resp:?}");
        assert_eq!(resp.control_paths[0].path, ".github/workflows/deploy.yml");
        assert_eq!(resp.large_blobs.len(), 1, "{resp:?}");
        assert_eq!(resp.large_blobs[0].path, "big.bin");
        assert!(resp.large_blobs[0].bytes.unwrap() > LARGE_BLOB_THRESHOLD_BYTES);
    }
}
