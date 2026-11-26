//! `git_checkout` decisions (engine.md §7.1, DR-008, DR-029): which PR or
//! branch each run repo is checked out at — never guessed. The node glue in
//! otto-server runs git and the forge and hands the facts in; everything here
//! is pure, so every rule is a unit test.
//!
//! PR mode: an explicit `pr` is authoritative (not open → unresolved, never
//! "corrected"); else the open PRs whose source is `pr_branch`; else the open
//! PRs whose source is `feature/<KEY>` (or starts with it) or whose title
//! names KEY. Several candidates are unresolved and listed. Branch mode: the
//! first of `pr_branch`, the entry's declared `source`, `feature/<KEY>` must
//! exist on `origin`, else the single remote branch naming KEY. A ref name
//! from input is checked here before git ever sees it.

use crate::context::RepoEntry;
use otto_core::api::PrState;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `params.mode`: `"pr"` (default) | `"branch"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Pr,
    Branch,
}

pub fn mode(params: &Value) -> Mode {
    match params.get("mode").and_then(Value::as_str) {
        Some("branch") => Mode::Branch,
        _ => Mode::Pr,
    }
}

/// How a row's source was found (`rows[].via`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Via {
    Pr,
    PrBranch,
    Jira,
    Declared,
    Convention,
}

/// A PR as the forge reports it, plus the local ref its head was pinned to
/// (`LocalGit::pr_head_ref`: `refs/remotes/origin/<head>` for a same-repo
/// PR, `refs/otto/pr-review/*` for a fork).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrFact {
    pub number: u64,
    pub state: PrState,
    pub source: String,
    pub target: String,
    pub title: String,
    pub url: String,
    pub head_ref: Option<String>,
}

/// Why an entry was not checked out, with what was considered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unresolved {
    pub repo: String,
    pub reason: String,
    #[serde(default)]
    pub candidates: Vec<String>,
}

/// A rule that found nothing usable: the one-line reason and the candidates.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Miss {
    pub reason: String,
    pub candidates: Vec<String>,
}

impl Miss {
    fn new(reason: impl Into<String>) -> Self {
        Self {
            reason: reason.into(),
            candidates: vec![],
        }
    }
    pub fn unresolved(self, repo: &str) -> Unresolved {
        Unresolved {
            repo: repo.to_string(),
            reason: self.reason,
            candidates: self.candidates,
        }
    }
}

/// `git check-ref-format --branch` (plus: never an option, never a full
/// ref) so an input value can be put after `origin/` safely.
pub fn valid_branch_name(name: &str) -> bool {
    otto_git::branch_sync::is_syncable_branch_name(name)
        && !name.contains("//")
        && !name.ends_with('.')
        && name
            .split('/')
            .all(|c| !c.starts_with('.') && !c.ends_with(".lock"))
}

fn checked(name: &str) -> Result<&str, Miss> {
    if valid_branch_name(name) {
        Ok(name)
    } else {
        Err(Miss::new(format!("invalid branch name: {name:?}")))
    }
}

fn given(v: Option<&str>) -> Option<&str> {
    v.map(str::trim).filter(|s| !s.is_empty())
}

/// The ticket key the convention rules look for: the Jira key in
/// `input.jira_ticket` (a pasted URL yields its key; "N/A" none).
pub fn ticket_key(input: &Value) -> Option<String> {
    let t = input.get("jira_ticket")?.clone();
    crate::prepare::extract_jira_key(&Value::Null, &serde_json::json!({ "jira_ticket": t }))
}

/// `text` contains `key` as a whole token (case-insensitive): GS-12 is not
/// in GS-123.
fn names_key(text: &str, key: &str) -> bool {
    let (text, key) = (text.to_ascii_uppercase(), key.to_ascii_uppercase());
    text.match_indices(&key).any(|(i, _)| {
        let before = text[..i].chars().next_back();
        let after = text[i + key.len()..].chars().next();
        !before.is_some_and(|c| c.is_ascii_alphanumeric())
            && !after.is_some_and(|c| c.is_ascii_alphanumeric())
    })
}

/// `feature/<KEY>` or a branch that starts with it (`feature/GS-1-fix`).
fn is_key_branch(branch: &str, key: &str) -> bool {
    let want = format!("feature/{key}").to_ascii_uppercase();
    let b = branch.to_ascii_uppercase();
    b.strip_prefix(&want)
        .is_some_and(|rest| !rest.starts_with(|c: char| c.is_ascii_alphanumeric()))
}

fn listed(prs: &[&PrFact]) -> Vec<String> {
    prs.iter()
        .map(|p| format!("#{} {}", p.number, p.source))
        .collect()
}

fn state_word(s: PrState) -> &'static str {
    match s {
        PrState::Open | PrState::All => "open",
        PrState::Merged => "merged",
        PrState::Declined => "declined",
    }
}

/// PR mode for one entry. `explicit` is the PR the input named for this
/// entry (looked up by number); `open` the repo's open PRs (only read when
/// there is no explicit PR).
pub fn pick_pr<'a>(
    explicit: Option<&'a PrFact>,
    open: &'a [PrFact],
    pr_branch: Option<&str>,
    key: Option<&str>,
) -> Result<(&'a PrFact, Via), Miss> {
    if let Some(pr) = explicit {
        if pr.state != PrState::Open {
            return Err(Miss::new(format!(
                "PR #{} is {}",
                pr.number,
                state_word(pr.state)
            )));
        }
        return checked_pr(pr).map(|pr| (pr, Via::Pr));
    }
    let open: Vec<&PrFact> = open.iter().filter(|p| p.state == PrState::Open).collect();
    if let Some(branch) = given(pr_branch) {
        let branch = checked(branch)?;
        let hits: Vec<&PrFact> = open
            .iter()
            .copied()
            .filter(|p| p.source == branch)
            .collect();
        if let Some(pr) = one(&hits, &format!("from {branch}"))? {
            return checked_pr(pr).map(|pr| (pr, Via::PrBranch));
        }
    }
    let Some(key) = key else {
        return Err(Miss::new(match given(pr_branch) {
            Some(b) => format!("no open PR from {b}"),
            None => "no PR, branch or ticket to find the PR by".to_string(),
        }));
    };
    let hits: Vec<&PrFact> = open
        .iter()
        .copied()
        .filter(|p| is_key_branch(&p.source, key) || names_key(&p.title, key))
        .collect();
    match one(&hits, &format!("for {key}"))? {
        Some(pr) => checked_pr(pr).map(|pr| (pr, Via::Jira)),
        None => Err(Miss::new(format!("no open PR for {key}"))),
    }
}

/// Exactly one candidate, none, or "several" listing them.
fn one<'a>(hits: &[&'a PrFact], what: &str) -> Result<Option<&'a PrFact>, Miss> {
    match hits {
        [] => Ok(None),
        [pr] => Ok(Some(pr)),
        many => Err(Miss {
            reason: format!("{} open PRs {what}: pick one with pr", many.len()),
            candidates: listed(many),
        }),
    }
}

fn checked_pr(pr: &PrFact) -> Result<&PrFact, Miss> {
    checked(&pr.source)?;
    checked(&pr.target)?;
    Ok(pr)
}

/// What a picked PR is checked out at: its pinned head ref, never
/// `origin/<source>` (a fork PR's source is not on origin, and a same-named
/// origin branch is not the PR).
pub fn pr_checkout_ref(pr: &PrFact) -> Result<&str, Miss> {
    pr.head_ref
        .as_deref()
        .ok_or_else(|| Miss::new(format!("the head of PR #{} is not available", pr.number)))
}

/// Branch mode's feature branch. `remote` = the names under
/// `refs/remotes/origin/` (without the prefix; `HEAD` ignored).
pub fn pick_branch(
    pr_branch: Option<&str>,
    declared: Option<&str>,
    key: Option<&str>,
    remote: &[String],
) -> Result<(String, Via), Miss> {
    let convention = key.map(|k| format!("feature/{k}"));
    let first = [
        (given(pr_branch), Via::PrBranch),
        (given(declared), Via::Declared),
        (convention.as_deref(), Via::Convention),
    ]
    .into_iter()
    .find_map(|(b, via)| b.map(|b| (b, via)));
    if let Some((branch, via)) = first {
        let branch = checked(branch)?;
        if remote.iter().any(|r| r == branch) {
            return Ok((branch.to_string(), via));
        }
        if key.is_none() {
            return Err(Miss::new(format!("origin/{branch} does not exist")));
        }
    }
    let Some(key) = key else {
        return Err(Miss::new("no branch or ticket to check out"));
    };
    let hits: Vec<&String> = remote
        .iter()
        .filter(|r| r.as_str() != "HEAD" && names_key(r, key))
        .collect();
    match hits.as_slice() {
        [b] => checked(b).map(|b| (b.to_string(), Via::Jira)),
        [] => Err(Miss::new(format!("no branch on origin names {key}"))),
        many => Err(Miss {
            reason: format!(
                "{} branches on origin name {key}: pick one with pr_branch",
                many.len()
            ),
            candidates: many.iter().map(|b| b.to_string()).collect(),
        }),
    }
}

/// Branch mode's destination: the input's `base` unless it is the feature
/// branch itself, else `origin/HEAD`'s branch, else the first of develop /
/// main / master on origin.
pub fn pick_base(
    input_base: Option<&str>,
    feature: &str,
    origin_head: Option<&str>,
    remote: &[String],
) -> Result<String, Miss> {
    if let Some(b) = given(input_base).filter(|b| *b != feature) {
        return checked(b).map(str::to_string);
    }
    if let Some(b) = given(origin_head) {
        return checked(b).map(str::to_string);
    }
    ["develop", "main", "master"]
        .into_iter()
        .find(|b| remote.iter().any(|r| r == b))
        .map(str::to_string)
        .ok_or_else(|| Miss::new("no base branch: origin has no HEAD, develop, main or master"))
}

/// After the checkout: the diff must not be empty.
pub fn check_diff(merge_base: &str, head: &str, files_changed: u32) -> Result<(), Miss> {
    if merge_base == head || files_changed == 0 {
        return Err(Miss::new("empty diff — the base is the branch itself"));
    }
    Ok(())
}

/// One checked-out entry (`rows[]`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Row {
    pub repo: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<u64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    pub via: Via,
    pub source: String,
    pub dest: String,
    pub merge_base: String,
    pub head: String,
    pub ahead: u32,
    pub files_changed: u32,
}

/// The node's output (plus the forwarded input keys the glue adds).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CheckoutOutput {
    pub repos: Vec<RepoEntry>,
    pub resolved: usize,
    pub unresolved: Vec<Unresolved>,
    pub rows: Vec<Row>,
}

impl CheckoutOutput {
    pub fn new(repos: Vec<RepoEntry>, rows: Vec<Row>, unresolved: Vec<Unresolved>) -> Self {
        Self {
            repos,
            resolved: rows.len(),
            unresolved,
            rows,
        }
    }

    /// Nothing resolved fails the step with every reason (downstream steps
    /// are error-skipped at once); otherwise one ⚠ log line per unresolved.
    pub fn verdict(&self) -> Result<Vec<String>, String> {
        let lines = self
            .unresolved
            .iter()
            .map(|u| {
                let also = if u.candidates.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", u.candidates.join(", "))
                };
                format!("{}: {}{also}", u.repo, u.reason)
            })
            .collect::<Vec<_>>();
        if self.rows.is_empty() {
            return Err(if lines.is_empty() {
                "no repository to check out".to_string()
            } else {
                lines.join("; ")
            });
        }
        Ok(lines.into_iter().map(|l| format!("⚠ {l}")).collect())
    }

    /// The step file: the rows as a table, then the final `repos.json`.
    pub fn step_file(&self) -> String {
        let mut md = String::from(
            "| Repo | PR | Via | Source | Destination | Merge base | HEAD | Ahead | Files |\n\
             |---|---|---|---|---|---|---|---|---|\n",
        );
        for r in &self.rows {
            let pr = match (r.pr, &r.url) {
                (Some(n), Some(u)) => format!("[#{n}]({u})"),
                (Some(n), None) => format!("#{n}"),
                _ => "—".into(),
            };
            let via = serde_json::to_value(r.via).ok();
            md.push_str(&format!(
                "| {} | {pr} | {} | {} | {} | {} | {} | {} | {} |\n",
                r.repo,
                via.as_ref().and_then(Value::as_str).unwrap_or_default(),
                r.source,
                r.dest,
                short(&r.merge_base),
                short(&r.head),
                r.ahead,
                r.files_changed
            ));
        }
        for u in &self.unresolved {
            md.push_str(&format!("\n⚠ {}: {}", u.repo, u.reason));
            if !u.candidates.is_empty() {
                md.push_str(&format!(" ({})", u.candidates.join(", ")));
            }
        }
        let repos = serde_json::to_string_pretty(&self.repos).unwrap_or_default();
        md.push_str(&format!("\n\n`repos.json`:\n\n```json\n{repos}\n```\n"));
        md
    }
}

fn short(sha: &str) -> &str {
    &sha[..sha.len().min(12)]
}

#[cfg(test)]
#[path = "checkout_tests.rs"]
mod tests;
