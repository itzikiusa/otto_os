//! Creating an issue: the request / response shapes shared by
//! `POST /issue/{account_id}/issues` and the `otto.create_issue` MCP tool, and
//! the checks that need no tracker round trip. The Jira pre-checks against
//! createmeta and the read-back verification live in `jira::create`.

use otto_core::Error;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

/// Jira's summary limit (characters).
pub const SUMMARY_MAX: usize = 255;

/// `POST /api/v1/issue/{account_id}/issues`.
///
/// `project_key` is a Jira project key (Linear: team key), `issue_type` a type
/// name or id from `…/{project_key}/issue-types` (case-insensitive), and
/// `description` Markdown (converted to ADF for Jira). `labels`, `priority`
/// (name or id), `parent` / `epic` (issue keys — in Jira Cloud an epic IS the
/// parent) and `fields` (extra Jira-native field values keyed by field id,
/// e.g. a required custom field) are Jira-only.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct CreateIssueReq {
    pub project_key: String,
    pub issue_type: String,
    pub summary: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub labels: Vec<String>,
    #[serde(default)]
    pub priority: Option<String>,
    #[serde(default)]
    pub parent: Option<String>,
    #[serde(default)]
    pub epic: Option<String>,
    #[serde(default)]
    pub fields: Map<String, Value>,
}

/// One post-create comparison.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationCheck {
    pub name: String,
    pub ok: bool,
    pub detail: String,
}

/// The read-back of a created issue against the request. `ok` is false when
/// any check failed — the issue still exists (it is never deleted).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IssueVerification {
    pub ok: bool,
    pub checks: Vec<VerificationCheck>,
}

impl IssueVerification {
    pub fn from_checks(checks: Vec<VerificationCheck>) -> Self {
        Self {
            ok: !checks.is_empty() && checks.iter().all(|c| c.ok),
            checks,
        }
    }
}

/// Response of `POST /issue/{account_id}/issues`. `verification` is present
/// for Jira; Linear has no read-back yet.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CreateIssueResp {
    pub key: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verification: Option<IssueVerification>,
}

pub(crate) fn check(name: &str, ok: bool, detail: impl Into<String>) -> VerificationCheck {
    VerificationCheck {
        name: name.to_string(),
        ok,
        detail: detail.into(),
    }
}

/// Compare two block outlines (`adf_outline` / `storage_outline`); a mismatch
/// names the first differing block and, for tables, both shapes.
pub(crate) fn outline_check(name: &str, sent: &[String], stored: &[String]) -> VerificationCheck {
    let tables = |o: &[String]| -> String {
        let t: Vec<&str> = o
            .iter()
            .filter(|e| e.starts_with("table["))
            .map(String::as_str)
            .collect();
        if t.is_empty() {
            "no tables".into()
        } else {
            t.join(", ")
        }
    };
    if sent == stored {
        return check(
            name,
            true,
            format!("{} blocks match ({})", sent.len(), tables(sent)),
        );
    }
    let n = sent.iter().zip(stored).take_while(|(x, y)| x == y).count();
    check(
        name,
        false,
        format!(
            "block {n} differs: sent {}, stored {} (sent {} blocks with {}; stored {} blocks with {})",
            sent.get(n).map_or("(end)", String::as_str),
            stored.get(n).map_or("(end)", String::as_str),
            sent.len(),
            tables(sent),
            stored.len(),
            tables(stored)
        ),
    )
}

/// Compare two whitespace-collapsed texts.
pub(crate) fn text_check(name: &str, sent: &str, stored: &str) -> VerificationCheck {
    if sent == stored {
        return check(
            name,
            true,
            format!("{} characters match", sent.chars().count()),
        );
    }
    let n = sent
        .chars()
        .zip(stored.chars())
        .take_while(|(x, y)| x == y)
        .count();
    let around = |s: &str| {
        s.chars()
            .skip(n.saturating_sub(10))
            .take(30)
            .collect::<String>()
    };
    check(
        name,
        false,
        format!(
            "text differs from character {n}: sent \"…{}…\", stored \"…{}…\"",
            around(sent),
            around(stored)
        ),
    )
}

pub(crate) fn non_empty(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// Problems detectable without the tracker: blank project / type, a summary
/// that is empty, multi-line or over [`SUMMARY_MAX`] characters, and labels
/// that are empty or contain whitespace.
pub fn local_problems(req: &CreateIssueReq) -> Vec<String> {
    let mut p = Vec::new();
    if req.project_key.trim().is_empty() {
        p.push("project_key must not be empty".to_string());
    }
    if req.issue_type.trim().is_empty() {
        p.push("issue_type must not be empty".to_string());
    }
    let summary = req.summary.trim();
    if summary.is_empty() {
        p.push("summary must not be empty".to_string());
    } else if summary.contains(['\n', '\r']) {
        p.push("summary must be a single line (put details in the description)".to_string());
    }
    let n = summary.chars().count();
    if n > SUMMARY_MAX {
        p.push(format!(
            "summary is {n} characters; the limit is {SUMMARY_MAX}"
        ));
    }
    let bad: Vec<&str> = req
        .labels
        .iter()
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
    p
}

/// One `400` naming every problem, so a caller fixes them in one round trip.
pub fn problems_error(project: &str, issue_type: &str, problems: &[String]) -> Error {
    Error::Invalid(format!(
        "cannot create the issue in project '{project}' as '{issue_type}':\n- {}",
        problems.join("\n- ")
    ))
}

/// Linear takes only project (team), type, summary and description.
pub fn linear_problems(req: &CreateIssueReq) -> Vec<String> {
    let mut extra = Vec::new();
    if !req.labels.is_empty() {
        extra.push("labels");
    }
    if non_empty(&req.priority).is_some() {
        extra.push("priority");
    }
    if non_empty(&req.parent).is_some() {
        extra.push("parent");
    }
    if non_empty(&req.epic).is_some() {
        extra.push("epic");
    }
    if !req.fields.is_empty() {
        extra.push("fields");
    }
    if extra.is_empty() {
        return Vec::new();
    }
    vec![format!(
        "{} {} Jira-only; a Linear account takes project_key (the team key), issue_type, \
         summary and description",
        extra.join(", "),
        if extra.len() == 1 { "is" } else { "are" }
    )]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> CreateIssueReq {
        CreateIssueReq {
            project_key: "PRJ".into(),
            issue_type: "Task".into(),
            summary: "Fix it".into(),
            ..Default::default()
        }
    }

    #[test]
    fn local_checks_cover_summary_and_labels() {
        assert!(local_problems(&req()).is_empty());
        let mut r = req();
        r.summary = "a\nb".into();
        r.labels = vec!["ok".into(), "has space".into(), " ".into()];
        let p = local_problems(&r);
        assert_eq!(p.len(), 2, "{p:?}");
        assert!(p[0].contains("single line"));
        assert!(
            p[1].contains("\"has space\"") && p[1].contains("\" \""),
            "{}",
            p[1]
        );
        r = req();
        r.summary = "é".repeat(SUMMARY_MAX);
        assert!(local_problems(&r).is_empty(), "the limit counts characters");
        r.summary.push('x');
        assert!(local_problems(&r)[0].contains("256 characters"));
        r = req();
        r.project_key = " ".into();
        r.summary = String::new();
        assert_eq!(local_problems(&r).len(), 2);
    }

    #[test]
    fn linear_refuses_jira_only_arguments_by_name() {
        assert!(linear_problems(&req()).is_empty());
        let mut r = req();
        r.priority = Some("High".into());
        r.fields.insert("customfield_1".into(), Value::Null);
        let p = linear_problems(&r);
        assert!(p[0].starts_with("priority, fields are Jira-only"), "{p:?}");
    }

    #[test]
    fn verification_is_ok_only_when_every_check_passed() {
        let c = |ok| VerificationCheck {
            name: "x".into(),
            ok,
            detail: String::new(),
        };
        assert!(IssueVerification::from_checks(vec![c(true), c(true)]).ok);
        assert!(!IssueVerification::from_checks(vec![c(true), c(false)]).ok);
        assert!(!IssueVerification::from_checks(Vec::new()).ok);
    }
}
