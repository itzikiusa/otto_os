// Contract mirror: memory. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

// ---------------------------------------------------------------------------
// Memory layer (TS mirror of crates/otto-memory + otto_state::memory)
// ---------------------------------------------------------------------------

export type MemoryScope = 'workspace' | 'story' | 'entity';
export type MemorySearchMode = 'hybrid' | 'semantic' | 'keyword';

export interface MemoryRef {
  kind: string;
  ref: string;
  url: string | null;
  label: string | null;
}

export interface Memory {
  id: string;
  workspace_id: string;
  collection: string;
  record_type: string;
  scope: MemoryScope;
  story_id: string | null;
  kind: string;
  title: string;
  body: string;
  entities: string[];
  tags: string[];
  source_kind: string;
  source_ref: string | null;
  refs: MemoryRef[];
  confidence: number;
  salience: number;
  content_hash: string;
  active: boolean;
  superseded_by: string | null;
  version: number;
  created_by: string;
  created_at: string;
  updated_at: string;
  last_accessed_at: string | null;
  access_count: number;
  expires_at: string | null;
  /** 'shared' (all workspace members) or 'private' (creator-only). */
  visibility: string;
  /** Lifecycle state (0056_memory_lifecycle.sql). */
  state: 'suggested' | 'accepted' | 'stale' | 'contradicted';
  /** JSON provenance record — op + source ids (merge/split/import). */
  provenance_json: string | null;
  /** Unix epoch seconds when soft-deleted; null = live. */
  forgotten_at: number | null;
}

export interface NewMemory {
  collection?: string;
  record_type?: string;
  scope: MemoryScope;
  story_id?: string | null;
  kind: string;
  title: string;
  body: string;
  entities?: string[];
  tags?: string[];
  source_kind: string;
  source_ref?: string | null;
  refs?: MemoryRef[];
  confidence?: number | null;
  salience?: number | null;
  /** 'shared' (default) or 'private'. */
  visibility?: string;
}

export interface MemoryPatch {
  title?: string | null;
  body?: string | null;
  tags?: string[] | null;
  entities?: string[] | null;
  confidence?: number | null;
  salience?: number | null;
  active?: boolean | null;
}

export interface MemoryQuery {
  text?: string;
  collection?: string;
  scope?: MemoryScope;
  story_id?: string;
  kinds?: string[];
  tags?: string[];
  entities?: string[];
  /** Zero/omitted defaults to 20; capped at 200 by the service. */
  k?: number;
  mode?: MemorySearchMode;
  include_inactive?: boolean;
  recency_half_life_days?: number;
}

/** Structured "why this was selected" reason (Vault v2 explainability). */
export interface ContextReason {
  kind: string; // vector|keyword|symbol|graph|recent|test|doc|scope|hybrid
  detail: string;
  score: number;
}

export interface MemoryHit {
  memory: Memory;
  score: number;
  why: string[];
  reasons?: ContextReason[];
}

export interface BriefSection {
  heading: string;
  body_md: string;
  refs: MemoryRef[];
}

export interface RecallBrief {
  story_id: string;
  sections: BriefSection[];
  token_estimate: number;
  used: string[];
}

export interface MemoryLink {
  src_id: string;
  dst_id: string;
  rel: string;
  weight: number;
  certainty: string | null;
}

export interface MemoryGraphNode {
  id: string;
  label: string;
  kind: string;
  collection: string;
}

export interface MemoryGraphData {
  nodes: MemoryGraphNode[];
  edges: MemoryLink[];
}

export interface MemoryImportStats {
  nodes: number;
  edges: number;
}

export interface MemoryEntityGraph {
  links: MemoryLink[];
  neighbors: Memory[];
}

export interface IngestTextReq {
  collection?: string;
  path: string;
  content: string;
}
