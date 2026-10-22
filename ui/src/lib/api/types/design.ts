// Contract mirror: design. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// Design Hall — the artifact graph (mirrors crates/otto-design/src/types.rs;
// contract: docs/contracts/api.md § Design Hall, ws.md design_* events).
// ---------------------------------------------------------------------------

export type DesignStudio =
  | 'frames'
  | 'graphics'
  | 'site'
  | '3d'
  | 'whiteboard'
  | 'brand'
  | 'spatial';

/** Stored artifact formats (`otto-site`/`-layout`/`-brand`/`-exhibit` are Phase 1+). */
export type DesignArtifactFormat =
  | 'html'
  | 'mermaid'
  | 'd2'
  | 'excalidraw'
  | 'scene3d'
  | 'otto-canvas'
  | 'otto-site'
  | 'otto-layout'
  | 'otto-brand'
  | 'otto-exhibit'
  | 'svg'
  | 'png'
  | 'jpeg'
  | 'gif'
  | 'webp'
  | 'pdf'
  | 'glb'
  | 'gltf';

export type DesignStatus = 'draft' | 'review' | 'approved' | 'shipped' | 'archived';
export type DesignAuthorKind = 'user' | 'agent' | 'system';
export type DesignVersionKind = 'autosave' | 'named' | 'agent' | 'import' | 'sync' | 'restore';
export type DesignLinkRel =
  | 'embeds'
  | 'uses_component'
  | 'uses_tokens'
  | 'describes'
  | 'derived_from'
  | 'references'
  | 'implements'
  | 'variant_of'
  | 'resized_from'
  | 'published_as'
  | 'exported_to'
  | 'created_in';
export type DesignLinkDstKind =
  | 'artifact'
  | 'story'
  | 'session'
  | 'swarm_project'
  | 'vault_note'
  | 'pr'
  | 'url'
  | 'attachment'
  | 'publish';
export type DesignLinkPolicy = 'follow_approved' | 'follow_latest' | 'pinned';
export type DesignSignalKind =
  | 'variant_chosen'
  /** Server-recorded when `POST …/variants/{v}/accept` fast-forwards main. */
  | 'variant_accepted'
  | 'variant_rejected'
  /** A design-assist turn committed a version (main or a variant branch). */
  | 'agent_draft'
  | 'edit_after_draft'
  | 'review_comment'
  | 'critique_finding'
  | 'a11y_fix'
  | 'brand_correction'
  | 'rule_feedback'
  | 'status_change'
  | 'shipped'
  /** An older version saved again as the new head — `version_id` = the new
   *  head, `payload.from_version_id` (required) = the version restored. */
  | 'restored'
  /** A library artifact added as a reference — `payload.target_artifact_id`
   *  (required). */
  | 'reference_added'
  /** "Start from this": recorded on the NEW artifact, `payload.source_artifact_id`
   *  (required; `source_version_id` optional). */
  | 'forked';
export type DesignArtifactChange =
  | 'created'
  | 'content'
  | 'meta'
  | 'approved'
  | 'archived'
  | 'deleted'
  /** An UNCOMMITTED, validated mid-turn edit by a design-assist agent
   *  (`version_id: null`); the turn's commit follows as `content`. */
  | 'live'
  /** A new rendered thumbnail was stored (`PUT …/thumbnail`) — refresh the
   *  image only (not an edit; never re-render in response). */
  | 'thumbnail';
export type DesignLinkUpdateReason =
  | 'created'
  | 'deleted'
  | 'extracted'
  | 'target_approved'
  | 'target_updated'
  | 'target_deleted';

export interface DesignProject {
  id: Id;
  workspace_id: Id;
  name: string;
  description: string;
  epic_story_id: Id | null;
  swarm_project_id: Id | null;
  brand_kit_id: Id | null;
  cover_artifact_id: Id | null;
  archived: boolean;
  meta: Record<string, unknown>;
  created_by: Id;
  created_at: string;
  updated_at: string;
  /** Non-archived artifacts filed in this project. */
  artifact_count: number;
}

export interface DesignArtifact {
  id: Id;
  /** `null` = unfiled (e.g. freshly imported legacy rows). */
  project_id: Id | null;
  workspace_id: Id;
  studio: DesignStudio;
  format: DesignArtifactFormat | string;
  mime: string;
  title: string;
  status: DesignStatus;
  head_version_id: Id | null;
  /** `seq` of the head version (the "v12" badge). */
  head_seq: number | null;
  approved_version_id: Id | null;
  tags: string[];
  /** sha256 of the thumbnail blob — PNG or WebP (`GET …/thumbnail` serves
   *  it; `PUT …/thumbnail` stores a UI-rendered one). */
  thumb_blob: string | null;
  /** Imported rows carry `meta.imported_from = {kind, id, story_id?, …}`. */
  meta: Record<string, unknown>;
  source_kind: 'product_attachment' | 'canvas_scene' | null;
  source_id: Id | null;
  created_by: Id;
  created_by_kind: DesignAuthorKind;
  created_session_id: Id | null;
  created_at: string;
  updated_at: string;
  // Resolved on read (never written):
  /** Display name of `created_by` (display name, else username); `null` for
   *  a system author such as the legacy import. */
  created_by_name: string | null;
  /** `author_id` / `author_kind` of the head version — who saved last. */
  last_editor_id: string | null;
  last_editor_kind: DesignAuthorKind | null;
  /** Display name of `last_editor_id` (`null` when it is not a user). */
  last_editor_name: string | null;
  /** Product stories this artifact is linked to (`implements`), sorted. */
  story_ids: Id[];
  /** Title of the `created_session_id` session (`null`: unset or gone). */
  created_session_title: string | null;
}

export interface DesignVersion {
  id: Id;
  artifact_id: Id;
  seq: number;
  parent_version_id: Id | null;
  branch: string;
  blob_sha256: string;
  size_bytes: number;
  kind: DesignVersionKind;
  author_kind: DesignAuthorKind;
  author_id: string;
  session_id: Id | null;
  message: string;
  provenance: Record<string, unknown>;
  created_at: string;
  /** Display name of `author_id`, resolved on read (`null`: not a user). */
  author_name: string | null;
}

export interface DesignLink {
  id: Id;
  src_artifact_id: Id;
  src_version_id: Id | null;
  src_node: string | null;
  dst_kind: DesignLinkDstKind;
  dst_id: string;
  dst_node: string | null;
  rel: DesignLinkRel;
  policy: DesignLinkPolicy;
  pinned_version_id: Id | null;
  origin: 'explicit' | 'extracted';
  /** Dangling target (missing artifact / version / node) — show a badge. */
  broken: boolean;
  meta: Record<string, unknown>;
  created_by: string;
  created_at: string;
}

export interface DesignSignal {
  id: Id;
  workspace_id: Id;
  artifact_id: Id;
  version_id: Id | null;
  kind: DesignSignalKind;
  actor_kind: DesignAuthorKind;
  actor_id: string;
  session_id: Id | null;
  /** Bounded summary (≤ 8 KB) — never raw document content. */
  payload: Record<string, unknown>;
  created_at: string;
}

/** `GET /design/artifacts/{id}` (`?content=true[&version=v3]` inlines the source). */
export interface DesignArtifactDetail {
  artifact: DesignArtifact;
  head: DesignVersion | null;
  approved: DesignVersion | null;
  links_out: number;
  links_in: number;
  /** Editable working copy on disk (text formats only). */
  work_path: string | null;
  thumbnail_path: string | null;
  content: string | null;
  content_version_id: Id | null;
  content_truncated: boolean;
}

export interface DesignBrokenRef {
  uri: string;
  src_node: string | null;
  reason: 'missing_artifact' | 'missing_version' | 'missing_node' | 'malformed';
}

export interface DesignLinkReport {
  extracted: number;
  broken: DesignBrokenRef[];
  /** Render links NOT stored because they would close a cycle. */
  cycles: string[];
  /** Render chains deeper than 4 (stored; rendering stops at depth 4). */
  depth_exceeded: string[];
}

/** Result of create / content save / named commit. */
export interface DesignSaveResult {
  artifact: DesignArtifact;
  version: DesignVersion;
  /** `false` when the bytes equalled the head (no new version written). */
  created: boolean;
  links: DesignLinkReport;
}

export interface DesignLinksResp {
  links: DesignLink[];
  /** The artifacts on the other end that the caller may view. */
  artifacts: DesignArtifact[];
}

export interface DesignSearchHit {
  artifact: DesignArtifact;
  snippet: string;
  score: number;
  reference_count: number;
  story_ids: Id[];
}

export interface DesignImportReport {
  attachments_scanned: number;
  scenes_scanned: number;
  created: number;
  synced: number;
  unchanged: number;
  skipped: number;
  links_created: number;
  errors: string[];
}

export interface DesignPruneReport {
  applied: boolean;
  artifacts_scanned: number;
  versions: Id[];
  blobs: string[];
  /** Bytes the removed blobs held (dry run: would free). */
  reclaimable_bytes: number;
  reclaimable_blobs: number;
}

/** One daily scheduled retention pass (`GET /design/admin/storage`). */
export interface DesignScheduledPruneRun {
  at: string;
  versions_removed: number;
  blobs_removed: number;
}

/** `GET /design/admin/storage` — the Design Hall blob-store gauge. */
export interface DesignStorageReport {
  /** Files / bytes on disk under `<data>/design/blobs` (after dedupe). */
  blob_count: number;
  blob_bytes: number;
  /** Version rows and the bytes they reference (before dedupe). */
  version_count: number;
  version_bytes: number;
  /** Auto-tidy is in effect: the Settings toggle, unless the env override forces it. */
  auto_prune: boolean;
  /** The persisted Settings → Design Hall toggle (default false — opt-in). */
  auto_tidy: boolean;
  /** `OTTO_DESIGN_AUTO_PRUNE` override when set (`false` = hard off, beats the toggle). */
  auto_tidy_forced: boolean | null;
  /** Only autosaves older than this are candidates (default 7 days). */
  min_age_secs: number;
  /** What a pass would reclaim right now (dry run). */
  reclaimable: { versions: number; blobs: number; bytes: number };
  last_prune: DesignScheduledPruneRun | null;
}

/** `PUT /design/admin/auto-tidy` — returns the refreshed `DesignStorageReport`. */
export interface DesignAutoTidyReq {
  enabled: boolean;
}

/** `POST /design/artifacts/{id}/versions/{v}/restore`. */
export interface DesignRestoreReq {
  base_version?: Id;
  message?: string;
}

export interface CreateDesignProjectReq {
  workspace_id: Id;
  name: string;
  description?: string;
  epic_story_id?: Id;
  swarm_project_id?: Id;
  brand_kit_id?: Id;
  meta?: Record<string, unknown>;
}

/** `""` clears an optional id; omitted fields are unchanged. */
export interface UpdateDesignProjectReq {
  name?: string;
  description?: string;
  epic_story_id?: string;
  swarm_project_id?: string;
  brand_kit_id?: string;
  cover_artifact_id?: string;
  archived?: boolean;
  meta?: Record<string, unknown>;
}

export interface CreateDesignArtifactReq {
  workspace_id: Id;
  project_id?: Id;
  studio?: DesignStudio;
  format: DesignArtifactFormat;
  title: string;
  tags?: string[];
  meta?: Record<string, unknown>;
  /** UTF-8 source (text formats) — or `content_b64` for any format. */
  content?: string;
  content_b64?: string;
  /** Link to a product story (`implements`). */
  story_id?: Id;
  /** Fork (`derived_from`, pinned); omitted version → approved, else head. */
  derived_from?: { artifact_id: Id; version_id?: Id };
  author_kind?: 'user' | 'agent';
  session_id?: Id;
  message?: string;
}

export interface UpdateDesignArtifactReq {
  title?: string;
  /** `""` unfiles the artifact. */
  project_id?: string;
  studio?: DesignStudio;
  status?: DesignStatus;
  tags?: string[];
  meta?: Record<string, unknown>;
  /** PNG thumbnail, base64, ≤ 2 MB. */
  thumb_b64?: string;
}

export interface DesignContentPutReq {
  content?: string;
  content_b64?: string;
  /** The head version id the editor loaded (`""` = none yet); mismatch → 409. */
  base_version?: string;
  message?: string;
  author_kind?: 'user' | 'agent';
  session_id?: Id;
  provenance?: Record<string, unknown>;
}

/** `POST /design/artifacts/{id}/versions` — without content it snapshots the working copy. */
export interface DesignCommitReq {
  message: string;
  content?: string;
  content_b64?: string;
  base_version?: string;
  author_kind?: 'user' | 'agent';
  session_id?: Id;
  provenance?: Record<string, unknown>;
}

export interface CreateDesignLinkReq {
  rel: DesignLinkRel;
  dst_kind: DesignLinkDstKind;
  dst_id: string;
  dst_node?: string;
  src_node?: string;
  policy?: DesignLinkPolicy;
  pinned_version_id?: Id;
  meta?: Record<string, unknown>;
}

export interface DesignSignalReq {
  artifact_id: Id;
  kind: DesignSignalKind;
  version_id?: Id;
  actor_kind?: DesignAuthorKind;
  session_id?: Id;
  /** Bounded JSON object (≤ 8 KB, ≤ 8 levels). */
  payload?: Record<string, unknown>;
}

export interface DesignPruneReq {
  artifact_id?: Id;
  /** Dry run unless true. */
  apply?: boolean;
  window_secs?: number;
}
