// Contract mirror: core. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

export type Id = string;

// ---------------------------------------------------------------------------
// Domain
// ---------------------------------------------------------------------------

export type GlobalRole = 'root' | 'member';
export type WorkspaceRole = 'viewer' | 'editor' | 'admin';

export interface User {
  id: Id;
  username: string;
  display_name: string;
  is_root: boolean;
  disabled: boolean;
  created_at: string;
}

/** API-client preferences under `Workspace.settings.api_client` (written by
 *  `PATCH /workspaces/{id}` `settings_patch`; read by the daemon's history
 *  pruning and private-target guard). Every key is optional. */
export interface ApiClientWsSettings {
  /** Allow requests to localhost / private networks (off by default). */
  allow_local?: boolean;
  /** History retention; 0 or absent = no limit (opt-in pruning). */
  history_max_rows?: number;
  history_max_days?: number;
  /** Newest finished runs kept per automation; 0 or absent = keep all. */
  automation_runs_keep?: number;
}

/** The workspace settings document. The Rust side stores a free JSON object
 *  (docs/contracts/api.md "Workspace settings"); these are the keys the UI
 *  reads and writes. Unknown keys round-trip untouched. `context` and the
 *  context fields are written only through `PUT /workspaces/{id}/context`. */
export interface WorkspaceSettings {
  /** Per-workspace notes (Info panel). */
  notes?: string;
  /** Default agent CLI for new sessions; '' / absent = the global default. */
  default_provider?: string;
  api_client?: ApiClientWsSettings;
  [key: string]: unknown;
}

export interface Workspace {
  id: Id;
  name: string;
  root_path: string;
  settings: WorkspaceSettings;
  archived: boolean;
  created_at: string;
}

export type SessionKind = 'agent' | 'connection';
export type SessionStatus = 'running' | 'working' | 'idle' | 'exited' | 'reconnectable';

// ---------------------------------------------------------------------------
// Provider updates
// ---------------------------------------------------------------------------

export interface UpdateProvidersReq {
  /** When set, only update that specific provider; omit to update all. */
  provider?: string | null;
}

// ---------------------------------------------------------------------------
// Filesystem (GET /fs/read)
// ---------------------------------------------------------------------------

/** Bounded regular-file read using daemon OS account permissions; endpoint token scopes still apply. */
export interface FsRead {
  path: string;
  content: string;
  language: string;
  truncated: boolean;
}

// ---------------------------------------------------------------------------
// LSP capabilities (GET /lsp/capabilities)
// ---------------------------------------------------------------------------

export interface LspServerStatus {
  lang: string;
  available: boolean;
  command: string;
  install_command: string | null;
}

export interface LspCapabilities {
  servers: LspServerStatus[];
}

// ---------------------------------------------------------------------------
// Email sender (Gmail App Password → Keychain; mobile plan Task 7.1).
// Mirrors crates/otto-core/src/api.rs SetEmailSenderReq / EmailSenderResp.
// ---------------------------------------------------------------------------

/** `PUT /api/v1/email-sender` request body. The app password is stored in the
 *  macOS Keychain (never the DB) and validated via a real Gmail SMTP login. */
export interface SetEmailSenderReq {
  /** The Gmail address mail is sent from (also the SMTP AUTH username). */
  gmail_address: string;
  /** The 16-char Gmail App Password. Never echoed back nor stored in the DB. */
  app_password: string;
}

/** Response for `PUT` / `GET /api/v1/email-sender` and
 *  `POST /api/v1/email-sender/verify` (re-check with the Keychain password). Never carries the app
 *  password. `gmail_address` is absent on GET when no sender is configured. */
export interface EmailSenderResp {
  /** The configured Gmail address, or absent when no sender is set up. */
  gmail_address?: string;
  /** `true` once the app password passed a real Gmail SMTP login. */
  verified: boolean;
}

// ---------------------------------------------------------------------------
// Context-packet (B2a) — send API/DB/broker payloads to agent sessions
// ---------------------------------------------------------------------------

/** Source kind for a context packet. */
export type ContextPacketKind = 'api' | 'db' | 'broker';

/** Request body for `/context-packet/preview` and `/context-packet/send`. */
export interface ContextPacketReq {
  kind: ContextPacketKind;
  payload: unknown;
}

/** Per-kind redaction tally entry (mirrors `RedactionHit` in otto-core). */
export interface RedactionHit {
  kind: string;
  count: number;
}

/** Response from `POST /context-packet/preview`. */
export interface ContextPacketPreviewResp {
  redacted: unknown;
  redactions: RedactionHit[];
  size_bytes: number;
}

/** Response from `POST /context-packet/send`. */
export interface ContextPacketSendResp {
  ok: boolean;
  size_bytes: number;
  redactions: RedactionHit[];
}

// ---------------------------------------------------------------------------
// Cross-module search  (GET /workspaces/{id}/search?q=)
// ---------------------------------------------------------------------------

/**
 * One ranked result from the cross-module search endpoint. Sources require
 * their own feature View grant and retain workspace/private-memory visibility;
 * at most five matches per source are returned.
 * `kind` discriminates the object type; `actions[0]` is always the primary
 * "open" navigation action.
 */
export interface SearchHit {
  /** Object type: "story" | "workflow" | "api_request" | "swarm_task" |
   *  "swarm_project" | "memory" | "repo" | "broker_cluster" */
  kind: string;
  /** Row id in the originating table. */
  id: string;
  /** Primary display title. */
  title: string;
  /** Secondary display text (method+URL, status, collection, …). */
  subtitle?: string;
  /** Contextual action labels; first entry is the primary "open" action. */
  actions: string[];
}

/** One product-analysis lens offered in the Analysis tab. Curated subset of the
 *  bundled product skills (only those that emit the Findings contract).
 *  Mirrors `otto_core::api::ProductLens`; returned by
 *  `GET /workspaces/{id}/product/lenses`. */
export interface ProductLens {
  /** Library skill name (e.g. "po-story-overview"). */
  skill: string;
  /** Human label shown in the UI checkbox row. */
  label: string;
  /** One-line description of what the lens does. */
  description: string;
  /** Whether the lens is checked by default in the Analysis tab. */
  default_on: boolean;
}

// ---------------------------------------------------------------------------
// Model catalog — per-provider model lists (`GET /providers/models`).
// ---------------------------------------------------------------------------

/** One model in a provider's catalog. `source` says where the id came from
 *  (cli | docs | modelsdev). */
export interface CatalogModelEntry {
  id: string;
  label?: string | null;
  source?: string | null;
}

/** A provider's fetched model list. A failed refresh never wipes the last
 *  good list — `stale` + `last_error` say so instead. */
export interface ProviderModelCatalog {
  models: CatalogModelEntry[];
  fetched_at?: string | null;
  stale?: boolean;
  last_error?: string | null;
}

/** `GET /providers/models` / `POST /providers/models/refresh` response. */
export interface ModelCatalogResp {
  providers: Record<string, ProviderModelCatalog>;
}
