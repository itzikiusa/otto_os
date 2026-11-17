//! Git-ignored marks for one Project-tree listing (GoLand dims ignored files): ONE
//! `git check-ignore --stdin -z` per listed directory, never one git per entry. Git itself
//! applies nested `.gitignore`s, `!` negations, `.git/info/exclude`, the global excludes
//! file and ignored parent folders, and skips tracked files. Not a repo (exit 128), a
//! timeout or a spawn failure leaves every entry unmarked: the tree never fails over it.

use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt;

use crate::types::IdeFsEntry;

const CHECK_IGNORE_TIMEOUT: Duration = Duration::from_secs(2);

/// Set `ignored` on the entries of `dir` (all direct children of it) that git ignores.
pub(super) async fn mark_git_ignored(dir: &Path, entries: &mut [IdeFsEntry]) {
    if entries.is_empty() {
        return;
    }
    let mut cmd = otto_git_cmd::hardened_command();
    #[cfg(test)]
    otto_git_cmd::test_hermetic(cmd.as_std_mut());
    cmd.env("GIT_OPTIONAL_LOCKS", "0")
        .current_dir(dir)
        .args(["check-ignore", "--stdin", "-z"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut input = Vec::new();
    for e in entries.iter() {
        input.extend_from_slice(e.name.as_bytes());
        input.push(0);
    }
    let run = async {
        let mut child = cmd.spawn().ok()?;
        let mut stdin = child.stdin.take()?;
        // Write while git reads (a pipe buffer is smaller than a big folder's names).
        let feed = async move {
            let _ = stdin.write_all(&input).await;
        };
        let (_, out) = tokio::join!(feed, child.wait_with_output());
        // 0: some ignored, 1: none, 128: not a repo / error.
        let out = out.ok()?;
        (out.status.code() == Some(0)).then_some(out.stdout)
    };
    let Ok(Some(stdout)) = tokio::time::timeout(CHECK_IGNORE_TIMEOUT, run).await else {
        return;
    };
    let ignored: std::collections::HashSet<&[u8]> = stdout
        .split(|b| *b == 0)
        .filter(|s| !s.is_empty())
        .collect();
    for e in entries.iter_mut() {
        e.ignored = ignored.contains(e.name.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::IdeFsEntryKind;

    async fn git(dir: &Path, args: &[&str]) {
        let mut cmd = otto_git_cmd::hardened_command();
        otto_git_cmd::test_hermetic(cmd.as_std_mut());
        let st = cmd.current_dir(dir).args(args).status().await.unwrap();
        assert!(st.success(), "git {args:?}");
    }

    fn entry(dir: &Path, name: &str) -> IdeFsEntry {
        let path = dir.join(name);
        IdeFsEntry {
            name: name.into(),
            kind: if path.is_dir() {
                IdeFsEntryKind::Dir
            } else {
                IdeFsEntryKind::File
            },
            path: path.to_string_lossy().into_owned(),
            size: 0,
            mtime: None,
            git_status: None,
            language: None,
            ignored: false,
        }
    }

    async fn ignored_names(dir: &Path, names: &[&str]) -> Vec<String> {
        let mut entries: Vec<_> = names.iter().map(|n| entry(dir, n)).collect();
        mark_git_ignored(dir, &mut entries).await;
        entries
            .into_iter()
            .filter(|e| e.ignored)
            .map(|e| e.name)
            .collect()
    }

    #[tokio::test]
    async fn one_batch_marks_nested_negated_dir_and_spaced_entries() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        git(root, &["init", "-q"]).await;
        std::fs::write(root.join(".gitignore"), "*.iml\nout/\n*.log\n!keep.log\n").unwrap();
        for d in ["out", "sub/build", "sub/src"] {
            std::fs::create_dir_all(root.join(d)).unwrap();
        }
        std::fs::write(root.join("sub/.gitignore"), "build\nlocal notes.txt\n").unwrap();
        for f in [
            "app.iml",
            "my run.log",
            "keep.log",
            "main.go",
            "tracked.log",
            "out/a.class",
            "sub/build/z",
            "sub/local notes.txt",
            "sub/src/x.go",
        ] {
            std::fs::write(root.join(f), "x").unwrap();
        }
        git(root, &["add", "-f", "tracked.log"]).await;

        let top = ignored_names(
            root,
            &[
                ".git",
                ".gitignore",
                "app.iml",
                "keep.log",
                "main.go",
                "my run.log",
                "out",
                "sub",
                "tracked.log",
            ],
        )
        .await;
        // `!keep.log` re-includes it, a tracked file is never "ignored", `.git` isn't listed.
        assert_eq!(top, ["app.iml", "my run.log", "out"]);

        // The nested `.gitignore` applies in its folder (an ignored dir, a spaced name).
        let sub = root.join("sub");
        let nested = ignored_names(&sub, &[".gitignore", "build", "local notes.txt", "src"]).await;
        assert_eq!(nested, ["build", "local notes.txt"]);

        // Inside an ignored folder every entry is ignored.
        assert_eq!(
            ignored_names(&root.join("out"), &["a.class"]).await,
            ["a.class"]
        );
        // Nothing ignored here: exit 1, nothing marked.
        assert!(ignored_names(&sub.join("src"), &["x.go"]).await.is_empty());
    }

    #[tokio::test]
    async fn outside_a_repo_nothing_is_marked() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("a.iml"), "x").unwrap();
        std::fs::write(tmp.path().join(".gitignore"), "*.iml\n").unwrap();
        assert!(ignored_names(tmp.path(), &["a.iml"]).await.is_empty());
    }
}
