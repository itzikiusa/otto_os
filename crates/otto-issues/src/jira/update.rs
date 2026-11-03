//! Checked issue edits for Jira Cloud (REST v3).
//!
//! 1. **Local checks** ([`update_problems`]) — one `400` listing every problem.
//! 2. **Resolve** what needs the tracker: a priority name → id against the
//!    issue's editmeta, a parent / epic key that must exist (an epic must be an
//!    Epic), and the description guard (a description holding content the
//!    Markdown round trip would drop is never overwritten).
//! 3. **Write** `PUT /rest/api/3/issue/{key}` (`fields` + label `update` ops;
//!    204), then the assignee through its own endpoint (it is rarely on the
//!    edit screen). Jira's 400 field errors come back as one readable `400`.

use otto_core::{Error, Result};
use serde_json::{json, Map, Value};

use super::create::jira_error_text;
use super::{path_seg, JiraClient};
use crate::issue_create::non_empty;
use crate::issue_update::{
    update_problems, update_problems_error, AssigneeChange, UpdateIssueReq, UpdateIssueResp,
};
use crate::md_adf::markdown_to_adf;

/// The `PUT /rest/api/3/issue/{key}` body for the resolved values: `fields`
/// for summary / description / labels (replace) / priority / parent, and
/// `update` ops for `add_labels` / `remove_labels`. Pure — unit-tested.
pub fn build_update_issue_body(
    req: &UpdateIssueReq,
    description_adf: Option<Value>,
    priority_id: Option<&str>,
    parent_key: Option<&str>,
) -> Value {
    let mut fields = Map::new();
    if let Some(s) = non_empty(&req.summary) {
        fields.insert("summary".into(), json!(s));
    }
    if let Some(adf) = description_adf {
        fields.insert("description".into(), adf);
    }
    if let Some(labels) = &req.labels {
        let set: Vec<&str> = labels.iter().map(|l| l.trim()).collect();
        fields.insert("labels".into(), json!(set));
    }
    if let Some(id) = priority_id {
        fields.insert("priority".into(), json!({ "id": id }));
    }
    if let Some(k) = parent_key {
        fields.insert("parent".into(), json!({ "key": k }));
    }
    let ops: Vec<Value> = req
        .add_labels
        .iter()
        .map(|l| json!({ "add": l.trim() }))
        .chain(
            req.remove_labels
                .iter()
                .map(|l| json!({ "remove": l.trim() })),
        )
        .collect();
    let mut body = Map::new();
    if !fields.is_empty() {
        body.insert("fields".into(), Value::Object(fields));
    }
    if !ops.is_empty() {
        body.insert("update".into(), json!({ "labels": ops }));
    }
    Value::Object(body)
}

/// The field names a request writes, in a stable order (the response's
/// `updated`).
fn written(req: &UpdateIssueReq) -> Vec<String> {
    let mut out = Vec::new();
    let mut push = |on: bool, name: &str| {
        if on {
            out.push(name.to_string());
        }
    };
    push(req.summary.is_some(), "summary");
    push(req.description.is_some(), "description");
    push(
        req.labels.is_some() || !req.add_labels.is_empty() || !req.remove_labels.is_empty(),
        "labels",
    );
    push(req.priority.is_some(), "priority");
    push(req.parent.is_some() || req.epic.is_some(), "parent");
    push(req.assignee.is_some(), "assignee");
    out
}

impl JiraClient {
    /// `PUT /rest/api/3/issue/{key}/assignee`. Jira Cloud unassigns on
    /// `accountId: null` — `"-1"` would pick the project's DEFAULT assignee.
    async fn put_assignee(&self, key: &str, change: AssigneeChange<'_>) -> Result<()> {
        let account = match change {
            AssigneeChange::Unassign => Value::Null,
            AssigneeChange::To(id) => json!(id),
        };
        let req = self
            .t
            .http()?
            .put(format!(
                "{}/rest/api/3/issue/{}/assignee",
                self.t.base,
                path_seg(key)
            ))
            .header("Authorization", &self.t.auth_header)
            .header("Accept", "application/json")
            .json(&json!({ "accountId": account }));
        self.t
            .send(req, &format!("jira update_issue assignee {key}"))
            .await?;
        Ok(())
    }

    /// Refuse to overwrite a description holding content Otto's Markdown
    /// can't carry: it came from `adf_to_markdown`, which drops screenshots,
    /// mentions, smart links, dates, status lozenges and emoji, and writing it
    /// back (via `markdown_to_adf`, which has no syntax for them) silently
    /// DELETED them from the issue.
    pub(super) async fn ensure_description_preservable(&self, key: &str) -> Result<()> {
        let current = self.description_adf(key).await?;
        let lossy = crate::adf::adf_lossy_nodes(&current);
        if lossy.is_empty() {
            return Ok(());
        }
        Err(Error::Conflict(format!(
            "the description of {key} contains content Otto can't preserve ({}); saving would \
             delete it — edit the description in Jira instead",
            lossy.join(", ")
        )))
    }

    /// Edit an issue: see the module docs. Returns which fields were written.
    pub async fn update_issue_checked(
        &self,
        key: &str,
        req: &UpdateIssueReq,
    ) -> Result<UpdateIssueResp> {
        let key = key.trim();
        if key.is_empty() {
            return Err(Error::Invalid("issue key must not be empty".into()));
        }
        let mut problems = update_problems(req);
        if !problems.is_empty() {
            return Err(update_problems_error(key, &problems));
        }
        let description = match non_empty(&req.description) {
            Some(md) => {
                let adf = markdown_to_adf(md)?;
                self.ensure_description_preservable(key).await?;
                Some(adf)
            }
            None => None,
        };

        let mut priority_id = None;
        if let Some(p) = non_empty(&req.priority) {
            let meta = self.editmeta(key).await?;
            match meta.iter().find(|f| f.key == "priority") {
                None => problems.push(format!("`priority` is not editable on {key}")),
                Some(f) => match f
                    .allowed_values
                    .iter()
                    .find(|o| o.id == p || o.label.eq_ignore_ascii_case(p))
                {
                    Some(o) => priority_id = Some(o.id.clone()),
                    None => problems.push(format!(
                        "priority '{p}' is not valid; valid priorities: {}",
                        f.allowed_values
                            .iter()
                            .map(|o| o.label.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                },
            }
        }

        let epic = non_empty(&req.epic);
        let parent = epic.or(non_empty(&req.parent));
        if let Some(target) = parent {
            match self.issue_type_of(target).await? {
                None => problems.push(format!(
                    "{} '{target}' does not exist or is not visible to this account",
                    if epic.is_some() { "epic" } else { "parent" }
                )),
                Some((name, level))
                    if epic.is_some() && level != 1 && !name.eq_ignore_ascii_case("epic") =>
                {
                    problems.push(format!("epic '{target}' is a {name}, not an Epic"))
                }
                Some(_) => {}
            }
        }
        if !problems.is_empty() {
            return Err(update_problems_error(key, &problems));
        }

        let body = build_update_issue_body(req, description, priority_id.as_deref(), parent);
        if body.as_object().is_some_and(|o| !o.is_empty()) {
            let put = self
                .t
                .http()?
                .put(format!(
                    "{}/rest/api/3/issue/{}",
                    self.t.base,
                    path_seg(key)
                ))
                .header("Authorization", &self.t.auth_header)
                .header("Accept", "application/json")
                .json(&body);
            // `send_raw` never re-sends a write; 401/403/429 are classified by
            // the transport, a 400 keeps Jira's own field errors.
            let what = format!("jira update_issue {key}");
            let resp = self.t.send_raw(put, &what).await?;
            let status = resp.status();
            if status.as_u16() == 400 {
                let text = resp.text().await.unwrap_or_default();
                let body = serde_json::from_str(&text).unwrap_or(Value::String(text));
                return Err(Error::Invalid(format!(
                    "Jira refused the update of {key}: {}",
                    jira_error_text(&body)
                )));
            }
            if !status.is_success() {
                return Err(self.t.fail(resp, &what).await);
            }
        }
        if let Some(change) = req.assignee_change() {
            self.put_assignee(key, change).await.map_err(|e| {
                if body.as_object().is_some_and(|o| !o.is_empty()) {
                    Error::Upstream(format!(
                        "{key}: the other fields were saved, but setting the assignee failed: {e}"
                    ))
                } else {
                    e
                }
            })?;
        }
        Ok(UpdateIssueResp {
            url: format!("{}/browse/{key}", self.t.base),
            key: key.to_string(),
            updated: written(req),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn req() -> UpdateIssueReq {
        UpdateIssueReq::default()
    }

    #[test]
    fn test_build_update_issue_body_sets_fields() {
        let r = UpdateIssueReq {
            summary: Some("  New title ".into()),
            labels: Some(vec![" a ".into(), "b".into()]),
            ..req()
        };
        let adf = json!({"type": "doc", "version": 1, "content": []});
        let body = build_update_issue_body(&r, Some(adf.clone()), Some("3"), Some("EPIC-1"));
        assert_eq!(
            body,
            json!({"fields": {
                "summary": "New title",
                "description": adf,
                "labels": ["a", "b"],
                "priority": {"id": "3"},
                "parent": {"key": "EPIC-1"},
            }})
        );
        assert_eq!(
            written(&UpdateIssueReq {
                priority: Some("High".into()),
                ..r
            }),
            vec!["summary", "labels", "priority"]
        );
    }

    #[test]
    fn test_build_update_issue_body_label_ops_and_clear() {
        let r = UpdateIssueReq {
            add_labels: vec!["new".into()],
            remove_labels: vec![" old ".into()],
            ..req()
        };
        assert_eq!(
            build_update_issue_body(&r, None, None, None),
            json!({"update": {"labels": [{"add": "new"}, {"remove": "old"}]}})
        );
        let clear = UpdateIssueReq {
            labels: Some(Vec::new()),
            ..req()
        };
        assert_eq!(
            build_update_issue_body(&clear, None, None, None),
            json!({"fields": {"labels": []}})
        );
        // An assignee-only edit writes no PUT body (it has its own endpoint).
        let assign = UpdateIssueReq {
            assignee: Some(Some("acc-1".into())),
            ..req()
        };
        assert_eq!(
            build_update_issue_body(&assign, None, None, None),
            json!({})
        );
    }

    /// A loopback server answering `replies` in order (the last one repeats),
    /// recording each raw request.
    async fn scripted(
        replies: Vec<&'static str>,
    ) -> (String, std::sync::Arc<std::sync::Mutex<Vec<String>>>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let log = seen.clone();
        tokio::spawn(async move {
            loop {
                let Ok((mut sock, _)) = listener.accept().await else {
                    return;
                };
                let mut buf = vec![0u8; 16384];
                let n = sock.read(&mut buf).await.unwrap_or(0);
                let idx = {
                    let mut l = log.lock().unwrap();
                    l.push(String::from_utf8_lossy(&buf[..n]).into_owned());
                    l.len() - 1
                };
                let reply = replies[idx.min(replies.len() - 1)];
                let _ = sock.write_all(reply.as_bytes()).await;
            }
        });
        (format!("http://{addr}"), seen)
    }

    const NO_CONTENT: &str =
        "HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

    /// A JSON reply with the right Content-Length.
    fn json_reply(status: &str, body: &str) -> &'static str {
        Box::leak(
            format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .into_boxed_str(),
        )
    }

    #[tokio::test]
    async fn a_summary_edit_is_one_put_and_reports_what_it_wrote() {
        let (base, seen) = scripted(vec![NO_CONTENT]).await;
        let r = UpdateIssueReq {
            summary: Some("Renamed".into()),
            add_labels: vec!["triaged".into()],
            ..req()
        };
        let resp = JiraClient::new(&base, "e", "t")
            .update_issue_checked("PRJ-7", &r)
            .await
            .unwrap();
        assert_eq!(resp.key, "PRJ-7");
        assert_eq!(resp.url, format!("{base}/browse/PRJ-7"));
        assert_eq!(resp.updated, vec!["summary", "labels"]);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1, "{seen:?}");
        assert!(
            seen[0].starts_with("PUT /rest/api/3/issue/PRJ-7 "),
            "{}",
            seen[0]
        );
        assert!(seen[0].contains(r#""summary":"Renamed""#), "{}", seen[0]);
        assert!(seen[0].contains(r#""update":{"labels":[{"add":"triaged"}]}"#));
    }

    /// `null` / `"none"` unassign with Jira's `accountId: null` (never
    /// `"-1"`, the project's default assignee) and write no field PUT.
    #[tokio::test]
    async fn an_unassign_sends_a_null_account_id_to_the_assignee_endpoint() {
        for assignee in [None, Some("none".to_string())] {
            let (base, seen) = scripted(vec![NO_CONTENT]).await;
            let r = UpdateIssueReq {
                assignee: Some(assignee),
                ..req()
            };
            let resp = JiraClient::new(&base, "e", "t")
                .update_issue_checked("PRJ-7", &r)
                .await
                .unwrap();
            assert_eq!(resp.updated, vec!["assignee"]);
            let seen = seen.lock().unwrap();
            assert_eq!(seen.len(), 1, "{seen:?}");
            assert!(
                seen[0].starts_with("PUT /rest/api/3/issue/PRJ-7/assignee "),
                "{}",
                seen[0]
            );
            assert!(seen[0].ends_with(r#"{"accountId":null}"#), "{}", seen[0]);
        }
    }

    #[tokio::test]
    async fn a_jira_400_names_the_field_error() {
        let (base, _) = scripted(vec![json_reply(
            "400 Bad Request",
            r#"{"errorMessages":[],"errors":{"labels":"Field cannot be set"}}"#,
        )])
        .await;
        let r = UpdateIssueReq {
            labels: Some(vec!["x".into()]),
            ..req()
        };
        let err = JiraClient::new(&base, "e", "t")
            .update_issue_checked("PRJ-7", &r)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, Error::Invalid(m) if m.contains("labels: Field cannot be set")),
            "{err:?}"
        );
    }

    #[tokio::test]
    async fn an_unknown_priority_lists_the_valid_ones_and_writes_nothing() {
        let editmeta = r#"{"fields":{"priority":{"name":"Priority","schema":{"type":"priority"},"allowedValues":[{"id":"1","name":"High"},{"id":"3","name":"Low"}]}}}"#;
        let (base, seen) = scripted(vec![json_reply("200 OK", editmeta)]).await;
        let r = UpdateIssueReq {
            priority: Some("urgent".into()),
            ..req()
        };
        let err = JiraClient::new(&base, "e", "t")
            .update_issue_checked("PRJ-7", &r)
            .await
            .unwrap_err();
        assert!(
            matches!(&err, Error::Invalid(m) if m.contains("valid priorities: High, Low")),
            "{err:?}"
        );
        let seen = seen.lock().unwrap();
        assert!(seen.iter().all(|r| !r.starts_with("PUT")), "{seen:?}");
    }

    #[tokio::test]
    async fn local_problems_never_reach_jira() {
        let (base, seen) = scripted(vec![NO_CONTENT]).await;
        let err = JiraClient::new(&base, "e", "t")
            .update_issue_checked("PRJ-7", &req())
            .await
            .unwrap_err();
        assert!(matches!(&err, Error::Invalid(m) if m.contains("nothing to update")));
        assert!(seen.lock().unwrap().is_empty());
    }
}
