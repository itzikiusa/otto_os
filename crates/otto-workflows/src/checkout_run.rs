//! The `git_checkout` step (engine.md §7.1, DR-008, DR-029): the Checkout
//! agent's fixed recipe in code. For every valid `repos.json` entry it picks
//! the PR or branch by [`crate::checkout`]'s rules (never a guess), fetches,
//! checks the head out detached, takes the merge-base and rewrites the
//! entry's `source` / `base` (merge-base SHA) / `base_branch`. PRs found by
//! rule are added to `prs.json`. Every git call goes through the hardened
//! builder; every ref from input was checked by the rules and is passed as a
//! full `refs/…` name. The daemon hands in repo access ([`CheckoutHost`]).

use crate::checkout::{self, CheckoutOutput, Miss, Mode, PrFact, Row, Via};
use crate::context::{RepoEntry, RunContextFiles};
use crate::run_refs::{self, RunPr, PRS_FILE};
use otto_core::api::PrState;
use otto_git::transport::TransportCredential;
use otto_git::{GitProvider, RemoteRef};
use serde_json::Value;
use std::sync::Arc;

/// What the step needs from the daemon for one registered repo.
#[async_trait::async_trait]
pub trait CheckoutHost: Send + Sync {
    /// The repo row and its transport credential (never a prompt).
    async fn repo(
        &self,
        repo_id: &str,
    ) -> Option<(otto_core::domain::Repo, Option<TransportCredential>)>;
    /// The repo's forge, acting as the run's user.
    async fn forge(
        &self,
        repo: &otto_core::domain::Repo,
    ) -> Result<(Arc<dyn GitProvider>, RemoteRef), String>;
}

/// Run the step: `(output, logs)`, or the error when no entry resolved.
/// `step_md` is the step file the table and the final `repos.json` go to.
pub async fn run(
    host: &dyn CheckoutHost,
    files: &RunContextFiles,
    params: &Value,
    run_input: &Value,
    input: &Value,
    step_md: Option<std::path::PathBuf>,
) -> Result<(Value, Vec<String>), String> {
    let run = Run {
        mode: checkout::mode(params),
        key: checkout::ticket_key(run_input),
        pr_branch: run_input.get("pr_branch").and_then(Value::as_str),
        base: run_input.get("base").and_then(Value::as_str),
    };
    let entries = files.repos();
    let explicit =
        run_refs::match_pr_targets(&run_refs::parse_pr_spec(run_input.get("pr")), &entries);
    let mut prs: Vec<RunPr> = files
        .read_named(PRS_FILE)
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let (mut rows, mut unresolved, mut found) = (Vec::new(), Vec::new(), Vec::new());
    for (i, e) in entries.iter().enumerate() {
        if e.error.is_some() {
            continue;
        }
        let label = e.repo_name.clone().unwrap_or_else(|| e.repo.clone());
        let number = explicit.iter().find(|(j, _)| *j == i).map(|(_, n)| *n);
        match run.entry(host, e, number, &prs, &label).await {
            Ok((row, pr)) => {
                found.push((e.repo.clone(), e.name.clone(), Ok(row.clone())));
                if let Some(pr) = pr {
                    prs.retain(|p| !(p.repo_id == pr.repo_id && p.number == pr.number));
                    prs.push(pr);
                }
                rows.push(row);
            }
            Err(miss) => {
                let u = miss.unresolved(&label);
                found.push((e.repo.clone(), e.name.clone(), Err(u.reason.clone())));
                unresolved.push(u);
            }
        }
    }
    let repos = files.update_repos(|repos| {
        for (repo, name, outcome) in &found {
            let Some(e) = repos
                .iter_mut()
                .find(|e| &e.repo == repo && &e.name == name)
            else {
                continue;
            };
            match outcome {
                Ok(row) => {
                    e.source = Some(row.source.clone());
                    e.base = Some(row.merge_base.clone());
                    e.base_branch = Some(row.dest.clone());
                }
                Err(reason) => e.error = Some(reason.clone()),
            }
        }
        repos.clone()
    });
    if !prs.is_empty() {
        if let Ok(js) = serde_json::to_string_pretty(&prs) {
            files.write_named(PRS_FILE, &js);
        }
    }
    let result = CheckoutOutput::new(repos, rows, unresolved);
    if let Some(path) = step_md {
        let _ = tokio::fs::write(path, result.step_file()).await;
    }
    let logs = result.verdict()?;
    let mut out = serde_json::to_value(&result)
        .ok()
        .and_then(|v| v.as_object().cloned())
        .unwrap_or_default();
    for k in ["repo_id", "worktree", "pr", "pr_branch"] {
        if let Some(v) = input.get(k).filter(|v| !v.is_null()) {
            out.entry(k).or_insert_with(|| v.clone());
        }
    }
    Ok((Value::Object(out), logs))
}

/// The run's inputs the rules read.
struct Run<'a> {
    mode: Mode,
    key: Option<String>,
    pr_branch: Option<&'a str>,
    base: Option<&'a str>,
}

fn miss(reason: impl Into<String>) -> Miss {
    Miss {
        reason: reason.into(),
        candidates: vec![],
    }
}

impl Run<'_> {
    /// One entry: pick, fetch, check out, measure. The PR is returned when it
    /// must be (re)written to `prs.json`.
    async fn entry(
        &self,
        host: &dyn CheckoutHost,
        e: &RepoEntry,
        explicit: Option<u64>,
        prs: &[RunPr],
        label: &str,
    ) -> std::result::Result<(Row, Option<RunPr>), Miss> {
        let wt = e
            .worktree
            .as_deref()
            .ok_or_else(|| miss("no worktree to check out in"))?;
        let (repo, token) = match &e.repo_id {
            Some(id) => host
                .repo(id)
                .await
                .map_or((None, None), |(r, t)| (Some(r), t)),
            None => (None, None),
        };
        otto_git::LocalGit::new(wt)
            .fetch(token.clone())
            .await
            .map_err(|err| miss(format!("git fetch failed: {err}")))?;
        let (target, pr, row_pr, via, source, dest) = match self.mode {
            Mode::Pr => {
                let repo = repo.ok_or_else(|| miss("not a registered repository"))?;
                let (pr, via) = self.pick_pr(host, &repo, wt, explicit, prs, token).await?;
                let target = pr.head_ref.clone().unwrap_or_default();
                let fact = (Some(pr.number), pr.url.clone());
                let (source, dest) = (pr.head.clone(), pr.base.clone());
                (target, Some(pr), fact, via, source, dest)
            }
            Mode::Branch => {
                let remote = git(
                    wt,
                    &[
                        "for-each-ref",
                        "--format=%(refname:strip=3)",
                        "refs/remotes/origin",
                    ],
                )
                .await
                .map_err(miss)?;
                let remote: Vec<String> = remote.lines().map(str::to_string).collect();
                let origin_head = git(wt, &["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
                    .await
                    .ok()
                    .and_then(|h| h.strip_prefix("origin/").map(str::to_string));
                let (feature, via) = checkout::pick_branch(
                    self.pr_branch,
                    e.source.as_deref(),
                    self.key.as_deref(),
                    &remote,
                )?;
                let dest =
                    checkout::pick_base(self.base, &feature, origin_head.as_deref(), &remote)?;
                (
                    format!("refs/remotes/origin/{feature}"),
                    None,
                    (None, None),
                    via,
                    feature,
                    dest,
                )
            }
        };
        git(wt, &["checkout", "-q", "--detach", &target])
            .await
            .map_err(miss)?;
        let head = git(wt, &["rev-parse", "HEAD"]).await.map_err(miss)?;
        let want = git(
            wt,
            &["rev-parse", "--verify", &format!("{target}^{{commit}}")],
        )
        .await
        .map_err(miss)?;
        if head != want {
            return Err(miss(format!("HEAD is {head}, not {target}")));
        }
        let dest_ref = format!("refs/remotes/origin/{dest}");
        git(
            wt,
            &["rev-parse", "--verify", &format!("{dest_ref}^{{commit}}")],
        )
        .await
        .map_err(|_| miss(format!("origin/{dest} does not resolve")))?;
        let mb = git(wt, &["merge-base", &dest_ref, "HEAD"])
            .await
            .map_err(miss)?;
        let files = git(wt, &["diff", "--name-only", &mb, "HEAD"])
            .await
            .map_err(miss)?;
        let files_changed = files.lines().filter(|l| !l.is_empty()).count() as u32;
        let ahead = git(wt, &["rev-list", "--count", &format!("{dest_ref}..HEAD")])
            .await
            .map_err(miss)?
            .parse()
            .unwrap_or(0);
        checkout::check_diff(&mb, &head, files_changed)?;
        let row = Row {
            repo: label.to_string(),
            pr: row_pr.0,
            url: row_pr.1,
            via,
            source,
            dest,
            merge_base: mb,
            head,
            ahead,
            files_changed,
        };
        Ok((row, pr))
    }

    /// PR mode's pick through the forge: the explicit PR (fresh from the
    /// forge, its head ref from admission's `prs.json` when pinned), else the
    /// repo's open PRs. The head of a PR found by rule is pinned here.
    async fn pick_pr(
        &self,
        host: &dyn CheckoutHost,
        repo: &otto_core::domain::Repo,
        wt: &str,
        explicit: Option<u64>,
        prs: &[RunPr],
        token: Option<TransportCredential>,
    ) -> std::result::Result<(RunPr, Via), Miss> {
        let (provider, remote) = host
            .forge(repo)
            .await
            .map_err(|err| miss(format!("no forge for {}: {err}", repo.name)))?;
        let fact = |s: otto_core::api::PrSummary| {
            let pinned = prs
                .iter()
                .find(|p| p.repo_id == repo.id && p.number == s.number)
                .and_then(|p| p.head_ref.clone());
            (
                PrFact {
                    number: s.number,
                    state: s.state,
                    source: s.source_branch,
                    target: s.target_branch,
                    title: s.title,
                    url: s.url,
                    head_ref: pinned,
                },
                s.head_sha,
            )
        };
        let (explicit, open): (Option<_>, Vec<_>) = match explicit {
            Some(n) => {
                let d = provider
                    .get_pr(&remote, n)
                    .await
                    .map_err(|err| miss(format!("PR #{n}: {err}")))?;
                (Some(fact(d.summary)), vec![])
            }
            None => {
                let page = provider
                    .list_prs(&remote, PrState::Open, 1, 100)
                    .await
                    .map_err(|err| miss(format!("listing open PRs failed: {err}")))?;
                (None, page.items.into_iter().map(fact).collect())
            }
        };
        let facts: Vec<PrFact> = open.iter().map(|(f, _)| f.clone()).collect();
        let (picked, via) = checkout::pick_pr(
            explicit.as_ref().map(|(f, _)| f),
            &facts,
            self.pr_branch,
            self.key.as_deref(),
        )?;
        let head_sha = explicit
            .iter()
            .chain(open.iter())
            .find(|(f, _)| f.number == picked.number)
            .and_then(|(_, sha)| sha.clone());
        let head_ref = match &picked.head_ref {
            Some(r) => r.clone(),
            // The worktree shares the repo's refs; fork heads land there too.
            None => otto_git::LocalGit::new(wt)
                .pr_head_ref(
                    picked.number,
                    Some(&picked.source),
                    head_sha.as_deref(),
                    token,
                )
                .await
                .map_err(|err| {
                    miss(format!(
                        "the head of PR #{} is not available: {err}",
                        picked.number
                    ))
                })?,
        };
        Ok((
            RunPr {
                repo_id: repo.id.clone(),
                repo_path: repo.path.clone(),
                number: picked.number,
                head: picked.source.clone(),
                base: picked.target.clone(),
                head_sha,
                head_ref: Some(head_ref),
                state: Some(picked.state),
                url: Some(picked.url.clone()),
                title: Some(picked.title.clone()),
            },
            via,
        ))
    }
}

/// One hardened git call in `dir`: trimmed stdout, or stderr's last line.
async fn git(dir: &str, args: &[&str]) -> std::result::Result<String, String> {
    let out = otto_git::hardened_command()
        .current_dir(dir)
        .args(args)
        .output()
        .await
        .map_err(|e| format!("git {}: {e}", args[0]))?;
    if out.status.success() {
        return Ok(String::from_utf8_lossy(&out.stdout).trim().to_string());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    Err(format!(
        "git {}: {}",
        args[0],
        err.trim().lines().last().unwrap_or("failed")
    ))
}
