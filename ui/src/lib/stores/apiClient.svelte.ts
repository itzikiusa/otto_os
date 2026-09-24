import type { ApiAutomationRun, StartApiAutomationRunReq } from '../api/types';
// API client ("Postman") store — workspace-scoped collections, requests,
// environments, history, plus a live "draft" request the builder edits and
// executes through the daemon. Reads `ws.currentId` only (never mutates it).

import { api, isAbortError, newHostConfirmHost } from '../api/client';
import { confirmer } from '../confirm.svelte';
import type {
  ApiAuth,
  ApiAutomation,
  ApiBodyMode,
  ApiCollection,
  ApiEnvironment,
  ApiHistoryEntry,
  ApiHistorySummary,
  ApiKeyVal,
  ApiRequest,
  ApiRequestExtras,
  ApiResponse,
  ApiRunResult,
  ApiSecretable,
  Connection,
  ExecuteApiReq,
  Id,
  ImportCurlReq,
  ParsedCurl,
  UpsertApiAutomationReq,
  UpsertApiCollectionReq,
  UpsertApiEnvironmentReq,
  UpsertApiRequestReq,
} from '../api/types';
import { isSecretRef } from '../api/types';
import { forDuplicate, stripSecretsForStorage, unmaskHistory } from '../api/apiSecretShapes';
import { ws } from './workspace.svelte';
import { HistoryRefresh, HistoryDetail } from './apiHistory';
import { toasts } from '../toast.svelte';
import type { PreRequestReq, TestResult } from '../api/scripts';
import { runScript } from '../api/scriptRunner';
import {
  detectAndParse,
  collectionToPostman,
  isImportedEnvironment,
  type ImportedDoc,
  type ImportedEnvironment,
} from '../api/importers';

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
}

export const HTTP_METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'];

function blankDraft(): ApiDraft {
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
    `${where} hasn't received this stored secret before. Secrets are bound to the hosts of the requests they were saved with.`,
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
function liveKv(rows: ApiKeyVal[]): ApiKeyVal[] {
  return rows.filter((r) => r.enabled !== false && r.key.trim() !== '');
}

/** Masked rendering for exports/snippets: markers become `***`. */
function secretMasked(v: ApiSecretable | undefined | null): string {
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
function extrasToDraft(d: ApiDraft, extras: ApiRequestExtras | null | undefined): ApiDraft {
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

function tabsKey(wid: Id): string {
  return `otto_api_tabs_v1:${wid}`;
}
/** Debounce for tab writes — the draft setter fires on every keystroke. */
const TABS_WRITE_DELAY_MS = 250;
/** Body ceiling for the quota-exceeded fallback rewrite (chars). */
const TABS_FALLBACK_BODY_MAX = 200_000;

interface PersistedTabs {
  tabs: ApiDraft[];
  active: number;
}

const DRAFT_KINDS: ApiRequestKind[] = ['http', 'sse', 'websocket', 'grpc'];
const BODY_MODES: ApiBodyMode[] = ['none', 'json', 'raw', 'form', 'multipart', 'graphql'];

/** Rebuild a draft from an untrusted persisted blob; null when hopeless.
 *  Lenient by design (schema drift across versions must not lose a tab), but
 *  every field the builder renders is coerced to a safe shape. */
function sanitizeDraft(raw: unknown): ApiDraft | null {
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

function errMsg(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

class ApiClientStore {
  collections: ApiCollection[] = $state([]);
  requests: ApiRequest[] = $state([]);
  environments: ApiEnvironment[] = $state([]);
  history: ApiHistorySummary[] = $state([]);
  historyLoadingId: string | null = $state(null);
  historyAgentOnly = $state(false);
  automations: ApiAutomation[] = $state([]);
  /** Workspace `ssh`-kind connections, for the Settings-tab "SSH tunnel" picker. */
  sshConnections: Connection[] = $state([]);

  /** Last runAutomation() report, shown in the run panel. */
  lastRun: ApiRunResult | null = $state(null);
  /** In-flight automation run. */
  running = $state(false);
  automationRuns: ApiAutomationRun[] = $state([]);
  currentRun: ApiAutomationRun | null = $state(null);

  /** Open request tabs; the active one is edited via `draft`. */
  tabs: ApiDraft[] = $state([blankDraft()]);
  activeTab = $state(0);
  /** The request currently in the builder (proxies the active tab). */
  get draft(): ApiDraft {
    return this.tabs[this.activeTab] ?? this.tabs[0];
  }
  set draft(d: ApiDraft) {
    this.tabs[this.activeTab] = {...d, tabId: d.tabId ?? this.tabs[this.activeTab]?.tabId ?? crypto.randomUUID()};
    this.persistTabs();
  }
  /** A short label for a tab. */
  tabLabel(d: ApiDraft): string {
    if (d.name?.trim()) return d.name;
    try {
      const u = new URL(d.url);
      return `${d.method} ${u.pathname || u.host}`;
    } catch {
      return d.url?.trim() ? `${d.method} ${d.url}` : 'New Request';
    }
  }
  openTab(d: ApiDraft = blankDraft()): void {
    this.tabs = [...this.tabs, {...d, tabId: crypto.randomUUID()}];
    this.activeTab = this.tabs.length - 1;
    this.lastResponse = null;
    this.lastError = null;
    this.persistTabs();
  }
  switchTab(i: number): void {
    if (i >= 0 && i < this.tabs.length) {
      this.activeTab = i;
      this.lastResponse = null;
      this.lastError = null;
      this.persistTabs();
    }
  }
  closeTab(i: number): void {
    if (this.tabs[i]?.tabId === this._executeTab) this.cancelExecute();
    if (this.tabs.length === 1) {
      this.tabs = [blankDraft()];
      this.activeTab = 0;
    } else {
      this.tabs = this.tabs.filter((_, idx) => idx !== i);
      if (this.activeTab >= this.tabs.length) this.activeTab = this.tabs.length - 1;
      else if (i < this.activeTab) this.activeTab -= 1;
    }
    this.lastResponse = null;
    this.lastError = null;
    this.persistTabs();
  }

  // ── Open-tab persistence (see module header above sanitizeDraft) ──────────

  /** Workspace whose tabs are in memory; null until the first restore. Gates
   *  persistence so a not-yet-restored blank tab can't clobber a saved set. */
  private tabsWid: Id | null = null;
  private tabsWriteTimer: ReturnType<typeof setTimeout> | null = null;

  constructor() {
    // Flush a pending debounced write when the window/app goes away, so the
    // last keystrokes before quit are not lost.
    if (typeof window !== 'undefined') {
      window.addEventListener('pagehide', () => this.flushTabsWrite());
    }
  }

  /** Queue a debounced write of the open tabs to this workspace's slot. */
  private persistTabs(): void {
    if (this.tabsWid === null) return;
    if (this.tabsWriteTimer !== null) clearTimeout(this.tabsWriteTimer);
    this.tabsWriteTimer = setTimeout(() => this.flushTabsWrite(), TABS_WRITE_DELAY_MS);
  }

  /** Write immediately (debounce elapsed, workspace swap, or page hide). */
  private flushTabsWrite(): void {
    if (this.tabsWriteTimer !== null) {
      clearTimeout(this.tabsWriteTimer);
      this.tabsWriteTimer = null;
    }
    if (this.tabsWid === null || typeof localStorage === 'undefined') return;
    const key = tabsKey(this.tabsWid);
    // Never at rest outside the Keychain: plaintext credentials typed into a
    // tab (or imported from curl / fetched via OAuth) are stripped from the
    // persisted copy (see stripSecretsForStorage).
    const blob: PersistedTabs = {
      tabs: ($state.snapshot(this.tabs) as ApiDraft[]).map((t) =>
        stripSecretsForStorage(t, t.requestId ? this.requests.find((r) => r.id === t.requestId) : undefined),
      ),
      active: this.activeTab,
    };
    try {
      localStorage.setItem(key, JSON.stringify(blob));
    } catch {
      // Quota: retry once without the heavyweight fields (proto uploads, huge
      // bodies) — a slim tab set beats losing the whole set.
      try {
        const slim = blob.tabs.map((t) => ({
          ...t,
          proto: '',
          body: t.body.length > TABS_FALLBACK_BODY_MAX ? '' : t.body,
        }));
        localStorage.setItem(key, JSON.stringify({ tabs: slim, active: blob.active }));
      } catch {
        /* storage unavailable — non-fatal */
      }
    }
  }

  /** Swap in `wid`'s persisted tabs. No-op when already showing them; flushes
   *  the previous workspace's pending write first so nothing is lost. */
  private restoreTabs(wid: Id): void {
    if (this.tabsWid === wid) return;
    this.historyRefresh.reset();
    this.historyDetail.cancel();
    this.history = [];
    this.flushTabsWrite();
    this.cancelExecute();
    this.running = false; this.lastRun = null; this.currentRun = null; this.automationRuns = [];
    this.sending = false;
    this.scriptLogs = []; this.testResults = [];
    this.tabsWid = wid;
    let next: ApiDraft[] = [];
    let active = 0;
    try {
      const raw = localStorage.getItem(tabsKey(wid));
      if (raw) {
        const p = JSON.parse(raw) as Partial<PersistedTabs>;
        next = (Array.isArray(p.tabs) ? p.tabs : [])
          .map(sanitizeDraft)
          .filter((d): d is ApiDraft => d !== null);
        if (typeof p.active === 'number' && Number.isFinite(p.active)) active = Math.trunc(p.active);
      }
    } catch {
      /* corrupt/unavailable → start fresh */
    }
    if (next.length === 0) next = [blankDraft()];
    this.tabs = next;
    this.activeTab = Math.min(Math.max(0, active), next.length - 1);
    this.lastResponse = null;
  }
  /** Last execute() result, shown in the ResponseViewer. */
  lastResponse: ApiResponse | null = $state(null);
  /** Why the active tab's last send failed (shown inline in the response
   *  pane instead of the previous response); null after a success. */
  lastError: string | null = $state(null);
  /** In-flight send. */
  sending = $state(false);
  loading = $state(false);
  /** Why the last loadAll() failed (shown inline with Retry); null when fine. */
  loadError: string | null = $state(null);
  /** AbortController for the currently in-flight execute() call; null when idle. */
  private _abortCtrl: AbortController | null = null;
  private _executeTab: string | null = null;
  /** Cancel the in-flight HTTP request (if any). No-op when idle. */
  cancelExecute(): void {
    this._abortCtrl?.abort();
    this._abortCtrl = null;
    this._executeTab = null;
    this.sending = false;
  }

  /** Active environment (is_active), or null. */
  activeEnv: ApiEnvironment | null = $derived(
    this.environments.find((e) => e.is_active) ?? null,
  );

  private wsId(): Id | null {
    return ws.currentId;
  }

  private base(): string | null {
    const id = this.wsId();
    return id ? `/workspaces/${id}/api-client` : null;
  }

  // ── Loading ───────────────────────────────────────────────────────────────

  /** Load everything for the current workspace (collections + requests + envs + history). */
  async loadAll(): Promise<void> {
    const wid = this.wsId();
    const base = this.base();
    if (!wid || !base) return;
    // Restore this workspace's persisted open tabs up front (works even when
    // the fetches below fail — the drafts are device-local, not server data).
    this.restoreTabs(wid);
    this.loading = true;
    this.loadError = null;
    try {
      const [collections, requests, environments] = await Promise.all([
        api.get<ApiCollection[]>(`${base}/collections`),
        api.get<ApiRequest[]>(`${base}/requests`),
        api.get<ApiEnvironment[]>(`${base}/environments`),
        this.loadHistory(),
      ]);
      if (this.wsId() !== wid) return;
      this.collections = collections;
      this.requests = requests;
      this.environments = environments;
      // Unlink restored drafts whose saved request no longer exists, so their
      // "Save" creates anew instead of PATCHing a deleted id.
      const live = new Set(this.requests.map((r) => r.id));
      if (this.tabs.some((t) => t.requestId && !live.has(t.requestId))) {
        this.tabs = this.tabs.map((t) =>
          t.requestId && !live.has(t.requestId) ? { ...t, requestId: null } : t,
        );
        this.persistTabs();
      }
    } catch (e) {
      if (this.wsId() === wid) this.loadError = errMsg(e);
    } finally {
      if (this.wsId() === wid) this.loading = false;
    }
    if (this.wsId() === wid) void this.loadSshConnections();
  }

  /** Load the workspace's `ssh`-kind connections for the SSH-tunnel picker.
   * Best-effort: a Viewer without connections access just sees an empty list. */
  async loadSshConnections(): Promise<void> {
    const wid = this.wsId();
    if (!wid) return;
    try {
      const all = await api.get<Connection[]>(`/workspaces/${wid}/connections`);
      if (this.wsId() === wid) this.sshConnections = all.filter((c) => c.kind === 'ssh');
    } catch {
      this.sshConnections = [];
    }
  }

  async loadCollections(): Promise<void> {
    const base = this.base();
    if (!base) return;
    try {
      this.collections = await api.get<ApiCollection[]>(`${base}/collections`);
    } catch (e) {
      toasts.error('Could not load collections', errMsg(e));
    }
  }

  async loadRequests(): Promise<void> {
    const base = this.base();
    if (!base) return;
    try {
      this.requests = await api.get<ApiRequest[]>(`${base}/requests`);
    } catch (e) {
      toasts.error('Could not load requests', errMsg(e));
    }
  }

  async loadEnvironments(): Promise<void> {
    const base = this.base();
    if (!base) return;
    try {
      this.environments = await api.get<ApiEnvironment[]>(`${base}/environments`);
    } catch (e) {
      toasts.error('Could not load environments', errMsg(e));
    }
  }

  private historyRefresh = new HistoryRefresh<ApiHistorySummary>({
    workspace: () => this.wsId(),
    fetch: (wid, signal) => api.get<ApiHistorySummary[]>(`/workspaces/${wid}/api-client/history/summaries`, signal),
    publish: (rows) => { this.history = rows; },
    error: (error) => toasts.error('Could not load history', errMsg(error)),
  });

  private historyDetail = new HistoryDetail<ApiHistoryEntry>({
    owner: () => ({workspace: this.wsId(), tab: this.draft.tabId, draft: this.draft}),
    fetch: (id, signal) => api.get<ApiHistoryEntry>(`${this.base()}/history/${encodeURIComponent(id)}`, signal),
    publish: (entry) => this.loadHistoryIntoDraft(entry),
    pending: (id) => { this.historyLoadingId = id; },
    error: (error) => toasts.error('Could not load history request', errMsg(error)),
  });

  async loadHistory(): Promise<void> {
    await this.historyRefresh.request();
  }

  /** Direct sends and WS append events share one metadata-only refresh. */
  noteHistoryAppended(workspaceId: string, entryId?: string): void {
    if (workspaceId === this.wsId()) void this.historyRefresh.request(entryId);
  }

  historySource(h: ApiHistorySummary): ApiHistorySummary['source'] {
    return h.source;
  }

  async selectHistory(id: string): Promise<void> {
    if (this.wsId()) await this.historyDetail.select(id);
  }

  // ── Cookie jar (daemon-global) ────────────────────────────────────────────
  cookies: ApiCookie[] = $state([]);
  async loadCookies(): Promise<void> {
    const base = this.base();
    if (!base) return;
    try {
      this.cookies = await api.get<ApiCookie[]>(`${base}/cookies`);
    } catch (e) {
      toasts.error('Could not load cookies', errMsg(e));
    }
  }
  async clearCookies(): Promise<void> {
    const base = this.base();
    if (!base) return;
    try {
      await api.del(`${base}/cookies`);
      this.cookies = [];
      toasts.success('Cookies cleared');
    } catch (e) {
      toasts.error('Clear cookies failed', errMsg(e));
    }
  }

  /** One-pass sweep moving every plaintext secret (request auth members +
   * secret-shaped environment variables) into the macOS Keychain. */
  async secureAll(): Promise<void> {
    const base = this.base();
    if (!base) return;
    try {
      const res = await api.post<{ requests_secured: number; env_keys_secured: number }>(
        `${base}/secure-all`, {},
      );
      toasts.success(
        'Secrets secured',
        `${res.requests_secured} request(s) · ${res.env_keys_secured} env value(s) moved to the Keychain`,
      );
      // Rows were rewritten (markers / stripped variables) — refresh both.
      void this.loadRequests();
      void this.loadEnvironments();
    } catch (e) {
      toasts.error('Secure secrets failed', errMsg(e));
    }
  }

  // ── Collections ─────────────────────────────────────────────────────────

  async saveCollection(req: UpsertApiCollectionReq, id?: Id): Promise<ApiCollection | null> {
    const base = this.base();
    if (!base) return null;
    try {
      const saved = id
        ? await api.patch<ApiCollection>(`${base}/collections/${id}`, req)
        : await api.post<ApiCollection>(`${base}/collections`, req);
      this.collections =
        id != null && this.collections.some((c) => c.id === saved.id)
          ? this.collections.map((c) => (c.id === saved.id ? saved : c))
          : [...this.collections, saved];
      return saved;
    } catch (e) {
      toasts.error('Save collection failed', errMsg(e));
      return null;
    }
  }

  /** Create a collection (+ nested folders + requests) from an imported doc.
   *  A Postman ENVIRONMENT export routes to {@link importEnvironment} instead.
   *  `quiet` suppresses the per-item success toast (bulk account sync). */
  async importParsed(parsed: ImportedDoc, quiet = false): Promise<void> {
    if (ws.myRole === 'viewer') {
      toasts.error('Read-only', 'You have viewer access to this workspace');
      return;
    }
    if (isImportedEnvironment(parsed)) {
      await this.importEnvironment(parsed, quiet);
      return;
    }
    const root = await this.saveCollection({ name: parsed.name, parent_id: null });
    if (!root) return;
    const folderCache = new Map<string, Id>();
    for (const req of parsed.requests) {
      let parentId: Id = root.id;
      let pathKey = '';
      for (const folder of req.folderPath) {
        pathKey += '/' + folder;
        if (!folderCache.has(pathKey)) {
          const f = await this.saveCollection({ name: folder, parent_id: parentId });
          if (f) folderCache.set(pathKey, f.id);
        }
        parentId = folderCache.get(pathKey) ?? parentId;
      }
      await this.saveRequest({
        collection_id: parentId, name: req.name, method: req.method, url: req.url,
        headers: req.headers, query: req.query, body_mode: req.body_mode, body: req.body, auth: req.auth,
        extras: req.extras ?? null,
      });
    }
    await this.loadRequests();
    if (!quiet) {
      toasts.success('Imported', `${parsed.name} · ${parsed.requests.length} request(s) (${parsed.format})`);
    }
  }

  /** Import a Postman environment export as a new API environment. */
  async importEnvironment(env: ImportedEnvironment, quiet = false): Promise<void> {
    const saved = await this.saveEnvironment({
      name: env.name,
      variables: env.variables,
    });
    if (saved && !quiet) {
      toasts.success(
        'Environment imported',
        `${env.name} · ${Object.keys(env.variables).length} variable(s)`,
      );
    }
  }

  /** One-shot Postman ACCOUNT sync: the daemon fetches every collection +
   *  environment via the Postman API (api.getpostman.com) and this imports
   *  them all through the normal import pipeline — no per-collection manual
   *  export. Returns true when the sync call itself succeeded. */
  async postmanSync(apiKey: string, remember: boolean): Promise<boolean> {
    const base = this.base();
    if (!base) return false;
    let res: {
      collections: unknown[];
      environments: { name?: string }[];
      failed: { name: string; error: string }[];
    };
    try {
      res = await api.post(`${base}/postman/sync`, {
        api_key: apiKey || null,
        remember,
      });
    } catch (e) {
      toasts.error('Postman sync failed', errMsg(e));
      return false;
    }
    let cols = 0;
    let envs = 0;
    const failed = [...res.failed];
    for (const doc of res.collections) {
      const name =
        (doc as { info?: { name?: string } }).info?.name ?? 'collection';
      try {
        await this.importParsed(detectAndParse(JSON.stringify(doc), 'postman.json'), true);
        cols++;
      } catch (e) {
        failed.push({ name, error: errMsg(e) });
      }
    }
    for (const env of res.environments) {
      try {
        await this.importParsed(
          detectAndParse(JSON.stringify(env), 'postman_environment.json'),
          true,
        );
        envs++;
      } catch (e) {
        failed.push({ name: env.name ?? 'environment', error: errMsg(e) });
      }
    }
    toasts.success(
      'Postman sync complete',
      `${cols} collection(s), ${envs} environment(s) imported`,
    );
    for (const f of failed.slice(0, 3)) {
      toasts.error(`Skipped: ${f.name}`, f.error);
    }
    if (failed.length > 3) {
      toasts.error('More items skipped', `${failed.length - 3} further item(s) failed`);
    }
    return true;
  }

  /** Pull Postman collection files from a connected git repo and import them. */
  async gitPullCollections(repoId: Id): Promise<void> {
    try {
      const res = await api.post<{ files: { name: string; content: string }[] }>(`/repos/${repoId}/api-collections/pull`, {});
      let imported = 0;
      for (const f of res.files) {
        try {
          await this.importParsed(detectAndParse(f.content, f.name));
          imported++;
        } catch { /* skip non-collection json */ }
      }
      toasts.success('Pulled from git', `${imported} collection file(s)`);
    } catch (e) {
      toasts.error('Git pull failed', errMsg(e));
    }
  }

  /** Export all root collections to Postman files and push them to a git repo. */
  async gitPushCollections(repoId: Id, message: string, branch: string | null): Promise<boolean> {
    const roots = this.collections.filter((c) => (c.parent_id ?? null) === null);
    if (roots.length === 0) {
      toasts.error('Nothing to push', 'No collections to export.');
      return false;
    }
    const files = roots.map((c) => ({
      name: `${c.name.replace(/[^\w.-]+/g, '_')}.postman_collection.json`,
      content: JSON.stringify(collectionToPostman(c.id, this.collections, this.requests), null, 2),
    }));
    try {
      const res = await api.post<{ commit: string; files: number }>(`/repos/${repoId}/api-collections/push`, {
        files, message: message || 'Update API collections', branch: branch || null,
      });
      toasts.success('Pushed to git', `${res.files} file(s) · ${res.commit.slice(0, 8)}`);
      return true;
    } catch (e) {
      toasts.error('Git push failed', errMsg(e));
      return false;
    }
  }

  async deleteCollection(id: Id): Promise<void> {
    const base = this.base();
    if (!base) return;
    try {
      await api.del(`${base}/collections/${id}`);
      // Drop the collection + descendant folders; orphan their requests locally.
      const removed = new Set<Id>();
      const collect = (cid: Id): void => {
        removed.add(cid);
        for (const c of this.collections) if (c.parent_id === cid) collect(c.id);
      };
      collect(id);
      this.collections = this.collections.filter((c) => !removed.has(c.id));
      this.requests = this.requests.map((r) =>
        r.collection_id && removed.has(r.collection_id) ? { ...r, collection_id: null } : r,
      );
    } catch (e) {
      toasts.error('Delete collection failed', errMsg(e));
    }
  }

  // ── Requests ────────────────────────────────────────────────────────────

  async saveRequest(req: UpsertApiRequestReq, id?: Id): Promise<ApiRequest | null> {
    const base = this.base();
    if (!base) return null;
    try {
      const saved = id
        ? await api.patch<ApiRequest>(`${base}/requests/${id}`, req)
        : await api.post<ApiRequest>(`${base}/requests`, req);
      if (this.base() !== base) return saved;
      this.requests = this.requests.some((r) => r.id === saved.id)
        ? this.requests.map((r) => (r.id === saved.id ? saved : r))
        : [...this.requests, saved];
      return saved;
    } catch (e) {
      toasts.error('Save request failed', errMsg(e));
      return null;
    }
  }

  /** OAuth may finish after navigation. Update the owning workspace's tabs,
   * keeping auth edits made after the flow started. Saved values are markers. */
  applySavedAuth(wid: Id, request: ApiRequest, previousAuth: ApiAuth): void {
    const before = JSON.stringify(previousAuth);
    const update = (tab: ApiDraft): ApiDraft => tab.requestId === request.id && JSON.stringify(tab.auth) === before
      ? {...tab,auth:{...request.auth}} : tab;
    if (this.tabsWid === wid) {
      this.tabs = this.tabs.map(update); this.persistTabs();
      if (this.wsId() === wid) this.requests = this.requests.map(r => r.id === request.id ? request : r);
    } else {
      try {
        const raw = localStorage.getItem(tabsKey(wid));
        if (raw) {const saved = JSON.parse(raw) as PersistedTabs; if (Array.isArray(saved.tabs)) {
          saved.tabs = saved.tabs.map(update);localStorage.setItem(tabsKey(wid),JSON.stringify(saved));
        }}
      } catch { /* The server retains the saved request if browser storage is unavailable. */ }
    }
  }

  /** Persist the current draft into a collection. Returns the saved request. */
  async saveDraft(name: string, collectionId: Id | null): Promise<ApiRequest | null> {
    const d = this.draft;
    const wid = this.wsId(), tabId = d.tabId;
    const submittedAuth = JSON.stringify(d.auth);
    const body: UpsertApiRequestReq = {
      collection_id: collectionId,
      name,
      method: d.method,
      url: d.url,
      headers: d.headers,
      query: d.query,
      body_mode: d.body_mode,
      body: d.body,
      auth: d.auth,
      ssh_connection_id: d.ssh_connection_id ?? null,
      // Scripts / docs / settings / GraphQL variables / transport persist with
      // the request now — a reload no longer discards them.
      extras: draftToExtras(d),
    };
    const saved = await this.saveRequest(body, d.requestId ?? undefined);
    if (saved) {
      // Re-adopt the SAVED auth: secret members the daemon just moved to the
      // Keychain come back as `$secret` markers, and the builder shows them
      // masked instead of holding plaintext in tab-persisted localStorage.
      if (this.wsId() !== wid) return saved;
      const index = this.tabs.findIndex(t => t.tabId === tabId);
      if (index >= 0) {
        const current = this.tabs[index];
        this.tabs[index] = { ...current, requestId: saved.id, name: saved.name,
          auth: JSON.stringify(current.auth) === submittedAuth ? {...saved.auth} : current.auth };
        this.persistTabs();
      }
      toasts.success('Request saved', saved.name);
    }
    return saved;
  }

  async deleteRequest(id: Id): Promise<void> {
    const base = this.base();
    if (!base) return;
    try {
      await api.del(`${base}/requests/${id}`);
      this.requests = this.requests.filter((r) => r.id !== id);
      if (this.draft.requestId === id) this.draft = { ...this.draft, requestId: null };
    } catch (e) {
      toasts.error('Delete request failed', errMsg(e));
    }
  }

  // ── Environments ────────────────────────────────────────────────────────

  async saveEnvironment(req: UpsertApiEnvironmentReq, id?: Id): Promise<ApiEnvironment | null> {
    const base = this.base();
    if (!base) return null;
    try {
      const saved = id
        ? await api.patch<ApiEnvironment>(`${base}/environments/${id}`, req)
        : await api.post<ApiEnvironment>(`${base}/environments`, req);
      this.environments = this.environments.some((e) => e.id === saved.id)
        ? this.environments.map((e) => (e.id === saved.id ? saved : e))
        : [...this.environments, saved];
      return saved;
    } catch (e) {
      toasts.error('Save environment failed', errMsg(e));
      return null;
    }
  }

  async deleteEnvironment(id: Id): Promise<void> {
    const base = this.base();
    if (!base) return;
    try {
      await api.del(`${base}/environments/${id}`);
      this.environments = this.environments.filter((e) => e.id !== id);
    } catch (e) {
      toasts.error('Delete environment failed', errMsg(e));
    }
  }

  async activateEnvironment(id: Id): Promise<void> {
    const base = this.base();
    if (!base) return;
    try {
      await api.post(`${base}/environments/${id}/activate`, {});
      // Exactly one active env: reflect it locally without a refetch.
      this.environments = this.environments.map((e) => ({ ...e, is_active: e.id === id }));
    } catch (e) {
      toasts.error('Activate environment failed', errMsg(e));
    }
  }

  // ── Execute ─────────────────────────────────────────────────────────────

  /** Runtime/session variables (set by scripts or by hand) sent as overrides. */
  private runtimeScopes: Record<string, Record<string, string>> = $state({});
  get runtimeVars(): Record<string, string> { return this.runtimeScopes[this.wsId() ?? ''] ?? {}; }
  set runtimeVars(vars: Record<string, string>) {
    const id = this.wsId();
    if (id) this.runtimeScopes = {...this.runtimeScopes, [id]: vars};
  }
  setRuntimeVar(key: string, value: string): void {
    this.runtimeVars = { ...this.runtimeVars, [key]: value };
  }
  renameRuntimeVar(oldKey: string, newKey: string): void {
    if (oldKey === newKey) return;
    const next = { ...this.runtimeVars };
    const v = next[oldKey] ?? '';
    delete next[oldKey];
    if (newKey.trim()) next[newKey] = v;
    this.runtimeVars = next;
  }
  removeRuntimeVar(key: string): void {
    const next = { ...this.runtimeVars };
    delete next[key];
    this.runtimeVars = next;
  }
  /** Combined pre/post script console output for the last run. */
  scriptLogs: string[] = $state([]);
  /** Post-response test results for the last run. */
  testResults: TestResult[] = $state([]);

  /** Send the given draft through the daemon. Sets lastResponse + refreshes history. */
  async execute(draft: ApiDraft = this.draft): Promise<ApiResponse | null> {
    const base = this.base();
    if (!base) { toasts.error('No workspace selected'); return null; }
    if (!draft.url.trim()) { toasts.error('URL is empty'); return null; }

    // Own the complete pre → HTTP → post sequence before any user code runs.
    // Snapshot reactive fields so editing another draft cannot alter this send.
    draft = $state.snapshot(draft);
    const wid = this.wsId()!, tabId = draft.tabId ?? this.draft.tabId;
    const environmentId = this.activeEnv?.id ?? null;
    this.cancelExecute();
    const controller = new AbortController();
    this._abortCtrl = controller; this._executeTab = tabId ?? null;
    this.sending = true; this.testResults = []; this.scriptLogs = []; this.lastError = null;
    const { signal } = controller;
    const ownsExecution = () => this._abortCtrl === controller && !signal.aborted
      && this.wsId() === wid && this.tabs.some(tab => tab.tabId === tabId);
    const ownsView = () => ownsExecution() && this.draft.tabId === tabId;
    const checkCurrent = () => { if (!ownsExecution()) throw new DOMException('Request canceled', 'AbortError'); };
    let runtimeVars = {...this.runtimeVars};
    const logs: string[] = [];
    let reqCtx: PreRequestReq = {
      method: draft.method, url: draft.url,
      headers: draft.headers.map(h => ({...h})), body: draft.body,
    };
    try {
      if (draft.pre_request_script?.trim()) {
        const pre = await runScript({ kind:'pre', code:draft.pre_request_script, request:reqCtx, vars:runtimeVars }, signal);
        checkCurrent();
        logs.push(...pre.run.logs.map(l => `[pre] ${l}`));
        if (pre.run.error) throw new Error(`Pre-request script failed: ${pre.run.error}`);
        if (!pre.request) throw new Error('Pre-request script returned no request.');
        reqCtx = pre.request; runtimeVars = pre.vars;
        this.runtimeScopes = {...this.runtimeScopes, [wid]: {...runtimeVars}};
      }
      checkCurrent();
      let effectiveBody = reqCtx.body;
      if (draft.body_mode === 'graphql') {
        let variables: unknown = {};
        try { variables = draft.graphql_variables?.trim() ? JSON.parse(draft.graphql_variables) : {}; } catch { /* Preserve the existing empty-object fallback. */ }
        effectiveBody = JSON.stringify({query:reqCtx.body, variables});
      }
      const settings = draft.settings;
      const body: ExecuteApiReq = {
        method:reqCtx.method, url:reqCtx.url, headers:liveKv(reqCtx.headers), query:liveKv(draft.query),
        body_mode:draft.body_mode, body:effectiveBody, auth:draft.auth,
        environment_id:environmentId,
        timeout_ms:settings?.timeout_ms ?? null, follow_redirects:settings?.follow_redirects ?? true,
        verify_ssl:settings?.verify_ssl ?? true,
        vars:Object.keys(runtimeVars).length ? runtimeVars : undefined,
        ssh_connection_id:draft.ssh_connection_id ?? null,
      };
      let resp: ApiResponse;
      try {
        resp = await api.post<ApiResponse>(`${base}/execute`, body, signal);
      } catch (e) {
        // A stored secret would leave the host it is bound to (e.g. the URL of
        // a saved request was edited before saving): the daemon refuses until
        // a person confirms — ask, then re-send once with the confirmation.
        const host = newHostConfirmHost(e);
        if (host === null) throw e;
        checkCurrent();
        // Show the URL as it will be sent: plain variables filled in (secret
        // values are never resolved client-side).
        const env = this.environments.find((e) => e.id === environmentId) ?? null;
        const shownUrl = reqCtx.url.replace(/\{\{\s*([^{}]+?)\s*\}\}/g, (m, n: string) => runtimeVars[n] ?? env?.variables[n] ?? m);
        const ok = await confirmNewHost(host, { method: reqCtx.method, url: shownUrl, secrets: draftSecrets(draft, env) });
        checkCurrent();
        if (!ok) throw new DOMException('Request canceled', 'AbortError');
        resp = await api.post<ApiResponse>(`${base}/execute`, { ...body, confirm_new_host: true }, signal);
      }
      checkCurrent();
      if (ownsView()) this.lastResponse = resp;
      void this.loadHistory();
      if (draft.post_response_script?.trim()) {
        const headers: Record<string, string> = {};
        for (const h of resp.headers) headers[h.key.toLowerCase()] = h.value;
        const post = await runScript({kind:'post', code:draft.post_response_script,
          response:{code:resp.status,status:resp.status_text,responseTime:resp.duration_ms,headers,bodyText:resp.body}, vars:runtimeVars}, signal);
        checkCurrent();
        logs.push(...post.run.logs.map(l => `[test] ${l}`));
        if (ownsView()) this.testResults = post.run.tests;
        if (post.run.error) logs.push(`[test] error: ${post.run.error}`);
        else this.runtimeScopes = {...this.runtimeScopes, [wid]: {...post.vars}};
      }
      if (ownsView()) this.scriptLogs = logs;
      return resp;
    } catch (e) {
      if (ownsView()) this.scriptLogs = [...logs, `[error] ${errMsg(e)}`];
      if (!signal.aborted && ownsExecution() && !isAbortError(e)) {
        // The response pane shows the failure in place (with the reason); a
        // send finishing on another tab still reports through a toast.
        if (ownsView()) { this.lastError = errMsg(e); this.lastResponse = null; }
        else toasts.error('Request failed', errMsg(e));
      }
      return null;
    } finally {
      if (this._abortCtrl === controller) {
        this.sending = false; this._abortCtrl = null; this._executeTab = null;
      }
    }
  }

  /** Introspected GraphQL schema (types + fields), or null. */
  graphqlSchema: { name: string; kind: string; fields: string[] }[] | null = $state(null);
  graphqlIntrospecting = $state(false);

  /** Run a GraphQL introspection query against the draft URL. */
  async graphqlIntrospect(): Promise<void> {
    const base = this.base();
    if (!base || !this.draft.url.trim()) {
      toasts.error('No URL', 'Enter the GraphQL endpoint URL first.');
      return;
    }
    const q = `query{__schema{queryType{name}mutationType{name}types{name kind fields{name}}}}`;
    this.graphqlIntrospecting = true;
    try {
      const resp = await api.post<ApiResponse>(`${base}/execute`, {
        method: 'POST', url: this.draft.url,
        headers: [{ key: 'Content-Type', value: 'application/json', enabled: true }],
        query: [], body_mode: 'json', body: JSON.stringify({ query: q }),
        auth: this.draft.auth, environment_id: this.activeEnv?.id ?? null,
      });
      const data = JSON.parse(resp.body) as { data?: { __schema?: { types?: { name: string; kind: string; fields?: { name: string }[] }[] } } };
      const types = (data.data?.__schema?.types ?? [])
        .filter((t) => !t.name.startsWith('__') && (t.kind === 'OBJECT' || t.kind === 'INPUT_OBJECT' || t.kind === 'ENUM' || t.kind === 'INTERFACE'))
        .map((t) => ({ name: t.name, kind: t.kind, fields: (t.fields ?? []).map((f) => f.name) }));
      this.graphqlSchema = types;
      toasts.success('Schema introspected', `${types.length} types`);
    } catch (e) {
      toasts.error('Introspection failed', errMsg(e));
    } finally {
      this.graphqlIntrospecting = false;
    }
  }

  // ── Curl import / export ──────────────────────────────────────────────────

  /** Parse a curl command (daemon) and fill the draft with the result. */
  async importCurl(curl: string): Promise<boolean> {
    if (!curl.trim()) return false;
    const req: ImportCurlReq = { curl };
    try {
      const p = await api.post<ParsedCurl>('/api-client/import-curl', req);
      // Fills the current tab only while it holds nothing unsaved; otherwise
      // the import opens in a new tab (placeDraft).
      this.placeDraft({
        tabId: crypto.randomUUID(),
        requestId: null,
        name: this.isDirty(this.draft) ? '' : this.draft.name,
        kind: 'http',
        method: p.method,
        url: p.url,
        headers: p.headers,
        query: p.query,
        body_mode: p.body_mode,
        body: p.body,
        auth: p.auth,
        proto: '',
        grpc_method: '',
      });
      toasts.success('Imported curl', `${p.method} ${p.url}`);
      return true;
    } catch (e) {
      toasts.error('Import curl failed', errMsg(e));
      return false;
    }
  }

  /** Build a curl command string from a draft (for "Copy as curl"). */
  toCurl(draft: ApiDraft = this.draft): string {
    const sh = (s: string): string => `'${s.replace(/'/g, `'\\''`)}'`;
    const parts: string[] = ['curl'];
    if (draft.method && draft.method.toUpperCase() !== 'GET') {
      parts.push('-X', draft.method.toUpperCase());
    }

    // URL with enabled query params appended.
    let url = draft.url;
    const qs = liveKv(draft.query)
      .map((q) => `${encodeURIComponent(q.key)}=${encodeURIComponent(q.value)}`)
      .join('&');
    if (qs) url += (url.includes('?') ? '&' : '?') + qs;
    parts.push(sh(url));

    // Headers.
    for (const h of liveKv(draft.headers)) {
      parts.push('-H', sh(`${h.key}: ${h.value}`));
    }

    // Auth. Keychain-backed members render as `***` — a copied snippet must
    // never leak a resolved secret.
    const a = draft.auth;
    if (a.type === 'bearer' && (isSecretRef(a.token) || a.token)) {
      parts.push('-H', sh(`Authorization: Bearer ${secretMasked(a.token)}`));
    } else if (a.type === 'basic') {
      parts.push('-u', sh(`${a.username}:${secretMasked(a.password)}`));
    } else if (a.type === 'api_key' && a.key) {
      if (a.in === 'header') parts.push('-H', sh(`${a.key}: ${secretMasked(a.value)}`));
      else {
        const sep = url.includes('?') ? '&' : '?';
        // already appended to url above only for query rows; add the api_key here
        parts[parts.indexOf(sh(url))] = sh(
          url + sep + `${encodeURIComponent(a.key)}=${encodeURIComponent(secretMasked(a.value))}`,
        );
      }
    }

    // Body.
    if (draft.body_mode !== 'none' && draft.body.trim()) {
      if (draft.body_mode === 'json') {
        if (!liveKv(draft.headers).some((h) => h.key.toLowerCase() === 'content-type')) {
          parts.push('-H', sh('Content-Type: application/json'));
        }
        parts.push('--data', sh(draft.body));
      } else if (draft.body_mode === 'multipart') {
        // multipart/form-data → -F per field; files as -F key=@filename.
        try {
          const rows = JSON.parse(draft.body) as { key: string; type?: string; value: string; filename?: string }[];
          for (const r of rows) {
            if (!r.key) continue;
            if (r.type === 'file') parts.push('-F', sh(`${r.key}=@${r.filename || 'file'}`));
            else parts.push('-F', sh(`${r.key}=${r.value}`));
          }
        } catch {
          parts.push('--data', sh(draft.body));
        }
      } else if (draft.body_mode === 'form') {
        parts.push('--data', sh(draft.body));
      } else {
        parts.push('--data', sh(draft.body));
      }
    }

    return parts.join(' ');
  }

  // ── History ─────────────────────────────────────────────────────────────

  async clearHistory(): Promise<void> {
    const base = this.base();
    const wid = this.wsId();
    if (!base || !wid) return;
    try {
      await api.del(`${base}/history`);
      if (this.wsId() !== wid) return;
      this.historyRefresh.reset();
      this.historyDetail.cancel();
      this.history = [];
      await this.loadHistory();
    } catch (e) {
      toasts.error('Clear history failed', errMsg(e));
    }
  }

  // ── Automations (collection runner) ───────────────────────────────────────

  async loadAutomations(): Promise<void> {
    const base = this.base();
    if (!base) return;
    try {
      this.automations = await api.get<ApiAutomation[]>(`${base}/automations`);
    } catch (e) {
      toasts.error('Could not load automations', errMsg(e));
    }
  }

  async saveAutomation(
    req: UpsertApiAutomationReq,
    id?: Id,
  ): Promise<ApiAutomation | null> {
    const base = this.base();
    if (!base) return null;
    try {
      const saved = id
        ? await api.patch<ApiAutomation>(`${base}/automations/${id}`, req)
        : await api.post<ApiAutomation>(`${base}/automations`, req);
      this.automations = this.automations.some((a) => a.id === saved.id)
        ? this.automations.map((a) => (a.id === saved.id ? saved : a))
        : [...this.automations, saved];
      return saved;
    } catch (e) {
      toasts.error('Save automation failed', errMsg(e));
      return null;
    }
  }

  async deleteAutomation(id: Id): Promise<void> {
    const base = this.base();
    if (!base) return;
    try {
      await api.del(`${base}/automations/${id}`);
      this.automations = this.automations.filter((a) => a.id !== id);
      if (this.lastRun?.automation_id === id) this.lastRun = null;
    } catch (e) {
      toasts.error('Delete automation failed', errMsg(e));
    }
  }

  async loadAutomationRuns(automationId?: Id, before?: Id): Promise<void> {
    const base = this.base(); if (!base) return;
    const query = new URLSearchParams();
    if (automationId) query.set('automation_id',automationId);
    if (before) query.set('before',before);
    try {
      const runs = await api.get<ApiAutomationRun[]>(`${base}/automation-runs?${query}`);
      if (base === this.base()) this.automationRuns = before ? [...this.automationRuns,...runs] : runs;
    } catch (e) {toasts.error('Could not load run history',errMsg(e));}
  }
  async cancelAutomationRun(id: Id): Promise<void> {
    const base = this.base(); if (!base) return;
    try {await api.post(`${base}/automation-runs/${id}/cancel`,{});}
    catch (e) {toasts.error('Could not cancel run',errMsg(e));}
  }
  async runAutomation(id: Id, options: StartApiAutomationRunReq = {}): Promise<ApiRunResult | null> {
    const base = this.base(); if (!base) return null;
    this.running = true;
    try {
      let run = await api.post<ApiAutomationRun>(`${base}/automations/${id}/runs`,options);
      while (base === this.base()) {
        this.currentRun = run; this.lastRun = run.report;
        if (run.status !== 'running') {void this.loadAutomationRuns(id); return run.report;}
        await new Promise(resolve => setTimeout(resolve,500));
        if (base !== this.base()) break;
        run = await api.get<ApiAutomationRun>(`${base}/automation-runs/${run.id}`);
      }
      return null;
    } catch (e) {toasts.error('Run automation failed',errMsg(e)); return null;}
    finally {if (base === this.base()) this.running = false;}
  }

  // ── Draft helpers ─────────────────────────────────────────────────────────

  /** Open a fresh request in a new tab (optionally "in" a collection, which
   *  the save sheet then preselects). */
  newDraft(collectionHint: Id | null = null): void {
    this.openTab({ ...blankDraft(), collectionHint });
  }

  /** Open a copy of the active request in a new, unsaved tab. Keychain-backed
   *  credentials are not copied (they belong to the original's Keychain item). */
  duplicateDraft(): void {
    const src = $state.snapshot(this.draft) as ApiDraft;
    const saved = src.requestId ? this.requests.find((r) => r.id === src.requestId) : undefined;
    const { blanked, ...copy } = forDuplicate(src);
    this.openTab({
      ...copy,
      requestId: null,
      name: `${src.name?.trim() || this.tabLabel(src)} copy`,
      collectionHint: saved?.collection_id ?? src.collectionHint ?? null,
    });
    if (blanked) toasts.info('Duplicated without stored credentials', 'Re-enter them in Auth before sending, or use an environment {{variable}}.');
  }

  /** Put a loaded draft in front WITHOUT losing anyone's edits: a tab that
   *  already shows the same saved request is focused as-is; the active tab is
   *  reused only while it holds nothing unsaved (a pristine blank draft or an
   *  unmodified saved request); otherwise the draft opens in a new tab. */
  private placeDraft(d: ApiDraft): void {
    if (d.requestId) {
      const open = this.tabs.findIndex((t) => t.requestId === d.requestId);
      if (open >= 0) {
        this.switchTab(open);
        return;
      }
    }
    if (this.isDirty(this.draft)) {
      this.openTab(d);
      return;
    }
    this.draft = d;
    this.lastResponse = null;
    this.lastError = null;
  }

  /** Load a saved request into the builder (persisted extras included). */
  loadRequestIntoDraft(r: ApiRequest): void {
    this.placeDraft(extrasToDraft(
      {
        tabId: crypto.randomUUID(),
        requestId: r.id,
        name: r.name,
        kind: 'http',
        method: r.method,
        url: r.url,
        headers: r.headers.map((h) => ({ ...h })),
        query: r.query.map((q) => ({ ...q })),
        body_mode: r.body_mode,
        body: r.body,
        auth: { ...r.auth },
        ssh_connection_id: r.ssh_connection_id ?? null,
        proto: '',
        grpc_method: '',
      },
      r.extras,
    ));
  }

  /** True when a tab's draft differs from its saved request (or is a
   * non-empty unsaved draft) — includes the extras fields (scripts / docs /
   * settings / GraphQL variables / transport). Drives the tab's unsaved dot. */
  isDirty(d: ApiDraft): boolean {
    const saved = d.requestId ? this.requests.find((r) => r.id === d.requestId) : undefined;
    if (!saved) {
      // Unsaved draft: dirty once anything meaningful was entered.
      return Boolean(
        d.url.trim() || d.body.trim() || d.headers.length || d.query.length ||
        d.auth.type !== 'none' || Object.keys(draftToExtras(d)).length > 1,
      );
    }
    const kv = (rows: ApiKeyVal[]): string =>
      JSON.stringify(rows.filter((r) => r.key.trim() !== '' || r.value.trim() !== ''));
    return (
      (d.name.trim() !== '' && d.name !== saved.name) ||
      d.method !== saved.method ||
      d.url !== saved.url ||
      d.body !== saved.body ||
      d.body_mode !== saved.body_mode ||
      kv(d.headers) !== kv(saved.headers) ||
      kv(d.query) !== kv(saved.query) ||
      JSON.stringify(d.auth) !== JSON.stringify(saved.auth) ||
      (d.ssh_connection_id ?? null) !== (saved.ssh_connection_id ?? null) ||
      JSON.stringify(draftToExtras(d)) !== JSON.stringify(saved.extras ?? {v: 1})
    );
  }

  /** Load a history entry's request snapshot into the builder (best-effort).
   *  History stores credentials MASKED (`***`); those are refilled from the
   *  saved request the entry ran (by `request_id`, else the one saved request
   *  with the same method + URL), otherwise blanked with a re-enter hint —
   *  the mask itself is never sent as a credential. */
  loadHistoryIntoDraft(h: ApiHistoryEntry): void {
    const snap = (h.request ?? {}) as Partial<ExecuteApiReq> & { request_id?: Id | null };
    const method = snap.method ?? h.method;
    const url = snap.url ?? h.url;
    let saved = snap.request_id ? this.requests.find((r) => r.id === snap.request_id) : undefined;
    if (!saved) {
      const same = this.requests.filter((r) => r.method === method && r.url === url);
      if (same.length === 1) saved = same[0];
    }
    const un = unmaskHistory(
      { auth: snap.auth ?? { type: 'none' }, headers: snap.headers ?? [], query: snap.query ?? [] },
      saved,
    );
    this.placeDraft({
      tabId: crypto.randomUUID(),
      requestId: null,
      name: '',
      kind: 'http',
      method,
      url,
      headers: un.headers,
      query: un.query,
      body_mode: snap.body_mode ?? 'none',
      body: snap.body ?? '',
      auth: un.auth,
      ssh_connection_id: snap.ssh_connection_id ?? null,
      proto: '',
      grpc_method: '',
    });
    if (un.blanked) {
      toasts.info('Re-enter credentials', 'History stores secrets masked — fill the blanked credential fields before sending.');
    }
  }
}

export const apiClient = new ApiClientStore();
