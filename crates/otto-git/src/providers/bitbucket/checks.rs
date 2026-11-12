//! Bitbucket Cloud merge checks — the "1 of 3 checks passed" list Bitbucket
//! shows on a PR, derived from what the public 2.0 API exposes.
//!
//! A merge check exists only where a branch restriction asks for it, so the
//! rules come from `GET /repositories/{ws}/{repo}/branch-restrictions`,
//! matched against the PR's DESTINATION branch (exact name, glob, or a
//! branching-model kind). That list is per repository, so it is read once
//! and memoised for [`RULES_TTL`] — not on every PR open. Every read here is
//! soft: a token that may not read the restrictions (Bitbucket wants repo
//! admin for them) gets no checks at all rather than invented ones, and no
//! failure here ever fails the PR load.
//!
//! What each check counts (Bitbucket's own definitions):
//! * `require_approvals_to_merge` — approvals on the PR ≥ the value.
//! * `require_default_reviewer_approvals_to_merge` — approvals from the
//!   repository's effective default reviewers ≥ the value.
//! * `require_no_changes_requested` — no participant requested changes.
//! * `require_tasks_to_be_completed` — no open task.
//! * `require_passing_builds_to_merge` — successful builds on the head commit
//!   ≥ the value, with none failed and none still running.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use otto_core::api::{MergeCheck, TaskCount};
use otto_core::Error;
use serde_json::Value;

use super::{user_keys, Bitbucket};
use crate::providers::{varr, vbool, vstr, vu64, RemoteRef};
use crate::types::CiStatus;

/// How long a repository's merge rules are reused. Restrictions change
/// rarely; a few minutes keeps a rule edit visible soon after.
const RULES_TTL: Duration = Duration::from_secs(5 * 60);

/// The restriction kinds that are merge checks.
const CHECK_KINDS: [&str; 5] = [
    "require_approvals_to_merge",
    "require_default_reviewer_approvals_to_merge",
    "require_no_changes_requested",
    "require_tasks_to_be_completed",
    "require_passing_builds_to_merge",
];

/// One repository's merge rules as this credential can read them.
#[derive(Debug, Clone, Default)]
pub(crate) struct Rules {
    /// Branch restrictions of a [`CHECK_KINDS`] kind.
    pub(crate) restrictions: Vec<Value>,
    /// The branching model (`development` / `production` branches and the
    /// `branch_types` prefixes) — read only when a restriction is keyed on it.
    pub(crate) model: Option<Value>,
    /// Stable-id keys of the effective default reviewers — read only when a
    /// default-reviewer restriction exists; `None` when unreadable (that
    /// check is then omitted).
    pub(crate) default_reviewers: Option<Vec<Vec<String>>>,
}

/// `(stored at, rules)`; `None` = the forge refused the restrictions read
/// (remembered so a token without the scope is not re-asked per PR open).
type Memo = Mutex<HashMap<String, (Instant, Option<Rules>)>>;

fn memo() -> &'static Memo {
    static M: OnceLock<Memo> = OnceLock::new();
    M.get_or_init(Default::default)
}

/// Drop the memoised rules of an API root (`enable_cache_for_tests`).
#[cfg(test)]
pub(crate) fn forget_rules_for_tests(base: &str) {
    let mut guard = memo().lock().unwrap_or_else(|p| p.into_inner());
    guard.retain(|k, _| !k.starts_with(base));
}

impl Bitbucket {
    /// The repository's merge rules — memoised per (API root, repo,
    /// credential). `None` when they cannot be read.
    pub(crate) async fn merge_rules(&self, r: &RemoteRef) -> Option<Rules> {
        let key = format!(
            "{}|{}/{}|{}",
            self.base,
            r.owner,
            r.repo,
            self.identity_key()
        );
        // Like the GET cache, the memo is opt-in under test (pooled wiremock
        // ports would hand one test another's rules).
        let enabled = crate::providers::client::cache_enabled();
        if enabled {
            let guard = memo().lock().unwrap_or_else(|p| p.into_inner());
            if let Some((at, rules)) = guard.get(&key) {
                if at.elapsed() < RULES_TTL {
                    return rules.clone();
                }
            }
        }
        let rules = match self.read_rules(r).await {
            Ok(rules) => Some(rules),
            // The token may not read branch restrictions (or the repo hides
            // them): a stable answer, remembered like a success.
            Err(Error::Forbidden(_) | Error::Unauthorized | Error::NotFound(_)) => None,
            Err(e) => {
                tracing::debug!("bitbucket: merge rules unavailable: {e}");
                return None;
            }
        };
        if enabled {
            let mut guard = memo().lock().unwrap_or_else(|p| p.into_inner());
            guard.retain(|_, (at, _)| at.elapsed() < RULES_TTL);
            guard.insert(key, (Instant::now(), rules.clone()));
        }
        rules
    }

    async fn read_rules(&self, r: &RemoteRef) -> otto_core::Result<Rules> {
        let repo = format!("/repositories/{}/{}", r.owner, r.repo);
        let restrictions: Vec<Value> = self
            .paginate_values_with(&format!("{repo}/branch-restrictions?pagelen=100"), false)
            .await?
            .into_iter()
            .filter(|v| CHECK_KINDS.contains(&vstr(v, &["kind"]).as_str()))
            .collect();
        let uses =
            |field: &str, value: &str| restrictions.iter().any(|v| vstr(v, &[field]) == value);
        let model = if uses("branch_match_kind", "branching_model") {
            self.send_json(
                reqwest::Method::GET,
                &format!("{repo}/branching-model"),
                None,
            )
            .await
            .ok()
        } else {
            None
        };
        let default_reviewers = if uses("kind", "require_default_reviewer_approvals_to_merge") {
            self.paginate_values_with(
                &format!("{repo}/effective-default-reviewers?pagelen=100"),
                false,
            )
            .await
            .ok()
            .map(|vals| {
                vals.iter()
                    // `{reviewer_type, user}` rows; a bare user object too.
                    .map(|v| user_keys(v.get("user").unwrap_or(v)))
                    .collect()
            })
        } else {
            None
        };
        Ok(Rules {
            restrictions,
            model,
            default_reviewers,
        })
    }

    /// The PR's tasks by state (`GET …/pullrequests/{id}/tasks`). `None` when
    /// unreadable — the open count then falls back to the PR's `task_count`.
    pub(crate) async fn pr_tasks(&self, r: &RemoteRef, number: u64) -> Option<TaskCount> {
        let path = format!(
            "{}?pagelen=100",
            Self::pr_path(r, &format!("/{number}/tasks"))
        );
        let tasks = self.paginate_values(&path).await.ok()?;
        let resolved = tasks
            .iter()
            .filter(|t| vstr(t, &["state"]) == "RESOLVED")
            .count() as u32;
        Some(TaskCount {
            open: tasks.len() as u32 - resolved,
            resolved,
        })
    }
}

/// Bitbucket's branch glob: `*` matches any run of characters (`/`
/// included), everything else literally; the whole name must match. A
/// pattern without `*` is an exact name.
pub(crate) fn glob_match(pattern: &str, name: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    if parts.len() == 1 {
        return pattern == name;
    }
    let (first, last) = (parts[0], parts[parts.len() - 1]);
    if !name.starts_with(first) || name.len() < first.len() + last.len() || !name.ends_with(last) {
        return false;
    }
    let mut rest = &name[first.len()..name.len() - last.len()];
    for mid in &parts[1..parts.len() - 1] {
        match rest.find(mid) {
            Some(i) => rest = &rest[i + mid.len()..],
            None => return false,
        }
    }
    true
}

/// Whether restriction `rule` applies to destination branch `dest`.
fn applies(rule: &Value, dest: &str, model: Option<&Value>) -> bool {
    match vstr(rule, &["branch_match_kind"]).as_str() {
        "branching_model" => {
            let Some(model) = model else { return false };
            match vstr(rule, &["branch_type"]).as_str() {
                kind @ ("development" | "production") => {
                    // `production` may be switched off in the model.
                    vbool(model, &[kind, "enabled"]) != Some(false)
                        && !vstr(model, &[kind, "branch", "name"]).is_empty()
                        && vstr(model, &[kind, "branch", "name"]) == dest
                }
                kind => varr(model, &["branch_types"]).iter().any(|t| {
                    let prefix = vstr(t, &["prefix"]);
                    vstr(t, &["kind"]) == kind && !prefix.is_empty() && dest.starts_with(&prefix)
                }),
            }
        }
        // `glob` (the default); an absent pattern matches nothing.
        _ => {
            let pattern = vstr(rule, &["pattern"]);
            !pattern.is_empty() && glob_match(&pattern, dest)
        }
    }
}

fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// The merge checks `rules` impose on a PR into `dest`. `pr` is the PR
/// payload (participants, `task_count`), `ci` its build statuses,
/// `open_tasks` the tasks endpoint's open count when it was readable.
pub(crate) fn merge_checks(
    rules: &Rules,
    dest: &str,
    pr: &Value,
    ci: &CiStatus,
    open_tasks: Option<u32>,
) -> Vec<MergeCheck> {
    // Several restrictions of one kind may match (a `*` rule and an exact
    // one): the strictest value wins; `Some(_)` = the kind applies.
    let required = |kind: &str| -> Option<u64> {
        rules
            .restrictions
            .iter()
            .filter(|r| vstr(r, &["kind"]) == kind && applies(r, dest, rules.model.as_ref()))
            .map(|r| vu64(r, &["value"]))
            .max()
    };
    let participants = varr(pr, &["participants"]);
    let approved: Vec<&Value> = participants
        .iter()
        .filter(|p| vbool(p, &["approved"]).unwrap_or(false))
        .collect();
    let mut out = Vec::new();
    if let Some(n) = required("require_approvals_to_merge").map(|n| n.max(1)) {
        let have = approved.len() as u64;
        out.push(MergeCheck {
            key: "approvals".into(),
            label: format!("At least {}", plural(n, "approval", "approvals")),
            passed: have >= n,
            detail: Some(format!("{have} of {n}")),
        });
    }
    if let (Some(n), Some(defaults)) = (
        required("require_default_reviewer_approvals_to_merge").map(|n| n.max(1)),
        rules.default_reviewers.as_ref(),
    ) {
        let have = approved
            .iter()
            .filter(|p| {
                p.get("user").is_some_and(|u| {
                    let keys = user_keys(u);
                    defaults
                        .iter()
                        .any(|d| d.iter().any(|k| !k.is_empty() && keys.contains(k)))
                })
            })
            .count() as u64;
        out.push(MergeCheck {
            key: "default_reviewer_approvals".into(),
            label: format!(
                "At least {} from default reviewers",
                plural(n, "approval", "approvals")
            ),
            passed: have >= n,
            detail: Some(format!("{have} of {n}")),
        });
    }
    if required("require_no_changes_requested").is_some() {
        let asked = participants
            .iter()
            .filter(|p| vstr(p, &["state"]) == "changes_requested")
            .count() as u64;
        out.push(MergeCheck {
            key: "no_changes_requested".into(),
            label: "No changes requested".into(),
            passed: asked == 0,
            detail: (asked > 0).then(|| {
                format!(
                    "{} requested changes",
                    plural(asked, "reviewer", "reviewers")
                )
            }),
        });
    }
    if required("require_tasks_to_be_completed").is_some() {
        // The PR payload's `task_count` is its OPEN task count.
        let open = open_tasks.map_or_else(|| vu64(pr, &["task_count"]), u64::from);
        out.push(MergeCheck {
            key: "tasks".into(),
            label: "All tasks resolved".into(),
            passed: open == 0,
            detail: (open > 0).then(|| plural(open, "open task", "open tasks")),
        });
    }
    if let Some(n) = required("require_passing_builds_to_merge").map(|n| n.max(1)) {
        let (passed, failed) = (u64::from(ci.passed), u64::from(ci.failed));
        let running = u64::from(ci.total).saturating_sub(passed + failed);
        let mut detail = format!("{passed} of {n} successful");
        if failed > 0 {
            detail.push_str(&format!(", {failed} failed"));
        }
        if running > 0 {
            detail.push_str(&format!(", {running} in progress"));
        }
        out.push(MergeCheck {
            key: "builds".into(),
            label: format!(
                "At least {}",
                plural(n, "successful build", "successful builds")
            ),
            passed: passed >= n && failed == 0 && running == 0,
            detail: Some(detail),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn glob_matches_bitbucket_style() {
        assert!(glob_match("main", "main"));
        assert!(!glob_match("main", "main2"));
        assert!(glob_match("release/*", "release/1.2"));
        assert!(glob_match("release/*", "release/a/b"), "`*` crosses `/`");
        assert!(!glob_match("release/*", "hotfix/1"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("feat*-x", "feat/a-x"));
        assert!(!glob_match("a*b*c", "acb"));
        assert!(glob_match("a*b*c", "a-b-c"));
    }

    #[test]
    fn branching_model_kinds_resolve_through_the_model() {
        let model = json!({
            "development": { "branch": { "name": "develop" } },
            "production": { "branch": { "name": "main" }, "enabled": true },
            "branch_types": [ { "kind": "release", "prefix": "release/" } ]
        });
        let rule = |t: &str| json!({ "branch_match_kind": "branching_model", "branch_type": t });
        assert!(applies(&rule("production"), "main", Some(&model)));
        assert!(applies(&rule("development"), "develop", Some(&model)));
        assert!(!applies(&rule("development"), "main", Some(&model)));
        assert!(applies(&rule("release"), "release/3", Some(&model)));
        assert!(!applies(&rule("hotfix"), "release/3", Some(&model)));
        assert!(
            !applies(&rule("production"), "main", None),
            "no model, no match"
        );
        let off = json!({ "production": { "branch": { "name": "main" }, "enabled": false } });
        assert!(!applies(&rule("production"), "main", Some(&off)));
    }
}
