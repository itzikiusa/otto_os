//! Real-git tests for the `/diff` surface: summary mode (`--raw --numstat
//! -z`), per-file `path`/`old_path` requests, the caps/budget, three-dot
//! ranges — each checked against what the uncapped full diff says.

use std::path::{Path, PathBuf};

use otto_core::api::{DiffResp, FileChangeStatus, FileDiff};

use crate::local::{DiffOpts, DiffTarget, LocalGit};
use crate::parse::DiffCaps;

fn sh_git(dir: &Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {:?} failed: {}",
        args,
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().to_string()
}

fn write(dir: &Path, rel: &str, content: &[u8]) {
    let p = dir.join(rel);
    if let Some(parent) = p.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(p, content).unwrap();
}

fn lines(prefix: &str, n: usize) -> String {
    (0..n).map(|i| format!("{prefix} line {i}\n")).collect()
}

fn init() -> (tempfile::TempDir, PathBuf) {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("repo");
    std::fs::create_dir(&dir).unwrap();
    sh_git(&dir, &["init", "-q", "-b", "main"]);
    sh_git(&dir, &["config", "user.email", "otto@test.local"]);
    sh_git(&dir, &["config", "user.name", "Otto Test"]);
    sh_git(&dir, &["config", "commit.gpgsign", "false"]);
    (tmp, dir)
}

/// A commit exercising every summary edge: a rename with edits whose names
/// carry spaces, unicode and a tab; a binary change; a delete; an add; a
/// symlink → file type change; a plain modification.
fn edge_repo() -> (tempfile::TempDir, PathBuf, String) {
    let (tmp, dir) = init();
    write(&dir, "sp ace/old name.txt", lines("mv", 40).as_bytes());
    write(&dir, "bin.dat", b"\x00\x01\x02base");
    write(&dir, "gone.rs", b"fn gone() {}\n");
    write(&dir, "plain.rs", lines("plain", 10).as_bytes());
    write(&dir, "caf\u{e9}.md", b"one\n");
    std::os::unix::fs::symlink("plain.rs", dir.join("link")).unwrap();
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-q", "-m", "base"]);

    std::fs::remove_file(dir.join("sp ace/old name.txt")).unwrap();
    let mut moved = lines("mv", 40);
    moved.push_str("appended after move\n");
    write(&dir, "n\u{fc}w dir/new\tname.txt", moved.as_bytes());
    write(&dir, "bin.dat", b"\x00\x01\x02changed");
    std::fs::remove_file(dir.join("gone.rs")).unwrap();
    write(&dir, "fresh file.ts", b"a\nb\nc\n");
    let mut plain = lines("plain", 10);
    plain = plain.replacen("plain line 3\n", "PLAIN LINE 3\n", 1);
    write(&dir, "plain.rs", plain.as_bytes());
    write(&dir, "caf\u{e9}.md", b"one\ntwo\n");
    std::fs::remove_file(dir.join("link")).unwrap();
    write(&dir, "link", b"now a file\n");
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-q", "-m", "edges"]);
    let head = sh_git(&dir, &["rev-parse", "HEAD"]);
    (tmp, dir, head)
}

fn by_path<'a>(d: &'a DiffResp, p: &str) -> &'a FileDiff {
    d.files
        .iter()
        .find(|f| f.path == p)
        .unwrap_or_else(|| panic!("{p} missing from {:?}", paths(d)))
}

fn paths(d: &DiffResp) -> Vec<&str> {
    d.files.iter().map(|f| f.path.as_str()).collect()
}

fn summary() -> DiffOpts {
    DiffOpts {
        summary: true,
        ..DiffOpts::default()
    }
}

#[tokio::test]
async fn summary_covers_renames_binary_and_odd_names_without_a_patch() {
    let (_tmp, dir, head) = edge_repo();
    let git = LocalGit::new(&dir);
    let target = DiffTarget::Commit(head.clone());
    let s = git.diff_with(&target, &summary()).await.unwrap();

    assert!(s.files.iter().all(|f| f.hunks.is_empty()));
    assert!(s.files.iter().all(|f| f.hunks_omitted == Some(true)));

    let ren = by_path(&s, "n\u{fc}w dir/new\tname.txt");
    assert_eq!(ren.status, Some(FileChangeStatus::Renamed));
    assert_eq!(ren.old_path.as_deref(), Some("sp ace/old name.txt"));
    assert_eq!((ren.added, ren.deleted), (Some(1), Some(0)));

    let bin = by_path(&s, "bin.dat");
    assert!(bin.is_binary);
    assert_eq!((bin.added, bin.deleted), (None, None));

    let gone = by_path(&s, "gone.rs");
    assert_eq!(gone.status, Some(FileChangeStatus::Deleted));
    assert_eq!((gone.added, gone.deleted), (Some(0), Some(1)));

    let fresh = by_path(&s, "fresh file.ts");
    assert_eq!(fresh.status, Some(FileChangeStatus::Added));
    assert_eq!(fresh.added, Some(3));
    assert_eq!(fresh.language.as_deref(), Some("typescript"));

    assert_eq!(
        by_path(&s, "link").status,
        Some(FileChangeStatus::Typechange)
    );
    let cafe = by_path(&s, "caf\u{e9}.md");
    assert_eq!(
        (cafe.status, cafe.added),
        (Some(FileChangeStatus::Modified), Some(1))
    );

    // Counts agree with the full parsed patch, file by file (the full diff
    // splits a type change into delete + add, so compare the rest).
    let full = git.diff(target, None).await.unwrap();
    for f in full.files.iter().filter(|f| f.path != "link") {
        let sf = by_path(&s, &f.path);
        assert_eq!((sf.added, sf.deleted), (f.added, f.deleted), "{}", f.path);
        assert_eq!(sf.status, f.status, "{}", f.path);
        assert_eq!(sf.old_path, f.old_path, "{}", f.path);
    }
    assert_eq!(s.total_added, Some(1 + 3 + 1 + 1 + 1));
    assert_eq!(s.total_added, full.total_added);
    assert_eq!(s.total_deleted, full.total_deleted);
}

#[tokio::test]
async fn per_file_request_with_old_path_keeps_the_rename_pairing() {
    let (_tmp, dir, head) = edge_repo();
    let git = LocalGit::new(&dir);
    let target = DiffTarget::Commit(head);
    let new = "n\u{fc}w dir/new\tname.txt";

    // The new path alone: git sees only an add.
    let alone = git
        .diff_with(
            &target,
            &DiffOpts {
                path: Some(new.into()),
                ..DiffOpts::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(alone.files.len(), 1);
    assert_eq!(alone.files[0].status, Some(FileChangeStatus::Added));

    // With old_path: one renamed file with just the edit, and the same bytes
    // (fingerprint) the whole-commit diff shows for it.
    let one = git
        .diff_with(
            &target,
            &DiffOpts {
                path: Some(new.into()),
                old_path: Some("sp ace/old name.txt".into()),
                caps: Some(DiffCaps::DEFAULT),
                ..DiffOpts::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(one.files.len(), 1, "{:?}", paths(&one));
    let f = &one.files[0];
    assert_eq!(f.status, Some(FileChangeStatus::Renamed));
    assert_eq!(f.old_path.as_deref(), Some("sp ace/old name.txt"));
    assert_eq!((f.added, f.deleted), (Some(1), Some(0)));
    let whole = git.diff(target, None).await.unwrap();
    assert_eq!(by_path(&whole, new).fingerprint, f.fingerprint);
}

#[tokio::test]
async fn caps_mark_big_files_and_spend_the_response_budget_in_order() {
    let (_tmp, dir) = init();
    write(&dir, "seed", b"x\n");
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-q", "-m", "seed"]);
    // git orders files by path: a_big (over the per-file cap), then five
    // 4,500-line files (the 5th crosses 20k), then a tiny one after it.
    write(&dir, "a_big.txt", lines("big", 6_000).as_bytes());
    for i in 1..=5 {
        write(&dir, &format!("b{i}.txt"), lines("mid", 4_500).as_bytes());
    }
    write(&dir, "c_tiny.txt", b"tiny\n");
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-q", "-m", "big"]);
    let git = LocalGit::new(&dir);
    let target = DiffTarget::Commit("HEAD".into());
    let capped = git
        .diff_with(
            &target,
            &DiffOpts {
                caps: Some(DiffCaps::DEFAULT),
                ..DiffOpts::default()
            },
        )
        .await
        .unwrap();
    let full = git.diff(target.clone(), None).await.unwrap();

    let big = by_path(&capped, "a_big.txt");
    assert_eq!(big.too_large, Some(true));
    assert_eq!(big.hunks_omitted, Some(true));
    assert!(big.hunks.is_empty());
    assert_eq!(big.added, Some(6_000), "counts survive the cap");
    for i in 1..=4 {
        let f = by_path(&capped, &format!("b{i}.txt"));
        assert_eq!(f.hunks[0].lines.len(), 4_500);
        assert_eq!(f.too_large, None);
    }
    for p in ["b5.txt", "c_tiny.txt"] {
        let f = by_path(&capped, p);
        assert_eq!(f.hunks_omitted, Some(true), "{p}");
        assert_eq!(f.too_large, None, "{p}");
        assert!(f.hunks.is_empty(), "{p}");
    }
    assert_eq!(capped.truncated, Some(true));
    assert_eq!(full.truncated, None, "internal callers stay uncapped");
    assert_eq!(capped.total_added, full.total_added);
    // Fingerprints hash git's bytes whatever the caps kept.
    for f in &full.files {
        assert_eq!(by_path(&capped, &f.path).fingerprint, f.fingerprint);
    }
    // The post-hoc (provider) path reaches the same verdicts.
    let view = crate::parse::capped_view(&full, |_| true, Some(&DiffCaps::DEFAULT), false);
    for f in &capped.files {
        let v = by_path(&view, &f.path);
        assert_eq!(
            (v.too_large, v.hunks_omitted, v.hunks.len()),
            (f.too_large, f.hunks_omitted, f.hunks.len()),
            "{}",
            f.path
        );
    }
    assert_eq!(view.truncated, Some(true));

    // `full` on one file lifts the per-file cap.
    let one = git
        .diff_with(
            &target,
            &DiffOpts {
                path: Some("a_big.txt".into()),
                caps: Some(DiffCaps::FULL_FILE),
                ..DiffOpts::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(one.files[0].too_large, None);
    assert_eq!(one.files[0].hunks[0].lines.len(), 6_000);
}

#[tokio::test]
async fn three_dot_range_diffs_against_the_merge_base() {
    let (_tmp, dir) = init();
    write(&dir, "shared.txt", b"base\n");
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-q", "-m", "base"]);
    sh_git(&dir, &["checkout", "-q", "-b", "feature"]);
    write(&dir, "feature.txt", b"mine\n");
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-q", "-m", "feature"]);
    sh_git(&dir, &["checkout", "-q", "main"]);
    write(&dir, "main-only.txt", b"theirs\n");
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-q", "-m", "main moves on"]);
    let git = LocalGit::new(&dir);

    let three = DiffTarget::parse("range:main...feature").unwrap();
    assert_eq!(
        three,
        DiffTarget::MergeBase("main".into(), "feature".into())
    );
    let d = git.diff_with(&three, &DiffOpts::default()).await.unwrap();
    assert_eq!(paths(&d), vec!["feature.txt"]);
    let s = git.diff_with(&three, &summary()).await.unwrap();
    assert_eq!(paths(&s), vec!["feature.txt"]);

    // Two dots keep their tree-vs-tree meaning for existing callers: the
    // base's own newer file shows up (as deleted from feature's view).
    let two = DiffTarget::parse("range:main..feature").unwrap();
    assert_eq!(two, DiffTarget::Range("main".into(), "feature".into()));
    let d2 = git.diff_with(&two, &summary()).await.unwrap();
    let mut p2 = paths(&d2);
    p2.sort();
    assert_eq!(p2, vec!["feature.txt", "main-only.txt"]);
    assert!(DiffTarget::parse("range:...feature").is_err());
    assert!(DiffTarget::parse("range:main...").is_err());
    assert!(DiffTarget::parse("range:--x...feature").is_err());
}

#[tokio::test]
async fn working_summary_counts_tracked_and_untracked_files() {
    let (_tmp, dir) = init();
    write(&dir, "t.txt", b"one\n");
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-q", "-m", "base"]);
    write(&dir, "t.txt", b"one\ntwo\n");
    write(&dir, "new dir/untracked.txt", b"a\nb\nno-newline");
    write(&dir, "blob.bin", b"\x00\x01binary");
    let git = LocalGit::new(&dir);
    let s = git
        .diff_with(&DiffTarget::Working, &summary())
        .await
        .unwrap();
    assert_eq!(by_path(&s, "t.txt").added, Some(1));
    let u = by_path(&s, "new dir/untracked.txt");
    assert_eq!(
        (u.status, u.added, u.deleted),
        (Some(FileChangeStatus::Added), Some(3), Some(0))
    );
    let b = by_path(&s, "blob.bin");
    assert!(b.is_binary);
    assert_eq!(b.added, None);
    // Same counts the full Working diff renders.
    let full = git.diff(DiffTarget::Working, None).await.unwrap();
    for f in &full.files {
        assert_eq!(by_path(&s, &f.path).added, f.added, "{}", f.path);
    }
    // Scoped to one untracked path.
    let one = git
        .diff_with(
            &DiffTarget::Working,
            &DiffOpts {
                path: Some("new dir/untracked.txt".into()),
                summary: true,
                ..DiffOpts::default()
            },
        )
        .await
        .unwrap();
    assert_eq!(paths(&one), vec!["new dir/untracked.txt"]);
}

#[tokio::test]
async fn worktree_and_staged_diffs_are_unchanged_for_normal_files() {
    let (_tmp, dir) = init();
    write(&dir, "w.txt", lines("w", 30).as_bytes());
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-q", "-m", "base"]);
    write(&dir, "w.txt", lines("W", 30).as_bytes());
    let git = LocalGit::new(&dir);
    let opts = DiffOpts {
        path: Some("w.txt".into()),
        caps: Some(DiffCaps::DEFAULT),
        ..DiffOpts::default()
    };
    let capped = git.diff_with(&DiffTarget::Worktree, &opts).await.unwrap();
    let plain = git.diff(DiffTarget::Worktree, Some("w.txt")).await.unwrap();
    let f = &capped.files[0];
    assert_eq!(f.hunks_omitted, None);
    assert_eq!(f.hunks.len(), plain.files[0].hunks.len());
    assert_eq!(f.fingerprint, plain.files[0].fingerprint);
    // …and the fingerprint is what a hunk op re-derives from its own diff.
    let raw = git.diff_raw(DiffTarget::Worktree, "w.txt").await.unwrap();
    use sha2::{Digest, Sha256};
    assert_eq!(f.fingerprint, hex::encode(Sha256::digest(&raw)));
    assert_eq!(f.status, Some(FileChangeStatus::Modified));
}

#[tokio::test]
async fn resolve_commit_returns_full_ids() {
    let (_tmp, dir, head) = edge_repo();
    let git = LocalGit::new(&dir);
    assert_eq!(git.resolve_commit("HEAD").await.unwrap(), head);
    assert_eq!(git.resolve_commit("main").await.unwrap(), head);
    assert_eq!(
        git.resolve_commit(&head.to_uppercase()).await.unwrap(),
        head
    );
    assert!(git.resolve_commit("no-such-branch").await.is_err());
    assert!(git.resolve_commit("--output=/tmp/x").await.is_err());
}

/// The four numbers a proof pack derives (`files_changed`, `additions`,
/// `deletions`, the path list behind `risky_files`) come from the summary
/// alone now — and must equal what the full parsed diff used to give, for
/// both of `assemble_diff`'s targets (a `base..HEAD` range and Working).
#[tokio::test]
async fn proof_counts_from_the_summary_equal_the_full_parse() {
    let (_tmp, dir, _head) = edge_repo();
    // Uncommitted work on top, so Working has tracked + untracked + binary.
    write(&dir, "plain.rs", lines("plain", 12).as_bytes());
    write(&dir, "untracked dir/u.txt", b"x\ny\n");
    write(&dir, "blob2.bin", b"\x00\x02bin");
    let git = LocalGit::new(&dir);
    // Paths deduped: the full patch emits a type change (symlink → file) as
    // TWO same-path entries (delete + add) — the UI already merges them
    // (`diff-load.ts` `fileFor`) — while `--raw` reports it once. So a
    // typechange now counts as one changed file, which is the right number;
    // the line totals are identical.
    let counts = |d: &DiffResp| {
        let mut p: Vec<String> = d.files.iter().map(|f| f.path.clone()).collect();
        p.sort();
        p.dedup();
        let add: u32 = d.files.iter().filter_map(|f| f.added).sum();
        let del: u32 = d.files.iter().filter_map(|f| f.deleted).sum();
        (p.len(), add, del, p)
    };
    for target in [
        DiffTarget::Range("HEAD~1".into(), "HEAD".into()),
        DiffTarget::Working,
    ] {
        let s = git.diff_with(&target, &summary()).await.unwrap();
        let full = git.diff_with(&target, &DiffOpts::default()).await.unwrap();
        assert_eq!(counts(&s), counts(&full), "{target:?}");
        assert_eq!(s.files.len(), counts(&s).0, "the summary never repeats a path");
        assert!(!s.files.is_empty());
    }
}

/// The proof text read is killed at its byte budget instead of buffering
/// the whole patch, and says so; under the budget it is the whole diff.
#[tokio::test]
async fn diff_text_capped_stops_at_the_budget() {
    let (_tmp, dir) = init();
    write(&dir, "big.txt", lines("base", 10).as_bytes());
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-q", "-m", "base"]);
    write(&dir, "big.txt", lines("changed", 20_000).as_bytes());
    let git = LocalGit::new(&dir);
    let (whole, cut) = git.diff_text_capped(None, 64 * 1024 * 1024).await.unwrap();
    assert!(!cut);
    assert_eq!(whole, git.working_diff_text().await.unwrap());
    let (head, cut) = git.diff_text_capped(None, 4096).await.unwrap();
    assert!(cut);
    assert_eq!(head.len(), 4096);
    assert!(whole.starts_with(&head));
    let (_, cut) = git.diff_text_capped(Some("HEAD"), 4096).await.unwrap();
    assert!(cut);
    assert!(git.diff_text_capped(Some("--x"), 10).await.is_err());
}

/// Past the `-l1000` rename limit git skips rename detection and says so on
/// stderr; the response carries `renames_incomplete` (summary and full) so
/// a delete + add pair isn't mistaken for the whole story. Under the limit
/// the flag is absent.
#[tokio::test]
async fn renames_incomplete_is_set_past_the_rename_limit() {
    let (_tmp, dir) = init();
    let n = 1_001;
    for i in 0..n {
        write(&dir, &format!("old/{i}.txt"), format!("old body {i}\n").as_bytes());
    }
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-q", "-m", "base"]);
    std::fs::remove_dir_all(dir.join("old")).unwrap();
    for i in 0..n {
        write(&dir, &format!("new/{i}.txt"), format!("new text {i}\n").as_bytes());
    }
    sh_git(&dir, &["add", "-A"]);
    sh_git(&dir, &["commit", "-q", "-m", "moved"]);
    let head = sh_git(&dir, &["rev-parse", "HEAD"]);
    let git = LocalGit::new(&dir);
    let target = DiffTarget::Commit(head);
    let s = git.diff_with(&target, &summary()).await.unwrap();
    assert_eq!(s.renames_incomplete, Some(true));
    assert_eq!(s.files.len(), 2 * n);
    let full = git.diff_with(&target, &DiffOpts::default()).await.unwrap();
    assert_eq!(full.renames_incomplete, Some(true));

    // A small rename stays paired and unflagged.
    let (_t2, d2, h2) = edge_repo();
    let small = LocalGit::new(&d2)
        .diff_with(&DiffTarget::Commit(h2), &summary())
        .await
        .unwrap();
    assert_eq!(small.renames_incomplete, None);
}
