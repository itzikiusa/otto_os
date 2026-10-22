// Contract mirror: review. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// PR Review (AI agents)
// ---------------------------------------------------------------------------

export type ReviewStatus = 'running' | 'done' | 'error' | 'cancelled';
export type ReviewCommentState = 'draft' | 'approved' | 'declined';
/** `skipped`: the agent failed and was NOT retried because a sibling row (same
 *  `lens`, another provider) had already finished the lens. */
export type ReviewAgentStatus = 'pending' | 'running' | 'waiting' | 'done' | 'error' | 'skipped';

/** One raw finding from a single review agent (mirrors Rust `otto_core::domain::ReviewFinding`).
 *  It carries no fingerprint or lifecycle state: those live on the persisted
 *  `ReviewFindingRow` from `/reviews/{id}/findings`. */
export interface ReviewFinding {
  path: string | null;
  line: number | null;
  /** Last line (inclusive) of a multi-line finding, when the reviewer gave one. */
  line_end?: number | null;
  severity: string; // 'info' | 'warn' | 'bug'
  body: string;
  /** Lens slug that produced the finding (orchestrator mode keeps it through the merge, so one
   *  agent row's findings span several lenses); absent for fan-out reviewers and pre-field rows. */
  lens?: string | null;
}

/** A persistent finding row from /reviews/{id}/findings (A1 verified-review loop). */
export interface ReviewFindingRow {
  id: string;
  review_id: string;
  fingerprint: string;
  path: string | null;
  line: number | null;
  severity: string;
  category: string | null;
  body: string;
  state: string; // open | fixing | resolved | regressed | declined
  fix_session_id: string | null;
  updated_at: string;
}

export interface ReviewAgentState {
  name: string;
  provider: string;
  model: string;
  status: ReviewAgentStatus;
  note: string;
  comment_count: number;
  /** Openable live session running this agent. */
  session_id?: string | null;
  /** This agent's own findings (before summarization). */
  findings?: ReviewFinding[];
  /** True on the summarizer row when its output came from the deterministic
   *  Rust-side dedupe/rank fallback (configured summarizer unavailable). */
  fallback?: boolean;
  /** The configured reviewer (lens) this row expanded from; rows sharing a
   *  lens are the same lens on different providers. Absent on the summarizer
   *  row and on reviews persisted before the field existed. */
  lens?: string;
  /** Orchestrator rows only: the lens slugs delegated to sub-agents. A retry
   *  re-spawns the row as the same orchestrator. */
  lens_slugs?: string[];
}

export interface ReviewComment {
  id: string;
  review_id: string;
  path: string | null;
  line: number | null;
  severity: 'info' | 'warn' | 'bug';
  body: string;
  state: ReviewCommentState;
  posted: boolean;
  created_at: string;
}

/** PATCH /pr-review-comments/{cid}: edit a draft's body and/or restore a declined comment. */
export interface EditReviewCommentReq {
  body?: string;
  restore_draft?: boolean;
  /** Record an approved-but-unposted comment as posted WITHOUT sending it
   *  (its copy was found on the PR). Alone; 409 unless approved + unposted. */
  mark_posted?: boolean;
}

export interface Review {
  id: string;
  repo_id: string;
  pr_number: number;
  status: ReviewStatus;
  error: string | null;
  comments: ReviewComment[];
  agents: ReviewAgentState[];
  created_at: string;
  verdict?: string | null;
  blocker_count?: number | null;
  summary_md?: string | null;
  /** true when the final comments came from the deterministic summarizer
   *  fallback (configured summarizer unavailable) — derived from the agents' fallback flags. */
  summary_fallback?: boolean;
}

export interface ReviewAgentCfg {
  name: string;
  /** "claude" | "codex" | "agy" — kept for back-compat; effective list is `providers` when non-empty */
  provider: string;
  /** List of CLIs to run this agent on. Expands to one run per entry. */
  providers?: string[];
  /** "haiku" | "sonnet" | "opus" | "" (empty = provider default) */
  model: string;
  /** The lens/instructions for this agent */
  prompt: string;
  /** Optional Library/bundled skill name. When non-empty its full text (SKILL.md + references)
   *  is inlined into the prompt at review time, for every provider. Rust `#[serde(default)]`:
   *  always sent by the server (empty = none); optional on writes. */
  skill?: string;
}

/** Review execution mode: one reviewer per lens × provider, or one orchestrator per provider running every lens as sub-agents. */
export type ReviewMode = 'fan_out' | 'orchestrator';

export interface ReviewConfig {
  agents: ReviewAgentCfg[];
  summarizer: ReviewAgentCfg;
  custom_presets?: ReviewAgentCfg[];
  /** Max total attempts per agent (initial + retries). Default 3. */
  max_attempts?: number | null;
  /** Per-agent timeout in seconds; overrides diff-size heuristic. */
  timeout_secs?: number | null;
  /** Execution mode; absent/null ⇒ fan_out. A review_run step's `params.mode` and a run's
   *  `review_mode` both override it. */
  mode?: ReviewMode | null;
}

/** A named, reusable full review configuration.
 *  GET/PUT /settings/pr-review/presets (PUT replaces the whole list). */
export interface ReviewConfigPreset {
  id: string;
  name: string;
  config: ReviewConfig;
}

/** PUT /repos/{id}/review-config body — exactly one of preset_id / config.
 *  Inline `config` wins if both are set; DELETE reverts the repo to global. */
export interface RepoReviewBinding {
  preset_id?: string | null;
  config?: ReviewConfig | null;
}

/** GET/PUT/DELETE /repos/{id}/review-config response: the repo's binding
 *  resolved to its scope plus the EFFECTIVE config reviews will run with. */
export interface RepoReviewConfigResp {
  scope: 'global' | 'preset' | 'custom';
  preset_id?: string | null;
  /** Resolved preset display name; null when the referenced preset was deleted. */
  preset_name?: string | null;
  config: ReviewConfig;
}

export interface MergeReadiness {
  unresolved_blocker_count: number;
  unresolved_total: number;
  resolved_count: number;
  last_updated?: string | null;
  // A1 additions from the /merge-readiness endpoint:
  total_findings?: number | null;
  /** Aggregated CI state: "success" | "failure" | "pending" | "none" */
  ci_status?: string | null;
  /** Number of human approvals on the PR. */
  approvals?: number | null;
  /** Whether the PR is mergeable according to the provider (null = unknown). */
  mergeable?: boolean | null;
  conflicts?: boolean | null;
  branch_freshness?: string | null;
  unpushed?: boolean | null;
}

export interface StartReviewReq {
  issue_account_id?: string | null;
  issue_key?: string | null;
  /** Free-text guidance for the review agents (e.g. "what to focus on"). Optional. */
  context?: string | null;
}

export interface LocalReviewReq {
  /** The base git ref to diff against (e.g. "origin/develop", "main"). */
  base: string;
}

export interface HandoffReq {
  /** The provider to spawn ("claude" | "codex" | "agy"). */
  provider: string;
  /** Optional list of comment ids to include. When absent, all non-declined comments are sent. */
  comment_ids?: string[] | null;
}

/** Durable delivery record stored in the target session's meta.handover. */
export interface HandoverDelivery {
  id: Id;
  source_id: Id;
  state: 'preparing' | 'awaiting_target' | 'sent' | 'acknowledged' | 'failed';
  brief: string;
  focus: string;
  archive_source: boolean;
  include_git: boolean;
  fast: boolean;
  error: string | null;
  updated_at: string;
}

/** POST /sessions/{id}/handover/{retry|acknowledge}; guards stale panel actions. */
export interface HandoverDeliveryActionReq { delivery_id: Id }

/** Where a handover brief is delivered. */
export type HandoverTarget =
  | { kind: 'new_agent'; provider: string }
  | { kind: 'existing_session'; session_id: Id };

/** POST /sessions/{id}/handover — pass one agent's context into another agent. */
export interface HandoverReq {
  target: HandoverTarget;
  /** What the receiving agent should focus on (weights the generated brief). */
  focus?: string | null;
  /** Title for the new session (new_agent only). */
  title?: string | null;
  /** Pre-reviewed brief; when present the server skips summarization. */
  brief?: string | null;
  /** Include git state in the generated brief. Defaults to true. */
  include_git?: boolean | null;
  /** Summarize with a fast model. Defaults to false. */
  fast?: boolean | null;
  /** Archive the source only after receipt is explicitly acknowledged. Defaults to false. */
  archive_source?: boolean | null;
}

/** POST /sessions/{id}/handover/brief — generate the brief for review. */
export interface HandoverBriefReq {
  focus?: string | null;
  include_git?: boolean | null;
  fast?: boolean | null;
}

export interface HandoverBriefResp {
  /** The generated brief (markdown); empty when there was no context. */
  brief: string;
  /** True when summarization was unavailable and `brief` is raw context. */
  fallback: boolean;
  /** True when some source context (transcript/scrollback/git) was found. */
  had_context: boolean;
}

// ---------------------------------------------------------------------------
// Review findings workflow (the 6-state finding lifecycle + Proof Pack)
// ---------------------------------------------------------------------------
// Mirrors `crates/otto-core/src/finding.rs`. A `Finding` is the tracked
// workflow record produced by the multi-agent code review; the action endpoints
// drive its `status` through six states and append an immutable `FindingEvent`
// audit trail (the "closing the loop with evidence" spine). See
// docs/superpowers/specs/2026-06-26-review-findings-workflow-design.md.

/** The 6-value workflow disposition of a finding. */
export type FindingStatus =
  | 'open'
  | 'accepted'
  | 'false_positive'
  | 'fixed'
  | 'verified'
  | 'waived';

/** Normalized finding severity (5-level scale). */
export type FindingSeverity = 'critical' | 'high' | 'medium' | 'low' | 'info';

/** A review finding as the workflow tracks it (`GET /reviews/{id}/findings`,
 *  the per-action responses). `finding_id` = `id`. */
export interface Finding {
  id: Id;
  review_id: Id;
  workspace_id: Id;
  repo_id: Id;
  pr_number: number | null;
  fingerprint: string;
  // --- the 11 required fields ---
  severity: FindingSeverity;
  category: string | null;
  path: string | null;
  /** Range start line (the legacy `line`). */
  line: number | null;
  /** Range end line. */
  line_end: number | null;
  title: string;
  body: string;
  evidence: string;
  agent_reasoning_summary: string;
  suggested_fix: string | null;
  status: FindingStatus;
  linked_commit: string | null;
  linked_test: string | null;
  /** The current disposition owner (producing agent at creation, triaging actor after). */
  reviewer: string;
  // --- workflow state / gates / artifacts ---
  /** Engine DETECTION lifecycle (read-only): open|fixing|resolved|regressed|declined. */
  state: string;
  /** Derived: the detection axis currently reads `regressed`. */
  regressed: boolean;
  requires_human_approval: boolean;
  approval_decision: string | null;
  approved_by: string | null;
  approved_at: string | null;
  jira_key: string | null;
  jira_url: string | null;
  produced_by_agent: string | null;
  repo_rule_id: Id | null;
  fix_session_id: Id | null;
  occurrence_count: number;
  created_at: string;
  updated_at: string;
}

/** One immutable audit-trail entry for a finding. */
export interface FindingEvent {
  id: Id;
  finding_id: Id;
  kind: string;
  actor: string;
  from_status: string | null;
  to_status: string | null;
  /** Free-form JSON payload (`{session_id?, commit?, test?, jira_key?, reason?, …}`). */
  detail: unknown;
  created_at: string;
}

/** A finding plus its full event timeline (`GET /findings/{id}`). */
export interface FindingDetail {
  finding: Finding;
  events: FindingEvent[];
}

/** The response of an action that may spawn a live agent session (fix / verify /
 *  regression-test). `session_id` is present when an openable session started. */
export interface FindingActionResp {
  finding: Finding;
  session_id?: Id | null;
  /** Verify only: the evidence behind a pass ("3 tests passed") or why the
   *  finding was NOT verified (no linked test, zero tests ran, …). */
  note?: string | null;
}

/** A repo rule generalized from a finding, fed into the Context Engine. */
export interface RepoRule {
  id: Id;
  workspace_id: Id;
  title: string;
  body: string;
  category: string | null;
  severity: string | null;
  glob: string | null;
  source_finding_id: Id | null;
  enabled: boolean;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

/** Aggregate counts for a Proof Pack. */
export interface ReviewProofPackSummary {
  total: number;
  by_status: Record<string, number>;
  by_severity: Record<string, number>;
  verified: number;
  fixed: number;
  open: number;
  with_commit: number;
  with_test: number;
}

/** One finding + its event timeline inside a Proof Pack. */
export interface ReviewProofPackEntry {
  finding: Finding;
  events: FindingEvent[];
}

/** The live-assembled evidence bundle for a review (`GET /reviews/{id}/proof-pack`). */
export interface ReviewProofPack {
  review_id: Id;
  workspace_id: Id;
  generated_at: string;
  summary: ReviewProofPackSummary;
  findings: ReviewProofPackEntry[];
  repo_rules: RepoRule[];
}

/** The persisted-snapshot response of `POST /reviews/{id}/proof-pack/export`. */
export interface ReviewProofPackExport {
  id: Id;
  review_id: Id;
  format: string;
  markdown: string;
  created_at: string;
}
