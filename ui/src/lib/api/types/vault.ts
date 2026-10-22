// Contract mirror: vault. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

// ---------------------------------------------------------------------------
// Vault v3 — the docs home (mirror of crates/otto-vault/src/types.rs)
// ---------------------------------------------------------------------------

/** A registered vault: a local directory of markdown files (source of truth). */
export interface Vault {
  id: number;
  ws_id: string;
  name: string;
  root_path: string;
  okf: boolean;
  created_at: string;
  last_scan_at: string | null;
  scan_state: string; // idle | scanning | error: <msg>
  notes: number;
  links: number;
}

/** File-backed recovery records; hidden content is excluded from indexing. */
export interface VaultTrashEntry {
  id: string;
  original_path: string;
  stored_path: string;
  deleted_at: string;
  kind: 'file' | 'dir';
}
export interface VaultRevision {
  id: string;
  path: string;
  created_at: string;
  before_hash: string | null;
  after_hash: string;
  reason: string;
  committed: boolean;
}
export interface VaultRevisionDetail extends VaultRevision {
  before: string | null;
  after: string;
}

export interface VaultStatus {
  /** Opaque change token; unchanged scans keep it stable. */
  generation?: string | null;
  /** Graph-shape token: moves only when links/titles/tags/types or the note
   *  set change (a body-only save keeps it) — graph views refetch on it. */
  graph_generation?: string | null;
  id: number;
  scan_state: string;
  last_scan_at: string | null;
  notes: number;
  links: number;
  unresolved: number;
  tags: number;
  attachments: number;
  /** Recovery dirs (`.otto-history`, `.trash`) git already tracks — committed
   *  before Otto ignored them; untrack with `git rm --cached -r <dir>`. Omitted when none. */
  tracked_recovery?: string[];
}

export interface VaultDirEntry {
  name: string;
  path: string;
  kind: 'dir' | 'note' | 'file';
  children: number;
  title: string | null;
  okf_type: string | null;
  reserved: boolean;
}

export interface VaultDirListing {
  path: string;
  entries: VaultDirEntry[];
}

export interface VaultHeading {
  level: number;
  text: string;
  line: number;
}

export interface VaultNoteMeta {
  path: string;
  title: string;
  okf_type: string | null;
  description: string | null;
  frontmatter: unknown;
  tags: string[];
  aliases: string[];
  headings: VaultHeading[];
  word_count: number;
  size: number;
  hash: string;
  reserved: boolean;
  has_frontmatter: boolean;
  parse_error: boolean;
  content_index_status?: 'full' | 'size_limited';
}

export interface VaultOutgoingLink {
  raw_target: string;
  dst_path: string | null;
  kind: 'wiki' | 'md' | 'embed';
  anchor: string | null;
  alias: string | null;
}

export interface VaultNote {
  meta: VaultNoteMeta;
  raw: string;
  outgoing: VaultOutgoingLink[];
}

export interface VaultTextFile {
  path: string;
  size: number;
  hash: string;
}

export interface WriteTextFileReq {
  path: string;
  content: string;
  if_hash?: string;
}

export interface VaultBacklink {
  path: string;
  title: string;
  /** Empty outside the requested context window (default first 100 rows). */
  context: string;
  kind: string;
}

export interface VaultRenameResult {
  from: string;
  to: string;
  links_updated: number;
  /** Sources whose links could not be rewritten (the move still succeeded). */
  links_failed?: string[];
}

export interface VaultSearchReq {
  query: string;
  tag?: string;
  path_prefix?: string;
  okf_type?: string;
  limit?: number;
}

export interface VaultSearchHit {
  path: string;
  title: string;
  snippet: string;
  score: number;
  reserved: boolean;
}

export interface VaultSwitchHit {
  path: string;
  title: string;
  alias: string | null;
  score: number;
}

export interface VaultTagCount {
  tag: string;
  count: number;
}

/** Compact graph wire format: parallel node arrays + flat [src,dst,…] pairs.
 * flags bits: 1=ghost (unresolved target), 2=tag node, 4=reserved file. */
/**
 * Per-node attributes are indices into the parallel `*_labels` tables, so the
 * wire stays compact on a 9k-note vault. Tags are CSR-encoded: node `i` owns
 * `tag_ids[tag_off[i]..tag_off[i + 1]]`, and `tag_off` has `n + 1` entries.
 */
export interface VaultGraphPayload {
  paths: string[];
  titles: string[];
  /** Normalized OKF type per node (`Flow`/`flow` fold into one bucket). */
  types: number[];
  type_labels: string[];
  /** Top-level folder per node — the repo/service bundle in an OKF vault. */
  services: number[];
  service_labels: string[];
  tag_off: number[];
  tag_ids: number[];
  tag_labels: string[];
  flags: number[];
  edges: number[];
  truncated: boolean;
}

export interface OkfFinding {
  rule: string; // E1|E2|E3|W1|W2|W3|W4|W5
  path: string;
  message: string;
}

export interface OkfReport {
  conformant: boolean;
  errors: OkfFinding[];
  warnings: OkfFinding[];
  checked_notes: number;
}

// Docs agents — multi-writer documentation runs + per-note refine sessions.

export interface VaultDocsAgent {
  index: number;
  name: string; // "writer-1 · claude" / "refine · claude"
  provider: string;
  model: string | null;
  state: 'pending' | 'running' | 'done' | 'error' | 'interrupted' | 'cancelled';
  session_id: string | null;
  error: string | null;
  drafts: string[]; // vault-relative draft note paths this agent produced
}
export interface VaultDocsSummarizer {
  provider: string;
  model: string | null;
  // skipped when 1 writer; interrupted when a daemon restart killed the run
  state: 'pending' | 'running' | 'done' | 'error' | 'skipped' | 'interrupted' | 'cancelled';
  session_id: string | null;
  error: string | null;
}

export type VaultDocsReviewSkill =
  | 'vault-docs-review'
  | 'vault-api-review'
  | 'vault-data-review'
  | 'vault-runtime-review'
  | 'vault-evidence-review';

export interface VaultDocsFindingEvidence {
  repo_path: string | null;
  line: number | null;
  doc_path: string | null;
  section: string | null;
}

export interface VaultDocsFinding {
  severity: 'blocking' | 'major' | 'minor';
  category: string;
  summary: string;
  evidence: VaultDocsFindingEvidence[];
  missed_item: string;
  required_fix: string;
}

export interface VaultDocsReviewer {
  index: number;
  provider: string;
  model: string | null;
  skill: VaultDocsReviewSkill;
  focus: string | null;
  state: 'pending' | 'running' | 'done' | 'error' | 'cancelled' | 'interrupted';
  session_id: string | null;
  findings: VaultDocsFinding[];
  error: string | null;
}

export interface VaultDocsRevision {
  state: 'skipped' | 'pending' | 'running' | 'done' | 'error' | 'cancelled' | 'interrupted';
  session_id: string | null;
  changed_paths: string[];
  error: string | null;
}

export interface VaultDocsReviewRound {
  iteration: number;
  state:
    | 'reviewing'
    | 'revising'
    | 'revised'
    | 'clean'
    | 'exhausted'
    | 'error'
    | 'cancelled'
    | 'interrupted';
  reviewers: VaultDocsReviewer[];
  revision: VaultDocsRevision;
}

export interface VaultDocsReview {
  state:
    | 'skipped'
    | 'pending'
    | 'reviewing'
    | 'revising'
    | 'clean'
    | 'exhausted'
    | 'error'
    | 'cancelled'
    | 'interrupted';
  max_iterations: number;
  current_iteration: number;
  outcome: string | null;
  /** Immutable resolved reviewer templates; live state/findings are per round. */
  reviewers: VaultDocsReviewer[];
  rounds: VaultDocsReviewRound[];
}

export interface VaultDocsRun {
  id: string;
  ws_id: string;
  vault_id: number;
  kind: 'docs' | 'refine'; // refine = one per-note edit turn
  prompt: string;
  target_dir: string;
  note_path: string; // refine: the note being edited ("" for docs runs)
  state:
    | 'running'
    | 'summarizing'
    | 'reviewing'
    | 'revising'
    | 'done'
    | 'done_with_findings'
    | 'error'
    | 'cancelled'
    | 'interrupted';
  agents: VaultDocsAgent[];
  summarizer: VaultDocsSummarizer;
  review: VaultDocsReview;
  written: string[]; // final note paths in the vault
  error: string | null;
  started_at: string;
  finished_at: string | null;
}
