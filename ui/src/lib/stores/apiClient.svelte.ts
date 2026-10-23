// API client ("Postman") store — workspace-scoped collections, requests,
// environments, history, plus a live "draft" request the builder edits and
// executes through the daemon. Reads `ws.currentId` only (never mutates it).
//
// The one façade the UI imports (arch-6-03). It owns the workspace's lists,
// their loading and the publication-ownership token, and forwards to its parts
// in ./apiClient/: `model` (types + pure helpers, re-exported below), `tabs`
// (open tabs, persistence, per-tab response slots), `drafts` (open/save/dirty),
// `execute` (send, gRPC, GraphQL, runtime vars), `curl`, `cookies`,
// `collectionsSync` (import, Postman, git) and `automations`. Each part takes a
// narrow `Pick<ApiClientStore, …>` host; members marked "(part host)" are
// public only for that contract.

import { api } from '../api/client';
import type {
  ApiAuth, ApiAutomation, ApiAutomationRun, ApiClientWsSettings, ApiClientStorage, ApiCollection, ApiEnvironment,
  ApiHistoryEntry, ApiHistorySummary, ApiOverviewRequest, ApiRequest, ApiRequestItem, ApiResponse, ApiRunResult,
  Connection, Id, StartApiAutomationRunReq, UpsertApiAutomationReq, UpsertApiCollectionReq, UpsertApiEnvironmentReq,
  UpsertApiRequestReq,
} from '../api/types';
import type { TestResult } from '../api/scripts';
import type { ImportedDoc, ImportedEnvironment } from '../api/importers';
import { ws } from './workspace.svelte';
import { announceModule } from '../lazyModule';
import { announce } from '../announce.svelte';
import { HistoryRefresh, HistoryDetail } from './apiHistory';
import { toasts } from '../toast.svelte';
import { loadErrorText } from '../loadError';
import { ApiAutomations } from './apiClient/automations.svelte';
import { ApiTabs } from './apiClient/tabs.svelte';
import { ApiDrafts } from './apiClient/drafts.svelte';
import { ApiExecute } from './apiClient/execute.svelte';
import { ApiCookies } from './apiClient/cookies.svelte';
import { ApiCollectionsSync } from './apiClient/collectionsSync';
import { importCurl, toCurl } from './apiClient/curl';
import { countOr0, RELOAD_FRESH_MS, summaryItems, type ApiCookie, type ApiDraft, type TabResponse } from './apiClient/model';

export {
  confirmNewHost, defaultSettings, draftSecrets, draftToExtras, historyResponse, HTTP_METHODS, MAX_TAB_RESPONSES,
  requestItem,
} from './apiClient/model';
export type {
  ApiCookie, ApiDraft, ApiRequestKind, ApiSettings, NewHostContext, TabResponse,
} from './apiClient/model';

export class ApiClientStore {
  // Raw: both lists are replaced wholesale (map/filter/spread), and a deep
  // proxy made every tree filter read 3k proxied rows per keystroke.
  collections: ApiCollection[] = $state.raw([]);
  /** Saved requests as summaries (tree, search, pickers) — perf2 N1. The full
   *  row of an opened request lives in `fullRequests`. */
  requests: ApiRequestItem[] = $state.raw([]);
  /** id → full saved request, for open tabs (dirty check, secret stripping,
   *  history unmask) and recently opened rows. Replaced wholesale. */
  fullRequests: Map<Id, ApiRequest> = $state.raw(new Map());
  environments: ApiEnvironment[] = $state([]);
  history: ApiHistorySummary[] = $state([]);
  historyLoadingId: string | null = $state(null);
  /** The workspace's history list has loaded at least once (so an empty list
   *  means "nothing sent yet", not "still loading"). */
  historyLoaded = $state(false);
  /** The list failed to load with nothing on screen — shown inline with Retry
   *  (over live rows a failed refresh is a toast instead). */
  historyLoadError: string | null = $state(null);
  historyAgentOnly = $state(false);
  /** Workspace `ssh`-kind connections, for the Settings-tab "SSH tunnel" picker. */
  sshConnections: Connection[] = $state([]);


  /** Bumped to ask ApiPage to show the request editor (it may be showing the
   *  environments or an automation) — used by agent UI control
   *  (lib/uiCommands/api.ts) so an agent-edited request is on screen. */
  requestViewTick = $state(0);
  showRequestView(): void {
    this.requestViewTick++;
  }

  // Workspace `settings.api_client` — owned here, not by the shell's ws store
  // (architecture-2-03). Writes are admin-gated atomic key patches.
  private get wsSettings(): ApiClientWsSettings | undefined { return ws.current?.settings?.api_client; }
  /** Opt-in for localhost/private-network targets (`allow_local`, off by default). */
  get allowLocal(): boolean { return this.wsSettings?.allow_local === true; }
  /** History retention (`history_max_rows` / `history_max_days`; the daemon
   *  trims after every run). 0 = no limit: pruning deletes runs, so it is opt-in. */
  get historyRetention(): { rows: number; days: number } {
    return { rows: countOr0(this.wsSettings?.history_max_rows), days: countOr0(this.wsSettings?.history_max_days) };
  }
  /** Newest finished runs kept per automation (`automation_runs_keep`; 0 = all). */
  get runsKeep(): number { return countOr0(this.wsSettings?.automation_runs_keep); }
  setAllowLocal(allow: boolean): Promise<void> { return this.patchWsSettings({ allow_local: allow }); }
  setHistoryRetention(rows: number, days: number): Promise<void> {
    return this.patchWsSettings({ history_max_rows: rows, history_max_days: days });
  }
  setRunsKeep(keep: number): Promise<void> { return this.patchWsSettings({ automation_runs_keep: keep }); }
  private async patchWsSettings(patch: ApiClientWsSettings): Promise<void> {
    if (ws.currentId && ws.current) await ws.patchSettings(ws.currentId, { api_client: patch });
  }

  /** Load the current workspace's API client unless this document already
   *  holds it (its tabs restored). An agent command can land before ApiPage's
   *  own load effect has run, and editing a not-yet-restored blank tab would be
   *  overwritten by the restore. */
  async ensureLoaded(): Promise<void> {
    const wid = this.wsId();
    if (wid && this.tabSet.wid !== wid) await this.loadAll();
  }


  // ── Open tabs + per-tab response slots: ./apiClient/tabs.svelte.ts ───────
  /** (part host) The open tabs, their persistence and response slots. */
  readonly tabSet: ApiTabs = new ApiTabs(this);
  /** Open request tabs; the active one is edited via `draft`. */
  get tabs(): ApiDraft[] { return this.tabSet.tabs; }
  set tabs(v: ApiDraft[]) { this.tabSet.tabs = v; }
  get activeTab(): number { return this.tabSet.activeTab; }
  set activeTab(v: number) { this.tabSet.activeTab = v; }
  /** The request currently in the builder (proxies the active tab). */
  get draft(): ApiDraft { return this.tabSet.draft; }
  set draft(d: ApiDraft) { this.tabSet.draft = d; }
  /** tabId → that tab's response state. */
  get responses(): Map<string, TabResponse> { return this.tabSet.responses; }
  get lastResponse(): ApiResponse | null { return this.tabSet.lastResponse; }
  set lastResponse(resp: ApiResponse | null) { this.tabSet.lastResponse = resp; }
  get lastError(): string | null { return this.tabSet.lastError; }
  set lastError(error: string | null) { this.tabSet.lastError = error; }
  get sending(): boolean { return this.tabSet.sending; }
  get scriptLogs(): string[] { return this.tabSet.scriptLogs; }
  get testResults(): TestResult[] { return this.tabSet.testResults; }
  get responseFromHistory(): TabResponse['history'] { return this.tabSet.responseFromHistory; }
  tabLabel(d: ApiDraft): string { return this.tabSet.tabLabel(d); }
  openTab(d?: ApiDraft): void { this.tabSet.openTab(d); }
  switchTab(i: number): void { this.tabSet.switchTab(i); }
  closeTab(i: number): void { this.tabSet.closeTab(i); }
  tabStatus(tabId: string | null | undefined): 'sending' | 'ok' | 'fail' | null { return this.tabSet.tabStatus(tabId); }
  cancelExecute(tabId?: string | null): void { this.tabSet.cancelExecute(tabId); }
  applySavedAuth(wid: Id, request: ApiRequest, previousAuth: ApiAuth): void {
    this.tabSet.applySavedAuth(wid, request, previousAuth);
  }

  /** Distinguish returning to a workspace from the visit that issued a request. */
  private workspaceGeneration = 0;
  private publicationWorkspace: Id | null = null;

  /** Swap in `wid`'s persisted tabs and forget the previous workspace's
   *  history, runs and storage gauge. No-op when already showing them. */
  private restoreTabs(wid: Id): void {
    if (this.tabSet.wid === wid) return;
    this.workspaceOwner();
    this.historyRefresh.reset();
    this.historyDetail.cancel();
    this.history = [];
    this.historyLoaded = false;
    this.historyLoadError = null;
    this.autos.resetRuns();
    this.storage = null; this.storageAt = null;
    this.tabSet.swapTo(wid);
  }

  loading = $state(false);
  /** The workspace whose lists settled (loaded or failed) at least once. Not
   *  cleared by a refetch, so a reload never re-enters the "not loaded" state. */
  listsSettledFor: Id | null = $state(null);
  /** The current workspace's lists are in flight OR have not been asked for
   *  yet. `loading` alone is false on the first render, before the page's
   *  effect starts `loadAll` — an empty-looking store then read as "nothing
   *  saved" and flashed onboarding → editor → onboarding. */
  get listsPending(): boolean {
    const wid = this.wsId();
    return this.loading || (wid !== null && this.listsSettledFor !== wid);
  }
  /** Why the collections + requests lists couldn't load (the sidebar tree's
   *  inline error with Retry); null when fine. Scoped per list so a failed
   *  environments refresh can never paint "Couldn’t load" over the tree. */
  requestsLoadError: string | null = $state(null);
  /** Same, for the environments list (EnvironmentsView). */
  envLoadError: string | null = $state(null);
  /** Either list failed — gates onboarding (a failed load is not "empty"). */
  loadError: string | null = $derived(this.requestsLoadError ?? this.envLoadError);
  /** Active environment (is_active), or null. */
  activeEnv: ApiEnvironment | null = $derived(
    this.environments.find((e) => e.is_active) ?? null,
  );

  /** (part host) The current workspace id. */
  wsId(): Id | null {
    return ws.currentId;
  }

  /** (part host) `/workspaces/{wid}/api-client`, or null with no workspace. */
  base(): string | null {
    const id = this.wsId();
    return id ? `/workspaces/${id}/api-client` : null;
  }

  /** Capture publication ownership; a server write may finish after this visit
   * ends, but its response/error must not change the newly displayed workspace. */
  workspaceOwner(): () => boolean {
    const wid = this.wsId();
    if (wid !== this.publicationWorkspace) {
      this.publicationWorkspace = wid;
      this.workspaceGeneration++;
      this.loadedAt = null;
      this.automationsLoadedAt = null;
    }
    const generation = this.workspaceGeneration;
    return () => this.wsId() === wid && this.workspaceGeneration === generation;
  }

  // ── Loading ───────────────────────────────────────────────────────────────

  /** When this workspace's lists last loaded successfully (stale-while-
   *  revalidate for page re-entry — perf F4). Cleared on a failed load. */
  private loadedAt: { wid: Id; at: number } | null = null;

  /** Load everything for the current workspace (collections + requests + envs + history).
   *  Re-entering the API page within RELOAD_FRESH_MS of a successful load
   *  skips the refetch (`force` overrides) — local edits update the store
   *  directly and history arrives live over `api_history_appended`. */
  async loadAll(opts: { force?: boolean } = {}): Promise<void> {
    const current = this.workspaceOwner();
    const wid = this.wsId();
    const base = this.base();
    if (!wid || !base) return;
    // Restore this workspace's persisted open tabs up front (works even when
    // the fetches below fail — the drafts are device-local, not server data).
    this.restoreTabs(wid);
    if (
      !opts.force &&
      this.loadedAt?.wid === wid &&
      Date.now() - this.loadedAt.at < RELOAD_FRESH_MS &&
      !this.requestsLoadError &&
      !this.envLoadError
    ) {
      return;
    }
    this.loadedAt = null;
    this.loading = true;
    this.requestsLoadError = null;
    this.envLoadError = null;
    try {
      const [collections, requests, environments] = await Promise.all([
        api.get<ApiCollection[]>(`${base}/collections`),
        api.get<ApiOverviewRequest[]>(`${base}/requests/summaries`),
        api.get<ApiEnvironment[]>(`${base}/environments`),
        this.loadHistory(),
      ]);
      if (!current()) return;
      this.collections = collections;
      this.requests = summaryItems(requests, wid);
      this.environments = environments;
      // Unlink restored drafts whose saved request no longer exists, so their
      // "Save" creates anew instead of PATCHing a deleted id.
      const live = new Set(this.requests.map((r) => r.id));
      if (this.tabs.some((t) => t.requestId && !live.has(t.requestId))) {
        this.tabs = this.tabs.map((t) =>
          t.requestId && !live.has(t.requestId) ? { ...t, requestId: null } : t,
        );
        this.tabSet.persist();
      }
      this.loadedAt = { wid, at: Date.now() };
      // Open tabs' saved rows (dirty dots, secret stripping) — one GET each.
      for (const t of this.tabs) if (t.requestId) void this.ensureRequest(t.requestId);
    } catch (e) {
      if (current()) {
        this.requestsLoadError = loadErrorText(e);
        this.envLoadError = loadErrorText(e);
      }
    } finally {
      if (current()) {
        this.loading = false;
        this.listsSettledFor = wid;
      }
    }
    if (current()) void this.loadSshConnections();
  }

  /** Load the workspace's `ssh`-kind connections for the SSH-tunnel picker.
   * Best-effort: a Viewer without connections access just sees an empty list. */
  async loadSshConnections(): Promise<void> {
    const current = this.workspaceOwner();
    const wid = this.wsId();
    if (!wid) return;
    try {
      const all = await api.get<Connection[]>(`/workspaces/${wid}/connections`);
      if (current()) this.sshConnections = all.filter((c) => c.kind === 'ssh');
    } catch {
      if (current()) this.sshConnections = [];
    }
  }

  /** A single-list refresh (after an action) failed. With nothing on screen it
   *  becomes THAT list's inline error (Retry = loadAll) — an empty sidebar must
   *  never read as "No saved requests yet"; over live rows the rows stay and
   *  the failed refresh is reported as the action result it followed. The
   *  list's next successful load clears its error. */
  private refreshFailed(what: string, list: 'requests' | 'environments', empty: boolean, e: unknown): void {
    if (!empty) toasts.error(`Couldn’t refresh ${what}`, loadErrorText(e));
    else if (list === 'environments') this.envLoadError = loadErrorText(e);
    else this.requestsLoadError = loadErrorText(e);
  }

  async loadCollections(): Promise<void> {
    const current = this.workspaceOwner();
    const base = this.base();
    if (!base) return;
    try {
      const rows = await api.get<ApiCollection[]>(`${base}/collections`);
      if (!current()) return;
      this.collections = rows;
      this.requestsLoadError = null;
    } catch (e) {
      if (!current()) return;
      this.refreshFailed('collections', 'requests', this.collections.length === 0 && this.requests.length === 0, e);
    }
  }

  async loadRequests(): Promise<void> {
    const current = this.workspaceOwner();
    const base = this.base();
    if (!base) return;
    try {
      const wid = this.wsId();
      const rows = await api.get<ApiOverviewRequest[]>(`${base}/requests/summaries`);
      if (!current()) return;
      if (wid) this.requests = summaryItems(rows, wid);
      this.requestsLoadError = null;
    } catch (e) {
      if (!current()) return;
      this.refreshFailed('requests', 'requests', this.collections.length === 0 && this.requests.length === 0, e);
    }
  }

  async loadEnvironments(): Promise<void> {
    const current = this.workspaceOwner();
    const base = this.base();
    if (!base) return;
    try {
      const rows = await api.get<ApiEnvironment[]>(`${base}/environments`);
      if (!current()) return;
      this.environments = rows;
      this.envLoadError = null;
    } catch (e) {
      if (!current()) return;
      this.refreshFailed('environments', 'environments', this.environments.length === 0, e);
    }
  }

  private historyRefresh = new HistoryRefresh<ApiHistorySummary>({
    workspace: () => this.wsId(),
    fetch: (wid, signal) => api.get<ApiHistorySummary[]>(`/workspaces/${wid}/api-client/history/summaries`, signal),
    publish: (rows) => { this.history = rows; this.historyLoaded = true; this.historyLoadError = null; },
    error: (error) => {
      if (this.history.length === 0) this.historyLoadError = loadErrorText(error);
      else toasts.error('Couldn’t refresh history', loadErrorText(error));
    },
  });

  private historyDetail = new HistoryDetail<ApiHistoryEntry>({
    owner: () => ({workspace: this.wsId(), tab: this.draft.tabId, draft: this.draft}),
    fetch: (id, signal) => api.get<ApiHistoryEntry>(`${this.base()}/history/${encodeURIComponent(id)}`, signal),
    publish: (entry) => this.loadHistoryIntoDraft(entry),
    // a11y-6-01: the "Loading request…" lines are plain text; speak it here.
    pending: (id) => { this.historyLoadingId = id; if (id) announce('Loading request…'); },
    error: (error) => toasts.error('Couldn’t load history request', loadErrorText(error)),
  });

  async loadHistory(): Promise<void> {
    await this.historyRefresh.request();
  }

  /** Retry a failed history load from its inline error. */
  retryHistory(): void {
    this.historyLoadError = null;
    void this.loadHistory();
  }

  /** Server-side history search — the sidebar list only holds the newest 100
   *  summaries. `q` matches method/URL (case-insensitive, literal); `status`
   *  is exact. Newest first, up to 500. */
  async searchHistory(params: { q?: string; status?: number }, signal?: AbortSignal): Promise<ApiHistorySummary[]> {
    const wid = this.wsId();
    if (!wid) return [];
    const query = new URLSearchParams({ limit: '500' });
    if (params.q) query.set('q', params.q);
    if (params.status != null) query.set('status', String(params.status));
    return api.get<ApiHistorySummary[]>(`/workspaces/${wid}/api-client/history/summaries?${query}`, signal);
  }

  /** Direct sends and WS append events share one metadata-only refresh —
   *  only once this workspace's list was asked for (loaded, failed, or in
   *  flight): a store some other surface loaded has no list on screen, and
   *  an automation run appends one entry per step (perf H1). */
  noteHistoryAppended(workspaceId: string, entryId?: string): void {
    if (workspaceId !== this.wsId()) return;
    if (!this.historyLoaded && this.historyLoadError === null && this.historyRefresh.idle) return;
    void this.historyRefresh.request(entryId);
  }

  // ── Storage gauge (perf2 N2) ──────────────────────────────────────────────
  /** What this workspace's history + automation runs occupy (History list). */
  storage: ApiClientStorage | null = $state.raw(null);
  private storageAt: { wid: Id; at: number } | null = null;

  /** Load the gauge (at most once a minute per workspace; the daemon caches
   *  it too). Best-effort: no gauge just hides the line. */
  async loadStorage(opts: { force?: boolean } = {}): Promise<void> {
    const wid = this.wsId();
    const base = this.base();
    if (!wid || !base) return;
    if (!opts.force && this.storageAt?.wid === wid && Date.now() - this.storageAt.at < RELOAD_FRESH_MS) return;
    this.storageAt = { wid, at: Date.now() };
    try {
      const s = await api.get<ApiClientStorage>(`${base}/storage`);
      if (this.wsId() === wid) this.storage = s;
    } catch {
      if (this.wsId() === wid) this.storage = null;
    }
  }

  /** Apply the workspace's (just saved) retention now and refresh the gauge
   *  and the history list. Only deletes what the configured limits say. */
  async applyRetention(): Promise<boolean> {
    const wid = this.wsId();
    const base = this.base();
    if (!wid || !base) return false;
    try {
      const s = await api.post<ApiClientStorage>(`${base}/storage/prune`, {});
      if (this.wsId() !== wid) return true;
      this.storage = s;
      this.storageAt = { wid, at: Date.now() };
      void this.historyRefresh.request();
      return true;
    } catch (e) {
      toasts.error('Couldn’t apply retention', loadErrorText(e));
      return false;
    }
  }

  // ── Changes made elsewhere (perf2 N4) ─────────────────────────────────────
  private changeQueue: { kinds: Set<string>; requests: Map<Id, boolean>; wid: Id } | null = null;
  private changeTimer: ReturnType<typeof setTimeout> | null = null;
  /** (part host) Imports in flight in this window — their echoes are skipped. */
  bulkWrites = 0;

  /** `api_client_changed`: a request / collection / environment / automation
   *  changed (an agent's MCP tool, another user, or our own save echoing).
   *  Another workspace's change just expires its 60 s reuse window; the
   *  loaded workspace is patched — one GET per changed request, one list
   *  reload per other kind — coalesced over 300 ms (an import is N events). */
  noteClientChanged(ev: { workspace_id: Id; kind: string; id: Id | null; deleted: boolean }): void {
    if (this.loadedAt?.wid !== ev.workspace_id || this.wsId() !== ev.workspace_id) {
      if (this.loadedAt?.wid === ev.workspace_id) this.loadedAt = null;
      if (ev.kind === 'automation') this.automationsLoadedAt = null;
      return;
    }
    // A request/collection echo of our own import: its final reload covers it.
    if (this.bulkWrites > 0 && (ev.kind === 'request' || ev.kind === 'collection')) return;
    const q = this.changeQueue?.wid === ev.workspace_id
      ? this.changeQueue
      : (this.changeQueue = { kinds: new Set(), requests: new Map(), wid: ev.workspace_id });
    if (ev.kind === 'request' && ev.id && q.requests.size < 50) q.requests.set(ev.id, ev.deleted);
    else q.kinds.add(ev.kind);
    if (this.changeTimer === null) this.changeTimer = setTimeout(() => void this.flushClientChanges(), 300);
  }

  private async flushClientChanges(): Promise<void> {
    this.changeTimer = null;
    const q = this.changeQueue;
    this.changeQueue = null;
    if (!q || this.wsId() !== q.wid) return;
    const base = this.base();
    if (q.kinds.has('request')) await this.loadRequests();
    else {
      for (const [id, deleted] of q.requests) {
        if (deleted) {
          this.dropRequest(id);
          continue;
        }
        const known = this.fullRequests.get(id);
        try {
          const r = await api.get<ApiRequest>(`${base}/requests/${encodeURIComponent(id)}`);
          if (this.base() !== base) return;
          // Our own save echoing back: same row, nothing to patch.
          if (known && known.updated_at === r.updated_at) continue;
          this.upsertRequest(r);
        } catch {
          // Gone or unreadable: the next full load reconciles.
          this.loadedAt = null;
        }
      }
    }
    if (q.kinds.has('collection')) await this.loadCollections();
    if (q.kinds.has('environment')) await this.loadEnvironments();
    if (q.kinds.has('automation')) {
      if (this.automationsLoadedAt?.base === base) await this.loadAutomations({ force: true });
      else this.automationsLoadedAt = null;
    }
  }

  historySource(h: ApiHistorySummary): ApiHistorySummary['source'] {
    return h.source;
  }

  async selectHistory(id: string): Promise<void> {
    if (this.wsId()) await this.historyDetail.select(id);
  }

  // ── Cookie jar (daemon-global) + secure-all: ./apiClient/cookies.svelte.ts ──
  readonly cookieJar: ApiCookies = new ApiCookies(this);
  get cookies(): ApiCookie[] { return this.cookieJar.cookies; }
  get cookiesLoading(): boolean { return this.cookieJar.cookiesLoading; }
  get cookiesError(): string | null { return this.cookieJar.cookiesError; }
  loadCookies(): Promise<void> { return this.cookieJar.loadCookies(); }
  clearCookies(): Promise<void> { return this.cookieJar.clearCookies(); }
  secureAll(): Promise<void> { return this.cookieJar.secureAll(); }

  // ── Collections ─────────────────────────────────────────────────────────

  async saveCollection(req: UpsertApiCollectionReq, id?: Id): Promise<ApiCollection | null> {
    const current = this.workspaceOwner();
    const base = this.base();
    if (!base) return null;
    try {
      const saved = id
        ? await api.patch<ApiCollection>(`${base}/collections/${id}`, req)
        : await api.post<ApiCollection>(`${base}/collections`, req);
      if (!current()) return saved;
      this.collections =
        id != null && this.collections.some((c) => c.id === saved.id)
          ? this.collections.map((c) => (c.id === saved.id ? saved : c))
          : [...this.collections, saved];
      return saved;
    } catch (e) {
      if (!current()) return null;
      toasts.error('Couldn’t save collection', loadErrorText(e));
      return null;
    }
  }

  // ── Import, Postman sync, git pull/push: ./apiClient/collectionsSync.ts ──
  readonly sync: ApiCollectionsSync = new ApiCollectionsSync(this);
  importFile(file: Pick<File, 'name' | 'text'>): Promise<void> { return this.sync.importFile(file); }
  importParsed(parsed: ImportedDoc, quiet = false): Promise<{ created: boolean; failed: number }> {
    return this.sync.importParsed(parsed, quiet);
  }
  importEnvironment(env: ImportedEnvironment, quiet = false): Promise<boolean> { return this.sync.importEnvironment(env, quiet); }
  postmanSync(apiKey: string, remember: boolean): Promise<boolean> { return this.sync.postmanSync(apiKey, remember); }
  gitPullCollections(repoId: Id): Promise<void> { return this.sync.gitPullCollections(repoId); }
  gitPushCollections(repoId: Id, message: string, branch: string | null): Promise<boolean> {
    return this.sync.gitPushCollections(repoId, message, branch);
  }

  async deleteCollection(id: Id): Promise<void> {
    const current = this.workspaceOwner();
    const base = this.base();
    if (!base) return;
    try {
      await api.del(`${base}/collections/${id}`);
      if (!current()) return;
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
      if ([...this.fullRequests.values()].some((r) => r.collection_id && removed.has(r.collection_id))) {
        this.fullRequests = new Map(
          [...this.fullRequests].map(([k, r]) => [k, r.collection_id && removed.has(r.collection_id) ? { ...r, collection_id: null } : r]),
        );
      }
    } catch (e) {
      if (!current()) return;
      toasts.error('Couldn’t delete collection', loadErrorText(e));
    }
  }

  // ── Requests ────────────────────────────────────────────────────────────

  async saveRequest(req: UpsertApiRequestReq, id?: Id): Promise<ApiRequest | null> {
    const current = this.workspaceOwner();
    const base = this.base();
    if (!base) return null;
    try {
      const saved = id
        ? await api.patch<ApiRequest>(`${base}/requests/${id}`, req)
        : await api.post<ApiRequest>(`${base}/requests`, req);
      if (!current()) return saved;
      this.upsertRequest(saved);
      return saved;
    } catch (e) {
      if (!current()) return null;
      toasts.error('Couldn’t save the request', loadErrorText(e));
      return null;
    }
  }

  async deleteRequest(id: Id): Promise<void> {
    const current = this.workspaceOwner();
    const base = this.base();
    if (!base) return;
    try {
      await api.del(`${base}/requests/${id}`);
      if (!current()) return;
      this.dropRequest(id);
      if (this.draft.requestId === id) this.draft = { ...this.draft, requestId: null };
    } catch (e) {
      if (!current()) return;
      toasts.error('Couldn’t delete request', loadErrorText(e));
    }
  }

  // ── Environments ────────────────────────────────────────────────────────

  async saveEnvironment(req: UpsertApiEnvironmentReq, id?: Id): Promise<ApiEnvironment | null> {
    const current = this.workspaceOwner();
    const base = this.base();
    if (!base) return null;
    try {
      const saved = id
        ? await api.patch<ApiEnvironment>(`${base}/environments/${id}`, req)
        : await api.post<ApiEnvironment>(`${base}/environments`, req);
      if (!current()) return saved;
      this.environments = this.environments.some((e) => e.id === saved.id)
        ? this.environments.map((e) => (e.id === saved.id ? saved : e))
        : [...this.environments, saved];
      return saved;
    } catch (e) {
      if (!current()) return null;
      toasts.error('Couldn’t save environment', loadErrorText(e));
      return null;
    }
  }

  async deleteEnvironment(id: Id): Promise<void> {
    const current = this.workspaceOwner();
    const base = this.base();
    if (!base) return;
    try {
      await api.del(`${base}/environments/${id}`);
      if (!current()) return;
      this.environments = this.environments.filter((e) => e.id !== id);
    } catch (e) {
      if (!current()) return;
      toasts.error('Couldn’t delete environment', loadErrorText(e));
    }
  }

  async activateEnvironment(id: Id): Promise<void> {
    const current = this.workspaceOwner();
    const base = this.base();
    if (!base) return;
    try {
      await api.post(`${base}/environments/${id}/activate`, {});
      if (!current()) return;
      // Exactly one active env: reflect it locally without a refetch.
      this.environments = this.environments.map((e) => ({ ...e, is_active: e.id === id }));
    } catch (e) {
      if (!current()) return;
      toasts.error('Couldn’t activate environment', loadErrorText(e));
    }
  }

  // ── Send: ./apiClient/execute.svelte.ts ─────────────────────────────────
  readonly exec: ApiExecute = new ApiExecute(this);
  /** Runtime/session variables (set by scripts or by hand) sent as overrides. */
  get runtimeVars(): Record<string, string> { return this.exec.runtimeVars; }
  set runtimeVars(vars: Record<string, string>) { this.exec.runtimeVars = vars; }
  setRuntimeVar(key: string, value: string): void { this.exec.setRuntimeVar(key, value); }
  renameRuntimeVar(oldKey: string, newKey: string): void { this.exec.renameRuntimeVar(oldKey, newKey); }
  removeRuntimeVar(key: string): void { this.exec.removeRuntimeVar(key); }
  replayIssue(draft: ApiDraft = this.draft): string | null { return this.exec.replayIssue(draft); }
  openHistorySavedRequest(): Promise<void> { return this.exec.openHistorySavedRequest(); }
  execute(draft: ApiDraft = this.draft): Promise<ApiResponse | null> { return this.exec.execute(draft); }
  executeGrpc(draft: ApiDraft = this.draft): Promise<ApiResponse | null> { return this.exec.executeGrpc(draft); }
  /** Introspected GraphQL schema (types + fields), or null. */
  get graphqlSchema(): { name: string; kind: string; fields: string[] }[] | null { return this.exec.graphqlSchema; }
  get graphqlIntrospecting(): boolean { return this.exec.graphqlIntrospecting; }
  graphqlIntrospect(): Promise<void> { return this.exec.graphqlIntrospect(); }

  // ── Curl import / export: ./apiClient/curl.ts ───────────────────────────
  importCurl(curl: string): Promise<boolean> { return importCurl(this, curl); }
  toCurl(draft: ApiDraft = this.draft): string { return toCurl(draft); }

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
      toasts.error('Couldn’t clear history', loadErrorText(e));
    }
  }

  // ── Automations (collection runner): lib/stores/apiClient/automations ────
  // The UI reads and writes these through the facade; the state lives in
  // `ApiAutomations` (arch-6-03).

  readonly autos = new ApiAutomations({ base: () => this.base(), workspaceOwner: () => this.workspaceOwner() });
  get automations(): ApiAutomation[] { return this.autos.automations; }
  set automations(v: ApiAutomation[]) { this.autos.automations = v; }
  get automationsLoading(): boolean { return this.autos.automationsLoading; }
  set automationsLoading(v: boolean) { this.autos.automationsLoading = v; }
  get automationsError(): string | null { return this.autos.automationsError; }
  set automationsError(v: string | null) { this.autos.automationsError = v; }
  get lastRun(): ApiRunResult | null { return this.autos.lastRun; }
  set lastRun(v: ApiRunResult | null) { this.autos.lastRun = v; }
  get running(): boolean { return this.autos.running; }
  set running(v: boolean) { this.autos.running = v; }
  get automationRuns(): ApiAutomationRun[] { return this.autos.automationRuns; }
  set automationRuns(v: ApiAutomationRun[]) { this.autos.automationRuns = v; }
  get runsLoading(): boolean { return this.autos.runsLoading; }
  set runsLoading(v: boolean) { this.autos.runsLoading = v; }
  get runsError(): string | null { return this.autos.runsError; }
  set runsError(v: string | null) { this.autos.runsError = v; }
  get currentRun(): ApiAutomationRun | null { return this.autos.currentRun; }
  set currentRun(v: ApiAutomationRun | null) { this.autos.currentRun = v; }
  get runLost(): Id | null { return this.autos.runLost; }
  set runLost(v: Id | null) { this.autos.runLost = v; }
  private get automationsLoadedAt(): { base: string; at: number } | null { return this.autos.automationsLoadedAt; }
  private set automationsLoadedAt(v: { base: string; at: number } | null) { this.autos.automationsLoadedAt = v; }

  loadAutomations(opts: { force?: boolean } = {}): Promise<void> { return this.autos.loadAutomations(opts); }
  saveAutomation(req: UpsertApiAutomationReq, id?: Id): Promise<ApiAutomation | null> { return this.autos.saveAutomation(req, id); }
  deleteAutomation(id: Id): Promise<void> { return this.autos.deleteAutomation(id); }
  noteRunProgress(runId: Id, status?: string): void { this.autos.noteRunProgress(runId, status); }
  getAutomationRun(id: Id): Promise<ApiAutomationRun | null> { return this.autos.getAutomationRun(id); }
  loadAutomationRuns(automationId?: Id, before?: Id): Promise<void> { return this.autos.loadAutomationRuns(automationId, before); }
  cancelAutomationRun(id: Id): Promise<void> { return this.autos.cancelAutomationRun(id); }
  runAutomation(id: Id, options: StartApiAutomationRunReq = {}): Promise<ApiRunResult | null> { return this.autos.runAutomation(id, options); }

  // ── Draft helpers + full-row cache: ./apiClient/drafts.svelte.ts ────────
  readonly drafts: ApiDrafts = new ApiDrafts(this);
  saveDraft(name: string, collectionId: Id | null): Promise<ApiRequest | null> { return this.drafts.saveDraft(name, collectionId); }
  newDraft(collectionHint: Id | null = null): void { this.drafts.newDraft(collectionHint); }
  duplicateDraft(): void { this.drafts.duplicateDraft(); }
  /** (part host) Put a draft in front without losing anyone's edits. */
  placeDraft(d: ApiDraft): void { this.drafts.placeDraft(d); }
  loadRequestIntoDraft(r: ApiRequest): void { this.drafts.loadRequestIntoDraft(r); }
  openRequest(id: Id): Promise<boolean> { return this.drafts.openRequest(id); }
  ensureRequest(id: Id): Promise<ApiRequest | null> { return this.drafts.ensureRequest(id); }
  /** (part host) Cache a saved / re-fetched full row and patch its tree row. */
  upsertRequest(r: ApiRequest): void { this.drafts.upsertRequest(r); }
  private dropRequest(id: Id): void { this.drafts.dropRequest(id); }
  isDirty(d: ApiDraft): boolean { return this.drafts.isDirty(d); }
  loadHistoryIntoDraft(h: ApiHistoryEntry): Promise<void> { return this.drafts.loadHistoryIntoDraft(h); }
}

export const apiClient = new ApiClientStore();
// `api_client_changed` is routed with `peek()` (events.svelte.ts): announce
// so a statically imported store still sees it.
announceModule('apiClient', apiClient);
