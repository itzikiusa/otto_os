// Contract mirror: mcp. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

/** A user-configured MCP server for a workspace. Enabled servers are merged into
 *  the workspace's `.mcp.json` (alongside Otto's managed entries) on agent spawn.
 *  Never auto-enabled. */
export interface McpServer {
  id: Id;
  workspace_id: Id;
  /** Key under `.mcp.json`'s mcpServers map; unique within the workspace. */
  name: string;
  command: string;
  args: string[];
  /** NON-secret env pairs only; secret values live in the macOS Keychain. */
  env: Record<string, string>;
  /** Names of env vars whose values are Keychain-backed (values never returned). */
  secret_env_keys: string[];
  /** Opaque Keychain blob ref when secrets exist (never a secret itself). */
  secret_ref?: string | null;
  /** Off by default — only written to `.mcp.json` once enabled. */
  enabled: boolean;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

export interface CreateMcpServerReq {
  name: string;
  command: string;
  args?: string[];
  env?: Record<string, string>;
  /** WRITE-ONLY: secret env values → Keychain (response returns key names only). */
  secret_env?: Record<string, string>;
  enabled?: boolean;
}

export interface UpdateMcpServerReq {
  name?: string;
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  /** WRITE-ONLY: when present, replaces the secret env set (values → Keychain). */
  secret_env?: Record<string, string>;
  enabled?: boolean;
}

// ---------------------------------------------------------------------------
// MCP Control Plane — mirrors crates/otto-state/src/mcp_control.rs +
// crates/otto-mcp/src/types.rs (wire JSON is snake_case). The legacy McpServer /
// CreateMcpServerReq / UpdateMcpServerReq above stay intact — they drive the
// `.mcp.json` config CRUD. The richer governed registry uses the DTOs below.
// ---------------------------------------------------------------------------

export type McpTransport = 'stdio' | 'http';
export type McpRiskLabel = 'read' | 'write' | 'dangerous' | 'unknown';
export type McpInjectionRisk = 'low' | 'medium' | 'high';
export type McpHealthStatus = 'unknown' | 'healthy' | 'unhealthy' | 'disabled';
export type McpToolAccess = 'allow' | 'deny';
export type McpPolicyEffect = 'allow' | 'deny' | 'require_approval' | 'require_dry_run';
/** `decision` on a governed invoke / audit row. */
export type McpDecision =
  | 'allowed'
  | 'approved'
  /** A mutating `otto.*` call that skipped the per-call approval because an
   *  auto-approve rule covers it; `decision_reason` names the rule. */
  | 'auto_approved'
  | 'denied'
  | 'dry_run'
  | 'pending_approval'
  | 'error';
export type McpApprovalStatus =
  | 'pending'
  | 'approved'
  | 'denied'
  | 'expired'
  | 'cancelled'
  | 'consumed';

/** Full registry view of a control-plane MCP server (augmented `mcp_servers`
 *  row). Secret VALUES are never returned — only the key-name lists + has_secret. */
export interface McpServerDetail {
  id: Id;
  workspace_id: Id;
  name: string;
  transport: McpTransport;
  command: string;
  args: string[];
  env: Record<string, string>;
  url: string | null;
  description: string | null;
  headers: Record<string, string>;
  secret_env_keys: string[];
  secret_header_keys: string[];
  has_secret: boolean;
  injection_risk: McpInjectionRisk;
  managed: boolean;
  default_tool_access: McpToolAccess;
  enabled: boolean;
  health_status: McpHealthStatus;
  health_checked_at: string | null;
  health_latency_ms: number | null;
  health_error: string | null;
  tools_count: number;
  tools_discovered_at: string | null;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

/** A discovered tool + its governance metadata (`McpTool` in Rust). */
export interface McpToolView {
  id: Id;
  server_id: Id;
  name: string;
  title: string | null;
  description: string | null;
  input_schema: unknown;
  annotations: unknown;
  risk_label: McpRiskLabel;
  injection_risk: McpInjectionRisk;
  mutating: boolean;
  supports_dry_run: boolean;
  enabled: boolean;
  require_approval: boolean;
  risk_overridden: boolean;
  created_at: string;
  updated_at: string;
}

/** `GET /mcp/servers/{id}` body. */
export interface McpServerWithTools {
  server: McpServerDetail;
  tools: McpToolView[];
}

/** `POST /workspaces/{ws}/mcp/servers` — secret_* values go to the keychain and
 *  are never returned (only their key names show up as secret_*_keys). */
export interface CreateMcpControlServerReq {
  name: string;
  transport?: McpTransport;
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string | null;
  description?: string | null;
  headers?: Record<string, string>;
  secret_env?: Record<string, string>;
  secret_headers?: Record<string, string>;
  injection_risk?: McpInjectionRisk;
  default_tool_access?: McpToolAccess;
  enabled?: boolean;
}

/** `PATCH /mcp/servers/{id}` — all fields optional; absent = keep current. */
export interface UpdateMcpControlServerReq {
  name?: string;
  description?: string | null;
  command?: string;
  args?: string[];
  env?: Record<string, string>;
  url?: string | null;
  headers?: Record<string, string>;
  /** Present replaces this map; omitted preserves it. Empty clears it. */
  secret_env?: Record<string, string>;
  /** Present replaces this map; omitted preserves it. Empty clears it. */
  secret_headers?: Record<string, string>;
  injection_risk?: McpInjectionRisk;
  default_tool_access?: McpToolAccess;
  enabled?: boolean;
}

/** `PATCH /mcp/tools/{toolId}` — per-tool permission + risk override. */
export interface PatchMcpToolReq {
  enabled?: boolean;
  require_approval?: boolean;
  risk_label?: McpRiskLabel;
  injection_risk?: McpInjectionRisk;
}

/** `POST /mcp/servers/{id}/tools/{name}/invoke` body. */
export interface McpInvokeReq {
  arguments?: unknown;
  dry_run?: boolean;
  workspace_id?: string | null;
}

/** The governed-invoke result (`InvokeResp` in Rust). */
export interface McpInvokeResp {
  decision: McpDecision;
  executed: boolean;
  dry_run: boolean;
  reason?: string | null;
  approval_id?: string | null;
  /** Redacted, capped tool result content when executed. */
  content?: unknown;
  is_error?: boolean | null;
  /** The dry-run preview when `dry_run`. */
  preview?: unknown;
  /** `POST /mcp/otto-tools/invoke` of an `otto.ui_*` tool from a session
   *  credential only: that session's UI-control grant after the call. */
  ui_granted?: boolean;
}

/** `POST /mcp/tool-calls` — the stdio bridge's own audit append (204). */
export interface McpToolCallAuditReq {
  tool: string;
  arguments?: unknown;
  ok: boolean;
  rows?: number | null;
}

export interface McpAllowlistEntry {
  id: Id;
  workspace_id: Id;
  server_id: Id;
  /** null = whole server. */
  tool_name: string | null;
  mode: McpToolAccess;
  created_by: Id;
  created_at: string;
}

/** One entry in the bulk allowlist PUT body. */
export interface McpAllowlistEntryInput {
  server_id: Id;
  tool_name?: string | null;
  mode: McpToolAccess;
}

/** `PUT /workspaces/{ws}/mcp/allowlist`. */
export interface SetMcpAllowlistReq {
  entries: McpAllowlistEntryInput[];
}

/** A policy-as-code rule (`match` is an arbitrary matcher object). */
export interface McpPolicy {
  id: Id;
  /** null = global. */
  workspace_id: Id | null;
  name: string;
  enabled: boolean;
  priority: number;
  match: unknown;
  effect: McpPolicyEffect;
  reason: string | null;
  created_by: Id;
  created_at: string;
  updated_at: string;
}

export interface CreateMcpPolicyReq {
  workspace_id?: string | null;
  name: string;
  enabled?: boolean;
  priority?: number;
  match?: unknown;
  effect: McpPolicyEffect;
  reason?: string | null;
}

export interface UpdateMcpPolicyReq {
  name?: string;
  enabled?: boolean;
  priority?: number;
  match?: unknown;
  effect?: McpPolicyEffect;
  reason?: string | null;
}

/** `GET /mcp/policies/export`. */
export interface McpPolicyExport {
  version: number;
  policies: McpPolicy[];
}

/** `POST /mcp/policies/import`. */
export interface ImportMcpPoliciesReq {
  policies: CreateMcpPolicyReq[];
  replace?: boolean;
}

export interface ImportMcpPoliciesResp {
  imported: number;
  replaced: boolean;
}

/** `POST /mcp/policies/evaluate` request. */
export interface EvaluateMcpReq {
  server_id: string;
  tool: string;
  workspace_id?: string | null;
  arguments?: unknown;
}

/** `POST /mcp/policies/evaluate` response (preview). */
export interface McpEvaluatePreview {
  server: string;
  tool: string;
  risk_label: string;
  injection_risk: string;
  policy_decision: string;
  reason: string | null;
}

export interface McpApproval {
  id: Id;
  workspace_id: string | null;
  /** 'tool_call' | 'human_ask'. */
  kind: string;
  server_id: string | null;
  server_name: string | null;
  tool: string | null;
  title: string;
  detail: string | null;
  /** Redacted display copy of the arguments (never the full/secret values). */
  args_redacted_json: string;
  risk_label: string | null;
  status: McpApprovalStatus;
  requested_by: string | null;
  requested_by_kind: string | null;
  /** The agent session that raised the request, when known. */
  requested_by_session_id?: string | null;
  decided_by: string | null;
  decision_note: string | null;
  created_at: string;
  decided_at: string | null;
  consumed_at: string | null;
  expires_at: string | null;
}

/** `POST /mcp/approvals/{id}/decide`. */
export interface DecideMcpApprovalReq {
  approved: boolean;
  note?: string | null;
}

/** One row of the governed-call audit ledger. */
export interface McpCallLogRow {
  id: Id;
  workspace_id: string | null;
  server_id: string | null;
  server_name: string | null;
  tool: string;
  /** 'outbound' | 'inbound'. */
  direction: string;
  caller_user_id: string | null;
  caller_kind: string | null;
  args_redacted_json: string;
  decision: string;
  decision_reason: string | null;
  risk_label: string | null;
  injection_risk: string | null;
  dry_run: boolean;
  ok: boolean;
  error: string | null;
  latency_ms: number | null;
  bytes: number | null;
  rows: number | null;
  approval_id: string | null;
  /** The agent session that made the call (Otto-minted session credentials only). */
  caller_session_id?: string | null;
  created_at: string;
}

/** `GET /mcp/approvals/count?status=` — the badge number, with the same
 *  visibility as `GET /mcp/approvals` but uncapped and without the rows. */
export interface McpApprovalCount {
  count: number;
}

/** Filters for `GET /mcp/audit`. */
export interface McpAuditQuery {
  server_id?: string;
  tool?: string;
  decision?: string;
  limit?: number;
  offset?: number;
}

/** `GET /mcp/audit?paged=true` — one page of the rows the caller may see.
 *  Visibility is checked AFTER the ledger read, so `rows` can be short (even
 *  empty) mid-ledger: page on `has_more` / `next_offset`, never on length. */
export interface McpAuditPage {
  rows: McpCallLogRow[];
  /** Ledger offset of the next page (`offset` + ledger rows read). */
  next_offset: number;
  /** The ledger read was a full `limit` — older rows may exist. */
  has_more: boolean;
}

/** Per-tool aggregate stats (cost = bytes proxy; latency / error counts). */
export interface McpToolStats {
  server_id: string | null;
  server_name: string | null;
  tool: string;
  calls: number;
  errors: number;
  error_rate: number;
  avg_latency_ms: number;
  max_latency_ms: number;
  total_bytes: number;
  avg_bytes: number;
  last_called_at: string | null;
}

// --- Otto-as-MCP-server (outward server admin) -----------------------------
// Response shape per the design contract (§7/§10). The daemon admin route is
// not yet wired into the otto-mcp router, so this DTO mirrors the documented
// contract rather than a Rust struct.

export interface McpOttoToolInfo {
  name: string;
  description: string;
  mutating: boolean;
  enabled: boolean;
  /** Feature group (e.g. "Workflows", "Message Brokers") for UI grouping. Optional
   * for forward-compat with daemons that predate the categorised catalog. */
  category?: string | null;
  /** True when the tool is auto-approved EVERYWHERE (an enabled global
   *  per-tool auto-approve rule — the catalog's Auto-approve switch): its calls
   *  skip the human approval (audited `auto_approved`). Optional for
   *  forward-compat with daemons that predate the per-tool exemption. */
  approval_exempt?: boolean;
  /** In the guardrail tier (merge a PR, kubectl ops, produce to a live
   *  topic/queue, arbitrary HTTP, hard deletes): never covered by a category
   *  rule; a per-tool rule needs `allow_irreversible`. */
  irreversible?: boolean;
  /** Every enabled auto-approve rule (any scope) that covers this tool. */
  auto_approved_by?: McpAutoApproveRef[];
}

/** `GET /mcp/otto-server` (+ `PATCH` reply, which may also carry `token` once). */
export interface McpOttoServerStatus {
  enabled: boolean;
  tools: McpOttoToolInfo[];
  has_token: boolean;
  token_prefix?: string | null;
  /** The freshly-minted token — returned ONCE on a mint/rotate, never again. */
  token?: string | null;
  /** Global `mcp_require_approval_dangerous` (default true). When false no
   *  otto.* call asks for approval, whatever the per-tool setting says. */
  require_approval_dangerous?: boolean;
  /** Bare names of the mutating tools that skip the per-call approval. */
  approval_exempt_tools?: string[];
}

/** `GET /mcp/otto-server/enabled` (CP24a) — the light read behind every agent
 *  session's `tools/list`: just the enabled full `otto.*` names, the master
 *  switch, and whether the CALLING session holds the UI-control grant. */
export interface McpOttoServerEnabled {
  enabled: string[];
  outward_enabled: boolean;
  /** False for a credential not bound to an Otto session. */
  ui_granted: boolean;
  /** The resolved MCP profile filter when `?profiles=` was sent, else null. */
  profile?: McpToolFilter | null;
}

// --- MCP profiles (`/mcp/profiles`, CP44–CP48) ------------------------------
// Named variants of the Otto MCP connection, each exposing only a subset of
// the tools. Mirrors `otto_state::McpProfile` / `otto_mcp::profiles`.

export interface McpProfile {
  id: Id;
  name: string;
  description: string;
  /** Every tool minus `exclude_tools` (the Full profile). */
  all_tools: boolean;
  /** Tool-group ids (`McpToolGroup.id`). */
  groups: string[];
  /** Canonical tool names added on top of the groups (`create_pr`). */
  include_tools: string[];
  /** Canonical tool names removed from the groups. */
  exclude_tools: string[];
  /** A seeded preset (Full, Database, Issues, Git & PRs, Cloud). */
  builtin: boolean;
  /** Read-only and undeletable (Full). */
  locked: boolean;
  /** Exactly one profile is the default. */
  is_default: boolean;
  created_by?: Id | null;
  created_at: string;
  updated_at: string;
}

/** One tool group a profile is built from. */
export interface McpToolGroup {
  id: string;
  label: string;
  description: string;
  /** Tool families the group also covers (`aws_` → every `aws_*` tool). */
  prefixes: string[];
  /** Canonical member names (exact names + every catalog tool in it). */
  tools: string[];
}

/** `GET /mcp/profiles`. */
export interface McpProfileList {
  profiles: McpProfile[];
  groups: McpToolGroup[];
  default_id: Id;
}

/** `POST /mcp/profiles`; `PATCH /mcp/profiles/{id}` (every field optional). */
export interface McpProfileReq {
  name?: string;
  description?: string;
  all_tools?: boolean;
  groups?: string[];
  include_tools?: string[];
  exclude_tools?: string[];
}

/** The resolved UNION of a session's / connection's profiles (CP24a). */
export interface McpToolFilter {
  ids: Id[];
  /** Names joined with " + ". */
  label: string;
  profiles: Pick<McpProfile, 'id' | 'name' | 'all_tools' | 'groups' | 'include_tools' | 'exclude_tools'>[];
}

// --- MCP auto-approve rules (`/mcp/auto-approve`) ---------------------------
// The explicit, opt-in policy under which a mutating `otto.*` tool call skips
// the per-call human approval. Mirrors `otto_state::McpAutoApproveRule`.

export type McpAutoApproveScope = 'global' | 'workspace' | 'session';
export type McpAutoApproveTargetKind = 'tool' | 'category';

/** The short rule reference on a catalog tool / a governed envelope
 *  (`auto_approved_by`). */
export interface McpAutoApproveRef {
  id: Id;
  name: string;
  scope: McpAutoApproveScope;
  workspace_id?: Id | null;
  session_id?: Id | null;
  target_kind: McpAutoApproveTargetKind;
  /** Bare tool name (`create_pr`) or category label (`Git`). */
  target: string;
}

export interface McpAutoApproveRule extends McpAutoApproveRef {
  enabled: boolean;
  /** The second explicit toggle — only ever true on a per-tool rule for an
   *  irreversible tool. */
  allow_irreversible: boolean;
  note?: string | null;
  created_by: Id;
  created_at: string;
  updated_at: string;
  /** Display labels (GET/POST/PATCH responses). */
  workspace_name?: string;
  session_title?: string;
  /** True when the rule's tool is in the irreversible tier. */
  irreversible?: boolean;
}

/** `GET /mcp/auto-approve`. */
export interface McpAutoApproveList {
  rules: McpAutoApproveRule[];
  /** The categories a category rule may name (each has ≥1 mutating tool),
   *  with their mutating tools and which are irreversible. */
  categories: { category: string; tools: { name: string; irreversible: boolean }[] }[];
}

/** `POST /mcp/auto-approve`. */
export interface CreateMcpAutoApproveReq {
  /** Defaults to "<target> — <scope>". */
  name?: string;
  enabled?: boolean;
  scope: McpAutoApproveScope;
  /** Required iff scope = workspace. */
  workspace_id?: Id;
  /** Required iff scope = session. */
  session_id?: Id;
  target_kind: McpAutoApproveTargetKind;
  /** Bare or `otto.`-prefixed tool name, or a category label. */
  target: string;
  /** Required (true) for an irreversible tool; refused on a category rule. */
  allow_irreversible?: boolean;
  note?: string;
}

/** `PATCH /mcp/auto-approve/{id}`. */
export interface UpdateMcpAutoApproveReq {
  name?: string;
  enabled?: boolean;
  allow_irreversible?: boolean;
  note?: string;
}

/** `PATCH /mcp/otto-server`. */
export interface UpdateMcpOttoServerReq {
  enabled?: boolean;
  tools?: string[];
  /** Compatibility shim over the auto-approve rules: the COMPLETE set of
   *  mutating tools auto-approved everywhere (global per-tool rules; bare or
   *  `otto.`-prefixed names; a read or unknown name is a 400, and so is an
   *  irreversible tool that is not already auto-approved). Pruned to the
   *  enabled set — disabling a tool drops its per-tool rules. Prefer
   *  `/mcp/auto-approve`. */
  approval_exempt_tools?: string[];
  rotate_token?: boolean;
}

/**
 * The per-token permission set of an outward MCP token. Each token (and each
 * user) may carry a different one — the mechanism behind "different users have
 * different accesses" over the MCP HTTP transport.
 * - `tools`: bare tool names this token may call; omit/`null` = all globally-enabled.
 * - `allow_writes`: when false (default) every mutating tool is denied.
 * - `workspace_id`: optional pin; a call to another workspace is denied.
 */
export interface McpScope {
  tools?: string[] | null;
  allow_writes: boolean;
  workspace_id?: string | null;
}

/** Metadata for one outward MCP token. NEVER carries the secret (only its prefix). */
export interface McpTokenInfo {
  id: Id;
  user_id: Id;
  username: string;
  label?: string | null;
  token_prefix: string;
  scope: McpScope;
  created_at: string;
  last_seen_at: string;
  expires_at: string;
}

/** `POST /mcp/tokens` — mint a scoped MCP token (admin). */
export interface CreateMcpTokenReq {
  /** Owning user; defaults to the caller when omitted. */
  user_id?: Id;
  label?: string;
  /** Per-token scope; omitted ⇒ unrestricted (every globally-enabled tool). */
  scope?: McpScope;
}

/** `POST /mcp/tokens` response: the raw secret is returned exactly once. */
export interface CreateMcpTokenResp {
  token: string;
  info: McpTokenInfo;
}

/** `POST /mcp/tokens/{id}/rotate` — rotates ONE token in place (same owner, label and
 * scope); the new raw secret is returned exactly once. Every other token is untouched. */
export interface RotateMcpTokenResp {
  token: string;
  info: McpTokenInfo;
  revoked_id: Id;
}

/** `GET` / `PATCH /workspaces/{wid}/mcp/session-attach` — whether sessions spawned in the
 * workspace get Otto's built-in `otto` MCP tool server (`otto_mcp_enabled`, per-workspace). */
export interface McpSessionAttach {
  workspace_id: Id;
  attached: boolean;
}

/** `PATCH /workspaces/{wid}/mcp/session-attach`. */
export interface UpdateMcpSessionAttachReq {
  enabled: boolean;
}
