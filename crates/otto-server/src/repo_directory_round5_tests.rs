use otto_core::auth::McpScope;
use otto_state::NewRepo;

use super::tests::{auth_for, entry, fixture, listed, user, world};
use super::*;

#[tokio::test]
async fn a_workspace_pinned_token_cannot_resolve_outside_its_pin() {
    let w = world().await;
    let pinned = auth_for(
        user("owner", false),
        Some(McpScope {
            tools: None,
            allow_writes: true,
            workspace_id: Some(w.ws_a.clone()),
        }),
    );
    for reference in ["games", w.repo_b.as_str()] {
        let err = resolve_repo_in(&w.src(), &pinned, None, Some(reference), None)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::NotFound(_)), "{reference}: {err}");
        assert!(!listed(&err.to_string(), w.repo_b.as_str()), "{err}");
    }
    let err = resolve_repo_in(&w.src(), &pinned, None, Some("games"), Some(&w.ws_b))
        .await
        .unwrap_err();
    assert!(matches!(err, Error::Forbidden(_)), "{err}");
    let r = resolve_repo_in(&w.src(), &pinned, None, Some("tools"), None)
        .await
        .unwrap();
    assert_eq!(r.entry.repo.id, w.repo_a);
}

#[test]
fn listing_puts_the_current_workspace_first() {
    let mut es = fixture();
    sort_for_listing(&mut es, Some("ws-b"));
    let first_a = es
        .iter()
        .position(|e| e.repo.workspace_id == "ws-a")
        .unwrap();
    assert!(es[..first_a].iter().all(|e| e.repo.workspace_id == "ws-b"));
    assert_eq!(es[0].repo.name, "games_management");
    let row = es[0].to_json(Some("ws-b"));
    assert_eq!(row["workspace_name"], "ws-b-name");
    assert_eq!(row["current"], true);
}

#[test]
fn match_path_rejects_broad_roots_even_when_stale_row_exists() {
    let home = std::env::var("HOME").expect("HOME set");
    let home = home.trim_end_matches('/');
    let es = vec![
        entry("r-home", "ws-a", "home", home, None),
        entry("r-tmp", "ws-a", "tmp", "/tmp", None),
        entry(
            "r-real",
            "ws-a",
            "subrepo",
            &format!("{home}/projects/subrepo"),
            None,
        ),
    ];
    assert!(match_path(&es, "~").is_empty());
    assert!(match_path(&es, home).is_empty());
    assert!(match_path(&es, "/tmp").is_empty());
    assert!(match_path(&es, &format!("{home}/Desktop/random-notes")).is_empty());
    let sub_hits = match_path(&es, &format!("{home}/projects/subrepo/src/main.rs"));
    assert_eq!(sub_hits.len(), 1);
    assert_eq!(sub_hits[0].repo.id, "r-real");
}

#[tokio::test]
async fn resolve_repo_rejects_broad_roots_while_resolving_nested_subrepo() {
    let w = world().await;
    let owner = auth_for(user("owner", false), None);

    // Even if a legacy broad-root row exists in SQLite, session cwd in `/tmp` or `$HOME`
    // must never resolve `/tmp` or `$HOME` as a repository.
    let home = std::env::var("HOME").expect("HOME set");
    let _legacy_tmp = w
        .git
        .create_repo(NewRepo {
            workspace_id: w.ws_a.clone(),
            name: "legacy-tmp".into(),
            path: "/tmp".into(),
            remote_url: None,
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap();

    for broad in ["/tmp", "/private/tmp", home.as_str(), "~"] {
        let err = resolve_repo_in(&w.src(), &owner, None, Some(broad), None)
            .await
            .unwrap_err();
        assert!(matches!(err, Error::NotFound(_)), "{broad}: {err}");
    }
    let session_in_tmp = Some(("/tmp".to_string(), w.ws_a.clone()));
    let err = resolve_repo_in(&w.src(), &owner, session_in_tmp, None, None)
        .await
        .unwrap_err();
    assert!(matches!(err, Error::NotFound(_)), "{err}");

    // Now create a parent `.git` at `<tempdir>` AND a nested real git repo at
    // `<tempdir>/subrepo` whose remote matches `w.repo_a` (`bitbucket.org/team/tools.git`).
    let td = tempfile::tempdir().unwrap();
    let parent = std::fs::canonicalize(td.path()).unwrap();
    let subrepo = parent.join("subrepo");
    std::fs::create_dir_all(&subrepo).unwrap();

    let status = otto_git::hardened_command()
        .args(["init", "-q"])
        .current_dir(&parent)
        .status()
        .await
        .unwrap();
    assert!(status.success());

    let status = otto_git::hardened_command()
        .args(["init", "-q"])
        .current_dir(&subrepo)
        .status()
        .await
        .unwrap();
    assert!(status.success());
    let status = otto_git::hardened_command()
        .args([
            "remote",
            "add",
            "origin",
            "https://bitbucket.org/team/tools.git",
        ])
        .current_dir(&subrepo)
        .status()
        .await
        .unwrap();
    assert!(status.success());

    let sub_src = subrepo.join("src");
    std::fs::create_dir_all(&sub_src).unwrap();
    let sub_src_str = sub_src.to_string_lossy().into_owned();
    let resolved = resolve_repo_in(&w.src(), &owner, None, Some(&sub_src_str), None)
        .await
        .expect("nested subrepo should resolve via checkout remote");
    assert_eq!(resolved.entry.repo.id, w.repo_a);
}

#[tokio::test]
async fn tilde_nested_subrepo_beats_expanded_parent_and_visible_repos_prunes_deleted_with_forge() {
    let home = std::env::var("HOME").expect("HOME set");
    let home = home.trim_end_matches('/');
    let es = vec![
        entry(
            "r-parent",
            "ws-a",
            "parent",
            &format!("{home}/ws_parent"),
            None,
        ),
        entry("r-tilde-sub", "ws-a", "sub", "~/ws_parent/sub", None),
    ];
    let hits = match_path(&es, &format!("{home}/ws_parent/sub/src/lib.rs"));
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].repo.id, "r-tilde-sub");

    let w = world().await;
    let owner = user("owner", false);
    std::fs::create_dir_all(format!("{}/gh-sdk", w.root_dir)).unwrap();
    let gh_id = w
        .git
        .create_repo(NewRepo {
            workspace_id: w.ws_a.clone(),
            name: "gh-sdk".into(),
            path: format!("{}/gh-sdk", w.root_dir),
            remote_url: Some("git@github.com:acme/gh-sdk.git".into()),
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap()
        .id;
    let missing_id = w
        .git
        .create_repo(NewRepo {
            workspace_id: w.ws_a.clone(),
            name: "deleted-checkout".into(),
            path: format!("{}/deleted-checkout", w.root_dir),
            remote_url: Some("https://github.com/acme/deleted.git".into()),
            provider: None,
            git_account_id: None,
        })
        .await
        .unwrap()
        .id;

    let visible = visible_repos(&w.src(), &owner, None).await.unwrap();
    assert!(!visible.iter().any(|e| e.repo.id == missing_id));
    let forge_of = |id: &str| {
        visible
            .iter()
            .find(|e| e.repo.id == id)
            .and_then(|e| e.repo.forge.as_deref())
    };
    assert_eq!(forge_of(&w.repo_a), Some("bitbucket"));
    assert_eq!(forge_of(&w.repo_b), Some("bitbucket"));
    assert_eq!(forge_of(&gh_id), Some("github"));
}
