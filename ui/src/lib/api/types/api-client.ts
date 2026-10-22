// Contract mirror: api-client. Part of lib/api/types (see ../types.ts, the barrel).
// Mirrors docs/contracts/api.md + ws.md — change them together.

import type {
  Id,
} from './core';

// ---------------------------------------------------------------------------
// API client ("Postman" section) — workspace-scoped
// ---------------------------------------------------------------------------

export interface ApiKeyVal {
  key: string;
  value: string;
  enabled?: boolean;
}

/** A Keychain-backed secret placeholder: the stored row (and any export)
 * carries this marker instead of the value; the daemon resolves it in-memory
 * at execute time only. */
export interface ApiSecretRef {
  $secret: string;
}

/** An auth member that is either plaintext (being typed) or a Keychain
 * marker (loaded from a saved request whose secret was already migrated). */
export type ApiSecretable = string | ApiSecretRef;

export function isSecretRef(v: ApiSecretable | undefined | null): v is ApiSecretRef {
  return typeof v === 'object' && v !== null && typeof v.$secret === 'string';
}

export type ApiAuth =
  | { type: 'none' }
  | { type: 'bearer'; token: ApiSecretable }
  | { type: 'basic'; username: string; password: ApiSecretable }
  | { type: 'api_key'; key: string; value: ApiSecretable; in: 'header' | 'query' }
  | {
      type: 'oauth2';
      grant: 'client_credentials' | 'password' | 'refresh_token' | 'authorization_code';
      authorization_url?: string;
      token_url: string;
      client_id: string;
      client_secret: ApiSecretable;
      scope: string;
      username: string;
      password: ApiSecretable;
      refresh_token: ApiSecretable;
      access_token: ApiSecretable;
      token_type: string;
    };

/** Versioned extension object persisted on a saved request (scripts / docs /
 * settings / GraphQL variables / transport). The UI owns this shape; the
 * daemon validates only that it is an object within the size cap. */
export interface ApiRequestExtras {
  /** Saved runnable unary/server-streaming gRPC schema and selected method. */
  grpc?: {proto: string; method: string};
  v: number;
  /** Transport kind; absent = http. */
  transport?: 'http' | 'sse' | 'websocket' | 'grpc';
  /** GraphQL variables, raw JSON text as authored. */
  graphql_variables?: string;
  /** Request docs, markdown. */
  docs_md?: string;
  scripts?: { pre?: string; post?: string };
  settings?: { timeout_ms?: number | null; follow_redirects?: boolean; tls_verify?: boolean };
}

export type ApiBodyMode = 'none' | 'json' | 'raw' | 'form' | 'multipart' | 'graphql';

export interface ApiCollection {
  id: Id;
  workspace_id: Id;
  name: string;
  parent_id: Id | null;
  position: number;
  created_at: string;
}

export interface ApiRequest {
  id: Id;
  workspace_id: Id;
  collection_id: Id | null;
  name: string;
  method: string;
  url: string;
  headers: ApiKeyVal[];
  query: ApiKeyVal[];
  body_mode: ApiBodyMode;
  body: string;
  auth: ApiAuth;
  /** Optional `ssh`-kind connection id to tunnel executions through (null = direct). */
  ssh_connection_id?: Id | null;
  /** Persisted scripts / docs / settings / GraphQL variables / transport. */
  extras?: ApiRequestExtras | null;
  position: number;
  created_at: string;
  updated_at: string;
}

export interface ApiEnvironment {
  id: Id;
  workspace_id: Id;
  name: string;
  /** Non-secret variables only; keys in `secret_keys` never appear here. */
  variables: Record<string, string>;
  /** Names of variables whose values are Keychain-backed (resolved only at
   * execute time; never returned over REST). */
  secret_keys: string[];
  is_active: boolean;
  created_at: string;
}

/** Metadata-only list item; fetch history/{id} before replaying a request. */
export interface ApiHistorySummary {
  id: Id;
  workspace_id: Id;
  method: string;
  url: string;
  status: number | null;
  duration_ms: number | null;
  executed_at: string;
  request_id: string | null;
  source: {kind: string; session_id: string | null; via: string | null};
}

export interface ApiHistoryEntry {
  id: Id;
  workspace_id: Id;
  method: string;
  url: string;
  status: number | null;
  duration_ms: number | null;
  request: unknown;
  response: unknown;
  executed_at: string;
}

export interface ApiOverviewCollection {
  id: Id;
  name: string;
  parent_id: Id | null;
}

export interface ApiOverviewRequest {
  id: Id;
  name: string;
  method: string;
  /** URL template as saved — `{{var}}` placeholders are not substituted. */
  url: string;
  collection_id: Id | null;
  /** `auth.type`; never an auth value. */
  auth_type: string;
  has_ssh: boolean;
  /** True when the request was created or edited through an agent session. */
  agent_authored: boolean;
  updated_at: string;
  /** Sidebar order inside its collection (`requests/summaries`; absent from
   *  daemons older than perf2 — the UI falls back to the list order). */
  position?: number;
}

/** A saved request as the API page's tree, search, pickers and ⌘K hold it:
 *  the `GET …/api-client/requests/summaries` projection (no body, headers,
 *  scripts, docs or files) stamped with the workspace it was loaded for. The
 *  full `ApiRequest` is fetched when a request is opened (perf2 N1). */
export type ApiRequestItem = ApiOverviewRequest & { workspace_id: Id; position: number };

/** `GET …/api-client/storage` — what this workspace's API history and
 *  automation runs occupy (cached ≤60 s daemon-side). Retention stays opt-in;
 *  the History list shows this and offers presets (perf2 N2). */
export interface ApiClientStorage {
  history_rows: number;
  history_bytes: number;
  run_rows: number;
  step_rows: number;
  run_bytes: number;
  /** Most finished runs any one automation holds. */
  max_runs_per_automation: number;
}

export interface ApiOverviewEnvironment {
  id: Id;
  name: string;
  is_active: boolean;
  /** Non-secret variables; secret-shaped keys have masked values. */
  variables: Record<string, string>;
  secret_keys: string[];
}

export interface ApiOverviewAutomation {
  id: Id;
  name: string;
  steps: number;
}

/** Agent-facing discovery view of the workspace API client. */
export interface ApiOverview {
  collections: ApiOverviewCollection[];
  requests: ApiOverviewRequest[];
  environments: ApiOverviewEnvironment[];
  automations: ApiOverviewAutomation[];
}

/** Execute one saved API-client request. */
export interface RunSavedRequestReq {
  environment_id?: Id | null;
  vars?: Record<string, string>;
  timeout_ms?: number;
  confirm?: boolean;
  confirm_new_host?: boolean;
  shape?: 'full' | 'agent';
  decode_jwt?: boolean;
}

export interface ApiResolvedRequest {
  method: string;
  /** Substituted URL with every resolved secret value scrubbed. */
  url: string;
  environment_id: Id | null;
  environment: string | null;
  /** Names of secrets resolved for the send; never their values. */
  secrets_used: string[];
}

export interface RunSavedRequestResp {
  history_id: Id;
  request_id: Id;
  name: string;
  response: ApiResponse;
  resolved: ApiResolvedRequest;
  jwt_claims?: Record<string, unknown> | null;
  warnings: string[];
  script_tests: unknown;
}

/** `request.source` inside an ApiHistoryEntry snapshot. */
export interface ApiHistorySource {
  kind: 'agent' | 'human';
  session_id?: Id | null;
  /** How an agent without an Otto session reached the route (e.g. `mcp-outward`). */
  via?: string;
}

export interface UpsertApiCollectionReq {
  name: string;
  parent_id?: Id | null;
}

export interface UpsertApiRequestReq {
  collection_id?: Id | null;
  name: string;
  method: string;
  url: string;
  headers?: ApiKeyVal[];
  query?: ApiKeyVal[];
  body_mode?: ApiBodyMode;
  body?: string;
  auth?: ApiAuth;
  /** Optional `ssh`-kind connection id to tunnel executions through. */
  ssh_connection_id?: Id | null;
  /** Persisted scripts / docs / settings / GraphQL variables / transport. */
  extras?: ApiRequestExtras | null;
}

export interface UpsertApiEnvironmentReq {
  name: string;
  /** Non-secret variables (keys listed in secret_keys are stripped server-side). */
  variables?: Record<string, string>;
  /** Names of variables whose values are Keychain-backed. On update, omitted
   *  keeps the stored set + values; an explicit list replaces it. */
  secret_keys?: string[] | null;
  /** WRITE-ONLY: new/changed secret values; absent keys keep stored values. */
  secret_values?: Record<string, string>;
  /** Update only: `{old_name: new_name}` — a renamed secret keeps its stored
   *  Keychain value (a value in secret_values for the new name still wins). */
  secret_renames?: Record<string, string>;
}

/** gRPC workspace resolution options; omitted environment selects the active one. */
export interface GrpcReflectReq {
  url: string;
  headers?: ApiKeyVal[];
  environment_id?: Id | null;
  vars?: Record<string, string> | null;
  confirm_new_host?: boolean;
}
export interface GrpcInvokeReq extends GrpcReflectReq {
  proto?: string;
  method: string;
  body?: string;
}

export interface ExecuteApiReq {
  method: string;
  url: string;
  headers?: ApiKeyVal[];
  query?: ApiKeyVal[];
  body_mode?: ApiBodyMode;
  body?: string;
  auth?: ApiAuth;
  environment_id?: Id | null;
  timeout_ms?: number | null;
  follow_redirects?: boolean | null;
  verify_ssl?: boolean | null;
  vars?: Record<string, string> | null;
  /** Route the request through this `ssh`-kind connection (SOCKS5 over SSH). */
  ssh_connection_id?: Id | null;
  /**
   * Confirms sending a stored secret (a `$secret` marker or Keychain env
   * variable) to a host it isn't bound to. Without it the daemon answers
   * `409 needs_confirm=new_host`. Honoured for a person's credential only.
   */
  confirm_new_host?: boolean;
}

export interface ApiResponse {
  status: number;
  status_text: string;
  headers: ApiKeyVal[];
  body: string;
  /** Full response bytes, base64 — only for `image/*` bodies ≤ 5 MiB (the
   *  preview); '' otherwise. An exact UTF-8 `body` is the whole payload. */
  body_base64: string;
  /** Set when `body` is NOT the exact payload (truncated / non-UTF-8): fetch the
   *  full bytes from `GET …/api-client/responses/{body_id}/raw` (kept 10 min). */
  body_id?: string | null;
  /** `body` was truncated for display (full bytes via `body_id`). */
  truncated: boolean;
  /** Body exceeded the inline cap: body + body_base64 are empty, only save-from-server unavailable. */
  too_large: boolean;
  duration_ms: number;
  size_bytes: number;
  content_type: string | null;
  /** Per-phase execution trace for the response "Trace" tab. */
  trace: TraceStep[];
}

export interface TraceStep {
  label: string;
  detail: string;
  ms: number | null;
  /** info | timing | redirect | success | error */
  level: string;
}

export interface ImportCurlReq {
  curl: string;
}

export interface ParsedCurl {
  method: string;
  url: string;
  headers: ApiKeyVal[];
  query: ApiKeyVal[];
  body_mode: ApiBodyMode;
  body: string;
  auth: ApiAuth;
}

export interface ApiAssertion {
  kind: 'status' | 'json_path' | 'header' | 'body_text' | 'duration_ms';
  /** JSON path into the response body (kind='json_path'), or the header name
   *  (kind='header', case-insensitive). */
  path?: string;
  /** A missing target (absent field/header, no response) fails every op
   *  except `not_exists`. `matches` is a regex (≤1000 chars). */
  op: 'eq' | 'ne' | 'contains' | 'lt' | 'gt' | 'lte' | 'gte' | 'exists' | 'not_exists' | 'matches';
  /** Ignored for `exists` / `not_exists`. */
  value: string;
}

export interface ApiExtract {
  /** JSON path into the response body, `header:<name>` for a response
   *  header, or `status`. A miss fails the step ("Save {{var}} from …: not
   *  found"). */
  path: string;
  /** Environment variable to set from the extracted value (used by later steps). */
  var: string;
}

export interface ApiAutomationStep {
  request_id: Id;
  assertions: ApiAssertion[];
  extract: ApiExtract[];
}

export interface ApiAutomation {
  id: Id;
  workspace_id: Id;
  name: string;
  steps: ApiAutomationStep[];
  created_at: string;
}

export interface UpsertApiAutomationReq {
  name: string;
  steps?: ApiAutomationStep[];
}

export interface ApiRunStepResult {
  request_id: Id;
  name: string;
  status: number | null;
  duration_ms: number;
  ok: boolean;
  /** `desc` is worded with the actual value ("Status code is less than 400:
   *  got 500"); `actual` is that value (null when missing). Extraction misses
   *  and script tests appear here too. */
  assertions: { desc: string; passed: boolean; actual?: unknown }[];
  error: string | null;
}

export interface ApiRunResult {
  automation_id: Id;
  steps: ApiRunStepResult[];
  passed: boolean;
}
