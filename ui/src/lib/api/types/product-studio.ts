// Contract mirror: product, part 2 — Design Arena, refinement / discovery chat
// threads and the Product↔Swarm closure types. modules/product/types.ts re-exports it.

import type { ProductStory, PublishAsRfcReq, PublishAsStoryReq } from './product';
import type { Swarm, SwarmMessage, SwarmProject, SwarmRun, SwarmTask } from './swarm';

// ---------------------------------------------------------------------------
// Design Arena (design/product-design-arena.md §2.2, §4.4)
// ---------------------------------------------------------------------------

/** The ONE design-format enum (Rust `DesignFormat`); unknown values are a 400. */
export type DesignFormat = 'html' | 'mermaid' | 'excalidraw' | 'scene3d';
export const DESIGN_FORMATS: readonly DesignFormat[] = ['html', 'mermaid', 'excalidraw', 'scene3d'];

/** Parsed `meta_json` of a `kind:'design'|'mockup'` attachment. */
export interface DesignMeta {
  format?: string;
  assist_session_id?: string;
  /** The arena's asset grouping (e.g. "Screens") — NOT the story `folder`. */
  group?: string;
  /** Set on Blender-render outputs: the scene3d attachment they came from. */
  derived_from?: string;
}

/** Body for `PUT /product/attachments/{aid}/content` → ProductAttachment.
 *  `base_updated_at` = the row's `updated_at` the editor last loaded/saved
 *  (optimistic concurrency): the server answers 409 when the row moved on. */
export interface SaveAttachmentContentReq {
  data_b64: string;
  base_updated_at?: string | null;
}

/** `GET /product/design/blender`. */
export interface BlenderStatus {
  installed: boolean;
  path: string | null;
  version: string | null;
}

/** `GET /product/design/jobs/{id}` — an in-memory (unpersisted) render job. */
export interface BlenderJob {
  id: string;
  attachment_id: string;
  status: 'queued' | 'running' | 'done' | 'error';
  error: string | null;
  /** New attachment ids (render.png + scene.glb). */
  outputs: string[];
  started_at: string;
  finished_at: string | null;
}

/** Body for `POST /product/stories/{sid}/mockups/assist` (in-place mockup agent). */
export interface ProductMockupAssistReq {
  prompt: string;
  /** `html` (default) | `mermaid` | `excalidraw` | `scene3d`. Only honored when
   *  creating a NEW artifact; unknown values are a 400. */
  format?: DesignFormat;
  /** Refine an existing agent mockup (resume its session); omit to create one. */
  mockup_id?: string;
  /** Agent provider (built-in or custom, e.g. `grok`). Honored only when a NEW
   *  mockup session is created; a refine resumes the existing one. */
  provider?: string;
  /** Optional model alias for the first turn. */
  model?: string;
}

export interface DiscoveryRun {
  id: string;
  story_id: string;
  workspace_id: string;
  swarm_id: string;
  project_id: string;
  status: string;
  brief_md: string;
  report_md: string | null;
  created_by: string;
  created_at: string;
  updated_at: string;
}

export interface DiscoveryRunSummary {
  run: DiscoveryRun;
  derived_status: string;
  task_count: number;
  done_count: number;
}

export interface DiscoveryRunDetail {
  run: DiscoveryRun;
  derived_status: string;
  tasks: SwarmTask[];
  task_summaries: [string, string | null][];
  messages: SwarmMessage[];
}

export interface DiscoverReq {
  swarm_id?: string | null;
  name?: string | null;
}

/** Response from `POST /product/stories/{sid}/discover`. */
export interface DiscoverResp {
  run: DiscoveryRun;
  swarm: Swarm;
  project: SwarmProject;
  tasks: SwarmTask[];
}

export interface MockupAnnotation {
  id: string;
  attachment_id: string;
  story_id: string;
  workspace_id: string;
  x_pct: number;
  y_pct: number;
  body: string;
  resolved: boolean;
  author_id: string;
  created_at: string;
  updated_at: string;
}

// ---------------------------------------------------------------------------
// ---------------------------------------------------------------------------
// Refinement thread types  (talk-to-agent, Phase C1)
// ---------------------------------------------------------------------------

export interface RefinementThread {
  id: string; story_id: string; workspace_id: string;
  discovery_run_id: string | null;
  cwd: string; title: string; status: string;
  model: string | null;
  created_by: string; created_at: string; updated_at: string;
}

export interface RefinementMessage {
  id: string; thread_id: string; role: string; body: string;
  meta_json: string | null; created_at: string;
}

export interface RefinementThreadDetail { thread: RefinementThread; messages: RefinementMessage[] }

export interface CreateThreadReq { discovery_run_id?: string | null; title?: string | null }

/** Body for `POST /product/refinement-threads/{tid}/messages`. */
export interface RefineSendReq {
  body: string;
  /** Agent provider for this turn (built-in or custom, e.g. `grok`). Each turn
   *  replays the full thread history, so a per-turn provider is self-contained. */
  provider?: string;
  /** Optional model alias for this turn. */
  model?: string;
}

export interface RefineTurnResp {
  user_message: RefinementMessage;
  agent_message: RefinementMessage;
  story_updated: boolean;
  version_no: number | null;
}

// ---------------------------------------------------------------------------
// Discovery Chat types  (talk-to-agent from an empty draft — early discovery)
// ---------------------------------------------------------------------------

export interface DiscoveryChat {
  id: string; story_id: string; workspace_id: string;
  cwd: string; title: string; status: string;
  model: string | null;
  /** The managed Otto session backing this chat (open/resume it in Agents). */
  session_id: string | null;
  created_by: string; created_at: string; updated_at: string;
}

export interface DiscoveryChatMessage {
  id: string; chat_id: string; role: string; body: string;
  /** Agent-proposed actions, as a JSON-array string (see DiscoveryAction). */
  actions_json: string | null;
  meta_json: string | null;
  created_at: string;
}

export interface DiscoveryChatDetail { chat: DiscoveryChat; messages: DiscoveryChatMessage[] }

export interface CreateDiscoveryChatReq { title?: string | null }

export interface DiscoveryChatTurn {
  user_message: DiscoveryChatMessage;
  agent_message: DiscoveryChatMessage;
}

/** A structured proposal the agent may emit (rendered as an Apply-able card). */
export type DiscoveryAction =
  | { type: 'apply_draft'; title: string; body_md: string }
  | { type: 'add_questions'; questions: { text: string; rationale?: string; category?: string }[] }
  | { type: 'add_notes'; notes: { body: string }[] }
  | { type: 'create_canvas'; title: string; mermaid?: string; nodes?: unknown[]; edges?: unknown[] };

export interface ApplyActionReq { action: DiscoveryAction }

export interface ApplyResult {
  story_updated: boolean;
  created_question_ids: string[];
  created_note_ids: string[];
  canvas_id: string | null;
}

// ---------------------------------------------------------------------------
// Product↔Swarm closure types  (GET /product/stories/{sid}/swarm)
// ---------------------------------------------------------------------------

/** Full swarm project view linked to a story via the Plan → Swarm hand-off.
 *  Returned by `GET /product/stories/{sid}/swarm`. */
export interface StorySwarmLink {
  /** The linked swarm project, null when no project exists for this story. */
  project: SwarmProject | null;
  tasks: SwarmTask[];
  runs: SwarmRun[];
  /** File paths / references collected from run result blobs (best-effort). */
  artifacts: string[];
  /** PR numbers / references (best-effort). */
  prs: string[];
  /** Review ids (best-effort). */
  reviews: string[];
  /** Accumulated cost in USD across all runs. */
  cost_usd: number;
}

/** Frozen dry-run workflow output, forwarded unchanged through human approval. */
export type ProductPublicationPreview = {
  story_id: string;
  body_md: string;
  account_label: string;
  account_url: string;
} & ({ kind: 'jira'; request: PublishAsStoryReq } | { kind: 'rfc'; request: PublishAsRfcReq });

// ---------------------------------------------------------------------------
// Swarm↔Product closure types  (GET /swarm/tasks/{tid}/story)
// ---------------------------------------------------------------------------

/** Back-link from a swarm task to the Product story that originated it.
 *  Returned by `GET /swarm/tasks/{tid}/story`. */
export interface TaskStoryLink {
  /** Source Product story, null when the task was not created from a story plan. */
  story: ProductStory | null;
  /** Acceptance criteria from the task description (convenience field). */
  acceptance: string | null;
}
