//! Native question adapters. Reject schemas we cannot faithfully render/validate.
use otto_core::agent_runtime::{QuestionKind, QuestionOption, RuntimeQuestion};
use serde_json::Value;
use std::collections::HashSet;

fn text(v: &Value, key: &str) -> Result<String, String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| format!("missing question {key}"))
}
fn unique(questions: Vec<RuntimeQuestion>) -> Result<Vec<RuntimeQuestion>, String> {
    let mut ids = HashSet::new();
    if questions.is_empty() || questions.len() > 32 {
        return Err("expected 1–32 questions".into());
    }
    for q in &questions {
        if q.id.is_empty() || !ids.insert(&q.id) {
            return Err("question IDs must be nonempty and unique".into());
        }
        // Reserved for Otto's own url-mode elicitation (`acp_url`); an id
        // from a model or a schema never takes it.
        if q.id == URL_QUESTION {
            return Err(format!("the question id {URL_QUESTION} is reserved"));
        }
        let mut values = HashSet::new();
        if q.options.iter().any(|o| !values.insert(&o.value)) {
            return Err("duplicate question option values".into());
        }
    }
    Ok(questions)
}

pub fn native_questions(params: &Value, claude: bool) -> Result<Vec<RuntimeQuestion>, String> {
    let raw = params
        .get("questions")
        .and_then(Value::as_array)
        .ok_or("missing questions")?;
    let mut questions = vec![];
    for q in raw {
        // Question answers are persisted as conversation state: never collect credentials here.
        if q.get("isSecret").and_then(Value::as_bool) == Some(true) {
            return Err(
                "secret questions are unsupported; use the provider's secure input flow".into(),
            );
        }
        let prompt = text(q, "question")?;
        let mut options = vec![];
        if let Some(raw) = q.get("options").filter(|o| !o.is_null()) {
            for option in raw.as_array().ok_or("invalid question options")? {
                let label = text(option, "label")?;
                options.push(QuestionOption {
                    value: label.clone(),
                    label,
                    description: option
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                });
            }
        }
        questions.push(RuntimeQuestion {
            id: if claude {
                prompt.clone()
            } else {
                text(q, "id")?
            },
            header: q
                .get("header")
                .and_then(Value::as_str)
                .unwrap_or("Question")
                .into(),
            prompt,
            kind: if options.is_empty() {
                QuestionKind::Text
            } else if claude && q.get("multiSelect").and_then(Value::as_bool) == Some(true) {
                QuestionKind::Multi
            } else {
                QuestionKind::Single
            },
            required: true,
            allow_empty: false,
            allow_free_text: true,
            open_url: None,
            options,
        });
    }
    unique(questions)
}

/// The question id of an ACP url-mode elicitation (`acp_url`).
pub const URL_QUESTION: &str = "acp_url";
/// The value of its one "open" choice.
pub const URL_OPEN: &str = "open";

/// ACP url-mode elicitation: the agent asks the person to open an http(s)
/// page (a sign-in, a consent screen). One choice names the URL; choosing it
/// answers `accept` and the UI opens the page in the system browser.
pub fn acp_url(params: &Value) -> Result<Vec<RuntimeQuestion>, String> {
    let url = params
        .get("url")
        .and_then(Value::as_str)
        .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
        .filter(|u| !u.chars().any(char::is_control))
        .ok_or("url elicitation needs an http(s) url")?;
    let message = params.get("message").and_then(Value::as_str).unwrap_or("");
    Ok(vec![RuntimeQuestion {
        id: URL_QUESTION.into(),
        header: "Open a link".into(),
        prompt: format!("{message}\n{url}").trim().to_string(),
        kind: QuestionKind::Single,
        required: true,
        allow_empty: false,
        allow_free_text: false,
        open_url: Some(url.into()),
        options: vec![QuestionOption {
            value: URL_OPEN.into(),
            label: "Open in the browser".into(),
            description: Some(url.into()),
        }],
    }])
}

/// The draft ACP elicitation/create form subset: plain strings and string enums/arrays.
/// Constraints and other primitive types are explicitly refused, never silently discarded.
pub fn acp_form(params: &Value) -> Result<Vec<RuntimeQuestion>, String> {
    if params.get("mode").and_then(Value::as_str) != Some("form") {
        return Err("only form elicitation is supported".into());
    }
    let schema = params
        .get("requestedSchema")
        .ok_or("missing requestedSchema")?;
    if schema.get("type").and_then(Value::as_str) != Some("object") {
        return Err("only flat object forms are supported".into());
    }
    if schema.as_object().is_none_or(|s| {
        s.keys().any(|k| {
            ![
                "type",
                "properties",
                "required",
                "title",
                "description",
                "_meta",
                "$schema",
            ]
            .contains(&k.as_str())
        })
    }) {
        return Err("unsupported object form constraint".into());
    }
    if schema.get("required").is_some_and(|r| !r.is_array()) {
        return Err("invalid required fields".into());
    }
    let props = schema
        .get("properties")
        .and_then(Value::as_object)
        .ok_or("missing form properties")?;
    let required = schema.get("required").and_then(Value::as_array);
    let mut questions = vec![];
    for (id, prop) in props {
        let object = prop.as_object().ok_or("invalid form field")?;
        for key in object.keys() {
            if ![
                "type",
                "title",
                "description",
                "enum",
                "oneOf",
                "items",
                "default",
                "_meta",
            ]
            .contains(&key.as_str())
            {
                return Err(format!("unsupported form constraint: {key}"));
            }
        }
        let multi = prop.get("type").and_then(Value::as_str) == Some("array");
        let source = if multi {
            prop.get("items").ok_or("missing array items")?
        } else {
            prop
        };
        if source.get("type").and_then(Value::as_str) != Some("string")
            && !(multi && source.get("type").is_none() && source.get("anyOf").is_some())
        {
            return Err("only text and string-choice fields are supported".into());
        }
        if multi
            && source.as_object().is_none_or(|o| {
                o.keys()
                    .any(|k| !["type", "enum", "anyOf"].contains(&k.as_str()))
            })
        {
            return Err("unsupported array item constraint".into());
        }
        let mut options = vec![];
        if let Some(values) = source.get("enum") {
            for v in values.as_array().ok_or("invalid enum")? {
                let value = v
                    .as_str()
                    .ok_or("only string enums are supported")?
                    .to_string();
                options.push(QuestionOption {
                    label: value.clone(),
                    value,
                    description: None,
                });
            }
        } else if let Some(values) = source.get(if multi { "anyOf" } else { "oneOf" }) {
            for v in values.as_array().ok_or("invalid oneOf")? {
                if v.as_object().is_none_or(|o| {
                    o.keys()
                        .any(|k| !["const", "title", "description"].contains(&k.as_str()))
                }) {
                    return Err("unsupported choice schema".into());
                }
                let value = text(v, "const")?;
                options.push(QuestionOption {
                    label: v
                        .get("title")
                        .and_then(Value::as_str)
                        .unwrap_or(&value)
                        .into(),
                    value,
                    description: v
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                });
            }
        }
        if (multi
            || source.get("enum").is_some()
            || source.get("oneOf").is_some()
            || source.get("anyOf").is_some())
            && options.is_empty()
        {
            return Err("array forms require choices".into());
        }
        let header = prop
            .get("title")
            .and_then(Value::as_str)
            .unwrap_or(id)
            .to_string();
        questions.push(RuntimeQuestion {
            id: id.clone(),
            prompt: prop
                .get("description")
                .and_then(Value::as_str)
                .unwrap_or(&header)
                .into(),
            header,
            kind: if multi {
                QuestionKind::Multi
            } else if options.is_empty() {
                QuestionKind::Text
            } else {
                QuestionKind::Single
            },
            required: required.is_some_and(|r| r.iter().any(|v| v.as_str() == Some(id))),
            allow_empty: true,
            allow_free_text: options.is_empty(),
            open_url: None,
            options,
        });
    }
    if required.is_some_and(|r| {
        r.iter()
            .any(|v| v.as_str().is_none_or(|id| !props.contains_key(id)))
    }) {
        return Err("invalid required fields".into());
    }
    unique(fold_other_fields(questions))
}

/// The free-text companion of an ACP single choice: `<id>__other`.
/// An MCP server's elicitation as the agent CLI forwards it — Claude's
/// `elicitation` control request (`requested_schema`), Codex's
/// `mcpServer/elicitation/request` (`requestedSchema`; its `openai/form`
/// modes are the same form) — as questions: the subset [`acp_form`] takes.
/// A URL-mode request or a form outside it is refused (the caller declines).
pub fn mcp_elicitation(params: &Value) -> Result<Vec<RuntimeQuestion>, String> {
    let mode = match params.get("mode").and_then(Value::as_str).unwrap_or("form") {
        "openai/form" | "openaiForm" => "form",
        "url" => return acp_url(params),
        m => m,
    };
    let schema = params
        .get("requestedSchema")
        .or_else(|| params.get("requested_schema"))
        .cloned()
        .unwrap_or(Value::Null);
    acp_form(&serde_json::json!({ "mode": mode, "requestedSchema": schema }))
}

/// An elicitation with no fields to fill (a missing, `true` or empty
/// `properties` schema, in any form mode): a yes/no for the person — Codex
/// asks MCP tool approval this way — shown as an approval, not a form.
pub fn is_confirmation(params: &Value) -> bool {
    let mode = params.get("mode").and_then(Value::as_str).unwrap_or("form");
    let schema = params
        .get("requestedSchema")
        .or_else(|| params.get("requested_schema"));
    mode != "url"
        && schema
            .and_then(|s| s.get("properties"))
            .and_then(Value::as_object)
            .is_none_or(|p| p.is_empty())
}

/// Allow / Deny for [`is_confirmation`].
pub fn confirmation_options() -> Vec<otto_core::agent_runtime::ApprovalOption> {
    use otto_core::agent_runtime::{ApprovalDecision, ApprovalOption};
    vec![
        ApprovalOption {
            decision: ApprovalDecision::Accept,
            label: "Allow".into(),
        },
        ApprovalOption {
            decision: ApprovalDecision::Decline,
            label: "Deny".into(),
        },
    ]
}

/// The elicitation result for an approval answer (no content to send).
pub fn confirmation_result(decision: otto_core::agent_runtime::ApprovalDecision) -> Value {
    use otto_core::agent_runtime::ApprovalDecision as D;
    let action = match decision {
        D::Accept | D::AcceptForSession | D::AcceptAlways => "accept",
        D::Decline | D::DeclineAlways => "decline",
        D::Cancel => "cancel",
    };
    // Codex's MCP tool approval offers `_meta.persist` (session / always).
    match decision {
        D::AcceptForSession => {
            serde_json::json!({"action": action, "content": {}, "_meta": {"persist": "session"}})
        }
        D::AcceptAlways => {
            serde_json::json!({"action": action, "content": {}, "_meta": {"persist": "always"}})
        }
        _ if action == "accept" => serde_json::json!({ "action": action, "content": {} }),
        _ => serde_json::json!({ "action": action }),
    }
}

/// The `{action: "accept", …}` result of an answered elicitation, for every
/// agent: a url question (the person opened the page) is a bare accept, a
/// form its fields as `content`.
pub fn elicitation_answer(
    questions: &[RuntimeQuestion],
    answers: &[otto_core::agent_runtime::QuestionAnswer],
) -> Value {
    if questions.iter().any(|q| q.id == URL_QUESTION) {
        return serde_json::json!({ "action": "accept" });
    }
    serde_json::json!({ "action": "accept", "content": form_content(questions, answers) })
}

/// The `content` of an accepted form elicitation: each answer under its
/// field id (typed text for a single choice under its `<id>__other` field).
pub fn form_content(
    questions: &[RuntimeQuestion],
    answers: &[otto_core::agent_runtime::QuestionAnswer],
) -> serde_json::Map<String, Value> {
    use otto_core::agent_runtime::{QuestionAnswer, QuestionKind};
    answers
        .iter()
        .map(|a| {
            let id = a.question_id();
            let single = questions
                .iter()
                .any(|q| q.id == id && q.kind == QuestionKind::Single);
            match a {
                QuestionAnswer::Multi { values, .. } => (id.to_string(), serde_json::json!(values)),
                QuestionAnswer::Text { text, .. } if single => {
                    (other_field(id), serde_json::json!(text))
                }
                _ => (id.to_string(), serde_json::json!(a.values()[0])),
            }
        })
        .collect()
}

pub fn other_field(id: &str) -> String {
    format!("{id}__other")
}

/// An optional `<id>__other` text field (OMP's ask dialog) is the free-text
/// answer of the optional single choice `<id>`: one question, not two
/// steps, whose typed answer goes back under the companion's id (see
/// [`other_field`]). A multi-select keeps both, since it can answer with
/// options and text at once (T3 Code #34867).
fn fold_other_fields(mut questions: Vec<RuntimeQuestion>) -> Vec<RuntimeQuestion> {
    let folded: Vec<String> = questions
        .iter()
        .filter(|q| q.kind == QuestionKind::Single && !q.required)
        .map(|q| other_field(&q.id))
        .filter(|other| {
            questions
                .iter()
                .any(|c| c.id == *other && c.kind == QuestionKind::Text && !c.required)
        })
        .collect();
    questions.retain(|q| !folded.contains(&q.id));
    for q in &mut questions {
        if folded.contains(&other_field(&q.id)) {
            q.allow_free_text = true;
        }
    }
    questions
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// Security 4: `acp_url` is reserved — a model's Codex question or a
    /// form's schema property cannot take it — and only Otto's own url
    /// question carries `open_url`.
    #[test]
    fn the_url_question_id_is_reserved_and_open_url_is_driver_only() {
        let codex = json!({"questions":[{"id":"acp_url","header":"Go","question":"Proceed?",
            "options":[{"label":"Yes","description":"https://attacker.example/?d=1"},{"label":"No"}]}]});
        assert!(native_questions(&codex, false)
            .unwrap_err()
            .contains("reserved"));
        let form = json!({"mode":"form","requestedSchema":{"type":"object","properties":{
            "acp_url":{"type":"string","oneOf":[{"const":"y","title":"Yes","description":"https://attacker.example"}]}}}});
        assert!(acp_form(&form).unwrap_err().contains("reserved"));
        let ok = native_questions(
            &json!({"questions":[{"id":"q","header":"H","question":"?"}]}),
            false,
        )
        .unwrap();
        assert_eq!(ok[0].open_url, None);
        let url = acp_url(&json!({"url":"https://example.com/login","message":"m"})).unwrap();
        assert_eq!(
            url[0].open_url.as_deref(),
            Some("https://example.com/login")
        );
        assert_eq!(url[0].options[0].value, URL_OPEN);
    }

    #[test]
    fn acp_keeps_native_enum_values_and_refuses_constraints() {
        let mut form = json!({"mode":"form","requestedSchema":{"type":"object","properties":{"style":{"type":"string","oneOf":[{"const":"safe_v1","title":"Safe"}]}},"required":["style"]}});
        let qs = acp_form(&form).unwrap();
        assert_eq!(qs[0].options[0].value, "safe_v1");
        assert_eq!(qs[0].options[0].label, "Safe");
        form["requestedSchema"]["properties"]["style"]["pattern"] = json!(".*");
        assert!(acp_form(&form).is_err());
        form["mode"] = json!("url");
        assert!(acp_form(&form).is_err());
    }
    #[test]
    fn an_optional_choice_and_its_other_text_field_are_one_question() {
        let form = json!({"mode":"form","requestedSchema":{"type":"object","properties":{
            "q0":{"type":"string","title":"Database","enum":["Postgres","MySQL"]},
            "q0__other":{"type":"string","title":"Other"},
            "q1":{"type":"array","items":{"type":"string","enum":["api","web"]}},
            "q1__other":{"type":"string"},
            "q2":{"type":"string","enum":["a"]},
            "q2__other":{"type":"string"}
        },"required":["q2"]}});
        let qs = acp_form(&form).unwrap();
        let ids: Vec<&str> = qs.iter().map(|q| q.id.as_str()).collect();
        // q0 folds; a multi-select (q1) and a required choice (q2) do not.
        assert_eq!(ids, ["q0", "q1", "q1__other", "q2", "q2__other"]);
        assert!(qs[0].allow_free_text);
        assert_eq!(qs[0].kind, QuestionKind::Single);
        assert!(!qs[3].allow_free_text);
    }
    #[test]
    fn claude_question_text_is_native_key_and_multi_is_typed() {
        let qs = native_questions(&json!({"questions":[{"question":"Choose?","header":"Choice","multiSelect":true,"options":[{"label":"A"}]}]}), true).unwrap();
        assert_eq!(qs[0].id, "Choose?");
        assert_eq!(qs[0].kind, QuestionKind::Multi);
        assert!(native_questions(
            &json!({"questions":[{"id":"x","question":"Secret?","isSecret":true}]}),
            false
        )
        .is_err());
    }
}
