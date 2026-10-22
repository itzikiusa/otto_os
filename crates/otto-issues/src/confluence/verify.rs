//! Read-back verification of Confluence writes.
//!
//! After a page create / update or a footer comment, the content is fetched
//! again with `body.storage` and compared with what was sent: title, space,
//! parent and version for pages, the container for comments, and for both
//! the body's block outline (headings, list sizes, task lists, table rows ×
//! columns + header row, code macros with their language, panels) and text.
//! The result rides next to the write's own response as
//! `verification: {ok, checks}`; a failure is reported, never undone.

use otto_core::api::ConfluencePageResp;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::ConfluenceClient;
use crate::issue_create::{check, outline_check, text_check, IssueVerification, VerificationCheck};
use crate::jira::{path_seg, CommentRef};
use crate::storage_check::{storage_outline, storage_plain_text};

/// What a page write should have left behind.
pub struct PageExpect<'a> {
    pub title: &'a str,
    pub space_key: Option<&'a str>,
    pub parent_id: Option<&'a str>,
    pub version: i64,
    /// The storage body that was sent.
    pub storage: &'a str,
}

/// `POST` / `PUT /issue/confluence/pages…`: the page plus its read-back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PageWriteResp {
    #[serde(flatten)]
    pub page: ConfluencePageResp,
    pub verification: IssueVerification,
}

/// `POST /issue/confluence/pages/{id}/comments`: the comment plus its read-back.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CommentWriteResp {
    #[serde(flatten)]
    pub comment: CommentRef,
    pub verification: IssueVerification,
}

impl ConfluenceClient {
    /// `GET /content/{id}?expand=…` as raw JSON (errors as text for a check).
    async fn read_back(&self, id: &str, expand: &str) -> Result<Value, String> {
        let resp = self
            .t
            .http()
            .map_err(|e| e.to_string())?
            .get(self.api(&format!("/content/{}", path_seg(id))))
            .header("Authorization", &self.t.auth_header)
            .header("Accept", "application/json")
            .query(&[("expand", expand)])
            .send()
            .await
            .map_err(|e| format!("request failed: {e}"))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        if !status.is_success() {
            return Err(format!(
                "answered {status}: {}",
                text.chars().take(300).collect::<String>()
            ));
        }
        serde_json::from_str(&text).map_err(|e| format!("unreadable JSON: {e}"))
    }

    /// Read a written page back and compare it with `exp`.
    pub async fn verify_page(&self, id: &str, exp: &PageExpect<'_>) -> IssueVerification {
        match self
            .read_back(id, "body.storage,version,space,ancestors")
            .await
        {
            Ok(body) => IssueVerification::from_checks(compare_page(exp, &body)),
            Err(e) => IssueVerification::from_checks(vec![check(
                "read_back",
                false,
                format!("the page was written but reading it back {e}"),
            )]),
        }
    }

    /// Read a new footer comment back and compare it with what was sent.
    pub async fn verify_comment(
        &self,
        comment_id: &str,
        page_id: &str,
        storage: &str,
    ) -> IssueVerification {
        match self.read_back(comment_id, "body.storage,container").await {
            Ok(body) => IssueVerification::from_checks(compare_comment(page_id, storage, &body)),
            Err(e) => IssueVerification::from_checks(vec![check(
                "read_back",
                false,
                format!("the comment was posted but reading it back {e}"),
            )]),
        }
    }
}

fn s(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Null => "(none)".into(),
        other => other.to_string(),
    }
}

fn body_checks(sent: &str, body: &Value, out: &mut Vec<VerificationCheck>) {
    let stored = body["body"]["storage"]["value"].as_str().unwrap_or("");
    out.push(outline_check(
        "body_structure",
        &storage_outline(sent),
        &storage_outline(stored),
    ));
    out.push(text_check(
        "body_text",
        &storage_plain_text(sent),
        &storage_plain_text(stored),
    ));
}

/// The page read-back checks (pure, unit-tested).
pub(crate) fn compare_page(exp: &PageExpect<'_>, body: &Value) -> Vec<VerificationCheck> {
    let mut out = Vec::new();
    let title = s(&body["title"]);
    out.push(check(
        "title",
        title == exp.title,
        format!("expected {:?}, got {title:?}", exp.title),
    ));
    if let Some(space) = exp.space_key {
        let got = s(&body["space"]["key"]);
        out.push(check(
            "space",
            got.eq_ignore_ascii_case(space),
            format!("expected {space}, got {got}"),
        ));
    }
    if let Some(parent) = exp.parent_id {
        let got = body["ancestors"]
            .as_array()
            .and_then(|a| a.last())
            .map_or("(none)".to_string(), |a| s(&a["id"]));
        out.push(check(
            "parent",
            got == parent,
            format!("expected {parent}, got {got}"),
        ));
    }
    let version = body["version"]["number"].as_i64();
    out.push(check(
        "version",
        version == Some(exp.version),
        format!(
            "expected {}, got {}",
            exp.version,
            version.map_or("(none)".into(), |v| v.to_string())
        ),
    ));
    body_checks(exp.storage, body, &mut out);
    out
}

/// The comment read-back checks (pure, unit-tested).
pub(crate) fn compare_comment(
    page_id: &str,
    storage: &str,
    body: &Value,
) -> Vec<VerificationCheck> {
    let container = s(&body["container"]["id"]);
    let mut out = vec![check(
        "container",
        container == page_id,
        format!("expected page {page_id}, got {container}"),
    )];
    body_checks(storage, body, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SENT: &str = "<h2>T</h2><table><tbody><tr><th><p>a</p></th><th><p>b</p></th></tr><tr><td><p>1</p></td><td><p>2</p></td></tr></tbody></table><ac:structured-macro ac:name=\"code\"><ac:parameter ac:name=\"language\">sql</ac:parameter><ac:plain-text-body><![CDATA[select 1]]></ac:plain-text-body></ac:structured-macro>";

    fn exp() -> PageExpect<'static> {
        PageExpect {
            title: "Plan",
            space_key: Some("ENG"),
            parent_id: Some("42"),
            version: 3,
            storage: SENT,
        }
    }

    fn page(storage: &str) -> Value {
        json!({"title": "Plan", "space": {"key": "ENG"}, "version": {"number": 3},
               "ancestors": [{"id": "1"}, {"id": "42"}],
               "body": {"storage": {"value": storage}}})
    }

    #[test]
    fn a_faithful_page_passes_every_check() {
        // Confluence normalises attrs (macro ids, table layout): still equal.
        let stored = SENT
            .replace("<table>", "<table data-layout=\"default\">")
            .replace(
                "ac:name=\"code\"",
                "ac:name=\"code\" ac:schema-version=\"1\" ac:macro-id=\"x\"",
            );
        let checks = compare_page(&exp(), &page(&stored));
        let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "title",
                "space",
                "parent",
                "version",
                "body_structure",
                "body_text"
            ]
        );
        assert!(checks.iter().all(|c| c.ok), "{checks:?}");
    }

    #[test]
    fn page_mismatches_name_both_sides() {
        let stored = SENT
            .replace("<tr><td><p>1</p></td><td><p>2</p></td></tr>", "")
            .replace(">sql<", ">text<");
        let mut body = page(&stored);
        body["version"]["number"] = json!(2);
        body["ancestors"] = json!([]);
        let v = IssueVerification::from_checks(compare_page(&exp(), &body));
        assert!(!v.ok);
        let bad: Vec<(&str, &str)> = v
            .checks
            .iter()
            .filter(|c| !c.ok)
            .map(|c| (c.name.as_str(), c.detail.as_str()))
            .collect();
        assert_eq!(bad[0], ("parent", "expected 42, got (none)"));
        assert_eq!(bad[1], ("version", "expected 3, got 2"));
        assert_eq!(bad[2].0, "body_structure");
        assert!(
            bad[2]
                .1
                .contains("sent table[2x2,header], stored table[1x2,header]"),
            "{}",
            bad[2].1
        );
        assert_eq!(bad[3].0, "body_text");
    }

    #[test]
    fn a_comment_on_another_page_fails_the_container_check() {
        let body = json!({"container": {"id": "7"}, "body": {"storage": {"value": "<p>hi</p>"}}});
        let checks = compare_comment("7", "<p>hi</p>", &body);
        assert!(checks.iter().all(|c| c.ok), "{checks:?}");
        let checks = compare_comment("8", "<p>hi</p>", &body);
        assert!(!checks[0].ok && checks[0].detail == "expected page 8, got 7");
    }
}
