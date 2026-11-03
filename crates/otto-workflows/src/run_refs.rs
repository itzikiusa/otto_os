//! Run-start ref resolution with no daemon dependency: resolve a run's
//! declared base to a commit, which branches each repo must have synced, and
//! the PRs a run names by number (`pr: 15` / `pr: "repo-a#15, repo-b#340"`).
//! The provider lookups and credentialed fetches stay in otto-server
//! (`workflow_engine/run/refs.rs`).

use crate::context::RepoEntry;
use serde_json::Value;
use std::path::Path;

/// Resolve a run's declared base to a commit: the ref as given (a local
/// branch, tag or sha), else the remote-tracking `origin/<base>`. Read-only.
pub async fn resolve_wf_base(git: &otto_git::LocalGit, base: &str) -> Option<String> {
    if let Ok(c) = git.rev_parse(base).await {
        return Some(c);
    }
    let remote = format!("origin/{base}");
    if let Ok(c) = git.rev_parse(&remote).await {
        return Some(c);
    }
    // Neither ref exists yet — typically a branch created on the remote after
    // the clone last fetched (a PR targeting a fresh `hotfix/*`) and the
    // run-start sync's fetch failed or didn't cover it. Fetch THAT ONE branch
    // into its remote-tracking ref and retry. Safe: it writes only
    // `refs/remotes/origin/<base>` — never the user's branches, HEAD or tree —
    // and runs non-interactively (BatchMode SSH), so it can't prompt.
    if !fetchable_branch_name(base) {
        return None;
    }
    match git.fetch_origin_branch(base, None).await {
        Ok(()) => tracing::info!("wf worktree: fetched missing base '{base}' from origin"),
        Err(err) => {
            tracing::warn!("wf worktree: fetching base '{base}' from origin failed: {err}");
            return None;
        }
    }
    git.rev_parse(&remote).await.ok()
}

/// A base name that is safe to splice into a fetch refspec: no option-like
/// prefix, no refspec/glob metacharacters, no whitespace or control chars.
/// `git` itself still validates the full ref name.
pub fn fetchable_branch_name(base: &str) -> bool {
    otto_git::branch_sync::is_syncable_branch_name(base)
}

/// The branches to sync per repo id, in declaration order: each valid entry's
/// base + source, the run's `pr_branch`, and every resolved PR's head + base
/// for that repo (blank names dropped; the sync dedupes). Pure.
pub fn branches_by_repo(
    entries: &[RepoEntry],
    pr_branch: Option<&str>,
    prs: &[RunPr],
) -> Vec<(String, Vec<String>)> {
    let mut wanted: Vec<(String, Vec<String>)> = Vec::new();
    for e in entries.iter().filter(|e| e.error.is_none()) {
        let Some(rid) = e.repo_id.clone() else {
            continue;
        };
        let pr_names = prs
            .iter()
            .filter(|p| p.repo_id == rid)
            .flat_map(|p| [p.head.as_str(), p.base.as_str()]);
        let names = [e.base.as_deref(), e.source.as_deref(), pr_branch]
            .into_iter()
            .flatten()
            .chain(pr_names)
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect::<Vec<_>>();
        match wanted.iter_mut().find(|(r, _)| *r == rid) {
            Some((_, v)) => v.extend(names),
            None => wanted.push((rid, names)),
        }
    }
    wanted
}

/// One PR a run names (`pr: 15` / `pr: "repo-a#15, repo-b#340"`), resolved
/// through the repo's provider: its head + base branches and head commit, and
/// — once [`pin_pr_heads`] ran — the local ref its head commit is read from
/// (`refs/remotes/origin/<head>`, or a fetched `refs/otto/pr-review/*` for a
/// fork PR). Written to the run's `prs.json`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct RunPr {
    pub repo_id: String,
    pub repo_path: String,
    pub number: u64,
    pub head: String,
    pub base: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_sha: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_ref: Option<String>,
}

/// The run context file listing the run's resolved PRs (read back by the
/// reaper to release fetched fork-head refs).
pub const PRS_FILE: &str = "prs.json";

/// Parse the run input's `pr`: a number, `"15"`, `"#15"`, or a comma list of
/// `repo#N` (`"repo-a#15, repo-b#340"`). `(None, n)` = no repo named.
/// Unparseable parts are skipped. Pure.
pub fn parse_pr_spec(v: Option<&Value>) -> Vec<(Option<String>, u64)> {
    match v {
        Some(Value::Number(n)) => n
            .as_u64()
            .filter(|n| *n > 0)
            .map(|n| vec![(None, n)])
            .unwrap_or_default(),
        Some(Value::String(s)) => s
            .split(',')
            .filter_map(|part| {
                let part = part.trim();
                let (repo, num) = match part.rsplit_once('#') {
                    Some((r, n)) => (Some(r.trim()).filter(|r| !r.is_empty()), n),
                    None => (None, part),
                };
                let n: u64 = num.trim().parse().ok().filter(|n| *n > 0)?;
                Some((repo.map(str::to_string), n))
            })
            .collect(),
        _ => vec![],
    }
}

/// Map parsed PR specs onto the run's valid entries → `(entry index, number)`.
/// A bare number applies only when the run has exactly ONE repo (otherwise it
/// is ambiguous and skipped); `repo#N` matches the entry's repo name or the
/// last path segment of its declaration / worktree, case-insensitively. Pure.
pub fn match_pr_targets(
    spec: &[(Option<String>, u64)],
    entries: &[RepoEntry],
) -> Vec<(usize, u64)> {
    let valid: Vec<usize> = (0..entries.len())
        .filter(|&i| entries[i].error.is_none() && entries[i].repo_id.is_some())
        .collect();
    let tail = |p: &str| {
        p.trim_end_matches('/')
            .rsplit('/')
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase()
    };
    let mut out: Vec<(usize, u64)> = Vec::new();
    for (repo, n) in spec {
        let hit = match repo {
            None if valid.len() == 1 => Some(valid[0]),
            None => None,
            Some(name) => {
                let want = tail(name);
                valid.iter().copied().find(|&i| {
                    let e = &entries[i];
                    e.repo_name
                        .as_deref()
                        .map(str::to_ascii_lowercase)
                        .as_deref()
                        == Some(want.as_str())
                        || tail(&e.repo) == want
                        || e.worktree.as_deref().map(tail).as_deref() == Some(want.as_str())
                })
            }
        };
        if let Some(i) = hit {
            if !out.iter().any(|(j, m)| *j == i && m == n) {
                out.push((i, *n));
            }
        }
    }
    out
}

/// Release the fork-head refs [`pin_pr_heads`] fetched for a run (listed in
/// its `prs.json`). Only `refs/otto/pr-review/*` is ever deleted.
pub async fn release_pr_heads(data_dir: &Path, run_id: &str) {
    let path = data_dir
        .join("workflow-context")
        .join(run_id)
        .join(PRS_FILE);
    let Ok(raw) = tokio::fs::read_to_string(&path).await else {
        return;
    };
    let prs: Vec<RunPr> = serde_json::from_str(&raw).unwrap_or_default();
    for pr in prs {
        if let Some(r) = pr
            .head_ref
            .as_deref()
            .filter(|r| r.starts_with("refs/otto/pr-review/"))
        {
            let _ = otto_git::LocalGit::new(&pr.repo_path)
                .delete_pr_review_ref(r)
                .await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn entry(name: &str, path: &str) -> RepoEntry {
        RepoEntry {
            repo: path.into(),
            repo_id: Some(format!("id-{name}")),
            repo_name: Some(name.into()),
            kind: "worktree".into(),
            name: path.into(),
            source: None,
            worktree: Some(path.into()),
            base: None,
            error: None,
        }
    }

    #[test]
    fn pr_spec_parses_numbers_and_repo_lists() {
        assert_eq!(parse_pr_spec(Some(&json!(15))), vec![(None, 15)]);
        assert_eq!(parse_pr_spec(Some(&json!("#15"))), vec![(None, 15)]);
        assert_eq!(
            parse_pr_spec(Some(&json!("repo-a#15, repo-b#340, junk, #0"))),
            vec![(Some("repo-a".into()), 15), (Some("repo-b".into()), 340)]
        );
        assert!(parse_pr_spec(Some(&json!(0))).is_empty());
        assert!(parse_pr_spec(None).is_empty());
    }

    #[test]
    fn pr_targets_match_by_repo_name_or_path_and_bare_numbers_only_for_one_repo() {
        let one = vec![entry("games", "/src/sinatra-games-go")];
        assert_eq!(match_pr_targets(&[(None, 15)], &one), vec![(0, 15)]);
        let two = vec![
            entry("games", "/src/sinatra-games-go"),
            entry("Gateway", "/src/sinatra-gateway-go/"),
        ];
        // A bare number is ambiguous across two repos.
        assert!(match_pr_targets(&[(None, 15)], &two).is_empty());
        assert_eq!(
            match_pr_targets(
                &[
                    (Some("sinatra-games-go".into()), 1),
                    (Some("gateway".into()), 2),
                    (Some("~/src/sinatra-gateway-go".into()), 2),
                    (Some("unknown".into()), 3),
                ],
                &two
            ),
            vec![(0, 1), (1, 2)]
        );
        // An errored entry is never a target.
        let mut bad = one.clone();
        bad[0].error = Some("x".into());
        assert!(match_pr_targets(&[(None, 15)], &bad).is_empty());
    }
}
