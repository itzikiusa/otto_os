//! Request / response DTOs of the assistant API (`docs/contracts/api.md`
//! "Otto Assistant"; TS mirror in `ui/src/lib/api/types.ts`). The stored rows
//! (`AssistantThread`, `AssistantTurn`, `AssistantTask`, `AssistantAttachment`)
//! come from `otto_state::assistant`; routing/limit shapes from the sibling
//! `router` / `limits` modules.

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

use otto_state::{AssistantThread, AssistantTurn};

use super::router::RouteDecision;

/// `Some(None)` for an explicit JSON `null`, `None` when the key is absent.
fn double_option<'de, D, T>(de: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(de).map(Some)
}

#[derive(Debug, Default, Deserialize)]
pub struct CreateThreadReq {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub space_slot: Option<i64>,
    #[serde(default)]
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
    #[serde(default)]
    pub incognito: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct UpdateThreadReq {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default, deserialize_with = "double_option")]
    pub space_slot: Option<Option<i64>>,
}

#[derive(Debug, Default, Deserialize)]
pub struct SendReq {
    pub text: String,
    #[serde(default)]
    pub attachment_ids: Vec<String>,
    #[serde(default)]
    pub origin: Option<String>,
    #[serde(default)]
    pub voice: bool,
}

#[derive(Debug, Serialize)]
pub struct SendResp {
    pub turn: AssistantTurn,
    pub route: RouteDecision,
    pub thread: AssistantThread,
}

#[derive(Debug, Deserialize)]
pub struct AttachmentReq {
    pub name: String,
    pub content_base64: String,
    #[serde(default)]
    pub mime: Option<String>,
}

/// `POST …/route` — `provider: null` clears the pin.
#[derive(Debug, Deserialize)]
pub struct RouteReq {
    pub provider: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub account_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct DelegateReq {
    pub agent_id: String,
    pub directive: String,
}

#[derive(Debug, Deserialize)]
pub struct PreviewReq {
    pub text: String,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub voice: bool,
}

#[derive(Debug, Default, Deserialize)]
pub struct TasksQuery {
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Default, Deserialize)]
pub struct TurnsQuery {
    #[serde(default)]
    pub before: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct CreateTaskReq {
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub detail: Option<String>,
    #[serde(default)]
    pub thread_id: Option<String>,
    #[serde(default)]
    pub run_at: Option<String>,
    #[serde(default)]
    pub timezone: Option<String>,
    #[serde(default)]
    pub origin: Option<String>,
}

/// Body of `POST /assistant/tasks/{id}/{action}`.
#[derive(Debug, Default, Deserialize)]
pub struct DecisionReq {
    #[serde(default)]
    pub reason: Option<String>,
    #[serde(default)]
    pub answer: Option<String>,
    #[serde(default)]
    pub always_allow: bool,
    #[serde(default)]
    pub provider: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MemoryQuery {
    #[serde(default)]
    pub q: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ProfileSave {
    pub content: String,
    pub version: String,
}

#[derive(Debug, Deserialize)]
pub struct PutMemoryReq {
    pub profile: ProfileSave,
}

#[derive(Debug, Deserialize)]
pub struct AddMemoryReq {
    pub text: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct UndoReq {
    pub undo_token: String,
}

#[derive(Debug, Deserialize)]
pub struct ForgetReq {
    pub query: String,
    #[serde(default)]
    pub thread_id: Option<String>,
}

/// `AssistantMemory` on the wire.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AssistantMemory {
    pub id: String,
    pub text: String,
    pub kind: String,
    pub tags: Vec<String>,
    pub state: String,
    pub source: MemorySource,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct MemorySource {
    pub kind: String,
    pub thread_id: Option<String>,
    pub file: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProfileDoc {
    pub content: String,
    pub version: String,
    pub exists: bool,
}

#[derive(Debug, Serialize)]
pub struct MemoryView {
    pub profile: ProfileDoc,
    pub memories: Vec<AssistantMemory>,
    pub pending: Vec<AssistantMemory>,
    pub memory_approval: bool,
}

#[derive(Debug, Serialize)]
pub struct ForgetResp {
    pub forgotten: Vec<AssistantMemory>,
    pub undo_tokens: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct HermesPreview {
    pub available: bool,
    pub files: Vec<HermesFileSummary>,
    pub entries: Vec<HermesEntry>,
}

#[derive(Debug, Serialize)]
pub struct HermesFileSummary {
    pub name: String,
    pub entries: usize,
}

#[derive(Debug, Serialize)]
pub struct HermesEntry {
    pub file: String,
    pub text: String,
    pub duplicate: bool,
}

#[derive(Debug, Serialize)]
pub struct HermesImportResp {
    pub queued: usize,
    pub duplicates: usize,
    pub files: Vec<String>,
}

/// The guideline approval card (where / what / who sees it / reason).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApprovalCard {
    #[serde(rename = "where")]
    pub where_: String,
    pub what: String,
    pub who_sees: String,
    pub reason: String,
    pub tool: Option<String>,
    pub destination: Option<String>,
    pub category: String,
    pub always_allow_allowed: bool,
}

/// Outward categories an approval may carry.
pub const APPROVAL_CATEGORIES: [&str; 8] = [
    "send", "post", "publish", "purchase", "delete", "submit", "prod", "other",
];

/// Categories that can NEVER be set to always-allow (Decision 6).
pub const NEVER_ALWAYS_ALLOW: [&str; 2] = ["purchase", "prod"];

/// Build + validate the card from an agent's `approval` tool args.
pub fn approval_card(args: &Value) -> Result<ApprovalCard, String> {
    let s = |k: &str| -> Result<String, String> {
        args.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.chars().take(4000).collect())
            .ok_or_else(|| format!("`{k}` is required"))
    };
    let opt = |k: &str| -> Option<String> {
        args.get(k)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|v| !v.is_empty())
            .map(|v| v.chars().take(400).collect())
    };
    let category = opt("category")
        .unwrap_or_else(|| "other".into())
        .to_lowercase();
    if !APPROVAL_CATEGORIES.contains(&category.as_str()) {
        return Err(format!(
            "`category` must be one of {}",
            APPROVAL_CATEGORIES.join("|")
        ));
    }
    Ok(ApprovalCard {
        where_: s("where")?,
        what: s("what")?,
        who_sees: s("who_sees")?,
        reason: s("reason")?,
        tool: opt("tool"),
        destination: opt("destination"),
        always_allow_allowed: !NEVER_ALWAYS_ALLOW.contains(&category.as_str()),
        category,
    })
}

/// The `agent_grants` resource an always-allow is keyed on: tool + destination.
/// `None` when either is missing (nothing specific enough to remember).
pub fn always_allow_resource(card: &ApprovalCard) -> Option<String> {
    let tool = card.tool.as_deref()?;
    let dest = card.destination.as_deref()?;
    Some(format!("{tool}|{dest}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn approval_card_has_the_guideline_shape() {
        let c = approval_card(&json!({
            "where":"Telegram DM to me","what":"Flight options summary","who_sees":"only you",
            "reason":"you asked for it on your phone","tool":"telegram_send","destination":"me",
            "category":"send"
        }))
        .unwrap();
        let v = serde_json::to_value(&c).unwrap();
        for k in [
            "where",
            "what",
            "who_sees",
            "reason",
            "tool",
            "destination",
            "category",
        ] {
            assert!(v.get(k).is_some(), "{k}");
        }
        assert_eq!(v["always_allow_allowed"], true);
        assert_eq!(
            always_allow_resource(&c).as_deref(),
            Some("telegram_send|me")
        );
    }

    #[test]
    fn purchases_and_prod_can_never_be_always_allowed() {
        for cat in NEVER_ALWAYS_ALLOW {
            let c = approval_card(&json!({
                "where":"x","what":"y","who_sees":"z","reason":"r","category":cat
            }))
            .unwrap();
            assert!(!c.always_allow_allowed, "{cat}");
        }
    }

    #[test]
    fn approval_card_requires_every_field_and_a_known_category() {
        assert!(approval_card(&json!({"where":"x","what":"y","who_sees":"z"})).is_err());
        assert!(approval_card(&json!({
            "where":"x","what":"y","who_sees":"z","reason":"r","category":"bribe"
        }))
        .is_err());
        let c =
            approval_card(&json!({"where":"x","what":"y","who_sees":"z","reason":"r"})).unwrap();
        assert_eq!(c.category, "other");
        assert!(always_allow_resource(&c).is_none());
    }

    #[test]
    fn update_thread_distinguishes_null_from_absent() {
        let r: UpdateThreadReq = serde_json::from_value(json!({"space_slot": null})).unwrap();
        assert_eq!(r.space_slot, Some(None));
        let r: UpdateThreadReq = serde_json::from_value(json!({"title":"x"})).unwrap();
        assert_eq!(r.space_slot, None);
        let r: UpdateThreadReq = serde_json::from_value(json!({"space_slot": 3})).unwrap();
        assert_eq!(r.space_slot, Some(Some(3)));
    }
}
