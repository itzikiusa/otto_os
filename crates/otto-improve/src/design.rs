//! The `design` evidence source (Design Hall Learning v1, suggest-only).
//!
//! Sits next to session transcripts and review-comment decisions: the server
//! runs the deterministic design-signal extractor (`otto_design::learn`) and
//! hands the READY candidates here as [`DesignRuleProposal`]s. Each one becomes
//! a **pending** edit of the workspace's `design-team-style` skill — never
//! auto-applied, whatever the workspace's autonomy — so a human approves,
//! rejects or rolls it back through the existing edit flow
//! (`POST /improvement/edits/{id}/approve|reject|rollback`) and version log.
//! The bundled `otto-design-*` skills are never touched: team rules live in
//! this separate overlay skill, one `- [rule:<key>] <text>` line per rule.
//!
//! Dedup uses the `learning_checkpoints` cursor (source
//! `design:<workspace>:<key>`, checkpoint `{"edit_id": …}`): a candidate is
//! proposed once; a pending / applied / rejected / rolled-back edit is final
//! (a human decided), while a `conflict` edit (another rule was approved first,
//! so its snapshot went stale) is re-proposed against the current file on the
//! next pass.

use std::path::PathBuf;

use otto_core::domain::{
    ImprovementEdit, ImprovementEditKind, ImprovementEditStatus, ImprovementRisk,
    ImprovementRunStatus, ImprovementTarget, ImprovementTrigger,
};
use otto_core::event::Event;
use otto_core::{Id, Result};
use otto_state::NewEdit;
use serde::{Deserialize, Serialize};

use crate::engine::ImprovementEngine;
use crate::pathsafe::resolve_target;

/// The overlay skill learned design rules are written to.
pub const DESIGN_SKILL: &str = "design-team-style";
/// Every rule line starts with this (`- [rule:<key>] <text>`).
pub const RULE_PREFIX: &str = "- [rule:";
/// Rules proposed per pass (bounds one extraction's approval queue).
pub const MAX_PROPOSALS_PER_PASS: usize = 10;
const MAX_KEY: usize = 120;
const MAX_RULE_CHARS: usize = 400;

/// One ready candidate from the design-signal extractor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignRuleProposal {
    /// Stable identity (e.g. `variant_direction:bold`) — the dedup key.
    pub key: String,
    /// The one-line rule.
    pub rule: String,
    /// Why (counts), shown to the approver.
    pub rationale: String,
    /// Evidence: design signal ids.
    pub evidence: Vec<String>,
}

/// What one pass proposed.
#[derive(Debug, Clone, Default, Serialize)]
pub struct DesignLearnOutcome {
    /// The improvement run the new edits belong to (none when nothing new).
    pub run_id: Option<Id>,
    /// Newly created pending edit ids.
    pub proposed: Vec<Id>,
    /// Candidates skipped: already proposed / decided / active, or leased by
    /// a concurrent pass.
    pub skipped: usize,
}

/// A rule key is `[a-z0-9_.:-]`, 1..=120 chars (it becomes part of a file line
/// and a checkpoint source).
pub fn valid_key(k: &str) -> bool {
    !k.is_empty()
        && k.len() <= MAX_KEY
        && k.chars().all(|c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, '_' | '.' | ':' | '-')
        })
}

/// The skill file a brand-new rule set starts from.
pub fn skill_header() -> String {
    format!(
        "---\nname: {DESIGN_SKILL}\ndescription: Team design rules Otto learned from this workspace's design decisions (variant picks, edits after agent drafts, accepted accessibility fixes). Every rule was approved by a person. Apply them to every design turn.\n---\n\
         # Design team style\n\n\
         Rules learned from this team's design decisions and approved by a person. Apply them \
         unless the request says otherwise, and say \"Applied your team rules: …\" when you do. \
         One rule per line; remove a line (or roll the edit back) to retire a rule.\n\n"
    )
}

/// `- [rule:<key>] <text>` (the text flattened to one line and capped).
pub fn rule_line(key: &str, rule: &str) -> String {
    let text: String = rule
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_RULE_CHARS)
        .collect();
    format!("{RULE_PREFIX}{key}] {text}")
}

/// Every `(key, text)` rule line in a skill body, in file order.
pub fn parse_rules(content: &str) -> Vec<(String, String)> {
    content
        .lines()
        .filter_map(|l| {
            let rest = l.trim().strip_prefix(RULE_PREFIX)?;
            let (key, text) = rest.split_once(']')?;
            let key = key.trim();
            valid_key(key).then(|| (key.to_string(), text.trim().to_string()))
        })
        .collect()
}

/// `current` (or a fresh header) with the rule appended; `None` when the key
/// is already present.
pub fn with_rule(current: Option<&str>, key: &str, rule: &str) -> Option<String> {
    let base = current.map(str::to_string).unwrap_or_else(skill_header);
    if parse_rules(&base).iter().any(|(k, _)| k == key) {
        return None;
    }
    let mut out = base;
    if !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&rule_line(key, rule));
    out.push('\n');
    Some(out)
}

/// The rule(s) an edit ADDS (rule lines in `after` that `before` lacks).
pub fn added_rules(edit: &ImprovementEdit) -> Vec<(String, String)> {
    let before = parse_rules(edit.before_content.as_deref().unwrap_or(""));
    parse_rules(&edit.after_content)
        .into_iter()
        .filter(|(k, _)| !before.iter().any(|(b, _)| b == k))
        .collect()
}

fn checkpoint_source(ws_id: &str, key: &str) -> String {
    format!("design:{ws_id}:{key}")
}

impl ImprovementEngine {
    /// Where this workspace's `design-team-style` skill lives (the library
    /// entry when one exists, else `<root>/.claude/skills/…/SKILL.md`) — the
    /// same resolution every skill edit uses.
    pub async fn design_skill_path(&self, ws_id: &Id) -> Result<PathBuf> {
        let ws = self.workspaces.get(ws_id).await?;
        resolve_target(
            &ws.root_path,
            ImprovementTarget::Skill,
            DESIGN_SKILL,
            Some(self.library_root.as_path()),
        )
    }

    /// The ACTIVE learned design rules (the skill file's rule lines).
    pub async fn design_rules_active(&self, ws_id: &Id) -> Result<Vec<(String, String)>> {
        let path = self.design_skill_path(ws_id).await?;
        let body = tokio::fs::read_to_string(&path).await.unwrap_or_default();
        Ok(parse_rules(&body))
    }

    /// Every edit of the design skill in this workspace (any status), newest
    /// first, capped.
    pub async fn design_rule_edits(&self, ws_id: &Id) -> Result<Vec<ImprovementEdit>> {
        let mut out = Vec::new();
        for status in [
            ImprovementEditStatus::Pending,
            ImprovementEditStatus::Applied,
            ImprovementEditStatus::Conflict,
            ImprovementEditStatus::Rejected,
            ImprovementEditStatus::RolledBack,
        ] {
            out.extend(
                self.improvements
                    .list_edits_by_status(ws_id, status)
                    .await?
                    .into_iter()
                    .filter(|e| {
                        e.target == ImprovementTarget::Skill && e.target_ref == DESIGN_SKILL
                    }),
            );
        }
        out.sort_by(|a, b| b.created_at.cmp(&a.created_at).then(a.id.cmp(&b.id)));
        out.truncate(200);
        Ok(out)
    }

    /// Propose `proposals` as PENDING edits of the design skill (suggest-only:
    /// the workspace's autonomy never auto-applies a design rule).
    pub async fn learn_design_rules(
        &self,
        ws_id: &Id,
        proposals: &[DesignRuleProposal],
    ) -> Result<DesignLearnOutcome> {
        let mut outcome = DesignLearnOutcome::default();
        if proposals.is_empty() {
            return Ok(outcome);
        }
        let path = self.design_skill_path(ws_id).await?;
        let path_str = path.to_string_lossy().to_string();
        let mut run_id: Option<Id> = None;
        for p in proposals.iter().take(MAX_PROPOSALS_PER_PASS) {
            if !valid_key(&p.key) || p.rule.trim().is_empty() {
                outcome.skipped += 1;
                continue;
            }
            let source = checkpoint_source(ws_id, &p.key);
            let Some(claim) = self.improvements.claim_evidence(&source).await? else {
                outcome.skipped += 1;
                continue;
            };
            // A human already decided (or is deciding) this rule → final.
            let prior = claim
                .checkpoint
                .get("edit_id")
                .and_then(|v| v.as_str())
                .map(str::to_string);
            let decided = match &prior {
                Some(eid) => match self.improvements.get_edit(eid).await {
                    Ok(e) => e.status != ImprovementEditStatus::Conflict,
                    Err(_) => false,
                },
                None => false,
            };
            // Re-read per rule: an approval may land between two proposals.
            let current = tokio::fs::read_to_string(&path).await.ok();
            let after = if decided {
                None
            } else {
                with_rule(current.as_deref(), &p.key, &p.rule)
            };
            let Some(after) = after else {
                // Decided, or already an active rule line (e.g. added by hand).
                self.improvements
                    .finish_evidence(&source, &claim.token, None)
                    .await?;
                outcome.skipped += 1;
                continue;
            };
            let rid = match &run_id {
                Some(r) => r.clone(),
                None => {
                    let run = self
                        .improvements
                        .create_run(ws_id, ImprovementTrigger::Live)
                        .await?;
                    let _ = self.events.send(Event::ImprovementRunStarted {
                        workspace_id: ws_id.clone(),
                        run_id: run.id.clone(),
                    });
                    run_id = Some(run.id.clone());
                    run.id
                }
            };
            let evidence: Vec<String> = p.evidence.iter().take(20).cloned().collect();
            let created = self
                .improvements
                .create_edit(NewEdit {
                    run_id: rid.clone(),
                    workspace_id: ws_id.clone(),
                    target: ImprovementTarget::Skill,
                    target_ref: DESIGN_SKILL.to_string(),
                    target_path: path_str.clone(),
                    kind: if current.is_none() {
                        ImprovementEditKind::Add
                    } else {
                        ImprovementEditKind::Modify
                    },
                    risk: ImprovementRisk::Low,
                    status: ImprovementEditStatus::Pending,
                    rationale: format!(
                        "[design] {} — {}",
                        p.rule.chars().take(MAX_RULE_CHARS).collect::<String>(),
                        p.rationale.chars().take(MAX_RULE_CHARS).collect::<String>()
                    ),
                    evidence,
                    before_content: current,
                    after_content: after,
                    actor: None,
                })
                .await;
            let edit = match created {
                Ok(e) => e,
                Err(e) => {
                    // Leave the cursor unchanged so the next pass retries.
                    let _ = self
                        .improvements
                        .finish_evidence(&source, &claim.token, None)
                        .await;
                    return Err(e);
                }
            };
            self.improvements
                .finish_evidence(
                    &source,
                    &claim.token,
                    Some(&serde_json::json!({ "edit_id": edit.id })),
                )
                .await?;
            let _ = self.events.send(Event::ImprovementApprovalPending {
                workspace_id: ws_id.clone(),
                run_id: rid,
                edit_id: edit.id.clone(),
                target_ref: DESIGN_SKILL.to_string(),
            });
            let _ = self.events.send(Event::ImprovementUpdated {
                kind: "approval_pending".into(),
                id: Some(edit.id.clone()),
            });
            outcome.proposed.push(edit.id);
        }
        if let Some(rid) = &run_id {
            let pending = outcome.proposed.len() as i64;
            self.improvements
                .finish_run(
                    rid,
                    ImprovementRunStatus::Done,
                    &format!(
                        "Design learning: {pending} team rule(s) proposed from design signals"
                    ),
                    0,
                    0,
                    pending,
                    None,
                )
                .await?;
            let _ = self.events.send(Event::ImprovementRunFinished {
                workspace_id: ws_id.clone(),
                run_id: rid.clone(),
                status: "done".into(),
                applied: 0,
                pending,
            });
        }
        outcome.run_id = run_id;
        Ok(outcome)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    use otto_core::auth::BoxFuture;
    use otto_state::{ImprovementsRepo, SessionsRepo, WorkspacesRepo};
    use tokio::sync::broadcast;

    use crate::producer::ProposalProducer;
    use crate::proposal::ImprovementProposal;

    struct NoProducer;
    impl ProposalProducer for NoProducer {
        fn produce<'a>(
            &'a self,
            _prompt: &'a str,
            _cwd: &'a str,
            _provider: &'a str,
        ) -> BoxFuture<'a, Result<ImprovementProposal>> {
            Box::pin(async move {
                Ok(ImprovementProposal {
                    run_summary: String::new(),
                    edits: vec![],
                })
            })
        }
    }

    async fn harness() -> (ImprovementEngine, Id, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let pool = otto_state::open(&dir.path().join("t.db")).await.unwrap();
        let workspaces = WorkspacesRepo::new(pool.clone());
        let users = otto_state::UsersRepo::new(pool.clone());
        let uid = users.create("root", "pw", "root", true).await.unwrap().id;
        let ws = workspaces
            .create("t", dir.path().to_str().unwrap(), &uid)
            .await
            .unwrap();
        let (events, _) = broadcast::channel(64);
        let engine = ImprovementEngine {
            improvements: ImprovementsRepo::new(pool.clone()),
            sessions: SessionsRepo::new(pool.clone()),
            workspaces,
            producer: Arc::new(NoProducer),
            events,
            library_root: dir.path().join("library"),
        };
        (engine, ws.id, dir)
    }

    fn prop(key: &str, rule: &str) -> DesignRuleProposal {
        DesignRuleProposal {
            key: key.into(),
            rule: rule.into(),
            rationale: "Chosen 3× across 2 artifacts.".into(),
            evidence: vec!["sig1".into(), "sig2".into()],
        }
    }

    #[test]
    fn rule_lines_round_trip_and_dedup() {
        let body = with_rule(None, "variant_direction:bold", "Lead with\n bold heroes.").unwrap();
        assert!(body.starts_with("---\nname: design-team-style\n"));
        assert_eq!(
            parse_rules(&body),
            vec![(
                "variant_direction:bold".to_string(),
                "Lead with bold heroes.".to_string()
            )]
        );
        assert!(with_rule(Some(&body), "variant_direction:bold", "again").is_none());
        let two = with_rule(Some(&body), "a11y:color-contrast", "Check contrast.").unwrap();
        assert_eq!(parse_rules(&two).len(), 2);
        assert!(!valid_key("Bad Key"));
        assert!(!valid_key(""));
        assert!(valid_key("edit_property:scene3d:material.color"));
        // Hand-written junk lines are ignored.
        assert!(parse_rules("- [rule:NOPE] x\n- plain bullet").is_empty());
    }

    #[tokio::test]
    async fn design_rules_are_pending_only_deduped_and_reproposed_after_conflict() {
        let (engine, ws, _dir) = harness().await;
        let out = engine
            .learn_design_rules(
                &ws,
                &[
                    prop("variant_direction:bold", "Lead with bold heroes."),
                    prop("a11y:contrast", "Check contrast."),
                ],
            )
            .await
            .unwrap();
        assert_eq!(out.proposed.len(), 2);
        assert!(out.run_id.is_some());
        // Suggest-only: nothing written until a human approves.
        let path = engine.design_skill_path(&ws).await.unwrap();
        assert!(!path.exists());
        let edits = engine.design_rule_edits(&ws).await.unwrap();
        assert_eq!(edits.len(), 2);
        assert!(edits
            .iter()
            .all(|e| e.status == ImprovementEditStatus::Pending));
        assert!(edits.iter().all(|e| e.evidence == vec!["sig1", "sig2"]));

        // A second pass proposes nothing new.
        let again = engine
            .learn_design_rules(
                &ws,
                &[prop("variant_direction:bold", "Lead with bold heroes.")],
            )
            .await
            .unwrap();
        assert!(again.proposed.is_empty());
        assert_eq!(again.skipped, 1);

        // Approve one → the file holds it; the other now conflicts on approve.
        let bold = edits
            .iter()
            .find(|e| added_rules(e)[0].0 == "variant_direction:bold")
            .unwrap();
        let contrast = edits.iter().find(|e| e.id != bold.id).unwrap();
        engine.approve_edit(&bold.id, "u1").await.unwrap();
        assert_eq!(engine.design_rules_active(&ws).await.unwrap().len(), 1);
        let c = engine.approve_edit(&contrast.id, "u1").await.unwrap();
        assert_eq!(c.status, ImprovementEditStatus::Conflict);
        // The conflicted rule is re-proposed against the current file.
        let re = engine
            .learn_design_rules(&ws, &[prop("a11y:contrast", "Check contrast.")])
            .await
            .unwrap();
        assert_eq!(re.proposed.len(), 1);
        let fresh = engine.improvements.get_edit(&re.proposed[0]).await.unwrap();
        engine.approve_edit(&fresh.id, "u1").await.unwrap();
        assert_eq!(engine.design_rules_active(&ws).await.unwrap().len(), 2);
        // Rejected stays rejected; rollback restores the previous file.
        engine.rollback_edit(&fresh.id, "u1").await.unwrap();
        assert_eq!(engine.design_rules_active(&ws).await.unwrap().len(), 1);
        let after = engine
            .learn_design_rules(&ws, &[prop("a11y:contrast", "Check contrast.")])
            .await
            .unwrap();
        assert!(
            after.proposed.is_empty(),
            "a rolled-back rule is a human decision"
        );
    }
}
