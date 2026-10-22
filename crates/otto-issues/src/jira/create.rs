//! Checked issue creation for Jira Cloud (REST v3).
//!
//! 1. **Pre-checks** against createmeta
//!    (`/rest/api/3/issue/createmeta/{project}/issuetypes[/{id}]`): the project
//!    exists and is creatable, the type is valid for it (name or id,
//!    case-insensitive), every required field is supplied, and labels /
//!    priority / parent / epic are settable and valid. Every problem is
//!    reported in ONE `400` listing the valid values.
//! 2. **Create** with the description converted Markdown → validated ADF.
//! 3. **Verify** by reading the issue back and comparing key, project, type,
//!    summary, labels, priority, parent and the description's block structure
//!    and text. A failed check is reported, never "fixed" by deleting.

use otto_core::{Error, Result};
use serde_json::{json, Map, Value};

use super::{path_seg, JiraClient};
use crate::adf_check::{adf_outline, adf_plain_text};
use crate::issue_create::{
    check, local_problems, non_empty, outline_check, problems_error, text_check, CreateIssueReq,
    CreateIssueResp, IssueVerification, VerificationCheck,
};
use crate::md_adf::markdown_to_adf;

/// Fields set through their own argument; `fields` may not override them.
const RESERVED: &[&str] = &[
    "project",
    "issuetype",
    "summary",
    "description",
    "labels",
    "priority",
    "parent",
];
/// Page cap for a createmeta walk (100 per page).
const META_PAGES_MAX: usize = 10;

struct TypeMeta {
    id: String,
    name: String,
    subtask: bool,
}

struct FieldMeta {
    id: String,
    name: String,
    required: bool,
    has_default: bool,
    /// `(id, name)` of each allowed value.
    allowed: Vec<(String, String)>,
}

/// What the read-back is compared with.
struct Expected {
    project: String,
    type_id: String,
    type_name: String,
    summary: String,
    labels: Vec<String>,
    priority: Option<(String, String)>,
    parent: Option<String>,
    description: Option<Value>,
}

/// Jira's `errorMessages` + `errors` as one line (the raw body otherwise).
fn jira_error_text(body: &Value) -> String {
    let mut parts: Vec<String> = body["errorMessages"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|m| m.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if let Some(errs) = body["errors"].as_object() {
        parts.extend(
            errs.iter()
                .map(|(k, v)| format!("{k}: {}", v.as_str().unwrap_or(&v.to_string()))),
        );
    }
    if parts.is_empty() {
        match body {
            Value::String(s) => s.chars().take(500).collect(),
            other => other.to_string().chars().take(500).collect(),
        }
    } else {
        parts.join("; ")
    }
}

impl JiraClient {
    async fn get_json(&self, url: &str, what: &str) -> Result<(reqwest::StatusCode, Value)> {
        let resp = self
            .t
            .http()?
            .get(url)
            .header("Authorization", &self.t.auth_header)
            .header("Accept", "application/json")
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira {what} request: {e}")))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let body = serde_json::from_str(&text).unwrap_or(Value::String(text));
        Ok((status, body))
    }

    /// Walk a paged createmeta listing, returning every item.
    async fn createmeta_pages(
        &self,
        path: &str,
        project: &str,
        keys: &[&str],
    ) -> Result<Vec<Value>> {
        let mut items = Vec::new();
        for _ in 0..META_PAGES_MAX {
            let url = format!(
                "{}/rest/api/3/issue/createmeta/{path}?startAt={}&maxResults=100",
                self.t.base,
                items.len()
            );
            let (status, body) = self.get_json(&url, "createmeta").await?;
            match status.as_u16() {
                200..=299 => {}
                400 | 403 | 404 => {
                    return Err(Error::Invalid(format!(
                        "project '{project}' does not exist or this Jira account cannot create \
                         issues in it ({status}: {})",
                        jira_error_text(&body)
                    )))
                }
                _ => {
                    return Err(Error::Upstream(format!(
                        "jira createmeta failed ({status}): {}",
                        jira_error_text(&body)
                    )))
                }
            }
            let page = keys
                .iter()
                .find_map(|k| body[*k].as_array())
                .cloned()
                .unwrap_or_default();
            let got = page.len();
            items.extend(page);
            let done = match body["total"].as_u64() {
                Some(total) => items.len() as u64 >= total,
                None => body["isLast"].as_bool() != Some(false),
            };
            if got == 0 || done {
                break;
            }
        }
        Ok(items)
    }

    async fn create_meta_types(&self, project: &str) -> Result<Vec<TypeMeta>> {
        let path = format!("{}/issuetypes", path_seg(project));
        let items = self
            .createmeta_pages(&path, project, &["issueTypes", "values"])
            .await?;
        Ok(items
            .iter()
            .filter_map(|t| {
                Some(TypeMeta {
                    id: t["id"].as_str()?.to_string(),
                    name: t["name"].as_str().unwrap_or_default().to_string(),
                    subtask: t["subtask"].as_bool().unwrap_or(false),
                })
            })
            .collect())
    }

    async fn create_meta_fields(&self, project: &str, type_id: &str) -> Result<Vec<FieldMeta>> {
        let path = format!("{}/issuetypes/{}", path_seg(project), path_seg(type_id));
        let items = self
            .createmeta_pages(&path, project, &["fields", "values", "results"])
            .await?;
        Ok(items
            .iter()
            .filter_map(|f| {
                let id = f["fieldId"].as_str().or_else(|| f["key"].as_str())?;
                Some(FieldMeta {
                    id: id.to_string(),
                    name: f["name"].as_str().unwrap_or(id).to_string(),
                    required: f["required"].as_bool().unwrap_or(false),
                    has_default: f["hasDefaultValue"].as_bool().unwrap_or(false),
                    allowed: f["allowedValues"]
                        .as_array()
                        .map(|a| {
                            a.iter()
                                .map(|v| {
                                    let name = v["name"].as_str().or_else(|| v["value"].as_str());
                                    (
                                        v["id"].as_str().unwrap_or_default().to_string(),
                                        name.unwrap_or_default().to_string(),
                                    )
                                })
                                .collect()
                        })
                        .unwrap_or_default(),
                })
            })
            .collect())
    }

    /// `(type name, hierarchy level)` of an existing issue, or None if it is
    /// missing / not visible.
    async fn issue_type_of(&self, key: &str) -> Result<Option<(String, i64)>> {
        let url = format!(
            "{}/rest/api/3/issue/{}?fields=issuetype",
            self.t.base,
            path_seg(key)
        );
        let (status, body) = self.get_json(&url, "parent lookup").await?;
        match status.as_u16() {
            200..=299 => {
                let t = &body["fields"]["issuetype"];
                Ok(Some((
                    t["name"].as_str().unwrap_or_default().to_string(),
                    t["hierarchyLevel"].as_i64().unwrap_or(0),
                )))
            }
            403 | 404 => Ok(None),
            _ => Err(Error::Upstream(format!(
                "jira lookup of {key} failed ({status}): {}",
                jira_error_text(&body)
            ))),
        }
    }

    /// Validate `req` against createmeta, create the issue, then read it back.
    pub async fn create_issue_checked(&self, req: &CreateIssueReq) -> Result<CreateIssueResp> {
        let project = req.project_key.trim();
        let wanted_type = req.issue_type.trim();
        let local = local_problems(req);
        if !local.is_empty() {
            return Err(problems_error(project, wanted_type, &local));
        }
        let description = match non_empty(&req.description) {
            Some(md) => Some(markdown_to_adf(md)?),
            None => None,
        };

        let types = self.create_meta_types(project).await?;
        let Some(it) = types
            .iter()
            .find(|t| t.id == wanted_type || t.name.eq_ignore_ascii_case(wanted_type))
        else {
            let names: Vec<&str> = types.iter().map(|t| t.name.as_str()).collect();
            return Err(Error::Invalid(format!(
                "issue type '{wanted_type}' is not valid for project '{project}'; valid types: {}",
                if names.is_empty() {
                    "(none — this account cannot create issues there)".to_string()
                } else {
                    names.join(", ")
                }
            )));
        };
        let meta = self.create_meta_fields(project, &it.id).await?;
        let field = |id: &str| meta.iter().find(|f| f.id == id);
        let screen = format!("the create screen of {project} / {}", it.name);
        let mut problems = Vec::new();
        let mut fields = Map::new();
        fields.insert("project".into(), json!({ "key": project }));
        fields.insert("issuetype".into(), json!({ "id": it.id }));
        fields.insert("summary".into(), json!(req.summary.trim()));

        if let Some(adf) = &description {
            if field("description").is_some() {
                fields.insert("description".into(), adf.clone());
            } else {
                problems.push(format!("`description` is not on {screen}; omit it"));
            }
        }
        let labels: Vec<String> = req.labels.iter().map(|l| l.trim().to_string()).collect();
        if !labels.is_empty() {
            if field("labels").is_some() {
                fields.insert("labels".into(), json!(labels));
            } else {
                problems.push(format!("`labels` is not on {screen}; omit it"));
            }
        }
        let mut priority = None;
        if let Some(p) = non_empty(&req.priority) {
            match field("priority") {
                None => problems.push(format!("`priority` is not on {screen}; omit it")),
                Some(f) => match f
                    .allowed
                    .iter()
                    .find(|(id, name)| id == p || name.eq_ignore_ascii_case(p))
                {
                    Some((id, name)) => {
                        fields.insert("priority".into(), json!({ "id": id }));
                        priority = Some((id.clone(), name.clone()));
                    }
                    None => problems.push(format!(
                        "priority '{p}' is not valid; valid priorities: {}",
                        f.allowed
                            .iter()
                            .map(|(_, n)| n.as_str())
                            .collect::<Vec<_>>()
                            .join(", ")
                    )),
                },
            }
        }

        let parent = non_empty(&req.parent);
        let epic = non_empty(&req.epic);
        if let (Some(p), Some(e)) = (parent, epic) {
            if !p.eq_ignore_ascii_case(e) {
                problems.push(format!(
                    "parent '{p}' and epic '{e}' disagree — in Jira Cloud the epic IS the \
                     parent; pass one of them"
                ));
            }
        }
        let target = epic.or(parent);
        if let Some(key) = target {
            if field("parent").is_none() {
                problems.push(format!(
                    "a parent / epic cannot be set on {screen} (no `parent` field)"
                ));
            } else {
                match self.issue_type_of(key).await? {
                    None => problems.push(format!(
                        "{} '{key}' does not exist or is not visible to this account",
                        if epic.is_some() { "epic" } else { "parent" }
                    )),
                    Some((name, level))
                        if epic.is_some() && level != 1 && !name.eq_ignore_ascii_case("epic") =>
                    {
                        problems.push(format!("epic '{key}' is a {name}, not an Epic"))
                    }
                    Some(_) => {
                        fields.insert("parent".into(), json!({ "key": key }));
                    }
                }
            }
        } else if it.subtask {
            problems.push(format!(
                "'{}' is a sub-task type: pass `parent` (the parent issue's key)",
                it.name
            ));
        }

        for (k, v) in &req.fields {
            if RESERVED.contains(&k.as_str()) {
                problems.push(format!("fields.{k}: set it with its own argument instead"));
            } else if field(k).is_none() {
                problems.push(format!("fields.{k}: no such field on {screen}"));
            } else {
                fields.insert(k.clone(), v.clone());
            }
        }
        let missing: Vec<String> = meta
            .iter()
            .filter(|f| f.required && !f.has_default && !fields.contains_key(&f.id))
            .map(|f| format!("{} (`{}`)", f.name, f.id))
            .collect();
        if !missing.is_empty() {
            problems.push(format!(
                "required fields missing: {} — pass them in `fields` keyed by field id, in \
                 Jira's native value shape",
                missing.join(", ")
            ));
        }
        if !problems.is_empty() {
            return Err(problems_error(project, &it.name, &problems));
        }

        let resp = self
            .t
            .http()?
            .post(format!("{}/rest/api/3/issue", self.t.base))
            .header("Authorization", &self.t.auth_header)
            .header("Accept", "application/json")
            .json(&json!({ "fields": fields }))
            .send()
            .await
            .map_err(|e| Error::Upstream(format!("jira create_issue request: {e}")))?;
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        let body: Value = serde_json::from_str(&text).unwrap_or(Value::String(text));
        if !status.is_success() {
            return Err(Error::Upstream(format!(
                "jira create_issue failed ({status}): {}",
                jira_error_text(&body)
            )));
        }
        let key = body["key"]
            .as_str()
            .filter(|k| !k.is_empty())
            .ok_or_else(|| Error::Upstream(format!("jira create_issue returned no key: {body}")))?
            .to_string();

        let expected = Expected {
            project: project.to_string(),
            type_id: it.id.clone(),
            type_name: it.name.clone(),
            summary: req.summary.trim().to_string(),
            labels,
            priority,
            parent: target.map(str::to_string),
            description,
        };
        let verification = self.verify_created(&key, &expected).await;
        Ok(CreateIssueResp {
            url: format!("{}/browse/{key}", self.t.base),
            key,
            verification: Some(verification),
        })
    }

    async fn verify_created(&self, key: &str, exp: &Expected) -> IssueVerification {
        let url = format!(
            "{}/rest/api/3/issue/{}?fields=summary,description,issuetype,project,labels,priority,parent",
            self.t.base,
            path_seg(key)
        );
        let body = match self.get_json(&url, "verify").await {
            Ok((s, body)) if s.is_success() => body,
            Ok((s, body)) => {
                return IssueVerification::from_checks(vec![check(
                    "read_back",
                    false,
                    format!(
                        "the issue was created but reading it back answered {s}: {}",
                        jira_error_text(&body)
                    ),
                )])
            }
            Err(e) => {
                return IssueVerification::from_checks(vec![check(
                    "read_back",
                    false,
                    format!("the issue was created but could not be read back: {e}"),
                )])
            }
        };
        IssueVerification::from_checks(compare(key, exp, &body))
    }
}

/// The read-back checks (pure, so the comparison is unit-tested).
fn compare(key: &str, exp: &Expected, body: &Value) -> Vec<VerificationCheck> {
    let f = &body["fields"];
    let got = |v: &Value| v.as_str().unwrap_or("(none)").to_string();
    let mut checks = Vec::new();
    let got_key = got(&body["key"]);
    checks.push(check("key", got_key == key, got_key.clone()));
    let p = got(&f["project"]["key"]);
    checks.push(check(
        "project",
        p.eq_ignore_ascii_case(&exp.project),
        format!("expected {}, got {p}", exp.project),
    ));
    let t = got(&f["issuetype"]["id"]);
    checks.push(check(
        "issue_type",
        t == exp.type_id,
        format!(
            "expected {}, got {}",
            exp.type_name,
            got(&f["issuetype"]["name"])
        ),
    ));
    let s = got(&f["summary"]);
    checks.push(check(
        "summary",
        s == exp.summary,
        if s == exp.summary {
            "matches".to_string()
        } else {
            format!("expected {:?}, got {s:?}", exp.summary)
        },
    ));
    if !exp.labels.is_empty() {
        let mut want = exp.labels.clone();
        let mut have: Vec<String> = f["labels"]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|l| l.as_str().map(str::to_string))
                    .collect()
            })
            .unwrap_or_default();
        want.sort();
        have.sort();
        checks.push(check(
            "labels",
            want == have,
            format!("expected [{}], got [{}]", want.join(", "), have.join(", ")),
        ));
    }
    if let Some((id, name)) = &exp.priority {
        let ok = f["priority"]["id"].as_str() == Some(id.as_str());
        checks.push(check(
            "priority",
            ok,
            format!("expected {name}, got {}", got(&f["priority"]["name"])),
        ));
    }
    if let Some(parent) = &exp.parent {
        let have = got(&f["parent"]["key"]);
        checks.push(check(
            "parent",
            have.eq_ignore_ascii_case(parent),
            format!("expected {parent}, got {have}"),
        ));
    }
    if let Some(sent) = &exp.description {
        let stored = &f["description"];
        if !stored.is_object() {
            checks.push(check(
                "description_structure",
                false,
                "a description was sent but the issue has none",
            ));
        } else {
            checks.push(outline_check(
                "description_structure",
                &adf_outline(sent),
                &adf_outline(stored),
            ));
            checks.push(text_check(
                "description_text",
                &adf_plain_text(sent),
                &adf_plain_text(stored),
            ));
        }
    }
    checks
}

#[cfg(test)]
mod tests {
    use super::*;

    fn expected() -> Expected {
        Expected {
            project: "PRJ".into(),
            type_id: "10001".into(),
            type_name: "Task".into(),
            summary: "Fix it".into(),
            labels: vec!["b".into(), "a".into()],
            priority: Some(("2".into(), "High".into())),
            parent: Some("PRJ-1".into()),
            description: Some(markdown_to_adf("# T\n\n| a | b |\n|---|---|\n| 1 | 2 |").unwrap()),
        }
    }

    fn stored(exp: &Expected) -> Value {
        json!({"key": "PRJ-7", "fields": {
            "project": {"key": "PRJ"}, "issuetype": {"id": "10001", "name": "Task"},
            "summary": "Fix it", "labels": ["a", "b"], "priority": {"id": "2", "name": "High"},
            "parent": {"key": "PRJ-1"}, "description": exp.description.clone().unwrap()
        }})
    }

    #[test]
    fn a_faithful_read_back_passes_every_check() {
        let exp = expected();
        let checks = compare("PRJ-7", &exp, &stored(&exp));
        let names: Vec<&str> = checks.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "key",
                "project",
                "issue_type",
                "summary",
                "labels",
                "priority",
                "parent",
                "description_structure",
                "description_text"
            ]
        );
        assert!(checks.iter().all(|c| c.ok), "{checks:?}");
        assert!(
            checks[7].detail.contains("table[2x2,header]"),
            "{}",
            checks[7].detail
        );
    }

    #[test]
    fn mismatches_are_named_with_both_sides() {
        let exp = expected();
        let mut body = stored(&exp);
        body["fields"]["labels"] = json!(["a"]);
        body["fields"]["priority"] = json!({"id": "3", "name": "Low"});
        // Jira dropped the last table row.
        body["fields"]["description"]["content"][1]["content"]
            .as_array_mut()
            .unwrap()
            .pop();
        let v = IssueVerification::from_checks(compare("PRJ-7", &exp, &body));
        assert!(!v.ok);
        let bad: Vec<&VerificationCheck> = v.checks.iter().filter(|c| !c.ok).collect();
        let names: Vec<&str> = bad.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "labels",
                "priority",
                "description_structure",
                "description_text"
            ]
        );
        assert!(bad[1].detail.contains("expected High, got Low"));
        assert!(
            bad[2]
                .detail
                .contains("sent table[2x2,header], stored table[1x2,header]"),
            "{}",
            bad[2].detail
        );
    }

    #[test]
    fn a_missing_stored_description_fails_loudly() {
        let exp = expected();
        let mut body = stored(&exp);
        body["fields"]["description"] = Value::Null;
        let checks = compare("PRJ-7", &exp, &body);
        let d = checks
            .iter()
            .find(|c| c.name == "description_structure")
            .unwrap();
        assert!(!d.ok && d.detail.contains("has none"));
    }

    #[test]
    fn jira_errors_are_flattened_to_one_line() {
        let body = json!({"errorMessages": ["bad"], "errors": {"customfield_1": "is required"}});
        assert_eq!(jira_error_text(&body), "bad; customfield_1: is required");
        assert_eq!(jira_error_text(&json!("plain")), "plain");
    }
}
