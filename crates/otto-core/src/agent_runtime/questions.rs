//! Provider-neutral form data. Native field IDs and option values are opaque.
use serde::{Deserialize, Serialize};
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QuestionKind {
    Single,
    Multi,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuestionOption {
    pub value: String,
    pub label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeQuestion {
    pub id: String,
    pub header: String,
    pub prompt: String,
    pub kind: QuestionKind,
    pub required: bool,
    /// ACP required means property presence, not a minimum string/array length.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub allow_empty: bool,
    pub allow_free_text: bool,
    pub options: Vec<QuestionOption>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum QuestionAnswer {
    Single {
        question_id: String,
        value: String,
    },
    Multi {
        question_id: String,
        values: Vec<String>,
    },
    Text {
        question_id: String,
        text: String,
    },
}
impl QuestionAnswer {
    pub fn question_id(&self) -> &str {
        match self {
            Self::Single { question_id, .. }
            | Self::Multi { question_id, .. }
            | Self::Text { question_id, .. } => question_id,
        }
    }
    pub fn values(&self) -> Vec<String> {
        match self {
            Self::Single { value, .. } => vec![value.clone()],
            Self::Multi { values, .. } => values.clone(),
            Self::Text { text, .. } => vec![text.clone()],
        }
    }
}

pub fn validate_question_answers(
    questions: &[RuntimeQuestion],
    answers: &[QuestionAnswer],
) -> Result<(), String> {
    let mut seen = HashSet::new();
    for answer in answers {
        let id = answer.question_id();
        if !seen.insert(id) {
            return Err("duplicate question answer".into());
        }
        let q = questions
            .iter()
            .find(|q| q.id == id)
            .ok_or("unknown question")?;
        let values = answer.values();
        match answer {
            QuestionAnswer::Text { text, .. } => {
                if !q.allow_free_text && q.kind != QuestionKind::Text {
                    return Err("free text is not supported for this question".into());
                }
                if q.required && !q.allow_empty && text.trim().is_empty() {
                    return Err("answer every required question".into());
                }
            }
            QuestionAnswer::Single { .. } if q.kind != QuestionKind::Single => {
                return Err("this question does not accept a single selection".into())
            }
            QuestionAnswer::Multi { .. } if q.kind != QuestionKind::Multi => {
                return Err("select one answer for this question".into())
            }
            _ => {
                if q.required && !q.allow_empty && values.is_empty() {
                    return Err("answer every required question".into());
                }
                if values.iter().collect::<HashSet<_>>().len() != values.len() {
                    return Err("duplicate selection".into());
                }
                if values
                    .iter()
                    .any(|v| !q.options.iter().any(|o| o.value == *v))
                {
                    return Err("select an offered answer".into());
                }
            }
        }
    }
    if questions
        .iter()
        .any(|q| q.required && !seen.contains(q.id.as_str()))
    {
        return Err("answer every required question".into());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn validates_native_values_types_and_required_fields() {
        let q = RuntimeQuestion {
            id: "x".into(),
            header: "".into(),
            prompt: "Choose".into(),
            kind: QuestionKind::Single,
            required: true,
            allow_empty: false,
            allow_free_text: false,
            options: vec![QuestionOption {
                value: "raw".into(),
                label: "Display".into(),
                description: None,
            }],
        };
        let good = QuestionAnswer::Single {
            question_id: "x".into(),
            value: "raw".into(),
        };
        assert!(
            validate_question_answers(std::slice::from_ref(&q), std::slice::from_ref(&good))
                .is_ok()
        );
        for answers in [
            vec![],
            vec![good.clone(), good],
            vec![QuestionAnswer::Single {
                question_id: "x".into(),
                value: "Display".into(),
            }],
            vec![QuestionAnswer::Text {
                question_id: "x".into(),
                text: "raw".into(),
            }],
        ] {
            assert!(validate_question_answers(std::slice::from_ref(&q), &answers).is_err());
        }
    }
}
