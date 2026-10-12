//! Request-supplied refs never reach git as options: `guard_ref` refuses an
//! option-like or control-character value, and the branch / merge entry
//! points run it before any argv is built (`--end-of-options` covers the
//! rest).

use std::path::Path;

use otto_core::api::LocalMergeStrategy;
use otto_core::Error;

use crate::local::LocalGit;

fn sh(dir: &Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("spawn git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn guard_ref_refuses_option_like_and_control_values() {
    assert!(LocalGit::guard_ref("feature/x").is_ok());
    assert!(LocalGit::guard_ref("v1.2.3").is_ok());
    assert!(LocalGit::guard_ref("HEAD~2").is_ok());
    assert!(LocalGit::guard_ref("--upload-pack=touch /tmp/pwn").is_err());
    assert!(LocalGit::guard_ref("-c").is_err());
    assert!(LocalGit::guard_ref("  --force").is_err());
    assert!(LocalGit::guard_ref("main\nrm").is_err());
}

#[tokio::test]
async fn branch_and_merge_refuse_option_like_names() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path();
    sh(dir, &["init", "-q", "-b", "main"]);
    sh(dir, &["config", "user.email", "otto@test.local"]);
    sh(dir, &["config", "user.name", "Otto Test"]);
    sh(dir, &["config", "commit.gpgsign", "false"]);
    sh(dir, &["commit", "-q", "--allow-empty", "-m", "base"]);
    let git = LocalGit::new(dir);
    for bad in ["--set-upstream-to=x", "-m", " --force"] {
        let r = git.create_branch(bad, None, false).await;
        assert!(matches!(r, Err(Error::Invalid(_))), "name {bad:?}: {r:?}");
        let r = git.create_branch("ok", Some(bad), true).await;
        assert!(matches!(r, Err(Error::Invalid(_))), "start {bad:?}: {r:?}");
        for (source, target) in [(bad, "main"), ("main", bad)] {
            let r = git
                .merge_branch(source, target, LocalMergeStrategy::MergeCommit, false)
                .await;
            assert!(
                matches!(r, Err(Error::Invalid(_))),
                "merge {source:?} into {target:?}: {r:?}"
            );
        }
    }
    let refs = git.refs().await.unwrap();
    assert_eq!(
        refs.local
            .iter()
            .map(|b| b.name.as_str())
            .collect::<Vec<_>>(),
        ["main"]
    );
    // A plain name still works through the same entry point.
    git.create_branch("feature/ok", None, false).await.unwrap();
}
