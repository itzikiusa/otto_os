//! Read-only textual working diffs for previews and evaluator prompts.
//! Every child and the aggregate result have bounded output; the real index is
//! never refreshed or staged, including when the repository has no commits.

use crate::local::{GitCmd, LocalGit};
use otto_core::{Error, Result};

impl LocalGit {
    /// Tracked staged + unstaged changes against HEAD, then untracked patches.
    /// Ignore rules are honored. The byte limit, 64 KiB filename window and
    /// 128 untracked-file ceiling all report incomplete output via `truncated`.
    pub async fn working_diff_with_untracked_capped(&self, max: usize) -> Result<(String, bool)> {
        let max = max.min(128 * 1024 * 1024);
        if max == 0 {
            return Ok((String::new(), true));
        }
        let base = if self.verify_commit_ref("HEAD").await {
            "HEAD".to_string()
        } else {
            // No -w: compute the repository-format empty tree ID without
            // creating objects or modifying its index (also supports SHA-256).
            let bytes = self
                .exec(
                    &GitCmd::read(&["hash-object", "-t", "tree", "--stdin"]),
                    Some(&[]),
                )
                .await?;
            if !bytes.0 {
                return Err(Error::Upstream(bytes.2));
            }
            String::from_utf8_lossy(&bytes.1).trim().to_string()
        };
        let (mut out, mut cut) = self
            .exec_truncated(
                &GitCmd::diff("diff")
                    .args(["--end-of-options", &base, "--"])
                    .truncate_stdout(max),
                None,
            )
            .await?;
        if !cut {
            let (names, names_cut) = self
                .exec_truncated(
                    &GitCmd::read(&["ls-files", "-z", "--others", "--exclude-standard"])
                        .truncate_stdout(64 * 1024),
                    None,
                )
                .await?;
            cut |= names_cut;
            // split_inclusive keeps an incomplete final pathname from becoming
            // a different file when the filename window ends mid-record.
            for (index, name) in names.split_inclusive(|b| *b == 0).enumerate() {
                if index == 128 || name.last() != Some(&0) || out.len() >= max {
                    cut = true;
                    break;
                }
                let Ok(file) = std::str::from_utf8(&name[..name.len() - 1]) else {
                    cut = true;
                    continue;
                };
                let remaining = max - out.len();
                let (ok, patch, error, code) = self
                    .exec(
                        &GitCmd::diff("diff")
                            .args(["--no-index"])
                            .paths(["/dev/null", file])
                            .truncate_stdout(remaining + 1),
                        None,
                    )
                    .await?;
                // --no-index uses exit 1 for a valid difference. Other failures
                // must not masquerade as a complete empty implementation.
                if !ok && code != Some(1) {
                    return Err(Error::Upstream(error));
                }
                cut |= patch.len() > remaining;
                out.extend_from_slice(&patch[..patch.len().min(remaining)]);
            }
        }
        let mut text = String::from_utf8_lossy(&out).into_owned();
        // Lossy decoding may expand a partial UTF-8 sequence at the boundary.
        if text.len() > max {
            let mut end = max;
            while !text.is_char_boundary(end) {
                end -= 1;
            }
            text.truncate(end);
            cut = true;
        }
        Ok((text, cut))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn git(dir: &Path, args: &[&str]) -> String {
        let out = crate::hardened_std_command()
            .current_dir(dir)
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().into()
    }

    #[tokio::test]
    async fn preview_preserves_index_and_includes_staged_unstaged_and_new_files() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        git(dir.path(), &["config", "user.name", "Fixture"]);
        git(
            dir.path(),
            &["config", "user.email", "fixture@example.invalid"],
        );
        git(dir.path(), &["config", "commit.gpgsign", "false"]);
        let write = |name: &str, body: &str| std::fs::write(dir.path().join(name), body).unwrap();
        write("tracked.txt", "base\n");
        write(".gitignore", "ignored.txt\n");
        git(dir.path(), &["add", "."]);
        git(dir.path(), &["commit", "-qm", "fixture"]);
        write("tracked.txt", "staged\n");
        git(dir.path(), &["add", "tracked.txt"]);
        write("tracked.txt", "unstaged newest\n");
        write("new file.txt", "new content\n");
        write("ignored.txt", "must be ignored\n");
        let index_path = dir.path().join(".git/index");
        let index = std::fs::read(&index_path).unwrap();
        let staged = git(dir.path(), &["diff", "--cached"]);
        let (diff, cut) = LocalGit::new(dir.path())
            .working_diff_with_untracked_capped(200 * 1024)
            .await
            .unwrap();
        assert!(!cut);
        assert!(diff.contains("+unstaged newest"));
        assert!(diff.contains("+new content"));
        assert!(!diff.contains("must be ignored"));
        assert_eq!(std::fs::read(&index_path).unwrap(), index);
        assert_eq!(git(dir.path(), &["diff", "--cached"]), staged);
        // A huge untracked patch is bounded before materialization and marks
        // the preview incomplete instead of silently returning full evidence.
        write("big.txt", &"א".repeat(200_000));
        let (diff, cut) = LocalGit::new(dir.path())
            .working_diff_with_untracked_capped(1025)
            .await
            .unwrap();
        assert!(cut);
        assert!(diff.len() <= 1025);
        assert_eq!(std::fs::read(index_path).unwrap(), index);
    }

    #[tokio::test]
    async fn preview_handles_unborn_head_without_staging_new_files() {
        let dir = tempfile::tempdir().unwrap();
        git(dir.path(), &["init", "-q"]);
        std::fs::write(dir.path().join("new.txt"), "new\n").unwrap();
        let (diff, cut) = LocalGit::new(dir.path())
            .working_diff_with_untracked_capped(4096)
            .await
            .unwrap();
        assert!(!cut);
        assert!(diff.contains("+new"));
        assert!(!dir.path().join(".git/index").exists());
    }
}
