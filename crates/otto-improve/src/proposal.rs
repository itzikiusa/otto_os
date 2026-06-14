//! The structured edit proposal the analysis agent returns, and a tolerant
//! parser that extracts it from the agent's reply text.

use otto_core::domain::{ImprovementEditKind, ImprovementRisk, ImprovementTarget};
use otto_core::{Error, Result};
use serde::Deserialize;

/// Full proposal returned by one analysis run.
#[derive(Debug, Clone, Deserialize)]
pub struct ImprovementProposal {
    #[serde(default)]
    pub run_summary: String,
    #[serde(default)]
    pub edits: Vec<ProposedEdit>,
}

/// One proposed change to a skill or memory file.
#[derive(Debug, Clone, Deserialize)]
pub struct ProposedEdit {
    #[serde(default)]
    pub id: String,
    pub target_type: ImprovementTarget,
    pub target_ref: String,
    pub kind: ImprovementEditKind,
    pub risk: ImprovementRisk,
    #[serde(default)]
    pub rationale: String,
    #[serde(default)]
    pub evidence: Vec<String>,
    #[serde(default)]
    pub dedup_checked: bool,
    #[serde(default)]
    pub dedup_quote: Option<String>,
    pub patch: EditPatch,
}

/// The before/after content. `before` is the agent's view of the current file
/// (informational); the engine snapshots the *actual* current file at apply
/// time. `after` is the FULL new file content.
#[derive(Debug, Clone, Deserialize)]
pub struct EditPatch {
    #[serde(default)]
    pub before: Option<String>,
    pub after: String,
}

/// Parse an `ImprovementProposal` out of an agent reply. Tolerates a leading/
/// trailing ```json fence and surrounding prose by slicing the outermost
/// `{ ... }` object (mirrors the JSON-array slicing in `run_review_core`).
pub fn parse_proposal(reply: &str) -> Result<ImprovementProposal> {
    let stripped = reply
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let start = stripped
        .find('{')
        .ok_or_else(|| Error::Invalid("no JSON object in proposal".into()))?;
    let end = stripped
        .rfind('}')
        .map(|i| i + 1)
        .ok_or_else(|| Error::Invalid("unterminated JSON object in proposal".into()))?;
    if end <= start {
        return Err(Error::Invalid("malformed JSON object in proposal".into()));
    }
    serde_json::from_str(&stripped[start..end])
        .map_err(|e| Error::Invalid(format!("proposal JSON: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"```json
{
  "run_summary": "Agents kept mis-routing refund tickets.",
  "edits": [
    {
      "id": "e1",
      "target_type": "skill",
      "target_ref": "support-triage-router",
      "kind": "add",
      "risk": "low",
      "rationale": "Add a refund routing rule.",
      "evidence": ["sess_1", "sess_2"],
      "dedup_checked": true,
      "dedup_quote": "no existing refund rule found",
      "patch": { "before": "OLD", "after": "OLD\n- refunds -> billing" }
    }
  ]
}
```"#;

    #[test]
    fn parses_fenced_proposal() {
        let p = parse_proposal(SAMPLE).unwrap();
        assert_eq!(p.edits.len(), 1);
        let e = &p.edits[0];
        assert_eq!(e.target_ref, "support-triage-router");
        assert_eq!(e.target_type, ImprovementTarget::Skill);
        assert_eq!(e.kind, ImprovementEditKind::Add);
        assert_eq!(e.risk, ImprovementRisk::Low);
        assert_eq!(e.patch.after, "OLD\n- refunds -> billing");
        assert_eq!(e.evidence, vec!["sess_1", "sess_2"]);
    }

    #[test]
    fn parses_with_surrounding_prose() {
        let text = "Here is my proposal:\n{\"run_summary\":\"x\",\"edits\":[]}\nDone.";
        let p = parse_proposal(text).unwrap();
        assert_eq!(p.run_summary, "x");
        assert!(p.edits.is_empty());
    }

    #[test]
    fn rejects_non_json() {
        assert!(parse_proposal("I could not find anything to improve.").is_err());
    }
}
