// API client ("Postman") store — workspace-scoped collections, requests,
// environments, history, plus a live "draft" request the builder edits and
// executes through the daemon. Reads `ws.currentId` only (never mutates it).

import { api } from '../api/client';
import type {
  ApiAuth,
  ApiAutomation,
  ApiBodyMode,
  ApiCollection,
  ApiEnvironment,
  ApiHistoryEntry,
  ApiKeyVal,
  ApiRequest,
  ApiResponse,
  ApiRunResult,
  ExecuteApiReq,
  Id,
  ImportCurlReq,
  ParsedCurl,
  UpsertApiAutomationReq,
  UpsertApiCollectionReq,
  UpsertApiEnvironmentReq,
  UpsertApiRequestReq,
} from '../api/types';
import { ws } from './workspace.svelte';
import { toasts } from '../toast.svelte';

/** The editable request the builder/panel work on (a request not yet saved). */
export interface ApiDraft {
  /** When the draft came from a saved request, its id (for "Save" = update). */
  requestId: Id | null;
  name: string;
  method: string;
  url: string;
  headers: ApiKeyVal[];
  query: ApiKeyVal[];
  body_mode: ApiBodyMode;
  body: string;
  auth: ApiAuth;
}

export const HTTP_METHODS = ['GET', 'POST', 'PUT', 'PATCH', 'DELETE', 'HEAD', 'OPTIONS'];

function blankDraft(): ApiDraft {
  return {
    requestId: null,
    name: '',
    method: 'GET',
    url: '',
    headers: [],
    query: [],
    body_mode: 'none',
    body: '',
    auth: { type: 'none' },
  };
}

/** Drop empty/disabled key-vals before sending; keep enabled (default true). */
function liveKv(rows: ApiKeyVal[]): ApiKeyVal[] {
  return rows.filter((r) => r.enabled !== false && r.key.trim() !== '');
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

  /** Last runAutomation() report, shown in the run panel. */
  lastRun: ApiRunResult | null = $state(null);
  /** In-flight automation run. */
  running = $state(false);

  /** The request currently in the builder. */
  draft: ApiDraft = $state(blankDraft());
  /** Last execute() result, shown in the ResponseViewer. */
  lastResponse: ApiResponse | null = $state(null);
  /** In-flight send. */
  sending = $state(false);
  loading = $state(false);

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
    const base = this.base();
    if (!base) return;
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
    } catch (e) {
      toasts.error('Could not load API client', errMsg(e));
    } finally {
      this.loading = false;
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
    };
    const saved = await this.saveRequest(body, d.requestId ?? undefined);
    if (saved) {
      this.draft = { ...this.draft, requestId: saved.id, name: saved.name };
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
    const body: ExecuteApiReq = {
      method: draft.method,
      url: draft.url,
      headers: liveKv(draft.headers),
      query: liveKv(draft.query),
      body_mode: draft.body_mode,
      body: draft.body,
      auth: draft.auth,
      environment_id: this.activeEnv?.id ?? null,
    };
    this.sending = true;
    try {
      const resp = await api.post<ApiResponse>(`${base}/execute`, body);
      this.lastResponse = resp;
      void this.loadHistory();
      return resp;
    } catch (e) {
      toasts.error('Request failed', errMsg(e));
      return null;
    } finally {
      this.sending = false;
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
        method: p.method,
        url: p.url,
        headers: p.headers,
        query: p.query,
        body_mode: p.body_mode,
        body: p.body,
        auth: p.auth,
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

    // Auth.
    const a = draft.auth;
    if (a.type === 'bearer' && a.token) {
      parts.push('-H', sh(`Authorization: Bearer ${a.token}`));
    } else if (a.type === 'basic') {
      parts.push('-u', sh(`${a.username}:${a.password}`));
    } else if (a.type === 'api_key' && a.key) {
      if (a.in === 'header') parts.push('-H', sh(`${a.key}: ${a.value}`));
      else {
        const sep = url.includes('?') ? '&' : '?';
        // already appended to url above only for query rows; add the api_key here
        parts[parts.indexOf(sh(url))] = sh(
          url + sep + `${encodeURIComponent(a.key)}=${encodeURIComponent(a.value)}`,
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

  /** Replace the draft with a blank one. */
  newDraft(): void {
    this.draft = blankDraft();
    this.lastResponse = null;
  }

  /** Load a saved request into the builder. */
  loadRequestIntoDraft(r: ApiRequest): void {
    this.draft = {
      requestId: r.id,
      name: r.name,
      method: r.method,
      url: r.url,
      headers: r.headers.map((h) => ({ ...h })),
      query: r.query.map((q) => ({ ...q })),
      body_mode: r.body_mode,
      body: r.body,
      auth: { ...r.auth },
    };
    this.lastResponse = null;
  }

  /** Load a history entry's request snapshot into the builder (best-effort). */
  loadHistoryIntoDraft(h: ApiHistoryEntry): void {
    const snap = (h.request ?? {}) as Partial<ExecuteApiReq>;
    this.draft = {
      requestId: null,
      name: '',
      method: snap.method ?? h.method,
      url: snap.url ?? h.url,
      headers: (snap.headers ?? []).map((x) => ({ ...x })),
      query: (snap.query ?? []).map((x) => ({ ...x })),
      body_mode: snap.body_mode ?? 'none',
      body: snap.body ?? '',
      auth: snap.auth ?? { type: 'none' },
    };
    this.lastResponse = null;
  }
}

export const apiClient = new ApiClientStore();
