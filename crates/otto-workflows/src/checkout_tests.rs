use super::*;
use serde_json::json;

fn pr(number: u64, state: PrState, source: &str, title: &str) -> PrFact {
    PrFact {
        number,
        state,
        source: source.into(),
        target: "develop".into(),
        title: title.into(),
        url: format!("https://forge.test/pr/{number}"),
        head_ref: Some(format!("refs/remotes/origin/{source}")),
    }
}

fn names(v: &[&str]) -> Vec<String> {
    v.iter().map(|s| s.to_string()).collect()
}

#[test]
fn an_explicit_pr_is_authoritative_and_a_merged_one_is_never_corrected() {
    let open = vec![pr(7, PrState::Open, "feature/GS-1", "GS-1 fix")];
    let merged = pr(54, PrState::Merged, "feature/GS-1", "GS-1 fix");
    let miss = pick_pr(Some(&merged), &open, Some("feature/GS-1"), Some("GS-1")).unwrap_err();
    assert_eq!(miss.reason, "PR #54 is merged");
    let declined = pr(55, PrState::Declined, "x", "");
    assert_eq!(
        pick_pr(Some(&declined), &open, None, None)
            .unwrap_err()
            .reason,
        "PR #55 is declined"
    );
    let explicit = pr(9, PrState::Open, "other", "");
    let (got, via) = pick_pr(Some(&explicit), &open, Some("feature/GS-1"), Some("GS-1")).unwrap();
    assert_eq!((got.number, via), (9, Via::Pr));
}

#[test]
fn pr_branch_comes_before_the_ticket_and_several_candidates_are_listed() {
    let open = vec![
        pr(1, PrState::Open, "fix/login", ""),
        pr(2, PrState::Open, "feature/GS-12-login", ""),
        pr(3, PrState::Merged, "fix/login", ""),
    ];
    let (got, via) = pick_pr(None, &open, Some("fix/login"), Some("GS-12")).unwrap();
    assert_eq!((got.number, via), (1, Via::PrBranch));
    // No PR from the branch → the ticket rule.
    let (got, via) = pick_pr(None, &open, Some("gone"), Some("GS-12")).unwrap();
    assert_eq!((got.number, via), (2, Via::Jira));
    let twice = vec![
        pr(4, PrState::Open, "fix/login", ""),
        pr(5, PrState::Open, "fix/login", ""),
    ];
    let miss = pick_pr(None, &twice, Some("fix/login"), None).unwrap_err();
    assert_eq!(miss.reason, "2 open PRs from fix/login: pick one with pr");
    assert_eq!(miss.candidates, vec!["#4 fix/login", "#5 fix/login"]);
}

#[test]
fn the_ticket_rule_matches_whole_keys_by_branch_or_title_and_only_open_prs() {
    let open = vec![
        pr(1, PrState::Open, "feature/GS-123", "unrelated"),
        pr(2, PrState::Open, "bugfix/x", "Fix gs-12: login"),
        pr(3, PrState::Merged, "feature/GS-12", ""),
    ];
    // GS-12 is not GS-123; the title match is case-insensitive.
    let (got, _) = pick_pr(None, &open, None, Some("GS-12")).unwrap();
    assert_eq!(got.number, 2);
    let both = vec![
        pr(1, PrState::Open, "feature/GS-5", ""),
        pr(2, PrState::Open, "x", "GS-5 part two"),
    ];
    let miss = pick_pr(None, &both, None, Some("GS-5")).unwrap_err();
    assert_eq!(miss.candidates.len(), 2);
    assert_eq!(
        pick_pr(None, &open, None, Some("GS-9")).unwrap_err().reason,
        "no open PR for GS-9"
    );
    assert_eq!(
        pick_pr(None, &open, Some("nope"), None).unwrap_err().reason,
        "no open PR from nope"
    );
}

#[test]
fn input_refs_are_hardened_before_git_sees_them() {
    for bad in [
        "-c core.sshCommand=x",
        "--upload-pack=x",
        "a..b",
        "a b",
        "refs/heads/x",
        "x.lock",
        "a//b",
        ".hidden",
        "a/.b",
        "x.",
        "@",
        "a@{1}",
        "x:y",
        "",
    ] {
        assert!(!valid_branch_name(bad), "{bad:?} must be refused");
    }
    for good in ["main", "feature/GS-1-fix", "release/1.2", "a-b_c"] {
        assert!(valid_branch_name(good), "{good}");
    }
    let open = vec![pr(1, PrState::Open, "-x", "")];
    let miss = pick_pr(None, &open, Some("-x"), None).unwrap_err();
    assert!(
        miss.reason.starts_with("invalid branch name"),
        "{}",
        miss.reason
    );
    // A PR whose own source/target is not a safe name is not checked out.
    let mut evil = pr(2, PrState::Open, "ok", "");
    evil.target = "--output=/tmp/x".into();
    assert!(pick_pr(Some(&evil), &[], None, None)
        .unwrap_err()
        .reason
        .starts_with("invalid branch name"));
    assert!(pick_branch(Some("a..b"), None, None, &names(&["a..b"])).is_err());
    assert!(pick_base(Some("-x"), "f", None, &[]).is_err());
}

#[test]
fn a_fork_pr_is_checked_out_at_its_pinned_head_never_origin_source() {
    let mut fork = pr(8, PrState::Open, "main", "");
    fork.head_ref = Some("refs/otto/pr-review/8".into());
    assert_eq!(pr_checkout_ref(&fork).unwrap(), "refs/otto/pr-review/8");
    fork.head_ref = None;
    assert_eq!(
        pr_checkout_ref(&fork).unwrap_err().reason,
        "the head of PR #8 is not available"
    );
}

#[test]
fn branch_mode_takes_the_first_given_branch_then_the_single_key_match() {
    let remote = names(&[
        "HEAD",
        "main",
        "fix/login",
        "feature/GS-7",
        "GS-8-spike",
        "gs-9-a",
        "gs-9-b",
    ]);
    assert_eq!(
        pick_branch(
            Some("fix/login"),
            Some("feature/GS-7"),
            Some("GS-7"),
            &remote
        )
        .unwrap(),
        ("fix/login".to_string(), Via::PrBranch)
    );
    assert_eq!(
        pick_branch(None, Some("feature/GS-7"), None, &remote).unwrap(),
        ("feature/GS-7".to_string(), Via::Declared)
    );
    assert_eq!(
        pick_branch(None, None, Some("GS-7"), &remote).unwrap(),
        ("feature/GS-7".to_string(), Via::Convention)
    );
    // feature/GS-8 is not on origin → the one branch naming the key.
    assert_eq!(
        pick_branch(None, None, Some("GS-8"), &remote).unwrap(),
        ("GS-8-spike".to_string(), Via::Jira)
    );
    let miss = pick_branch(None, None, Some("GS-9"), &remote).unwrap_err();
    assert_eq!(miss.candidates, vec!["gs-9-a", "gs-9-b"]);
    assert_eq!(
        pick_branch(Some("gone"), None, None, &remote)
            .unwrap_err()
            .reason,
        "origin/gone does not exist"
    );
    assert!(pick_branch(None, None, None, &remote).is_err());
}

#[test]
fn branch_mode_base_is_input_then_origin_head_then_develop_main_master() {
    let remote = names(&["main", "master", "f"]);
    assert_eq!(
        pick_base(Some("release"), "f", Some("main"), &remote).unwrap(),
        "release"
    );
    // The input's base is the feature branch itself → ignored.
    assert_eq!(
        pick_base(Some("f"), "f", Some("trunk"), &remote).unwrap(),
        "trunk"
    );
    assert_eq!(pick_base(None, "f", None, &remote).unwrap(), "main");
    assert_eq!(
        pick_base(None, "f", None, &names(&["develop", "main"])).unwrap(),
        "develop"
    );
    assert!(pick_base(None, "f", None, &names(&["f"])).is_err());
}

#[test]
fn an_empty_diff_is_refused() {
    let mb = "a".repeat(40);
    assert!(check_diff(&mb, &mb, 3).is_err());
    assert_eq!(
        check_diff(&mb, &"b".repeat(40), 0).unwrap_err().reason,
        "empty diff — the base is the branch itself"
    );
    assert!(check_diff(&mb, &"b".repeat(40), 2).is_ok());
}

fn row(repo: &str) -> Row {
    Row {
        repo: repo.into(),
        pr: Some(3),
        url: Some("https://forge.test/pr/3".into()),
        via: Via::PrBranch,
        source: "fix/login".into(),
        dest: "develop".into(),
        merge_base: "a".repeat(40),
        head: "b".repeat(40),
        ahead: 2,
        files_changed: 5,
    }
}

#[test]
fn nothing_resolved_fails_with_every_reason_otherwise_warns() {
    let u = |r: &str| Miss::new(format!("no open PR for GS-{r}")).unresolved(r);
    let none = CheckoutOutput::new(vec![], vec![], vec![u("1"), u("2")]);
    assert_eq!(
        none.verdict().unwrap_err(),
        "1: no open PR for GS-1; 2: no open PR for GS-2"
    );
    let some = CheckoutOutput::new(vec![], vec![row("api")], vec![u("2")]);
    assert_eq!(some.resolved, 1);
    assert_eq!(some.verdict().unwrap(), vec!["⚠ 2: no open PR for GS-2"]);
    let md = some.step_file();
    assert!(md.contains("| api | [#3](https://forge.test/pr/3) | pr_branch | fix/login | develop | aaaaaaaaaaaa |"), "{md}");
    assert!(md.contains("`repos.json`"), "{md}");
    let out = serde_json::to_value(&some).unwrap();
    assert_eq!(out["rows"][0]["via"], "pr_branch");
    assert_eq!(out["unresolved"][0]["repo"], "2");
}

#[test]
fn mode_and_ticket_key_read_params_and_input() {
    assert_eq!(mode(&json!({})), Mode::Pr);
    assert_eq!(mode(&json!({"mode": "branch"})), Mode::Branch);
    assert_eq!(
        ticket_key(&json!({"jira_ticket": "https://x.atlassian.net/browse/gs-16"})).as_deref(),
        Some("GS-16")
    );
    assert_eq!(ticket_key(&json!({"jira_ticket": "N/A"})), None);
    assert_eq!(ticket_key(&json!({"prompt": "fix GS-1"})), None);
}
