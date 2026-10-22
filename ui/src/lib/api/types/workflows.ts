// Contract mirror: workflows. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';
import type {
  ReviewMode,
} from './review';

// ---------------------------------------------------------------------------
// Workflow engine (mirrors otto_core::workflows)
// ---------------------------------------------------------------------------

/** Per-node retry policy (exponential backoff). */
export interface RetryPolicy {
  max_attempts: number;
  backoff_ms: number;
  factor: number;
}

export interface WorkflowNode {
  id: string;
  kind: string;
  name: string;
  x: number;
  y: number;
  params: unknown;
  /** Optional retry policy for this node (null/absent = run once). */
  retry?: RetryPolicy | null;
}

export interface WorkflowEdge {
  id: string;
  source: string;
  target: string;
  /** Optional expression gating this edge: the target runs only when truthy. */
  condition?: string | null;
}

export interface WorkflowGraph {
  nodes: WorkflowNode[];
  edges: WorkflowEdge[];
}

export interface Workflow {
  id: Id;
  workspace_id: Id;
  name: string;
  description: string;
  /** Standing free-text guidance every run/step follows (distinct from
   *  `description`, which is a summary). */
  instructions: string;
  graph: WorkflowGraph;
  /** Monotonic version counter, bumped on each graph/instructions edit/restore. */
  version?: number;
  /** Restart policy: 'resume' (default — the startup reconciler re-enters a
   *  run interrupted by a daemon restart) or 'fail' (legacy hard-fail). */
  on_restart?: 'resume' | 'fail';
  created_by: Id;
  created_at: string;
  updated_at: string;
}

// NOTE: named `WorkflowRunStatus` (not `RunStatus`) to avoid colliding with the
// Run-with-Otto `RunStatus` stage machine below. Rust namespaces these per crate
// module (`otto-core::workflows` vs `otto-core::run`); this flat TS file cannot.
export type WorkflowRunStatus = 'pending' | 'running' | 'success' | 'error' | 'canceled';
export type NodeStatus = 'pending' | 'running' | 'success' | 'error' | 'skipped';

/** One sub-agent / background task of a running agent step (status from the parent transcript only). */
export interface SubagentActivity {
  id: string;
  description: string;
  status: 'running' | 'done' | 'failed';
  started_at?: string | null;
  finished_at?: string | null;
}

/** Live phase + sub-agent snapshot of a RUNNING agent step; cleared on finish. Written at most
 *  every 5s and bounded (≤ 40 sub-agents, descriptions ≤ 80 chars) so the WS `node` payload stays
 *  under the 32 KiB inline limit. `phase` is the phase log text without its prefix glyph. */
export interface NodeActivity {
  phase: string;
  updated_at: string;
  last_progress_at?: string | null;
  pending_tasks: number;
  subagents: SubagentActivity[];
  hold_reason?: string | null;
}

export interface NodeRunState {
  /** Present on a lightweight summary; expand to fetch this exact body version. */
  detail_version?: string;
  log_count?: number;
  has_output?: boolean;
  node_id: string;
  status: NodeStatus;
  output?: unknown;
  error?: string | null;
  logs: string[];
  /** When the node started executing (RFC-3339) — drives the live elapsed
   *  timer shown on a running step. */
  started_at?: string | null;
  duration_ms?: number | null;
  /** Number of attempts made (>1 when a retry policy fired). */
  attempts?: number | null;
  /** Session ids this node drove (e.g. agent_prompt / review_run). */
  sessions?: string[];
  /** Reviews started by this step; retained for async completion and retries. */
  review_ids?: string[];
  /** Present only while the node runs (agent steps): phase + sub-agents. */
  activity?: NodeActivity | null;
}

export interface WorkflowRun {
  /** Additive progress response; input/output/log bodies are absent. */
  summary?: boolean;
  checkpoint_rev?: number;
  checkpoint_generation?: number;
  checkpoint_count?: number;
  checkpoint_done?: number;
  /** Durable loop attempt records; populated by the run detail endpoint. */
  checkpoints?: WorkflowCheckpoint[];
  id: Id;
  workflow_id: Id;
  workspace_id: Id;
  /** User who started the run (its agent sessions are owned by them); null for trigger/schedule/
   *  webhook/chat runs, which act as the workflow's latest changer: the later of the latest
   *  version's author and the firing trigger's `updated_by` (PC-3). */
  created_by?: Id | null;
  status: WorkflowRunStatus;
  input: unknown;
  nodes: NodeRunState[];
  error?: string | null;
  started_at: string;
  finished_at?: string | null;
  /** Monotonic revision, bumped on every persisted progress write. The UI
   *  discards snapshots/events whose rev is behind what it already shows. */
  rev?: number;
  /** True when the run is paused waiting for a human_approval node decision. */
  waiting_approval?: boolean;
  /** The node id of the human_approval node the run is paused at. */
  approval_node_id?: string | null;
  /** User id of the person who approved (null if rejected or pending). */
  approved_by?: string | null;
  /** Human note attached to the approval/rejection decision. */
  approval_note?: string | null;
  /** RFC-3339 timestamp when the decision was recorded. */
  approved_at?: string | null;
  /** The workflow version this run executed against. */
  workflow_version?: number | null;
  /** Proof pack assembled for this run, if any. */
  proof_pack_id?: string | null;
  /** How many times the startup reconciler resumed this run after a daemon
   *  restart (0/absent = never interrupted). */
  resume_attempts?: number;
  /** Absolute path of the run's context directory (instruction brief,
   *  repos.json, per-step handoff files). Present on `GET /workflow-runs/{id}`
   *  when the directory exists on disk; absent on list endpoints. Read its files
   *  with `browseRunContext` / `readRunContext` (`lib/api/workflows.ts`), which
   *  take paths RELATIVE to it — `/fs/*` refuses Otto's data dir to non-root users. */
  context_dir?: string | null;
}

export interface WorkflowCheckpoint {
  detail_version?: string;
  log_count?: number;
  has_output?: boolean;
  node_id: string;
  loop_id: string;
  iteration: number;
  step_index: number;
  kind: string;
  name: string;
  status: NodeStatus;
  attempts: number;
  input: unknown;
  /** Completed gate failures retain evidence; `_workflow_error` is reserved by the engine. */
  output: unknown;
  error?: string | null;
  logs: string[];
  updated_at: string;
}

export interface WorkflowValidationIssue {
  node_id: string | null;
  edge_id: string | null;
  field: string;
  message: string;
}

/** `POST /workflows/{id}/run` body. `review_mode` seeds `input.review_mode` and overrides every
 *  review_run step's own `params.mode` for this run (400 on an unknown value, or on a non-object
 *  `input` combined with it). */
export interface RunWorkflowReq {
  input?: unknown;
  start_node?: string;
  only_node?: boolean;
  review_mode?: ReviewMode;
}

/** Lightweight summary of an in-flight run for the "Running" sidebar list.
 *  Returned by `GET /workspaces/{wid}/workflow-runs/active`. */
export interface ActiveWorkflowRun {
  run_id: Id;
  workflow_id: Id;
  workspace_id: Id;
  workflow_name: string;
  status: WorkflowRunStatus;
  started_at: string;
  /** Total nodes and how many have finished (success|skipped) — "3/5 steps". */
  nodes_total: number;
  nodes_done: number;
  /** Paused on a human-approval node. */
  waiting_approval?: boolean;
}

// ---------------------------------------------------------------------------
// Workflow triggers (schedule / webhook / event / chat)
// ---------------------------------------------------------------------------

export type TriggerKind = 'schedule' | 'webhook' | 'event' | 'chat';

/** A workflow trigger row from the database. */
export interface WorkflowTrigger {
  id: Id;
  workflow_id: Id;
  kind: TriggerKind;
  /** Kind-specific configuration object. */
  spec: Record<string, unknown>;
  enabled: boolean;
  created_at: string;
  /** When the schedule was last (re)armed — created, resumed, or re-timed.
   *  Runs missed before it are never caught up. Null on pre-0151 rows. */
  armed_at?: string | null;
}

export interface CreateTriggerReq {
  kind: TriggerKind;
  spec?: Record<string, unknown>;
  enabled?: boolean;
}

export interface UpdateTriggerReq {
  spec?: Record<string, unknown>;
  enabled?: boolean;
}

// NOTE: named `WorkflowApproveRunReq` (not `ApproveRunReq`) to avoid colliding
// with the Run-with-Otto `ApproveRunReq` below — same per-module-vs-flat-file
// reason as WorkflowRunStatus.
export interface WorkflowApproveRunReq {
  /** Required for Product gates: version of the full pending node body actually displayed. */
  expected_detail_version?: string | null;
  node_id: string;
  approved: boolean;
  note?: string | null;
}

export interface NodeTypeSpec {
  kind: string;
  label: string;
  category: string;
  description: string;
  inputs: number;
  outputs: number;
  color: string;
  icon: string;
  /** JSON schema describing this node's output shape (for downstream wiring). */
  output_schema?: unknown;
  /** JSON schema describing this node's accepted params. */
  params_schema?: unknown;
}

/** An immutable snapshot of a workflow's graph (append-only version history). */
export interface WorkflowVersion {
  id: string;
  workflow_id: string;
  version: number;
  name: string;
  description: string;
  instructions: string;
  graph: WorkflowGraph;
  note: string;
  /** Restart policy snapshot (Rust `#[serde(default = "default_on_restart")]`, so always sent;
   *  pre-field rows read back as 'resume'). Restoring a version round-trips it. */
  on_restart: 'resume' | 'fail';
  created_by: string;
  created_at: string;
}

/** Metadata returned by a bounded workflow history request with summary=true. */
export type WorkflowVersionSummary = Pick<WorkflowVersion,
  'id' | 'workflow_id' | 'version' | 'note' | 'created_by' | 'created_at'>;

export interface CreateWorkflowReq {
  name: string;
  description?: string | null;
  instructions?: string | null;
  graph?: WorkflowGraph | null;
}

export interface UpdateWorkflowReq {
  name?: string | null;
  description?: string | null;
  instructions?: string | null;
  graph?: WorkflowGraph | null;
  /** Restart policy: 'resume' | 'fail' (see `Workflow.on_restart`); absent/null leaves it unchanged. */
  on_restart?: 'resume' | 'fail' | null;
}

export interface GenerateWorkflowReq {
  description: string;
  name?: string | null;
}
