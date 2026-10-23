//! The one stage runner both governed tool pipelines end in (architecture-5-03).
//!
//! Two entry points govern a tool call: [`crate::McpService::invoke`] for a
//! REGISTERED MCP server, and `otto-server`'s `mcp_outward::governed_invoke`
//! for Otto's own `otto.*` tools. Each runs its own admission gates first
//! (server/allow-list/per-tool permission; token scope/enable/agent policy/
//! reference resolution) — those differ by nature. What follows is the same
//! decision in the same order, and lives only here:
//!
//!   [policy-as-code] → approval gate ([auto-approve] may clear it) →
//!   approval (unless a dry-run was requested) → dry-run → audit insert
//!   (fail-closed) → hand back for execution.
//!
//! The bracketed stages are optional members of a [`StageSet`]; each entry
//! point passes a named constant that says which apply and why. The entry
//! point supplies the stage bodies through [`Stages`] (its own approval cards,
//! audit rows and envelopes), and executes the call itself once [`run`]
//! returns [`Verdict::Execute`] — after the audit row exists, never before.

use async_trait::async_trait;

/// Which optional stages an entry point runs. The order is fixed by [`run`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageSet {
    /// Policy-as-code rules (`mcp_policies`: deny / require approval /
    /// require dry-run), evaluated before the approval gate.
    pub policy_as_code: bool,
    /// Operator auto-approve rules (`mcp_auto_approve_rules`), which may clear
    /// an approval gate the entry point marks as auto-approvable.
    pub auto_approve: bool,
}

/// Registered MCP servers: policy-as-code applies (its match keys — server,
/// risk label, injection risk, direction — describe a registered server's
/// tools); auto-approve does not (its rules name `otto.*` tools and
/// categories, and a registered tool's approval is set per tool instead).
/// Today's behaviour, made explicit; deliberately unchanged.
pub const REGISTERED_SERVERS: StageSet = StageSet {
    policy_as_code: true,
    auto_approve: false,
};

/// Otto's own `otto.*` tools: auto-approve applies (it was built for them);
/// policy-as-code does not (`otto.*` calls are governed by the token scope,
/// the enabled-tool list and the per-tool "Ask before each call" toggle).
/// Today's behaviour, made explicit; deliberately unchanged.
pub const OTTO_TOOLS: StageSet = StageSet {
    policy_as_code: false,
    auto_approve: true,
};

/// What a policy-as-code evaluation decided for this call.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyVerdict {
    Allow,
    Deny(String),
    RequireApproval,
    RequireDryRun,
}

/// Whether the call needs a human approval before any opt-out.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ApprovalGate {
    pub needed: bool,
    /// An auto-approve rule may clear this gate (never a forced approval).
    pub auto_approvable: bool,
}

/// The audit decision of a call that goes on to execute.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// A person approved this call (a consumed, single-use approval).
    Approved,
    /// An operator's auto-approve rule cleared the gate.
    AutoApproved,
    /// No approval was needed.
    Allowed,
}

impl Decision {
    /// The `decision` column value.
    pub fn as_str(self) -> &'static str {
        match self {
            Decision::Approved => "approved",
            Decision::AutoApproved => "auto_approved",
            Decision::Allowed => "allowed",
        }
    }
}

/// The approval stage's answer.
pub enum Approval<O> {
    /// An approval was consumed for this call: continue.
    Granted,
    /// The call stops here (a pending card, a human denial, a used approval…)
    /// with the outcome the entry point already audited.
    Stop(O),
}

/// The runner's result.
pub enum Verdict<O, R> {
    /// A terminal outcome, already audited (denied, pending, dry-run).
    Done(O),
    /// Execute now: the fail-closed audit row `row` exists with `decision`.
    Execute { row: R, decision: Decision },
}

/// One entry point's stage bodies. Every terminal path audits exactly once.
#[async_trait]
pub trait Stages: Send {
    type Outcome: Send;
    /// The inserted audit row handed to execution (its id).
    type Row: Send;
    type Error: Send;

    /// Policy-as-code, for a [`StageSet`] that includes it.
    async fn policy(&mut self) -> Result<PolicyVerdict, Self::Error>;
    /// Audit and return a terminal denial (`reason` is the audit reason).
    async fn deny(&mut self, reason: &str) -> Result<Self::Outcome, Self::Error>;
    /// The approval gate, given whether a policy demanded approval.
    async fn approval_gate(&mut self, policy_requires: bool) -> ApprovalGate;
    /// Whether an auto-approve rule covers this call (recording which).
    async fn auto_approve(&mut self) -> bool;
    /// Find/consume or request the human approval.
    async fn approval(&mut self) -> Result<Approval<Self::Outcome>, Self::Error>;
    /// Audit and return the dry-run preview (nothing executes).
    async fn dry_run(&mut self) -> Result<Self::Outcome, Self::Error>;
    /// Insert the audit row BEFORE executing; an error aborts the call.
    async fn audit(&mut self, decision: Decision) -> Result<Self::Row, Self::Error>;
}

/// Run the shared stages in their one order (see the module header).
pub async fn run<S: Stages>(
    set: StageSet,
    stages: &mut S,
    dry_run_requested: bool,
) -> Result<Verdict<S::Outcome, S::Row>, S::Error> {
    let mut policy_dry_run = false;
    let mut policy_approval = false;
    if set.policy_as_code {
        match stages.policy().await? {
            PolicyVerdict::Deny(reason) => {
                let reason = format!("policy denied: {reason}");
                return Ok(Verdict::Done(stages.deny(&reason).await?));
            }
            PolicyVerdict::RequireDryRun => policy_dry_run = true,
            PolicyVerdict::RequireApproval => policy_approval = true,
            PolicyVerdict::Allow => {}
        }
    }
    let gate = stages.approval_gate(policy_approval).await;
    let auto =
        set.auto_approve && gate.needed && gate.auto_approvable && stages.auto_approve().await;
    let mut approved = false;
    // A dry-run request skips the approval (a preview executes nothing); a
    // policy's require-dry-run does not.
    if gate.needed && !auto && !dry_run_requested {
        match stages.approval().await? {
            Approval::Granted => approved = true,
            Approval::Stop(outcome) => return Ok(Verdict::Done(outcome)),
        }
    }
    if dry_run_requested || policy_dry_run {
        return Ok(Verdict::Done(stages.dry_run().await?));
    }
    let decision = if approved {
        Decision::Approved
    } else if auto {
        Decision::AutoApproved
    } else {
        Decision::Allowed
    };
    let row = stages.audit(decision).await?;
    Ok(Verdict::Execute { row, decision })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A scripted entry point: records which stages ran, in order.
    #[derive(Default)]
    struct Fake {
        policy: Option<PolicyVerdict>,
        gate: ApprovalGate,
        rule: bool,
        approval: Option<&'static str>, // None = granted, Some(stop outcome)
        audit_fails: bool,
        ran: Vec<String>,
    }

    #[async_trait]
    impl Stages for Fake {
        type Outcome = String;
        type Row = &'static str;
        type Error = &'static str;
        async fn policy(&mut self) -> Result<PolicyVerdict, &'static str> {
            self.ran.push("policy".into());
            Ok(self.policy.clone().unwrap_or(PolicyVerdict::Allow))
        }
        async fn deny(&mut self, reason: &str) -> Result<String, &'static str> {
            self.ran.push(format!("deny:{reason}"));
            Ok(format!("denied: {reason}"))
        }
        async fn approval_gate(&mut self, policy_requires: bool) -> ApprovalGate {
            self.ran.push(format!("gate:{policy_requires}"));
            ApprovalGate {
                needed: self.gate.needed || policy_requires,
                ..self.gate
            }
        }
        async fn auto_approve(&mut self) -> bool {
            self.ran.push("auto".into());
            self.rule
        }
        async fn approval(&mut self) -> Result<Approval<String>, &'static str> {
            self.ran.push("approval".into());
            Ok(match self.approval {
                None => Approval::Granted,
                Some(o) => Approval::Stop(o.into()),
            })
        }
        async fn dry_run(&mut self) -> Result<String, &'static str> {
            self.ran.push("dry_run".into());
            Ok("dry_run".into())
        }
        async fn audit(&mut self, d: Decision) -> Result<&'static str, &'static str> {
            self.ran.push(format!("audit:{}", d.as_str()));
            if self.audit_fails {
                Err("audit insert failed")
            } else {
                Ok("row")
            }
        }
    }

    /// `(outcome, stages that ran)`; `exec:<decision>` = handed to execution.
    async fn go(set: StageSet, mut f: Fake, dry_run: bool) -> (String, Vec<String>) {
        let out = match run(set, &mut f, dry_run).await {
            Ok(Verdict::Done(o)) => o,
            Ok(Verdict::Execute { row, decision }) => format!("exec:{}:{row}", decision.as_str()),
            Err(e) => format!("err:{e}"),
        };
        (out, f.ran)
    }

    fn dangerous() -> ApprovalGate {
        ApprovalGate {
            needed: true,
            auto_approvable: true,
        }
    }

    /// (name, stage set, scripted stages, executes?, audit decision, stages run)
    type Case<'a> = (&'a str, StageSet, Fake, bool, &'a str, &'a [&'a str]);

    #[tokio::test]
    async fn the_stage_table_for_both_entry_points() {
        let deny = Some(PolicyVerdict::Deny("no prod".into()));
        let cases: Vec<Case> = vec![
            // Policy-as-code applies to registered servers only.
            (
                "server: policy deny",
                REGISTERED_SERVERS,
                Fake {
                    policy: deny.clone(),
                    ..Fake::default()
                },
                false,
                "denied: policy denied: no prod",
                &["policy", "deny:policy denied: no prod"],
            ),
            (
                "otto: a policy is never consulted",
                OTTO_TOOLS,
                Fake {
                    policy: deny,
                    ..Fake::default()
                },
                false,
                "exec:allowed:row",
                &["gate:false", "audit:allowed"],
            ),
            (
                "server: policy require-dry-run",
                REGISTERED_SERVERS,
                Fake {
                    policy: Some(PolicyVerdict::RequireDryRun),
                    ..Fake::default()
                },
                false,
                "dry_run",
                &["policy", "gate:false", "dry_run"],
            ),
            (
                "server: policy require-approval, approved",
                REGISTERED_SERVERS,
                Fake {
                    policy: Some(PolicyVerdict::RequireApproval),
                    ..Fake::default()
                },
                false,
                "exec:approved:row",
                &["policy", "gate:true", "approval", "audit:approved"],
            ),
            (
                "server: policy allow",
                REGISTERED_SERVERS,
                Fake::default(),
                false,
                "exec:allowed:row",
                &["policy", "gate:false", "audit:allowed"],
            ),
            // Auto-approve applies to otto.* tools only.
            (
                "otto: dangerous, a rule matches",
                OTTO_TOOLS,
                Fake {
                    gate: dangerous(),
                    rule: true,
                    ..Fake::default()
                },
                false,
                "exec:auto_approved:row",
                &["gate:false", "auto", "audit:auto_approved"],
            ),
            (
                "otto: dangerous, no rule matches",
                OTTO_TOOLS,
                Fake {
                    gate: dangerous(),
                    ..Fake::default()
                },
                false,
                "exec:approved:row",
                &["gate:false", "auto", "approval", "audit:approved"],
            ),
            (
                "server: a rule is never consulted",
                REGISTERED_SERVERS,
                Fake {
                    gate: dangerous(),
                    rule: true,
                    ..Fake::default()
                },
                false,
                "exec:approved:row",
                &["policy", "gate:false", "approval", "audit:approved"],
            ),
            (
                "otto: a forced approval is never auto-approved",
                OTTO_TOOLS,
                Fake {
                    gate: ApprovalGate {
                        needed: true,
                        auto_approvable: false,
                    },
                    rule: true,
                    ..Fake::default()
                },
                false,
                "exec:approved:row",
                &["gate:false", "approval", "audit:approved"],
            ),
            (
                "otto: dangerous, approval pending",
                OTTO_TOOLS,
                Fake {
                    gate: dangerous(),
                    approval: Some("pending"),
                    ..Fake::default()
                },
                false,
                "pending",
                &["gate:false", "auto", "approval"],
            ),
            (
                "otto: not dangerous",
                OTTO_TOOLS,
                Fake::default(),
                false,
                "exec:allowed:row",
                &["gate:false", "audit:allowed"],
            ),
            // A dry-run request skips approval and never executes.
            (
                "otto: dry-run of a dangerous call",
                OTTO_TOOLS,
                Fake {
                    gate: dangerous(),
                    ..Fake::default()
                },
                true,
                "dry_run",
                &["gate:false", "auto", "dry_run"],
            ),
            (
                "server: dry-run of a dangerous call",
                REGISTERED_SERVERS,
                Fake {
                    gate: dangerous(),
                    ..Fake::default()
                },
                true,
                "dry_run",
                &["policy", "gate:false", "dry_run"],
            ),
            // Fail-closed: no audit row, no execution.
            (
                "server: audit insert fails",
                REGISTERED_SERVERS,
                Fake {
                    audit_fails: true,
                    ..Fake::default()
                },
                false,
                "err:audit insert failed",
                &["policy", "gate:false", "audit:allowed"],
            ),
            (
                "otto: audit insert fails",
                OTTO_TOOLS,
                Fake {
                    audit_fails: true,
                    ..Fake::default()
                },
                false,
                "err:audit insert failed",
                &["gate:false", "audit:allowed"],
            ),
        ];
        for (label, set, fake, dry_run, want, stages) in cases {
            let (out, ran) = go(set, fake, dry_run).await;
            assert_eq!(out, want, "{label}");
            assert_eq!(ran, stages.to_vec(), "{label}");
        }
    }
}
