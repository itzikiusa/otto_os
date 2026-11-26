// Contract mirror: workflows. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';
import type {
  ReviewMode,
} from './review';
import type {
  Attention,
  TranscriptDigest,
  TurnTokenUsage,
  WaitReason,
} from './agent-runtime';

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
  /** Where it came from (server-set): `goal`, or absent for an authored workflow. */
  origin?: string;
  /** `review:<id>` · `insights` · `goal_loop:<id>` · `mcp` (server-set). */
  origin_ref?: string;
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

// ---------------------------------------------------------------------------
// Run view: phases, people, threads and the run overview (api.md "Run view").
// Every field is optional on the wire; `unknown` / `other` values come from a
// newer daemon.
// ---------------------------------------------------------------------------

/** Where a RUNNING step is (absent once it settled). */
export type StepPhase =
  | 'queued' | 'waiting' | 'starting' | 'working' | 'needs_you' | 'finishing' | 'unknown';

export interface PhaseMark {
  phase: StepPhase;
  at: string;
  /** ≤ 200 chars ("attempt 2 of 3 — …", a hold kind, "canceled"). */
  note?: string;
}

/** A step's agent thread. `thread_id` is a PTY session id when `runner` is `pty`. */
export interface StepThread {
  thread_id: Id;
  runner: 'threads' | 'pty';
  provider: string;
  /** What actually ran. */
  model?: string;
  effort?: string;
  continued_from?: string;
  start_ordinal?: number;
  turns: number;
  usage: TurnTokenUsage;
  /** Absent on `/progress`; on the full node. */
  digest?: TranscriptDigest;
  closed_at?: string;
  /** Computed on read: false once retention deleted the thread. */
  available?: boolean;
}

export type ChildThreadStatus =
  | 'queued' | 'pending' | 'running' | 'success' | 'error' | 'skipped' | 'needs_you';

/** A reviewer, the scope agent, the summarizer, or a thread the step's agent opened. */
export interface ChildThread {
  id: string;
  thread_id?: Id;
  /** "Correctness · Codex". */
  label: string;
  provider: string;
  model?: string;
  effort?: string;
  status: ChildThreadStatus;
  started_at?: string;
  ended_at?: string;
  /** "12 findings". */
  note?: string;
  /** Absent on `/progress`; on the full node. */
  usage?: TurnTokenUsage;
}

export type DeliverableState = 'done' | 'partial' | 'failed' | 'pending_approval' | 'unknown';

/** Something a step put outside Otto. */
export interface Deliverable {
  label: string;
  state: DeliverableState;
  detail?: string;
  url?: string;
}

/** What a person said to a step; kept on the node (≤ 20). */
export interface HumanEntry {
  kind: 'answer' | 'verify' | 'steer';
  ask?: string;
  /** ≤ 2 KiB. */
  text: string;
  met?: boolean;
  revision?: string;
  iteration?: number;
  by: string;
  at: string;
}

/** The run state shown to people — computed, never a stored status. */
export type RunState =
  | 'queued' | 'running' | 'waiting' | 'needs_you' | 'succeeded' | 'failed' | 'canceled'
  | 'paused' | 'blocked' | 'exhausted' | 'unknown';

/** A step waiting on a person: the attention's fields plus the step. */
export interface AttentionRef extends Attention {
  node_id: string;
  name: string;
}

export interface FanProgress {
  done: number;
  total: number;
  running: number;
}

export interface RunNow {
  node_ids: string[];
  node_names: string[];
  phase: StepPhase;
  wait_text?: string;
  fan?: FanProgress;
}

export interface QueueInfo {
  /** 1-based, within the lane. */
  position: number;
  ahead: number;
  cap: number;
  lane?: 'normal' | 'long';
}

export interface FailedStep {
  node_id: string;
  name: string;
  error: string;
  error_class?: string;
}

/** Where the run's wall clock went; the buckets sum to it. */
export interface TimeBuckets {
  queued_ms: number;
  waiting_ms: number;
  with_you_ms: number;
  working_ms: number;
}

export interface RunOrigin {
  kind: 'chat' | 'manual' | 'schedule' | 'webhook' | 'event' | 'task' | 'mcp' | 'goal';
  channel_name?: string;
  user_name?: string;
  thread_url?: string;
}

export interface RunSubject {
  pr?: { repo: string; number: number; url: string; title?: string };
  ticket?: { key: string; url: string; title?: string };
  repo?: string;
  branch?: string;
}

/** A goal run's progress. */
export interface GoalProgress {
  iteration: number;
  max_iterations: number;
  criteria_met: number;
  criteria_total: number;
  active_ms: number;
  max_active_ms?: number;
  /** Criterion ids parked for a person. */
  awaiting?: string[];
}

export type HoldKind = 'paused' | 'blocked' | 'exhausted' | 'unknown';

/** A held run (paused = status canceled, blocked/exhausted = status error); a hold
 *  whose kind does not match the status is stale and never sent. */
export interface RunHold {
  kind: HoldKind;
  reason: string;
  since: string;
  by?: Id;
  node_ids: string[];
  iteration?: number;
}

export interface LoopLimits {
  max_iterations?: number;
  max_active_s?: number;
  unblocked_at?: number;
}

/** Run-level loop limits raised by Resume / Extend. */
export interface RunLimits {
  loops: Record<string, LoopLimits>;
}

/** The run's one computed summary (never `summary`, which is the `/progress` marker). */
export interface RunOverview {
  rev: number;
  state: RunState;
  /** One sentence, ≤ 160 chars. */
  headline: string;
  steps_done: number;
  steps_total: number;
  now?: RunNow;
  /** Oldest first. */
  attention: AttentionRef[];
  queue?: QueueInfo;
  failed_step?: FailedStep;
  deliverables: Deliverable[];
  usage: TurnTokenUsage;
  time: TimeBuckets;
  origin?: RunOrigin;
  subject?: RunSubject;
  elapsed_ms: number;
  typical_ms?: number;
  goal?: GoalProgress;
  hold?: RunHold;
}

/** How a criterion is checked; the UI calls `expr` "Rule". */
export type VerifyKind = 'command' | 'agent' | 'human' | 'expr' | 'unknown';

export interface Criterion {
  id: string;
  text: string;
  verify: VerifyKind;
  /** Agent / human: what to inspect. */
  how: string;
  command?: string;
  expr?: string;
  timeout_s?: number;
}

export interface CriterionResult {
  id: string;
  text: string;
  verify: VerifyKind;
  met: boolean;
  evidence: string;
  /** `command` · `rule` · `agent` · the person's name. */
  verified_by: string;
}

export type CheckVerdict = 'achieved' | 'continue' | 'blocked' | 'unknown';

/** A `check` step's output. */
export interface CheckOutput {
  criteria: CriterionResult[];
  all_met: boolean;
  met: number;
  total: number;
  progress_pct: number;
  verdict: CheckVerdict;
  feedback: string;
  feedback_md: string;
  repeat_signature: string;
  repeated_failures: number;
  review?: { approved: boolean; findings: string[] };
}

/** A file sent with the chat message that started or steered a run. */
export interface AttachmentRef {
  name: string;
  mime: string;
  size_bytes?: number;
  /** Context-relative (`attachments/<name>`); absent when the copy failed. */
  path?: string;
  error?: string;
  source: 'start' | 'steer';
}

// Goals (otto-core api/goals.rs).

export type GoalWork = { kind: 'build'; repo_id: Id; base?: string | null } | { kind: 'research' };

export interface GoalBudget {
  /** 1..=50, default 5. */
  iterations: number;
  /** 900..=28,800, default 3,600. */
  active_s: number;
  /** 300..=7,200, default 1,200. */
  step_timeout_s: number;
}

/** Empty = the run default. */
export interface AgentChoice {
  provider?: string;
  model?: string;
  effort?: string;
  instructions?: string;
}

export interface NamedAgentChoice extends AgentChoice {
  name: string;
}

export interface GoalSpec {
  title: string;
  goal: string;
  objectives?: string[];
  constraints?: string[];
  out_of_scope?: string[];
  criteria: Criterion[];
  work: GoalWork;
  budget?: GoalBudget;
  worker?: AgentChoice;
  extra_workers?: NamedAgentChoice[];
  checker?: AgentChoice;
  final_review?: boolean;
  links?: string[];
  skills?: string[];
  continue_loop_id?: Id;
  from?: 'insights';
}

export interface GoalStartResp {
  workflow: Workflow;
  run: WorkflowRun;
}

/** New totals for the loop that ran out. */
export interface ExtendReq {
  iterations?: number;
  active_s?: number;
}

export interface ResumeReq {
  extend?: ExtendReq;
}

export interface SteerReq {
  note: string;
}

export interface VerifyCriterionReq {
  met: boolean;
  evidence: string;
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
  /** The stable N of `stepN-…`. */
  step_no?: number;
  /** `step3-checkout-pr-branch` (no extension). */
  step_file?: string;
  /** Only while running. */
  phase?: StepPhase;
  /** ≤ 40 (`/progress`: the newest 12). */
  phases?: PhaseMark[];
  wait?: WaitReason;
  attention?: Attention;
  /** `usage_limit` · `rate_limit` · `auth` · `provider` · `stalled` · `timeout` · `spawn` ·
   *  `server_restart` · `canceled` · `script` · `outcome_unknown`. */
  error_class?: string;
  thread?: StepThread;
  children?: ChildThread[];
  deliverables?: Deliverable[];
  /** ≤ 20 (`/progress`: the newest 5). */
  human_log?: HumanEntry[];
  /** One-line summary, ≤ 200 chars. */
  brief?: string;
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
  /** How many times the startup reconciler resumed this run after a daemon
   *  restart (0/absent = never interrupted). */
  resume_attempts?: number;
  /** Absolute path of the run's context directory (instruction brief,
   *  repos.json, per-step handoff files). Present on `GET /workflow-runs/{id}`
   *  when the directory exists on disk; absent on list endpoints. Read its files
   *  with `browseRunContext` / `readRunContext` (`lib/api/workflows.ts`), which
   *  take paths RELATIVE to it — `/fs/*` refuses Otto's data dir to non-root users. */
  context_dir?: string | null;
  /** The computed run summary; absent (null on `/progress`) when the stored one is stale. */
  overview?: RunOverview | null;
  /** Set while the run is held. */
  hold?: RunHold | null;
  limits?: RunLimits;
  /** Steering notes not yet taken by an agent step. */
  steering_queued?: string[];
  /** Files sent with the starting and steering chat messages. */
  attachments?: AttachmentRef[];
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
  /** The attempt's time minus its time on a person (omitted at 0). */
  active_ms?: number;
  /** The inner step's thread and the turn ordinal its prompt started at. */
  thread_id?: Id;
  start_ordinal?: number;
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
/** One run-history row (`GET /workspaces/{wid}/workflow-runs`) — no bodies. */
export interface RunListItem {
  id: Id;
  workflow_id: Id;
  workflow_name: string;
  status: WorkflowRunStatus;
  started_at: string;
  finished_at?: string | null;
  duration_ms?: number | null;
  /** `manual`, `chat` (Slack/Telegram), `schedule`, `event`, `webhook`… */
  trigger: string;
  /** The ask: input `msg`, else `prompt` (≤ 160 chars). */
  message?: string | null;
  repo?: string | null;
  branch?: string | null;
  pr?: string | null;
  /** The running step's name (running runs only). */
  current_step?: string | null;
  error?: string | null;
  overview?: RunOverview;
}

/** A page of run history, newest first. */
export interface RunListPage {
  items: RunListItem[];
  next_cursor?: string | null;
}

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
  /** The step the run is on (first running node, else the next pending one
   *  once something ran) and its display name. Absent before the first step. */
  current_node_id?: string | null;
  current_step?: string | null;
  overview?: RunOverview;
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
  /** When the trigger last fired or was refused. Null = never fired. */
  last_fired_at?: string | null;
  /** What that attempt came to. */
  last_outcome?: TriggerOutcome | null;
  /** The short reason of a `rejected` attempt. */
  last_detail?: string | null;
}

/** Outcome of a trigger's latest attempt: a run started, a run queued behind
 *  the parallel-run cap, or no run (`last_detail` says why). */
export type TriggerOutcome = 'started' | 'queued' | 'rejected';

/** `GET /workspaces/{wid}/workflow-runs/queue` — the daemon-wide parallel-run
 *  cap and this workspace's queued runs with their place in the (global, FIFO)
 *  line; 1 = next to start. */
export interface WorkflowRunQueue {
  max_parallel_runs: number;
  queued: { run_id: Id; position: number }[];
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
  /** The `version` the edit started from; a newer stored version answers 409 and saves nothing. */
  expected_version?: number | null;
}

export interface GenerateWorkflowReq {
  description: string;
  name?: string | null;
}
