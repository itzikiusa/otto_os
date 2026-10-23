// The API client's model: the request draft the builder edits, the per-tab
// response slot, and the pure helpers both the store and its parts use
// (draft ⇄ persisted extras, tab-blob sanitising, history → response). Split
// out of apiClient.svelte.ts (arch-6-03), which re-exports every public name,
// so importers keep `from '…/stores/apiClient.svelte'`.

import { confirmer } from '../../confirm.svelte';
import type {
  ApiAuth, ApiBodyMode, ApiEnvironment, ApiHistoryEntry, ApiKeyVal, ApiOverviewRequest, ApiRequest, ApiRequestItem,
  ApiRequestExtras, ApiResponse, ApiSecretable, Id,
} from '../../api/types';
import { isSecretRef } from '../../api/types';
import type { TestResult } from '../../api/scripts';

/** Request transport: classic HTTP, server-sent events, WebSocket, or gRPC. */
export type ApiRequestKind = 'http' | 'sse' | 'websocket' | 'grpc';

/** Per-request execution settings (Settings tab). */
export interface ApiSettings {
  /** Request timeout in ms; null = daemon default (60s). */
  timeout_ms: number | null;
  follow_redirects: boolean;
  /** Verify TLS certificates (off = accept self-signed / invalid). */
  verify_ssl: boolean;
}

export function defaultSettings(): ApiSettings {
  return { timeout_ms: null, follow_redirects: true, verify_ssl: true };
}

/** A cookie from the daemon-global jar. */
export interface ApiCookie {
  name: string;
  value: string;
  domain: string;
  path: string;
}

/** The editable request the builder/panel work on (a request not yet saved). */
export interface ApiDraft {
  /** Stable local ownership key, independent of the saved request id. */
  tabId?: string;
  /** When the draft came from a saved request, its id (for "Save" = update). */
  requestId: Id | null;
  name: string;
  /** Transport kind (UI-only; HTTP executes via /execute, others stream). */
  kind: ApiRequestKind;
  method: string;
  url: string;
  headers: ApiKeyVal[];
  query: ApiKeyVal[];
  body_mode: ApiBodyMode;
  body: string;
  auth: ApiAuth;
  /** gRPC: the uploaded .proto source (parsed by the daemon on demand). */
  proto?: string;
  /** gRPC: selected "package.Service/Method". */
  grpc_method?: string;
  /** Per-request execution settings (optional; defaults applied when absent). */
  settings?: ApiSettings;
  /** Route the request through this `ssh`-kind connection (SOCKS5 over SSH),
   * so it egresses from the bastion's whitelisted IP. null = send directly. */
  ssh_connection_id?: Id | null;
  /** Pre-request JS (runs before send; can mutate request + set variables). */
  pre_request_script?: string;
  /** Post-response JS (runs after; reads response, sets variables, tests). */
  post_response_script?: string;
  /** GraphQL variables (JSON) — combined with the query body when body_mode=graphql. */
  graphql_variables?: string;
  /** Free-form Markdown documentation for this request. */
  docs?: string;
  /** UI-only: the collection a new draft was started in ("New request here"),
   *  preselected by the save sheet. Never sent to the daemon. */
  collectionHint?: Id | null;
  /** Incomplete historical input is not a runnable/savable request. */
  historyReplay?: { reason: string; body?: string; requestId?: Id };
}

export const HTTP_METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'];

export function blankDraft(): ApiDraft {
  return {
    tabId: crypto.randomUUID(),
    requestId: null,
    name: '',
    kind: 'http',
    method: 'GET',
    url: '',
    headers: [],
    query: [],
    body_mode: 'none',
    body: '',
    auth: { type: 'none' },
    proto: '',
    grpc_method: '',
  };
}

/** What a send would carry, for the new-host confirmation. */
export interface NewHostContext {
  method?: string;
  url?: string;
  /** Plain-language names of the stored secrets involved. */
  secrets?: string[];
}

/** Ask the person whether a stored secret may go to `host` (the daemon's
 *  `409 needs_confirm=new_host`). Only a person can confirm — agents can't.
 *  Says WHERE (host + method/URL), WHAT (which stored secret) and the risk. */
export function confirmNewHost(host: string, ctx: NewHostContext = {}): Promise<boolean> {
  const where = host || 'this host';
  const lines = [
    `${where} hasn’t received this stored secret before. Secrets are bound to the hosts of the requests they were saved with.`,
    '',
  ];
  if (ctx.method || ctx.url) lines.push(`Request: ${[ctx.method, ctx.url].filter(Boolean).join(' ')}`);
  lines.push(`Secret sent: ${ctx.secrets?.length ? ctx.secrets.join(', ') : 'a Keychain credential or a secret environment variable'}`);
  lines.push('', `Only continue if you trust ${where}: it will be able to read and reuse the secret.`);
  return confirmer.ask(lines.join('\n'), {
    title: 'Send a secret to a new host?',
    confirmLabel: 'Send anyway',
    danger: true,
  });
}

/** Stored secrets a draft would send: Keychain-backed auth members plus
 *  secret environment variables it references (names only, never values). */
export function draftSecrets(d: ApiDraft, env: ApiEnvironment | null): string[] {
  const out: string[] = [];
  const auth = d.auth as unknown as Record<string, unknown>;
  const LABEL: Record<string, string> = {
    token: 'bearer token', password: 'password', value: 'API key', client_secret: 'client secret',
    refresh_token: 'refresh token', access_token: 'access token',
  };
  for (const [k, v] of Object.entries(auth)) {
    if (isSecretRef(v as ApiSecretable)) out.push(`the saved ${LABEL[k] ?? k} (Keychain)`);
  }
  if (env?.secret_keys.length) {
    const text = [d.url, d.body, ...d.headers.map((h) => h.value), ...d.query.map((q) => q.value),
      ...Object.values(auth).filter((v): v is string => typeof v === 'string')].join('\n');
    for (const k of env.secret_keys) if (text.includes(`{{${k}}}`)) out.push(`{{${k}}} from “${env.name}”`);
  }
  return out;
}

/** Drop empty/disabled key-vals before sending; keep enabled (default true). */
export function liveKv(rows: ApiKeyVal[]): ApiKeyVal[] {
  return rows.filter((r) => r.enabled !== false && r.key.trim() !== '');
}

/** Masked rendering for exports/snippets: markers become `***`. */
export function secretMasked(v: ApiSecretable | undefined | null): string {
  if (isSecretRef(v)) return '***';
  return typeof v === 'string' ? v : '';
}

/** Serialize the draft's once-draft-only fields into the persisted `extras`
 * object. An explicit empty v1 object clears previously saved extras. */
export function draftToExtras(d: ApiDraft): ApiRequestExtras {
  const extras: ApiRequestExtras = { v: 1 };
  if (d.kind && d.kind !== 'http') { extras.transport = d.kind; }
  if (d.graphql_variables?.trim()) { extras.graphql_variables = d.graphql_variables; }
  if (d.docs?.trim()) { extras.docs_md = d.docs; }
  const pre = d.pre_request_script?.trim() ? d.pre_request_script : undefined;
  const post = d.post_response_script?.trim() ? d.post_response_script : undefined;
  if (pre || post) { extras.scripts = { ...(pre ? { pre } : {}), ...(post ? { post } : {}) }; }
  const s = d.settings;
  if (s && (s.timeout_ms != null || !s.follow_redirects || !s.verify_ssl)) {
    extras.settings = {
      timeout_ms: s.timeout_ms,
      follow_redirects: s.follow_redirects,
      tls_verify: s.verify_ssl,
    };
  }
  if (d.kind === 'grpc') extras.grpc = {proto: d.proto ?? '', method: d.grpc_method ?? ''};
  return extras;
}

/** Restore persisted `extras` onto a draft loaded from a saved request. */
export function extrasToDraft(d: ApiDraft, extras: ApiRequestExtras | null | undefined): ApiDraft {
  if (!extras) return d;
  const kinds: ApiRequestKind[] = ['http', 'sse', 'websocket', 'grpc'];
  const out = { ...d };
  if (extras.transport && kinds.includes(extras.transport)) out.kind = extras.transport;
  if (typeof extras.graphql_variables === 'string') out.graphql_variables = extras.graphql_variables;
  if (typeof extras.docs_md === 'string') out.docs = extras.docs_md;
  if (extras.scripts?.pre) out.pre_request_script = extras.scripts.pre;
  if (extras.scripts?.post) out.post_response_script = extras.scripts.post;
  if (extras.grpc) { out.proto = extras.grpc.proto; out.grpc_method = extras.grpc.method; }
  if (extras.settings) {
    out.settings = {
      timeout_ms: extras.settings.timeout_ms ?? null,
      follow_redirects: extras.settings.follow_redirects ?? true,
      verify_ssl: extras.settings.tls_verify ?? true,
    };
  }
  return out;
}

// ── Open-tab persistence ─────────────────────────────────────────────────────
// Open request tabs survive app relaunches: the full drafts + active index are
// written per-workspace to localStorage (same pattern as the Git page's open
// repo tabs) and restored by loadAll(). Two windows on the same workspace are
// last-write-wins per debounce window — acceptable for a device-local editor.

export function tabsKey(wid: Id): string {
  return `otto_api_tabs_v1:${wid}`;
}
/** Re-entering the API page within this long of a successful load reuses it. */
export const RELOAD_FRESH_MS = 60_000;
/** Full saved requests kept after open (tabs' rows are always kept). */
export const FULL_REQUESTS_MAX = 64;

/** The tree/search row of a full saved request (see `ApiRequestItem`). */
export function requestItem(r: ApiRequest): ApiRequestItem {
  return {
    id: r.id,
    workspace_id: r.workspace_id,
    name: r.name,
    method: r.method,
    url: r.url,
    collection_id: r.collection_id ?? null,
    auth_type: r.auth?.type ?? 'none',
    has_ssh: !!r.ssh_connection_id,
    agent_authored: !!(r.extras && typeof r.extras === 'object' && 'agent' in r.extras && r.extras.agent),
    updated_at: r.updated_at,
    position: r.position,
  };
}

/** `requests/summaries` rows → items (`position` falls back to list order,
 *  which the daemon sorts by position, for daemons that omit it). */
export function summaryItems(rows: ApiOverviewRequest[], wid: Id): ApiRequestItem[] {
  return rows.map((r, i) => ({ ...r, workspace_id: wid, position: r.position ?? i }));
}

/** Debounce for tab writes — the draft setter fires on every keystroke. */
export const TABS_WRITE_DELAY_MS = 250;
/** Body ceiling for the quota-exceeded fallback rewrite (chars). */
export const TABS_FALLBACK_BODY_MAX = 200_000;

export interface PersistedTabs {
  tabs: ApiDraft[];
  active: number;
}

const DRAFT_KINDS: ApiRequestKind[] = ['http', 'sse', 'websocket', 'grpc'];
const BODY_MODES: ApiBodyMode[] = ['none', 'json', 'raw', 'form', 'multipart', 'graphql'];

/** Rebuild a draft from an untrusted persisted blob; null when hopeless.
 *  Lenient by design (schema drift across versions must not lose a tab), but
 *  every field the builder renders is coerced to a safe shape. */
export function sanitizeDraft(raw: unknown): ApiDraft | null {
  if (typeof raw !== 'object' || raw === null) return null;
  const d = raw as Partial<ApiDraft>;
  const str = (v: unknown, fb: string): string => (typeof v === 'string' ? v : fb);
  const kv = (rows: unknown): ApiKeyVal[] =>
    Array.isArray(rows)
      ? rows.filter(
          (r): r is ApiKeyVal =>
            typeof r === 'object' && r !== null &&
            typeof (r as ApiKeyVal).key === 'string' &&
            typeof (r as ApiKeyVal).value === 'string',
        )
      : [];
  return {
    ...blankDraft(),
    ...d,
    tabId: typeof d.tabId === 'string' ? d.tabId : crypto.randomUUID(),
    requestId: typeof d.requestId === 'string' ? d.requestId : null,
    name: str(d.name, ''),
    kind: DRAFT_KINDS.includes(d.kind as ApiRequestKind) ? (d.kind as ApiRequestKind) : 'http',
    method: str(d.method, 'GET') || 'GET',
    url: str(d.url, ''),
    headers: kv(d.headers),
    query: kv(d.query),
    body_mode: BODY_MODES.includes(d.body_mode as ApiBodyMode) ? (d.body_mode as ApiBodyMode) : 'none',
    body: str(d.body, ''),
    auth:
      typeof d.auth === 'object' && d.auth !== null && typeof (d.auth as ApiAuth).type === 'string'
        ? (d.auth as ApiAuth)
        : { type: 'none' },
  };
}

/** A non-negative integer setting, else 0 ("no limit"). */
export const countOr0 = (v: unknown): number => (typeof v === 'number' && Number.isInteger(v) && v >= 0 ? v : 0);

/** One request tab's response state (see "Per-tab response slots"). */
export interface TabResponse {
  resp: ApiResponse | null;
  error: string | null;
  tests: TestResult[];
  logs: string[];
  sending: boolean;
  ctrl: AbortController | null;
  /** When the slot last changed (LRU order). */
  at: number;
  /** Set when `resp`/`error` was restored from a history entry rather than
   *  sent in this session: when it ran, and whether only a preview of the
   *  body was stored. */
  history: { at: string; truncated: boolean } | null;
}

/** Idle response slots kept beyond the visible one (LRU-evicted). */
export const MAX_TAB_RESPONSES = 20;

export function emptySlot(): TabResponse {
  return { resp: null, error: null, tests: [], logs: [], sending: false, ctrl: null, at: 0, history: null };
}

/** A history entry's stored response as a displayable `ApiResponse` (or the
 *  stored error). The stored body is a ≤64 KB preview; a `body_id` from the
 *  original send has long expired, so it is dropped. */
export function historyResponse(h: Pick<ApiHistoryEntry, 'response' | 'status' | 'duration_ms'>): { resp: ApiResponse | null; error: string | null } {
  const r = h.response;
  if (!r || typeof r !== 'object') return { resp: null, error: null };
  const o = r as Record<string, unknown>;
  if (typeof o.status !== 'number') {
    return { resp: null, error: typeof o.error === 'string' ? o.error : null };
  }
  const body = typeof o.body === 'string' ? o.body : '';
  const headers = Array.isArray(o.headers) ? (o.headers as ApiKeyVal[]) : [];
  return {
    error: null,
    resp: {
      status: o.status,
      status_text: typeof o.status_text === 'string' ? o.status_text : '',
      headers,
      body,
      body_base64: typeof o.body_base64 === 'string' ? o.body_base64 : '',
      body_id: null,
      truncated: o.truncated === true,
      too_large: o.too_large === true,
      duration_ms: typeof o.duration_ms === 'number' ? o.duration_ms : (h.duration_ms ?? 0),
      size_bytes: typeof o.size_bytes === 'number' ? o.size_bytes : body.length,
      content_type: typeof o.content_type === 'string' ? o.content_type
        : headers.find((x) => x.key.toLowerCase() === 'content-type')?.value ?? null,
      trace: Array.isArray(o.trace) ? (o.trace as ApiResponse['trace']) : [],
    },
  };
}
