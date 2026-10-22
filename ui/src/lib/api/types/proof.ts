// Contract mirror: proof. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// Proof Packs
// ---------------------------------------------------------------------------

export type ProofStatus = 'missing' | 'partial' | 'passed' | 'failed' | 'waived';
export type ProofArtifactKind =
  | 'command'
  | 'log'
  | 'screenshot'
  | 'video'
  | 'diff'
  | 'ci'
  | 'api'
  | 'db'
  | 'kafka'
  | 'review'
  | 'approval'
  | 'pr_check'
  | 'self_review';
export type ProofArtifactStatus = 'passed' | 'failed' | 'pending' | 'info';
export type WorkItemKind = 'session' | 'goal_loop' | 'review' | 'workflow_run' | 'task' | 'manual';
export type ProofBadge =
  | 'no_proof'
  | 'tests_passed'
  | 'tests_failed'
  | 'human_approved'
  | 'risky_change'
  | 'ci_missing'
  | 'ci_passed'
  | 'ci_failed'
  | 'ci_pending'
  | 'db_api_verified'
  | 'ui_verified'
  | 'pr_inconsistent'
  | 'review_unresolved'
  | 'waived';

export interface ProofPack {
  id: Id;
  workspace_id: Id;
  work_item_kind: WorkItemKind;
  work_item_id: string;
  title: string;
  status: ProofStatus;
  summary: string;
  risk_score: number;
  done_score: number;
  parent_pack_id?: string | null;
  repo_id?: string | null;
  pr_number?: number | null;
  waived_by?: string | null;
  waived_reason?: string | null;
  waived_at?: string | null;
  /** Set by the opt-in stale-session archive; cleared when the pack changes. */
  archived_at?: string | null;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

/** `POST /workspaces/{id}/proof-packs/archive-sessions` body (opt-in, ws admin). */
export interface ProofArchiveSessionsReq {
  /** Default 30, minimum 7. */
  older_than_days?: number;
  /** Dry run (count only) unless true. */
  apply?: boolean;
}

/** `POST /workspaces/{id}/proof-packs/archive-sessions` result. Nothing is deleted. */
export interface ProofArchiveSessionsResp {
  applied: boolean;
  older_than_days: number;
  cutoff: string;
  /** Packs that match (stale, session, no evidence, not waived/archived). */
  matched: number;
  /** Packs stamped `archived_at` by this call (0 on a dry run). */
  archived: number;
}

export interface ProofArtifact {
  id: Id;
  proof_pack_id: Id;
  workspace_id: Id;
  kind: ProofArtifactKind;
  title: string;
  content_ref?: string | null;
  status: ProofArtifactStatus;
  metadata: unknown;
  content_sha256?: string | null;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

/** A proof pack with its derived badge list + artifact count (list/summary row). */
export interface ProofPackResp extends ProofPack {
  badges: string[];
  artifact_count: number;
}

/** One artifact plus a capped inline preview for list/detail rendering. */
export interface ProofArtifactView extends ProofArtifact {
  preview?: string | null;
  truncated: boolean;
}

/** One line of the done-contract checklist. */
export interface ContractItem {
  key: string;
  label: string;
  required: boolean;
  satisfied: boolean;
  weight: number;
  detail: string;
}

/** The "done contract": an explainable readiness score + itemized checklist. */
export interface DoneContract {
  score: number;
  satisfied: number;
  required: number;
  items: ContractItem[];
}

/** Snapshot metadata (no bundle/reports). */
export interface ProofSnapshotMeta {
  id: string;
  proof_pack_id: string;
  seq: number;
  sha256: string;
  status: string;
  done_score: number;
  risk_score: number;
  note: string;
  created_by: string;
  created_at: string;
}

/** A full immutable snapshot. */
export interface ProofSnapshotResp extends ProofSnapshotMeta {
  bundle: unknown;
  report_md: string;
  report_html: string;
}

/** `GET /proof-packs/{id}` — the pack, its badges, artifacts, children, contract. */
export interface ProofPackDetail {
  pack: ProofPack;
  badges: string[];
  artifacts: ProofArtifactView[];
  children: ProofPackResp[];
  done_contract: DoneContract;
  snapshots: ProofSnapshotMeta[];
}

export interface ProofSummaryRow {
  work_item_kind: string;
  work_item_id: string;
  proof_pack_id: string;
  status: string;
  risk_score: number;
  done_score: number;
  badges: string[];
}

export interface ProofSummaryResp {
  rows: ProofSummaryRow[];
}

export interface CreateProofPackReq {
  work_item_kind: string;
  work_item_id: string;
  title?: string;
  parent_pack_id?: string;
  /** Link the pack to a registered repo so its proof policy applies (strengthen-only). */
  repo_id?: string;
}

/** `POST /proof-packs/{id}/waive` body. */
export interface WaiveReq {
  reason: string;
}

export interface AddArtifactReq {
  kind: string;
  title: string;
  content?: string;
  content_url?: string;
  status?: string;
  metadata?: unknown;
}

export interface AssembleReq {
  cwd?: string;
  base?: string;
  commands?: { cmd: string; kind?: string }[];
}

/** Per-repository proof requirements (R3 — strengthen-only). */
export interface RepoProofConfig {
  require_test?: boolean;
  test_cmd?: string | null;
  require_ci?: boolean;
  require_pr_consistency?: boolean;
  require_review?: boolean;
}

export interface RepoProofConfigResp extends RepoProofConfig {
  repo_id: string;
}

export interface CreateSnapshotReq {
  note?: string;
}

export interface AttachMediaReq {
  kind: 'screenshot' | 'video';
  title: string;
  mime: string;
  data_base64: string;
  metadata?: unknown;
}

export interface ApiEvidenceReq {
  title: string;
  method: string;
  url: string;
  status: number;
  duration_ms?: number;
  request?: string;
  response?: string;
  metadata?: unknown;
}

export interface DbEvidenceReq {
  title: string;
  engine?: string;
  query?: string;
  columns?: string[];
  row_count?: number;
  sample?: string;
  error?: string;
  metadata?: unknown;
}

export interface KafkaEvidenceReq {
  title: string;
  topic: string;
  message_count?: number;
  sample?: string;
  truncated?: boolean;
  error?: string;
  metadata?: unknown;
}

export interface PrCheckReq {
  title: string;
  description: string;
  base?: string;
  cwd?: string;
}

export interface CiRefreshReq {
  repo_id?: string;
  pr_number?: number;
}
