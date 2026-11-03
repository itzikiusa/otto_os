//! Editing an issue: the request / response shapes shared by
//! `PATCH /issue/{account_id}/issues/{key}` and the `otto.update_issue` MCP
//! tool, and the checks that need no tracker round trip. The Jira write lives
//! in `jira::update`.

use otto_core::Error;
use serde::{Deserialize, Serialize};

use crate::issue_create::{non_empty, SUMMARY_MAX};

/// `PATCH /api/v1/issue/{account_id}/issues/{key}` — every field optional,
/// at least one required; an omitted field is left unchanged.
///
/// `description` is Markdown (converted to ADF exactly as on create).
/// `labels` REPLACES the label set; `add_labels` / `remove_labels` edit it in
/// place (Jira `update` ops) and cannot be combined with `labels`. `priority`
/// is a name or id, `assignee` a Jira accountId, `parent` / `epic` an issue
/// key (in Jira Cloud the epic IS the parent).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UpdateIssueReq {
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub labels: Option<Vec<String>>,
    #[serde(default)]
    pub add_labels: Vec<String>,
    #[serde(default)]
    pub remove_labels: Vec<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub assignee: Option<String>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub epic: Option<String>,
}

/// Response of `PATCH /issue/{account_id}/issues/{key}`: `updated` names the
/// fields that were written, in request order.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UpdateIssueResp {
    pub key: String,
    pub url: String,
    pub updated: Vec<String>,
}

/// Problems detectable without the tracker: nothing to change, a blank
/// argument that was passed (omit it to leave the field alone), a summary that
/// is multi-line or over [`SUMMARY_MAX`] characters, malformed labels, `labels`
/// combined with `add_labels` / `remove_labels`, and a disagreeing
/// `parent` / `epic`.
pub fn update_problems(req: &UpdateIssueReq) -> Vec<String> {
    let mut p = Vec::new();
    let blank = |v: &Option<String>| v.as_ref().is_some_and(|s| s.trim().is_empty());
    for (name, v) in [
        ("summary", &req.summary),
        ("description", &req.description),
        ("priority", &req.priority),
        ("assignee", &req.assignee),
        ("parent", &req.parent),
        ("epic", &req.epic),
    ] {
        if blank(v) {
            p.push(format!(
                "{name} must not be empty (omit it to leave it unchanged)"
            ));
        }
    }
    if let Some(summary) = non_empty(&req.summary) {
        if summary.contains(['\n', '\r']) {
            p.push("summary must be a single line (put details in the description)".into());
        }
        let n = summary.chars().count();
        if n > SUMMARY_MAX {
            p.push(format!(
                "summary is {n} characters; the limit is {SUMMARY_MAX}"
            ));
        }
    }
    let label_ops = !req.add_labels.is_empty() || !req.remove_labels.is_empty();
    if req.labels.is_some() && label_ops {
        p.push(
            "pass `labels` (replace the set) OR `add_labels` / `remove_labels`, not both".into(),
        );
    }
    let bad: Vec<&str> = req
        .labels
        .iter()
        .flatten()
        .chain(&req.add_labels)
        .chain(&req.remove_labels)
        .map(String::as_str)
        .filter(|l| l.trim().is_empty() || l.trim().contains(char::is_whitespace))
        .collect();
    if !bad.is_empty() {
        p.push(format!(
            "labels cannot be empty or contain spaces: {} (use e.g. `my-label`)",
            bad.iter()
                .map(|l| format!("{l:?}"))
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    if let (Some(a), Some(e)) = (non_empty(&req.parent), non_empty(&req.epic)) {
        if !a.eq_ignore_ascii_case(e) {
            p.push(format!(
                "parent '{a}' and epic '{e}' disagree — in Jira Cloud the epic IS the parent; \
                 pass one of them"
            ));
        }
    }
    let nothing = req.summary.is_none()
        && req.description.is_none()
        && req.labels.is_none()
        && !label_ops
        && req.priority.is_none()
        && req.assignee.is_none()
        && req.parent.is_none()
        && req.epic.is_none();
    if nothing {
        p.push(
            "nothing to update: pass at least one of summary, description, labels, \
             add_labels, remove_labels, priority, assignee, parent, epic"
                .into(),
        );
    }
    p
}

/// One `400` naming every problem, so a caller fixes them in one round trip.
pub fn update_problems_error(key: &str, problems: &[String]) -> Error {
    Error::Invalid(format!(
        "cannot update issue '{key}':\n- {}",
        problems.join("\n- ")
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_request_is_refused() {
        let p = update_problems(&UpdateIssueReq::default());
        assert_eq!(p.len(), 1);
        assert!(p[0].starts_with("nothing to update"), "{p:?}");
    }

    #[test]
    fn blank_and_malformed_arguments_are_named() {
        let r = UpdateIssueReq {
            summary: Some("a\nb".into()),
            description: Some("  ".into()),
            labels: Some(vec!["ok".into(), "has space".into()]),
            add_labels: vec!["x".into()],
            parent: Some("P-1".into()),
            epic: Some("E-2".into()),
            ..Default::default()
        };
        let p = update_problems(&r);
        assert_eq!(p.len(), 5, "{p:?}");
        assert!(p[0].starts_with("description must not be empty"));
        assert!(p[1].contains("single line"));
        assert!(p[2].contains("not both"));
        assert!(p[3].contains("\"has space\""));
        assert!(p[4].contains("disagree"));
    }

    #[test]
    fn a_single_change_is_enough_and_the_summary_limit_counts_characters() {
        let mut r = UpdateIssueReq {
            summary: Some("é".repeat(SUMMARY_MAX)),
            ..Default::default()
        };
        assert!(update_problems(&r).is_empty());
        r.summary.as_mut().unwrap().push('x');
        assert!(update_problems(&r)[0].contains("256 characters"));
        let only_remove = UpdateIssueReq {
            remove_labels: vec!["stale".into()],
            ..Default::default()
        };
        assert!(update_problems(&only_remove).is_empty());
        // Clearing every label is a legitimate replace.
        let clear = UpdateIssueReq {
            labels: Some(Vec::new()),
            ..Default::default()
        };
        assert!(update_problems(&clear).is_empty());
    }
}
