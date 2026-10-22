// Contract mirror: product (moved from modules/product/types.ts, which re-exports
// it together with ./product-studio). The barrel re-exports only the names
// lib/ code needs; feature code imports from modules/product/types.
//
// Product Story Analysis — TypeScript mirrors of the Rust domain structs
// and request/response DTOs from otto-state/src/product.rs and
// otto-product/src/types.rs.  All field names are EXACT snake_case.
// DateTime<Utc> → string (ISO-8601 wire format), Id → string, i64 → number,
// bool → boolean.  JSON-as-TEXT columns stay as string on the wire.

import type { Swarm, SwarmProject, SwarmTask } from './swarm';

// ---------------------------------------------------------------------------
// Domain structs (otto-state/src/product.rs)
// ---------------------------------------------------------------------------

export interface ProductStory {
  id: string;
  workspace_id: string;
  source_kind: string;
  account_id: string;
  source_key: string;
  title: string;
  url: string;
  issue_type: string | null;
  stage: string;
  cwd: string | null;
  watch_enabled: boolean;
  watch_cadence_min: number;
  watch_cursor: string | null;
  confluence_tests_page_id: string | null;
  confluence_tests_url: string | null;
  tags: string;
  /** Epic tree (design §2.1): the parent epic's id, or null at top level. One
   *  level only — a child never has children. */
  parent_id: string | null;
  /** Otto's TREE role — `'story' | 'epic' | 'doc'` — distinct from `source_kind`
   *  and Jira's `issue_type`. The UI also treats a row with children as an epic. */
  tree_kind: string;
  /** Folder inside the parent epic (`''` = unfiled). Unrelated to a design
   *  attachment's `meta.group`. */
  folder: string;
  created_by: string;
  created_at: string;
  updated_at: string;
}

/** `tree_kind` values (mirrors the CHECK-free TEXT column). */
export type TreeKind = 'story' | 'epic' | 'doc';

export interface ProductStoryVersion {
  id: string;
  story_id: string;
  version_no: number;
  kind: string;
  title: string;
  body_md: string;
  /** Full only from `GET /product/versions/{vid}`. In a version LIST (and the
   *  story detail's `source`) it is slim: `{"version": n}` or `null`. */
  raw_json: string | null;
  change_notes: string | null;
  created_by: string;
  created_at: string;
}

export interface ProductAnalysis {
  id: string;
  story_id: string;
  source_version_id: string | null;
  status: string;
  summary: string;
  created_by: string;
  created_at: string;
  finished_at: string | null;
}

export interface ProductAnalysisAgent {
  id: string;
  analysis_id: string;
  name: string;
  skill: string;
  provider: string;
  model: string;
  status: string;
  /** The spawned session id; null until the agent session opens. */
  session_id?: string | null;
  /** JSON-as-string TEXT column; parse as needed. */
  findings_json: string | null;
  error: string | null;
  started_at: string | null;
  finished_at: string | null;
  /** Auto-resumes after a daemon restart (capped by the orphan reaper). */
  resume_count: number;
}

export interface ProductQuestion {
  id: string;
  story_id: string;
  analysis_id: string | null;
  text: string;
  rationale: string;
  category: string;
  status: string;
  answer: string | null;
  posted_ref: string | null;
  created_by: string;
  created_at: string;
  updated_at: string;
}

export interface ProductNote {
  id: string;
  story_id: string;
  section: string | null;
  body: string;
  author_id: string;
  created_at: string;
  updated_at: string;
}

export interface ProductEvent {
  id: string;
  story_id: string;
  section: string;
  kind: string;
  summary: string;
  actor_id: string | null;
  /** JSON-as-string TEXT column. */
  meta_json: string | null;
  created_at: string;
}

export interface ProductTestcaseRun {
  id: string;
  story_id: string;
  status: string;
  confluence_page_id: string | null;
  confluence_url: string | null;
  created_by: string;
  created_at: string;
}

/** Parsed shape of the `steps_json` column. */
export interface TestcaseSteps {
  preconditions: string[];
  steps: string[];
  expected: string;
}

export interface ProductTestcase {
  id: string;
  run_id: string;
  story_id: string;
  title: string;
  category: string;
  priority: string;
  /** JSON-as-string TEXT column; parse into TestcaseSteps as needed. */
  steps_json: string;
  status: string;
  review_note: string | null;
  order_idx: number;
  created_at: string;
  updated_at: string;
}

export interface ProductLearning {
  id: string;
  workspace_id: string;
  kind: string;
  title: string;
  body: string;
  tags: string;
  /** JSON-as-string TEXT column. */
  refs_json: string;
  source_story_id: string | null;
  active: boolean;
  created_by: string;
  created_at: string;
  updated_at: string;
}

// ---------------------------------------------------------------------------
// Response DTOs (otto-product/src/types.rs)
// ---------------------------------------------------------------------------

export interface StoryCounts {
  versions: number;
  analyses: number;
  open_questions: number;
  notes: number;
  testcases: number;
}

/** Back-link to the swarm project created from this story (Plan → Swarm). */
export interface SwarmStoryLink {
  project_id: string;
  swarm_id: string;
  project_name: string;
}

export interface ProductStoryDetail {
  story: ProductStory;
  /** Latest editable draft revision for local drafts; latest source revision for imports. */
  source: ProductStoryVersion | null;
  counts: StoryCounts;
  /** The swarm project created from this story (Plan → Swarm), or null. */
  swarm_link: SwarmStoryLink | null;
}

/** Request body for `POST /product/stories/{sid}/to-swarm`. */
export interface ToSwarmReq {
  /** Target swarm; omit to use the first swarm or auto-create a default one. */
  swarm_id?: string | null;
  /** Override the new project's name (defaults to the story title). */
  name?: string | null;
}

/** Response from `to-swarm`: the swarm + created project + seeded tasks. */
export interface ToSwarmResp {
  swarm: Swarm;
  project: SwarmProject;
  tasks: SwarmTask[];
  /** True when this call created the project; false on idempotent re-send. */
  created: boolean;
}

export interface ProductAnalysisDetail {
  analysis: ProductAnalysis;
  agents: ProductAnalysisAgent[];
}

export interface ProductTestcaseRunDetail {
  run: ProductTestcaseRun;
  cases: ProductTestcase[];
}

export interface InjectSection {
  heading: string;
  body: string;
}

export interface InjectBundle {
  markdown: string;
  sections: InjectSection[];
}

// ---------------------------------------------------------------------------
// Request DTOs (otto-product/src/types.rs)
// ---------------------------------------------------------------------------

export interface ImportStoryReq {
  source_kind: string;
  account_id: string;
  source_key: string;
  cwd?: string | null;
  watch_enabled?: boolean | null;
}

export interface UpdateStoryReq {
  cwd?: string | null;
  stage?: string | null;
  watch_enabled?: boolean | null;
  watch_cadence_min?: number | null;
  tags?: string;
  /** Epic tree: `null` detaches from the epic (Rust `Option<Option<Id>>`);
   *  omit to leave untouched. The server rejects a parent that itself has a parent. */
  parent_id?: string | null;
  tree_kind?: TreeKind | null;
  folder?: string | null;
}

/** Body for `POST /product/stories/{sid}/children` → ProductStoryDetail. */
export interface CreateChildReq {
  title: string;
  tree_kind: TreeKind;
  folder: string;
}

export interface NewQuestionReq {
  text: string;
  rationale?: string | null;
  category?: string | null;
}

export interface UpdateQuestionReq {
  text?: string | null;
  rationale?: string | null;
  category?: string | null;
  status?: string | null;
  answer?: string | null;
}

export interface PostQuestionsReq {
  ids: string[];
  format?: string | null;
}

export interface NewNoteReq {
  body: string;
  section?: string | null;
}

export interface UpdateNoteReq {
  body: string;
}

export interface NewLearningReq {
  kind: string;
  title: string;
  body: string;
  tags?: string | null;
  refs?: unknown;
  source_story_id?: string | null;
}

export interface UpdateLearningReq {
  kind?: string | null;
  title?: string | null;
  body?: string | null;
  tags?: string | null;
  refs?: unknown;
  active?: boolean | null;
}

export interface UpdateTestcaseReq {
  title?: string | null;
  category?: string | null;
  priority?: string | null;
  steps?: unknown;
  status?: string | null;
  review_note?: string | null;
  order_idx?: number | null;
}

export interface PublishTestsReq {
  space_key?: string | null;
  parent_id?: string | null;
}

export interface AnalyzeAgentReq {
  skill: string;
  name?: string | null;
  providers: string[];
  model?: string | null;
}

export interface AnalyzeReq {
  agents?: AnalyzeAgentReq[];
  summarizer_provider?: string | null;
  cwd?: string | null;
  focus?: string | null;
}

export interface RewriteReq {
  provider?: string | null;
  model?: string | null;
  cwd?: string | null;
  focus?: string | null;
}

export interface GenerateTestsReq {
  provider?: string | null;
  model?: string | null;
  cwd?: string | null;
  focus?: string | null;
}

export interface GeneratePlanReq {
  /** Single-provider back-compat; used only when `providers` is empty. */
  provider?: string | null;
  /** Multi-agent: one planning session per entry (visible side-by-side). */
  providers?: string[];
  /** Provider for the consolidating summarizer (only when >1 planner). */
  summarizer_provider?: string | null;
  /** `false` (default) ⇒ agents run unattended and don't ask questions. */
  interactive?: boolean | null;
  model?: string | null;
  cwd?: string | null;
  focus?: string | null;
}

export interface SavePlanReq {
  body_md: string;
}

export interface InjectSessionReq {
  provider?: string | null;
  model?: string | null;
  cwd?: string | null;
}

// ---------------------------------------------------------------------------
// Jira IssueFull types (OverviewTab rich section)
// ---------------------------------------------------------------------------

export interface JiraUser {
  account_id: string;
  display_name: string;
  avatar_url?: string | null;
}

export interface JiraField {
  key: string;
  name: string;
  value: string;
}

export interface JiraAttachment {
  id: string;
  filename: string;
  mime: string;
  size: number;
  created: string;
  author: string;
}

export interface JiraLink {
  rel: string;
  key: string;
  summary: string;
  status: string;
  issue_type: string;
}

export interface JiraTransition {
  id: string;
  name: string;
  to_status: string;
}

/** One selectable value for an editable field (option / version / component / user). */
export interface FieldOption {
  id: string;
  label: string;
}

/** A field the caller may edit, as reported by Jira `editmeta`. */
export interface EditableField {
  key: string;
  name: string;
  schema_type: string;
  items?: string | null;
  allowed_values: FieldOption[];
  required: boolean;
}

export interface JiraChangeItem {
  field: string;
  from: string | null;
  to: string | null;
}

export interface JiraChangelogEntry {
  author: string;
  created: string;
  items: JiraChangeItem[];
}

export interface JiraComment {
  id: string;
  author: string;
  body_md: string;
  created: string;
}

export interface IssueFull {
  key: string;
  /** Numeric Jira issue id (needed by the dev-status API). */
  id: string;
  summary: string;
  status: string;
  issue_type: string;
  url: string;
  description_md: string | null;
  assignee?: JiraUser | null;
  reporter?: JiraUser | null;
  priority?: string | null;
  labels: string[];
  fields: JiraField[];
  comments: JiraComment[];
  history: JiraChangelogEntry[];
  attachments: JiraAttachment[];
  links: JiraLink[];
  estimate?: string | null;
}

// ---------------------------------------------------------------------------
// Development info (Jira dev-status: linked branches / commits / PRs)
// ---------------------------------------------------------------------------

export interface DevBranch {
  name: string;
  url: string;
  repo: string;
  last_commit?: string | null;
}
export interface DevCommit {
  id: string;
  message: string;
  url: string;
  author: string;
  timestamp: string;
  repo: string;
}
export interface DevPr {
  id: string;
  name: string;
  url: string;
  status: string;
  repo: string;
  last_update: string;
}
export interface DevStatus {
  branches: DevBranch[];
  commits: DevCommit[];
  pull_requests: DevPr[];
}

// ---------------------------------------------------------------------------
// Confluence source types (SourceSearch component)
// ---------------------------------------------------------------------------

export interface ConfluenceSpace { key: string; name: string }
export interface ConfluencePageSummary { id: string; title: string; space_key: string; url: string }

// ---------------------------------------------------------------------------
// Discovery-mode types (drafts, transcripts, publish)
// ---------------------------------------------------------------------------

export interface ProductTranscript {
  id: string;
  story_id: string;
  title: string;
  body: string;
  created_by: string;
  created_at: string;
}

export interface ProductTranscriptSummary extends Omit<ProductTranscript, 'body'> {
  body_bytes: number;
}
export interface ProductTranscriptPage {
  items: ProductTranscriptSummary[];
  next_cursor: string | null;
}
export interface ProductTranscriptSearchPage {
  items: (ProductTranscriptSummary & { match_count: number })[];
  next_cursor: string | null;
}

export interface NewDraftReq {
  title?: string | null;
}

export interface UpdateDraftReq {
  title: string;
  body_md: string;
}

export interface NewTranscriptReq {
  title?: string | null;
  body: string;
}

/** The source bytes/metadata displayed in the publication confirmation.
 * SHA-256 is lowercase hex over the exact UTF-8 body, including whitespace. */
export interface ReviewedContent {
  version_id: string | null;
  body_sha256: string;
  title: string;
  source_kind: string;
  url: string;
}

export interface PublishAsRfcReq {
  reviewed_account_url?: string | null;
  reviewed_content: ReviewedContent;
  account_id: string;
  space_key: string;
  parent_id?: string | null;
  title?: string | null;
}

export interface PublishAsStoryReq {
  reviewed_account_url?: string | null;
  reviewed_content: ReviewedContent;
  account_id: string;
  project_key: string;
  issue_type: string;
}

// ---------------------------------------------------------------------------
// Attachment + Discovery + Mockup Annotation types  (C1)
// ---------------------------------------------------------------------------

export interface ProductAttachment {
  id: string;
  story_id: string;
  workspace_id: string;
  filename: string;
  mime: string;
  size_bytes: number;
  sha256: string | null;
  storage_path: string;
  kind: string;
  source: string;
  /** Free-form JSON: for agent mockups, `{assist_session_id, format}`. */
  meta_json: string | null;
  created_by: string;
  created_at: string;
  updated_at: string;
}

export interface UploadAttachmentReq {
  filename: string;
  mime: string;
  kind?: string;
  data_b64: string;
}

/** Product discovery/refinement exact-turn Stop. See contracts/api.md. */
export interface ProductStopTurnReq { turn_id: string }
export interface ProductStopTurnResp { stopped: boolean }
/** Optional on Product conversational message requests; fresh for every send. */
export interface ProductTurnIdentity { turn_id?: string }
