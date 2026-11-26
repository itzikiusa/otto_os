//! The shapes an unattended agent step reports while it runs and leaves
//! behind when it settles: why it waits for the machine ([`WaitReason`]),
//! what it needs a person for ([`Attention`]), and the compact record of its
//! thread that outlives the thread ([`TranscriptDigest`], [`SubThreadUse`]).
//! Shared by the step runners (`otto_agent_run::step`), the runtime and the
//! workflow run record (`NodeRunState`), so they live in otto-core.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::TurnTokenUsage;
use crate::Id;

/// Why a step has not started yet: the machine's admission gate holds it
/// (`slots` · `memory` · `load`) or the engine does (`run_slots` ·
/// `run_order`). `text` is the sentence the UI shows as is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WaitReason {
    pub code: String,
    pub text: String,
    pub since: DateTime<Utc>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<WaitDetail>,
}

/// The numbers behind a [`WaitReason`] (what fills the machine).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WaitDetail {
    pub live: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub floor: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ahead: Option<u32>,
    /// Labels of the agents holding the places.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub holders: Vec<String>,
}

/// What a step waits on a person for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionKind {
    Approval,
    Question,
    UsageLimit,
    /// A person verifies a goal criterion (goal loops).
    Verify,
    #[serde(other)]
    Unknown,
}

/// Where an approval comes from: the agent CLI's own request, or a governed
/// Otto MCP call's approval card.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AttentionVia {
    Provider,
    Mcp,
}

/// A live wait on a person (`StepEvent::NeedsYou`, `NodeRunState.attention`).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Attention {
    pub kind: AttentionKind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub via: Option<AttentionVia>,
    /// ≤ [`ATTENTION_TEXT_MAX`] characters.
    pub text: String,
    pub since: DateTime<Utc>,
    /// The thread whose card answers it (a sub-thread's id for its request).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread_id: Option<Id>,
    /// The runtime request id, or the MCP approval id when `via` is `mcp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub request_id: Option<Id>,
    /// `usage_limit`: when the provider's limit resets.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resets_at: Option<DateTime<Utc>>,
    /// `verify`: the criterion a person must record.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub criterion_id: Option<String>,
}

pub const ATTENTION_TEXT_MAX: usize = 300;

impl Attention {
    pub fn new(kind: AttentionKind, text: &str, since: DateTime<Utc>) -> Self {
        Self {
            kind,
            via: None,
            text: clip(text, ATTENTION_TEXT_MAX),
            since,
            thread_id: None,
            request_id: None,
            resets_at: None,
            criterion_id: None,
        }
    }
}

/// An Otto MCP call a step's thread made that reaches outside Otto (a PR
/// comment, a Jira issue…) and what the tool answered.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutwardAction {
    pub tool: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub target: Option<String>,
    pub outcome: OutwardOutcome,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OutwardOutcome {
    Ok,
    Failed,
    PendingApproval,
}

/// The compact record of a step's thread, taken at settle (≤ 8 KiB) so the run
/// view still has it after retention deletes the thread.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TranscriptDigest {
    pub turns: u32,
    pub tool_calls: u32,
    /// ≤ 50 paths.
    #[serde(default)]
    pub files_changed: Vec<String>,
    /// ≤ 4 KiB.
    #[serde(default)]
    pub last_answer_excerpt: String,
    /// ≤ 5.
    #[serde(default)]
    pub errors: Vec<String>,
    /// ≤ 20.
    #[serde(default)]
    pub actions: Vec<OutwardAction>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ended_at: Option<DateTime<Utc>>,
}

/// One timeline item of a digest's items file (`text` ≤ 300 characters).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DigestItem {
    pub role: String,
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
}

/// A sub-thread a step's agent opened itself, with what it used — kept even
/// when the agent removed the sub-thread before the step settled.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubThreadUse {
    pub id: Id,
    pub provider: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// The sub-thread's last turn: `completed` · `failed` · `interrupted`
    /// · `running` · `idle`.
    pub status: String,
    #[serde(default)]
    pub usage: TurnTokenUsage,
}

impl TurnTokenUsage {
    /// Add `other` into `self` (optional counts stay `None` until one side has them).
    pub fn add(&mut self, other: &TurnTokenUsage) {
        fn opt<T: Copy + std::ops::Add<Output = T>>(a: Option<T>, b: Option<T>) -> Option<T> {
            match (a, b) {
                (Some(a), Some(b)) => Some(a + b),
                (a, None) => a,
                (None, b) => b,
            }
        }
        self.input_tokens += other.input_tokens;
        self.output_tokens += other.output_tokens;
        self.cached_input_tokens = opt(self.cached_input_tokens, other.cached_input_tokens);
        self.reasoning_tokens = opt(self.reasoning_tokens, other.reasoning_tokens);
        self.cost_usd = opt(self.cost_usd, other.cost_usd);
    }
}

/// `s` cut to at most `max` characters (on a char boundary).
pub fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => s[..i].to_string(),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_adds_and_keeps_unknown_counts_unknown() {
        let mut a = TurnTokenUsage {
            input_tokens: 10,
            output_tokens: 2,
            cached_input_tokens: None,
            reasoning_tokens: Some(1),
            cost_usd: None,
        };
        a.add(&TurnTokenUsage {
            input_tokens: 5,
            output_tokens: 3,
            cached_input_tokens: Some(4),
            reasoning_tokens: Some(2),
            cost_usd: None,
        });
        assert_eq!((a.input_tokens, a.output_tokens), (15, 5));
        assert_eq!(a.cached_input_tokens, Some(4));
        assert_eq!(a.reasoning_tokens, Some(3));
        assert_eq!(a.cost_usd, None);
    }

    #[test]
    fn attention_text_is_clipped_and_unknown_kinds_decode() {
        let a = Attention::new(AttentionKind::Question, &"é".repeat(400), Utc::now());
        assert_eq!(a.text.chars().count(), ATTENTION_TEXT_MAX);
        let k: AttentionKind = serde_json::from_str("\"something_new\"").unwrap();
        assert_eq!(k, AttentionKind::Unknown);
        let json = serde_json::to_value(&a).unwrap();
        assert_eq!(json["kind"], "question");
        assert!(
            json.get("via").is_none(),
            "absent optionals are not written"
        );
        let verify = Attention {
            criterion_id: Some("tests-pass".into()),
            ..Attention::new(AttentionKind::Verify, "Check it", Utc::now())
        };
        let json = serde_json::to_value(&verify).unwrap();
        assert_eq!(json["criterion_id"], "tests-pass");
        assert_eq!(serde_json::from_value::<Attention>(json).unwrap(), verify);
    }
}
