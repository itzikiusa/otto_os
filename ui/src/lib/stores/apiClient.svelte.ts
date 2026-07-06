// API client ("Postman") store — workspace-scoped collections, requests,
// environments, history, plus a live "draft" request the builder edits and
// executes through the daemon. Reads `ws.currentId` only (never mutates it).

import { api, isAbortError } from '../api/client';
import type {
  ApiAuth,
  ApiAutomation,
  ApiBodyMode,
  ApiCollection,
  ApiEnvironment,
  ApiHistoryEntry,
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
import { ws } from './workspace.svelte';
import { toasts } from '../toast.svelte';
import { runPreRequest, runPostResponse, type PreRequestReq, type TestResult } from '../api/scripts';
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
}

export const HTTP_METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'];

function blankDraft(): ApiDraft {
  return {
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
 * object. Returns null when everything is unset/default so untouched requests
 * keep a NULL column. */
export function draftToExtras(d: ApiDraft): ApiRequestExtras | null {
  const extras: ApiRequestExtras = { v: 1 };
  let any = false;
  if (d.kind && d.kind !== 'http') { extras.transport = d.kind; any = true; }
  if (d.graphql_variables?.trim()) { extras.graphql_variables = d.graphql_variables; any = true; }
  if (d.docs?.trim()) { extras.docs_md = d.docs; any = true; }
  const pre = d.pre_request_script?.trim() ? d.pre_request_script : undefined;
  const post = d.post_response_script?.trim() ? d.post_response_script : undefined;
  if (pre || post) { extras.scripts = { ...(pre ? { pre } : {}), ...(post ? { post } : {}) }; any = true; }
  const s = d.settings;
  if (s && (s.timeout_ms != null || !s.follow_redirects || !s.verify_ssl)) {
    extras.settings = {
      timeout_ms: s.timeout_ms,
      follow_redirects: s.follow_redirects,
      tls_verify: s.verify_ssl,
    };
    any = true;
  }
  return any ? extras : null;
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
  history: ApiHistoryEntry[] = $state([]);
  automations: ApiAutomation[] = $state([]);
  /** Workspace `ssh`-kind connections, for the Settings-tab "SSH tunnel" picker. */
  sshConnections: Connection[] = $state([]);

  /** Last runAutomation() report, shown in the run panel. */
  lastRun: ApiRunResult | null = $state(null);
  /** In-flight automation run. */
  running = $state(false);

  /** Open request tabs; the active one is edited via `draft`. */
  tabs: ApiDraft[] = $state([blankDraft()]);
  activeTab = $state(0);
  /** The request currently in the builder (proxies the active tab). */
  get draft(): ApiDraft {
    return this.tabs[this.activeTab] ?? this.tabs[0];
  }
  set draft(d: ApiDraft) {
    this.tabs[this.activeTab] = d;
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
    this.tabs = [...this.tabs, d];
    this.activeTab = this.tabs.length - 1;
    this.lastResponse = null;
    this.persistTabs();
  }
  switchTab(i: number): void {
    if (i >= 0 && i < this.tabs.length) {
      this.activeTab = i;
      this.lastResponse = null;
      this.persistTabs();
    }
  }
  closeTab(i: number): void {
    if (this.tabs.length === 1) {
      this.tabs = [blankDraft()];
      this.activeTab = 0;
    } else {
      this.tabs = this.tabs.filter((_, idx) => idx !== i);
      if (this.activeTab >= this.tabs.length) this.activeTab = this.tabs.length - 1;
      else if (i < this.activeTab) this.activeTab -= 1;
    }
    this.lastResponse = null;
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
    const blob: PersistedTabs = {
      tabs: $state.snapshot(this.tabs) as ApiDraft[],
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
    this.flushTabsWrite();
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
  /** In-flight send. */
  sending = $state(false);
  loading = $state(false);
  /** AbortController for the currently in-flight execute() call; null when idle. */
  private _abortCtrl: AbortController | null = null;
  /** Cancel the in-flight HTTP request (if any). No-op when idle. */
  cancelExecute(): void {
    this._abortCtrl?.abort();
    this._abortCtrl = null;
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
    try {
      const [collections, requests, environments, history] = await Promise.all([
        api.get<ApiCollection[]>(`${base}/collections`),
        api.get<ApiRequest[]>(`${base}/requests`),
        api.get<ApiEnvironment[]>(`${base}/environments`),
        api.get<ApiHistoryEntry[]>(`${base}/history`),
      ]);
      this.collections = collections;
      this.requests = requests;
      this.environments = environments;
      this.history = history;
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
      toasts.error('Could not load API client', errMsg(e));
    } finally {
      this.loading = false;
    }
    void this.loadSshConnections();
  }

  /** Load the workspace's `ssh`-kind connections for the SSH-tunnel picker.
   * Best-effort: a Viewer without connections access just sees an empty list. */
  async loadSshConnections(): Promise<void> {
    const wid = this.wsId();
    if (!wid) return;
    try {
      const all = await api.get<Connection[]>(`/workspaces/${wid}/connections`);
      this.sshConnections = all.filter((c) => c.kind === 'ssh');
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

  async loadHistory(): Promise<void> {
    const base = this.base();
    if (!base) return;
    try {
      this.history = await api.get<ApiHistoryEntry[]>(`${base}/history`);
    } catch (e) {
      toasts.error('Could not load history', errMsg(e));
    }
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
      this.requests = this.requests.some((r) => r.id === saved.id)
        ? this.requests.map((r) => (r.id === saved.id ? saved : r))
        : [...this.requests, saved];
      return saved;
    } catch (e) {
      toasts.error('Save request failed', errMsg(e));
      return null;
    }
  }

  /** Persist the current draft into a collection. Returns the saved request. */
  async saveDraft(name: string, collectionId: Id | null): Promise<ApiRequest | null> {
    const d = this.draft;
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
      this.draft = { ...this.draft, requestId: saved.id, name: saved.name, auth: { ...saved.auth } };
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
  runtimeVars: Record<string, string> = $state({});
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
    if (!base) {
      toasts.error('No workspace selected');
      return null;
    }
    if (!draft.url.trim()) {
      toasts.error('URL is empty');
      return null;
    }

    const logs: string[] = [];
    this.testResults = [];

    // Working copy the pre-request script may mutate.
    const reqCtx: PreRequestReq = {
      method: draft.method,
      url: draft.url,
      headers: draft.headers.map((h) => ({ ...h })),
      body: draft.body,
    };
    if (draft.pre_request_script?.trim()) {
      const pre = runPreRequest(draft.pre_request_script, reqCtx, this.runtimeVars);
      logs.push(...pre.logs.map((l) => `[pre] ${l}`));
      if (pre.error) {
        this.scriptLogs = [...logs, `[pre] error: ${pre.error}`];
        toasts.error('Pre-request script failed', pre.error);
        return null;
      }
    }

    // GraphQL: combine the query body + variables into the standard JSON payload.
    let effectiveBody = reqCtx.body;
    if (draft.body_mode === 'graphql') {
      let variables: unknown = {};
      try {
        variables = draft.graphql_variables?.trim() ? JSON.parse(draft.graphql_variables) : {};
      } catch {
        variables = {};
      }
      effectiveBody = JSON.stringify({ query: reqCtx.body, variables });
    }

    const s = draft.settings;
    const body: ExecuteApiReq = {
      method: reqCtx.method,
      url: reqCtx.url,
      headers: liveKv(reqCtx.headers),
      query: liveKv(draft.query),
      body_mode: draft.body_mode,
      body: effectiveBody,
      auth: draft.auth,
      environment_id: this.activeEnv?.id ?? null,
      timeout_ms: s?.timeout_ms ?? null,
      follow_redirects: s?.follow_redirects ?? true,
      verify_ssl: s?.verify_ssl ?? true,
      vars: Object.keys(this.runtimeVars).length ? this.runtimeVars : undefined,
      ssh_connection_id: draft.ssh_connection_id ?? null,
    };
    this._abortCtrl = new AbortController();
    const { signal } = this._abortCtrl;
    this.sending = true;
    try {
      const resp = await api.post<ApiResponse>(`${base}/execute`, body, signal);
      this.lastResponse = resp;
      void this.loadHistory();

      // Post-response script: chaining (set vars) + tests.
      if (draft.post_response_script?.trim()) {
        const headersObj: Record<string, string> = {};
        for (const h of resp.headers) headersObj[h.key.toLowerCase()] = h.value;
        const post = runPostResponse(
          draft.post_response_script,
          { code: resp.status, status: resp.status_text, responseTime: resp.duration_ms, headers: headersObj, bodyText: resp.body },
          this.runtimeVars,
        );
        logs.push(...post.logs.map((l) => `[test] ${l}`));
        this.testResults = post.tests;
        if (post.error) logs.push(`[test] error: ${post.error}`);
        this.runtimeVars = { ...this.runtimeVars };
      }
      this.scriptLogs = logs;
      return resp;
    } catch (e) {
      this.scriptLogs = logs;
      if (!isAbortError(e)) toasts.error('Request failed', errMsg(e));
      return null;
    } finally {
      this.sending = false;
      this._abortCtrl = null;
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
      this.draft = {
        requestId: null,
        name: this.draft.name,
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
      };
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
    if (!base) return;
    try {
      await api.del(`${base}/history`);
      this.history = [];
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

  /** Run an automation through the daemon; stores + returns the run report. */
  async runAutomation(id: Id): Promise<ApiRunResult | null> {
    const base = this.base();
    if (!base) return null;
    this.running = true;
    try {
      const result = await api.post<ApiRunResult>(`${base}/automations/${id}/run`, {});
      this.lastRun = result;
      // Extracts may have written environment variables; refresh to reflect them.
      void this.loadEnvironments();
      return result;
    } catch (e) {
      toasts.error('Run automation failed', errMsg(e));
      return null;
    } finally {
      this.running = false;
    }
  }

  // ── Draft helpers ─────────────────────────────────────────────────────────

  /** Open a fresh request in a new tab. */
  newDraft(): void {
    this.openTab(blankDraft());
  }

  /** Load a saved request into the builder (persisted extras included). */
  loadRequestIntoDraft(r: ApiRequest): void {
    this.draft = extrasToDraft(
      {
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
    );
    this.lastResponse = null;
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
        d.auth.type !== 'none' || draftToExtras(d),
      );
    }
    const kv = (rows: ApiKeyVal[]): string =>
      JSON.stringify(rows.filter((r) => r.key.trim() !== '' || r.value.trim() !== ''));
    return (
      d.method !== saved.method ||
      d.url !== saved.url ||
      d.body !== saved.body ||
      d.body_mode !== saved.body_mode ||
      kv(d.headers) !== kv(saved.headers) ||
      kv(d.query) !== kv(saved.query) ||
      JSON.stringify(d.auth) !== JSON.stringify(saved.auth) ||
      (d.ssh_connection_id ?? null) !== (saved.ssh_connection_id ?? null) ||
      JSON.stringify(draftToExtras(d)) !== JSON.stringify(saved.extras ?? null)
    );
  }

  /** Load a history entry's request snapshot into the builder (best-effort). */
  loadHistoryIntoDraft(h: ApiHistoryEntry): void {
    const snap = (h.request ?? {}) as Partial<ExecuteApiReq>;
    this.draft = {
      requestId: null,
      name: '',
      kind: 'http',
      method: snap.method ?? h.method,
      url: snap.url ?? h.url,
      headers: (snap.headers ?? []).map((x) => ({ ...x })),
      query: (snap.query ?? []).map((x) => ({ ...x })),
      body_mode: snap.body_mode ?? 'none',
      body: snap.body ?? '',
      auth: snap.auth ?? { type: 'none' },
      ssh_connection_id: snap.ssh_connection_id ?? null,
      proto: '',
      grpc_method: '',
    };
    this.lastResponse = null;
  }
}

export const apiClient = new ApiClientStore();
