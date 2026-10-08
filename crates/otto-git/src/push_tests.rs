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
    String::from_utf8(out.stdout).unwrap().trim().to_string()
}

fn fixture() -> (tempfile::TempDir, LocalGit) {
    let dir = tempfile::tempdir().unwrap();
    git(dir.path(), &["init", "-q", "-b", "main"]);
    git(dir.path(), &["config", "user.name", "Fixture"]);
    git(
        dir.path(),
        &["config", "user.email", "fixture@example.test"],
    );
    git(dir.path(), &["config", "commit.gpgsign", "false"]);
    git(dir.path(), &["commit", "--allow-empty", "-qm", "base"]);
    git(
        dir.path(),
        &[
            "remote",
            "add",
            "upstream",
            "https://example.invalid/review.git",
        ],
    );
    git(dir.path(), &["config", "branch.main.remote", "upstream"]);
    git(
        dir.path(),
        &["config", "branch.main.merge", "refs/heads/review"],
    );
    git(dir.path(), &["config", "push.default", "upstream"]);
    git(
        dir.path(),
        &["update-ref", "refs/remotes/upstream/review", "HEAD"],
    );
    let local = LocalGit::new(dir.path());
    (dir, local)
}

#[tokio::test]
async fn bound_push_preserves_upstream_and_pins_source_after_preflight() {
    let (dir, local) = fixture();
    let target = local.push_target().await.unwrap();
    assert_eq!(target.remote, "upstream");
    assert_eq!(target.destination_ref, "refs/heads/review");
    let args = local.force_push_args(&target).await.unwrap();
    // The command is captured, not executed: no force push, even to a fixture.
    git(dir.path(), &["checkout", "-qb", "other"]);
    git(
        dir.path(),
        &["commit", "--allow-empty", "-qm", "other work"],
    );
    assert!(args.contains(&format!("{}:refs/heads/review", target.source_sha)));
    assert!(args.contains(&"https://example.invalid/review.git".to_string()));
    assert!(args.contains(&format!(
        "--force-with-lease=refs/heads/review:{}",
        target.remote_sha.clone().unwrap()
    )));
    assert!(!args.contains(&"HEAD".to_string()));
    assert!(matches!(
        local.force_push_args(&target).await,
        Err(Error::Conflict(_))
    ));
}

#[tokio::test]
async fn bound_push_rejects_changed_source_destination_and_tracking_ref() {
    for change in ["source", "destination", "tracking"] {
        let (dir, local) = fixture();
        let target = local.push_target().await.unwrap();
        match change {
            "source" => {
                git(dir.path(), &["commit", "--allow-empty", "-qm", "new work"]);
            }
            "destination" => {
                git(
                    dir.path(),
                    &[
                        "remote",
                        "set-url",
                        "upstream",
                        "https://example.invalid/other.git",
                    ],
                );
            }
            _ => {
                git(
                    dir.path(),
                    &["update-ref", "-d", "refs/remotes/upstream/review"],
                );
            }
        }
        assert!(
            matches!(
                local.force_push_args(&target).await,
                Err(Error::Conflict(_))
            ),
            "{change}"
        );
    }
}

#[tokio::test]
async fn bound_push_refuses_fetched_commits_never_integrated_into_branch() {
    let (dir, local) = fixture();
    git(dir.path(), &["checkout", "-qb", "other"]);
    git(
        dir.path(),
        &["commit", "--allow-empty", "-qm", "someone else's commit"],
    );
    let incoming = git(dir.path(), &["rev-parse", "HEAD"]);
    git(dir.path(), &["checkout", "-q", "main"]);
    git(
        dir.path(),
        &["update-ref", "refs/remotes/upstream/review", &incoming],
    );
    let target = local.push_target().await.unwrap();
    assert!(matches!(
        local.force_push_args(&target).await,
        Err(Error::Conflict(_))
    ));
}

#[tokio::test]
async fn bound_push_allows_our_own_rewrite_and_refuses_multi_ref_config() {
    let (dir, local) = fixture();
    git(dir.path(), &["commit", "--allow-empty", "-qm", "published"]);
    git(
        dir.path(),
        &["update-ref", "refs/remotes/upstream/review", "HEAD"],
    );
    git(
        dir.path(),
        &["commit", "--amend", "--allow-empty", "-qm", "rewritten"],
    );
    let target = local.push_target().await.unwrap();
    assert!(local.force_push_args(&target).await.is_ok());
    git(
        dir.path(),
        &[
            "config",
            "remote.upstream.push",
            "refs/heads/*:refs/heads/*",
        ],
    );
    assert!(
        local.push_target().await.is_err(),
        "multi-ref push needs its own explicit consent"
    );
}

#[tokio::test]
async fn bound_push_refuses_url_rewriting_instead_of_reapplying_it() {
    let (dir, local) = fixture();
    git(
        dir.path(),
        &[
            "config",
            "url.https://other.invalid/.insteadOf",
            "https://example.invalid/",
        ],
    );
    git(
        dir.path(),
        &[
            "config",
            "url.https://third.invalid/.pushInsteadOf",
            "https://other.invalid/",
        ],
    );
    assert!(matches!(local.push_target().await, Err(Error::Conflict(_))));
}

#[tokio::test]
async fn publication_push_pins_proved_revision_and_never_forces_newer_remote() {
    let (dir, local) = fixture();
    let bare = dir.path().join("origin.git");
    std::fs::create_dir(&bare).unwrap();
    git(&bare, &["init", "--bare", "-q"]);
    git(
        dir.path(),
        &["remote", "add", "origin", bare.to_str().unwrap()],
    );
    let proved = git(dir.path(), &["rev-parse", "HEAD"]);
    // Model the branch advancing while a caller awaits credentials.
    git(
        dir.path(),
        &["commit", "--allow-empty", "-qm", "unproved later work"],
    );
    let later = git(dir.path(), &["rev-parse", "HEAD"]);
    git(
        dir.path(),
        &["tag", "-a", "unproved-tag", "-m", "fixture", &proved],
    );
    git(dir.path(), &["config", "push.followTags", "true"]);
    local.push_revision(None, &proved, "main").await.unwrap();
    assert_eq!(git(&bare, &["rev-parse", "refs/heads/main"]), proved);
    assert!(git(&bare, &["tag", "--list"]).is_empty());
    assert_eq!(git(dir.path(), &["rev-parse", "HEAD"]), later);
    // A normal forward publication remains allowed. A subsequent stale proof
    // must be rejected, never overwriting another writer's newer remote commit.
    local.push_revision(None, &later, "main").await.unwrap();
    assert!(matches!(
        local.push_revision(None, &proved, "main").await,
        Err(Error::Conflict(_))
    ));
    assert_eq!(git(&bare, &["rev-parse", "refs/heads/main"]), later);
    assert!(matches!(
        local.push_revision(None, "HEAD", "main").await,
        Err(Error::Invalid(_))
    ));
}
