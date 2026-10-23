// Contract mirror: auth. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id, User, Workspace, WorkspaceRole,
} from './core';

// ---------------------------------------------------------------------------
// RBAC — features + capabilities
// ---------------------------------------------------------------------------

/** The 32 protected features (snake_case, mirrors Rust Feature enum). */
export type Feature =
  | 'agents'
  | 'connections'
  | 'database'
  | 'git'
  | 'issues'
  | 'product'
  | 'swarm'
  | 'api_client'
  | 'workflows'
  | 'channels'
  | 'skill_eval'
  | 'skills'
  | 'insights'
  | 'usage'
  | 'self_improvement'
  | 'context'
  | 'settings'
  | 'users'
  | 'canvas'
  /** Design Hall — the artifact graph (granted wherever `canvas` was). */
  | 'design'
  | 'proof_pack'
  | 'mcp'
  | 'mission_control'
  | 'scheduled_tasks'
  | 'run_with_otto'
  | 'browser'
  | 'aws'
  | 'aws_s3'
  | 'aws_sqs'
  | 'aws_ec2'
  | 'aws_athena'
  | 'aws_eks'
  | 'aws_rds'
  | 'kubernetes';

/** Capability ladder (None < View < Edit < Admin). */
export type Capability = 'none' | 'view' | 'edit' | 'admin';

/** One (feature, capability) pair — mirrors `GrantEntry` in api.rs. */
export interface GrantEntry {
  feature: string;
  capability: string;
}

/** `GET /api/v1/users/{id}/grants` response. */
export interface UserGrantsResp {
  grants: GrantEntry[];
}

/** `PUT /api/v1/users/{id}/grants` request body. */
export interface UserGrantsReq {
  grants: GrantEntry[];
}

/** `GET /api/v1/auth/capabilities` response — the caller's effective map.
 *  Root receives `admin` for every feature. */
export interface CapabilitiesResp {
  /** snake_case feature name → snake_case capability string. */
  capabilities: Record<string, string>;
}

// ---------------------------------------------------------------------------
// Admin active-sessions overview (RBAC Task 4.2/4.3)
// ---------------------------------------------------------------------------

/** One row of `GET /api/v1/admin/sessions` — mirrors `AdminSessionRow` in api.rs. */
export interface AdminSessionRow {
  /** Session id. */
  id: string;
  /** `created_by` — owning user's id. */
  owner_id: string;
  /** Owning user's username (falls back to owner_id when user no longer exists). */
  owner_username: string;
  /** Workspace the session belongs to. */
  workspace_id: string;
  /** `'agent'` | `'connection'`. */
  kind: string;
  /** CLI provider / connection driver (`'claude'`, `'codex'`, `'shell'`, `'mysql'`, …). */
  provider: string;
  /** Display title. */
  title: string;
  /** Persisted status (`'running'` | `'working'` | `'idle'` | `'reconnectable'` | `'exited'`). */
  status: string;
  /** True when the session has a live PTY in the daemon process. */
  live: boolean;
  /** Number of WS terminal viewers currently attached. */
  viewers: number;
}

/** `GET /api/v1/admin/sessions` response — mirrors `AdminSessionsResp` in api.rs. */
export interface AdminSessionsResp {
  sessions: AdminSessionRow[];
  /** Opaque cursor for the next (older) page; absent on the last page. */
  next_cursor?: string | null;
}

/** `GET /api/v1/admin/sessions` query: unarchived sessions (live + recent
 *  exited) by default, newest first, `limit` (default 200, max 1000) per page. */
export interface AdminSessionsQuery {
  include_archived?: boolean;
  limit?: number;
  /** `next_cursor` of the previous page (malformed → 400). */
  cursor?: string;
}

/**
 * `POST /api/v1/admin/impersonate/{user_id}` response — mirrors `ImpersonateResp`
 * in api.rs. The short-lived impersonation bearer token (returned once); the UI
 * swaps to it to act-as the target user and calls `/admin/impersonate/stop` to end.
 */
export interface ImpersonateResp {
  token: string;
}

// ---------------------------------------------------------------------------
// Meta / auth
// ---------------------------------------------------------------------------

export interface ToolStatus {
  name: string;
  found: boolean;
  version: string | null;
}

/** `GET /walkthroughs/resolve?url=` — final, directly-playable video URL. */
export interface ResolveWalkthroughResp {
  url: string;
}

export interface MetaResp {
  version: string;
  api_version: number;
  needs_onboarding: boolean;
  network_listener: boolean;
  tools: ToolStatus[];
  providers: string[];
  /** Configured default agent (provider name); null when unset. */
  default_provider: string | null;
  /** Per-provider: whether the CLI accepts a model flag (its spec carries a
   *  `model_args` template). Pickers hide the model control when false. */
  model_flags: Record<string, boolean>;
  /** Second LOOPBACK base for the same daemon (`http://localhost:<port>`; the
   *  daemon holds both 127.0.0.1 and [::1]) — a separate browser socket pool
   *  for background/slow calls. `null`/absent → single-host transport. */
  alt_loopback_base?: string | null;
}

export interface OnboardRootReq {
  password: string;
  display_name?: string | null;
}

export interface LoginReq {
  username: string;
  password: string;
}

export interface LoginResp {
  token: string;
  user: User;
}

/**
 * GET /api/v1/auth/me — mirrors `MeResp` in api.rs.
 * `user` is the effective identity (what authorisation runs against).
 * `real_user` is the token owner (== `user` for normal sessions).
 * `impersonating` is true when the caller holds an impersonation token.
 */
export interface MeResp {
  /** Effective user — the identity the session currently acts as. */
  user: User;
  /** Real token owner — equals `user` for a normal (non-impersonation) session. */
  real_user: User;
  /** true when real_user.id !== user.id (i.e. an impersonation token is active). */
  impersonating: boolean;
}

/** POST /api/v1/auth/tokens — mint a long-lived API (personal access) token. */
export interface CreateApiTokenReq {
  label?: string | null;
}

/** Metadata for one API token. NEVER carries the secret (only its prefix). */
export interface ApiTokenInfo {
  id: string;
  label?: string | null;
  /** First 12 chars of the raw token, for identification in a list. */
  token_prefix: string;
  created_at: string;
  last_seen_at: string;
  expires_at: string;
  session_id?: string | null;
  /** Legacy label candidate, not durable session ownership. */
  legacy_session_id?: string | null;
  session_exists?: boolean | null;
}

export interface ProviderAccount {
  id: string;
  provider: 'claude' | 'codex';
  label: string;
  created_at: string;
}

/** Response for POST /api/v1/auth/tokens: raw secret (shown once) + metadata. */
export interface CreateApiTokenResp {
  token: string;
  info: ApiTokenInfo;
}

// ---------------------------------------------------------------------------
// Trust & Safety: audit log + security posture (root only)
// ---------------------------------------------------------------------------

/**
 * One append-only entry in the security audit log. Written best-effort by the
 * daemon at sensitive sites; never updated or deleted.
 * `action` is a stable snake_case verb, e.g. 'login.success', 'token.mint',
 * 'token.revoke', 'settings.change', 'network_listener.toggle',
 * 'login.failure', 'login.lockout', 'db.write_confirmed'.
 */
export interface AuditEntry {
  id: Id;
  ts: string;
  /** Acting user; null for an unauthenticated actor (e.g. a failed login). */
  user_id: Id | null;
  action: string;
  target: string | null;
  detail: unknown | null;
  ip: string | null;
}

/** Query for GET /api/v1/audit-log (root only). All filters optional. */
export interface AuditLogQuery {
  /** Lower bound on `ts` (RFC3339), inclusive. */
  from?: string;
  /** Upper bound on `ts` (RFC3339), inclusive. */
  to?: string;
  /** Exact `action` match. */
  action?: string;
  /** Exact acting-user match. */
  user_id?: Id;
  limit?: number;
  offset?: number;
}

/** Response for GET /api/v1/audit-log: a page plus the filtered total. */
export interface AuditLogResp {
  entries: AuditEntry[];
  /** Total rows matching the filters (ignores limit/offset). */
  total: number;
}

/** GET /api/v1/security-posture (root only) — a derived security snapshot. */
export interface SecurityPostureResp {
  network_listener: boolean;
  network_listener_port: number | null;
  loopback_only: boolean;
  network_listener_restart_required: boolean;
  active_api_tokens: number;
}

// ---------------------------------------------------------------------------
// Users / workspaces
// ---------------------------------------------------------------------------

export interface CreateUserReq {
  username: string;
  password: string;
  display_name?: string | null;
}

export interface UpdateUserReq {
  display_name?: string | null;
  password?: string | null;
  disabled?: boolean | null;
  /** Required when changing your OWN password (S8-310). */
  current_password?: string | null;
}

export interface CreateWorkspaceReq {
  name: string;
  root_path: string;
}

export interface UpdateWorkspaceReq {
  name?: string | null;
  root_path?: string | null;
  settings?: Record<string, unknown> | null;
  /** Atomic RFC 7396 patch: null members remove keys. Cannot include context
   *  or be combined with settings (legacy full replacement). */
  settings_patch?: Record<string, unknown>;
  archived?: boolean | null;
}

export interface MemberEntry {
  user_id: Id;
  username: string;
  display_name: string;
  role: WorkspaceRole;
}

export interface SetMembersReq {
  members: { user_id: Id; role: WorkspaceRole }[];
}

export type WorkspaceWithRole = Workspace & { my_role: WorkspaceRole };
