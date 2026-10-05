// Browser module store: the workspace's tabs, the active tab's fetched
// reader-mode page, and per-page annotations. Like the scheduled-tasks/loops
// stores it does NOT import events.svelte.ts — the event dispatcher calls
// `browser.applyEvent(...)` on this singleton when a `browser_tab_updated` /
// `browser_annotation_added` WS event arrives.

import * as browserApi from '../api/browser';
import { nativeBrowserAvailable } from '../nativeBrowser';
import { browserLive } from './browserLive.svelte';
import type { BrowserAnnotation, BrowserAskReq, BrowserPage, BrowserTab, OttoEvent } from '../api/types';
import { announceModule } from '../lazyModule';

/** localStorage key for the agent session the Browser page's dock is attached
 *  to — per workspace, so switching workspaces re-attaches to that
 *  workspace's own browser agent (or none). */
const agentKey = (wsId: string) => `otto_browser_agent_${wsId}`;

/** How long a per-tab cached reader page is shown without revalidating —
 *  matches the daemon's own page-cache TTL. */
const PAGE_CACHE_FRESH_MS = 60_000;
/** Most reader pages the per-tab cache keeps (markdown only, no html). */
const PAGE_CACHE_MAX = 16;

function lsGet(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null;
  }
}
function lsSet(key: string, val: string | null): void {
  try {
    if (val === null) localStorage.removeItem(key);
    else localStorage.setItem(key, val);
  } catch {
    /* private mode / quota — the binding just won't persist */
  }
}

/** A `mode:"live"` tab only skips the reader fetch where something actually
 *  renders it live: the desktop app's child webview, or the daemon's
 *  streamed Chromium (browserLive). Only a daemon that predates live
 *  streaming (`supported === false`) off the desktop app falls back to
 *  reader, so the store must still fetch the page for it there. */
function isNativeLive(tab: BrowserTab): boolean {
  if (tab.mode !== 'live') return false;
  if (browserLive.renderer === 'native') return nativeBrowserAvailable;
  return browserLive.supported !== false;
}

class BrowserStore {
  tabs: BrowserTab[] = $state([]);
  loadingTabs = $state(false);
  /** The open tab's id, or null when nothing is open. */
  activeId: string | null = $state(null);
  /** The active tab's fetched page (reader mode), or null while loading/empty. */
  page: BrowserPage | null = $state(null);
  loadingPage = $state(false);
  pageError = $state('');
  /** Annotations for the active tab's URL. */
  annotations: BrowserAnnotation[] = $state([]);
  /** The agent session the Browser page's dock is attached to (embedded
   *  terminal + ask bar), or null when none. Persisted per workspace; the
   *  view validates it against the live session list on attach. */
  agentSessionId: string | null = $state(null);
  /** Bumped whenever a mark is created from this device, so the ask bar can
   *  react (focus + hint) without the dock having to diff the list. */
  markTick = $state(0);
  private wsId = '';
  /** Request token for page loads: a slow fetch (up to 30 s) for tab A must
   *  not land over tab B's page — or another workspace's — after a switch. */
  private pageSeq = 0;
  private annotationSeq = 0;
  private workspaceGeneration = 0;
  private tabsSequence = 0;
  private navigationSequence = new Map<string, number>();
  // Client response guards cannot order writes at the server. Keep at most
  // one pending chain per tab; settled chains release their cache entry.
  private tabMutations = new Map<string, Promise<BrowserTab>>();

  private patchTab(id: string, patch: Parameters<typeof browserApi.navigateTab>[1]): Promise<BrowserTab> {
    const previous = this.tabMutations.get(id);
    const task = previous
      ? previous.catch(() => undefined).then(() => browserApi.navigateTab(id, patch))
      : browserApi.navigateTab(id, patch);
    this.tabMutations.set(id, task);
    const release = () => { if (this.tabMutations.get(id) === task) this.tabMutations.delete(id); };
    void task.then(release, release);
    return task;
  }

  get activeTab(): BrowserTab | null {
    return this.tabs.find((t) => t.id === this.activeId) ?? null;
  }

  async loadTabs(workspaceId: string): Promise<void> {
    if (this.wsId !== workspaceId) {
      this.wsId = workspaceId;
      this.workspaceGeneration++;
      this.navigationSequence.clear();
      this.agentSessionId = lsGet(agentKey(workspaceId));
      // A workspace switch must not keep showing the previous workspace's
      // open page / marks (or let its in-flight page load land).
      this.pageSeq++;
      this.activeId = null;
      this.page = null;
      this.pageError = '';
      this.annotations = [];
      this.loadingPage = false;
      this.summary = '';
    }
    const generation = this.workspaceGeneration, sequence = ++this.tabsSequence;
    const current = () => generation === this.workspaceGeneration && sequence === this.tabsSequence;
    this.loadingTabs = true;
    try {
      const tabs = await browserApi.listTabs(workspaceId);
      if (!current()) return;
      this.tabs = tabs;
      if (!this.activeId && this.tabs.length) this.activeId = this.tabs[0].id;
    } catch {
      if (current()) this.tabs = [];
    } finally {
      if (current()) this.loadingTabs = false;
    }
  }

  async openTab(url: string): Promise<BrowserTab> {
    const generation = this.workspaceGeneration;
    const tab = await browserApi.createTab(this.wsId, url);
    if (generation !== this.workspaceGeneration) return tab;
    // The server's own `browser_tab_updated` broadcast for this exact create
    // can beat the HTTP response back to the client (WS push vs. awaited
    // fetch aren't ordered) — `applyEvent` may have already appended it. Dedupe
    // by id, same as `applyEvent` does for the reverse race.
    this.tabs = this.tabs.some((t) => t.id === tab.id)
      ? this.tabs.map((t) => (t.id === tab.id ? tab : t))
      : [...this.tabs, tab];
    this.activeId = tab.id;
    this.summary = '';
    await this.loadPage(url);
    return tab;
  }

  /** Deselect the active tab (the URL bar clears for a fresh "new tab" entry;
   *  nothing is closed or navigated until the user submits a URL). */
  deselect(): void {
    this.pageSeq++; // an in-flight reader load must not repopulate the blank tab
    this.activeId = null;
    this.page = null;
    this.annotations = [];
    this.summary = '';
  }

  select(id: string): void {
    // The summary panel belongs to the page it summarized.
    if (id !== this.activeId) this.summary = '';
    this.activeId = id;
    this.annotations = [];
    const tab = this.activeTab;
    // A live tab that's actually rendered natively skips the reader fetch —
    // off Tauri it still falls back to reader (isNativeLive is false there).
    if (tab && !isNativeLive(tab)) {
      // Switching back to a tab is instant from the per-tab page cache; only
      // an entry older than PAGE_CACHE_FRESH_MS revalidates (in the
      // background — the cached page stays on screen meanwhile).
      const hit = this.cachedPage(tab.url);
      if (hit) {
        this.pageSeq++;
        this.page = hit.page;
        this.pageError = '';
        this.loadingPage = false;
        void this.loadAnnotations(tab.url);
        if (Date.now() - hit.at > PAGE_CACHE_FRESH_MS) void this.loadPage(tab.url, { background: true });
      } else {
        void this.loadPage(tab.url);
      }
    } else {
      this.pageSeq++;
      this.page = null;
      this.pageError = '';
    }
  }

  /** Flip tab `id`'s mode. Switching to reader loads the reader-fetched page;
   *  switching to live drops it (the view hosts a native/embedded webview
   *  instead — see BrowserView). */
  async setMode(id: string, mode: 'reader' | 'live'): Promise<void> {
    const tab = this.tabs.find((t) => t.id === id);
    if (!tab || tab.mode === mode) return;
    const patched = await this.patchTab(id, { mode });
    this.tabs = this.tabs.map((t) => (t.id === patched.id ? patched : t));
    if (id !== this.activeId) return;
    if (isNativeLive(patched)) {
      this.page = null;
      this.pageError = '';
    } else {
      void this.loadPage(patched.url);
    }
  }

  /** Create a tab that opens directly in live mode (e.g. a `window.open()`
   *  fired from inside another live tab) — skips the reader fetch entirely. */
  async openLiveTab(url: string): Promise<BrowserTab> {
    const generation = this.workspaceGeneration;
    const tab = await browserApi.createTab(this.wsId, url);
    const patched = await this.patchTab(tab.id, { mode: 'live' });
    if (generation !== this.workspaceGeneration) return patched;
    this.tabs = this.tabs.some((t) => t.id === patched.id)
      ? this.tabs.map((t) => (t.id === patched.id ? patched : t))
      : [...this.tabs, patched];
    this.activeId = patched.id;
    this.page = null;
    this.pageError = '';
    return patched;
  }

  /** Record a live tab's in-page navigation (from the native webview's
   *  on-navigation event) locally — no server round-trip per keystroke-level
   *  nav; `navigate()`/PATCH still persists explicit address-bar submits. */
  trackLiveNav(id: string, url: string, title?: string): void {
    this.tabs = this.tabs.map((t) =>
      t.id === id && t.url !== url ? { ...t, url, title: title || t.title } : t,
    );
  }

  async closeTab(id: string): Promise<void> {
    this.navigationSequence.delete(id);
    const wasActive = this.activeId === id;
    const previous = { page: this.page, annotations: this.annotations, error: this.pageError };
    const workspace = this.workspaceGeneration;
    if (wasActive) {
      // Invalidate before either await: the old reader can finish while the
      // close request (or its preceding PATCH) is still pending.
      this.pageSeq++;
      this.annotationSeq++;
      this.page = null;
      this.annotations = [];
      this.pageError = '';
      this.loadingPage = false;
    }
    const sequence = this.pageSeq;
    try {
      await this.tabMutations.get(id)?.catch(() => undefined);
      await browserApi.closeTab(id);
    } catch (e) {
      if (wasActive && this.activeId === id && this.workspaceGeneration === workspace && this.pageSeq === sequence) {
        this.page = previous.page;
        this.annotations = previous.annotations;
        this.pageError = previous.error;
        const tab = this.activeTab;
        if (!this.page && tab && !isNativeLive(tab)) void this.loadPage(tab.url);
      }
      throw e;
    }
    const at = this.tabs.findIndex((t) => t.id === id);
    this.tabs = this.tabs.filter((t) => t.id !== id);
    if (this.activeId === id) {
      // Like any browser: the tab that slid into the closed one's place (or
      // the new last one), not a jump back to the first tab.
      const next = this.tabs[Math.min(Math.max(at, 0), this.tabs.length - 1)];
      this.activeId = next ? next.id : null;
      if (this.activeId) {
        const tab = this.activeTab;
        if (tab && !isNativeLive(tab)) await this.loadPage(tab.url);
        else {
          this.page = null;
          this.pageError = '';
        }
      } else {
        this.page = null;
        this.annotations = [];
      }
    }
  }

  /** Navigate the active tab to a new URL: fetches the page, then patches the
   *  tab (adopts the fetched title) so the tab strip + history stay in sync. */
  async navigate(url: string): Promise<void> {
    const tab = this.activeTab;
    if (!tab) {
      await this.openTab(url);
      return;
    }
    const generation = this.workspaceGeneration;
    const sequence = (this.navigationSequence.get(tab.id) ?? 0) + 1;
    this.navigationSequence.set(tab.id, sequence);
    const current = () => generation === this.workspaceGeneration && this.navigationSequence.get(tab.id) === sequence;
    if (isNativeLive(tab)) {
      // The native webview does the actual navigation (BrowserView's driver
      // effect picks up the URL change below); just persist it so the tab
      // strip and a future reload reflect it — no reader fetch.
      const patched = await this.patchTab(tab.id, { url, title: url });
      if (!current()) return;
      this.tabs = this.tabs.map((t) => (t.id === patched.id ? patched : t));
      return;
    }
    const page = await this.loadPage(url);
    if (!current()) return;
    const patched = await this.patchTab(tab.id, {
      url,
      title: page?.title || url,
    });
    if (!current()) return;
    this.tabs = this.tabs.map((t) => (t.id === patched.id ? patched : t));
  }

  /** Per-tab reader page cache, keyed `ws\nurl`, newest last (Map order) and
   *  capped at PAGE_CACHE_MAX entries — re-selecting a tab doesn't re-render. */
  private pageCache = new Map<string, { page: BrowserPage; at: number }>();

  private cachedPage(url: string): { page: BrowserPage; at: number } | undefined {
    return this.pageCache.get(`${this.wsId}\n${url}`);
  }

  private rememberPage(ws: string, url: string, page: BrowserPage): void {
    const key = `${ws}\n${url}`;
    this.pageCache.delete(key);
    this.pageCache.set(key, { page, at: Date.now() });
    while (this.pageCache.size > PAGE_CACHE_MAX) {
      const oldest = this.pageCache.keys().next().value;
      if (oldest === undefined) break;
      this.pageCache.delete(oldest);
    }
  }

  /** Fetch `url` into the reader. `fresh` bypasses both the per-tab cache and
   *  the daemon's one-minute page cache (Retry); `background` keeps the page
   *  that's on screen (no spinner, no error swap) while revalidating. */
  async loadPage(url: string, opts: { fresh?: boolean; background?: boolean } = {}): Promise<BrowserPage | undefined> {
    const mine = ++this.pageSeq;
    const ws = this.wsId;
    const current = () => mine === this.pageSeq && ws === this.wsId;
    if (!opts.background) {
      this.loadingPage = true;
      this.pageError = '';
    }
    try {
      // Page + annotations in parallel — they're independent reads.
      const [page, annotations] = await Promise.all([
        browserApi.getPage(ws, url, { fresh: opts.fresh }),
        browserApi.listAnnotations(ws, url).catch(() => [] as BrowserAnnotation[]),
      ]);
      this.rememberPage(ws, url, page);
      if (!current()) return page;
      this.page = page;
      this.annotations = annotations;
      return page;
    } catch (e) {
      if (!current() || opts.background) return;
      this.page = null;
      this.pageError = e instanceof Error ? e.message : 'Failed to load page';
    } finally {
      if (current() && !opts.background) this.loadingPage = false;
    }
  }

  async loadAnnotations(url: string): Promise<void> {
    const ws = this.wsId, tab = this.activeId, page = this.pageSeq;
    const request = ++this.annotationSeq;
    let next: BrowserAnnotation[];
    try {
      next = await browserApi.listAnnotations(ws, url);
    } catch {
      next = [];
    }
    if (ws === this.wsId && tab === this.activeId && page === this.pageSeq && request === this.annotationSeq && this.activeTab?.url === url) this.annotations = next;
  }

  async summarize(url: string, signal?: AbortSignal) {
    return browserApi.summarize(this.wsId, url, signal);
  }

  /** The summary panel's text (drafted by Otto) and whether one is running.
   *  Store state — not BrowserView's — so an agent-driven summarize (agent UI
   *  control, lib/uiCommands/browser.ts) shows in the same panel as a click. */
  summary = $state('');
  summarizing = $state(false);

  /** Summarize `url` into the summary panel. Returns the text; throws on
   *  failure (the caller reports it). A result for a page the user has since
   *  left is still returned but not shown. */
  async runSummarize(url: string): Promise<string> {
    this.summarizing = true;
    const ctl = new AbortController();
    this.summarizeCtl = ctl;
    try {
      const resp = await this.summarize(url, ctl.signal);
      if (this.activeTab?.url === url) this.summary = resp.summary;
      return resp.summary;
    } finally {
      if (this.summarizeCtl === ctl) this.summarizeCtl = null;
      this.summarizing = false;
    }
  }

  private summarizeCtl: AbortController | null = null;
  /** Stop the running summarize: abort its request — the daemon kills the
   *  backing agent session when the request goes away mid-turn. */
  stopSummarize(): void {
    this.summarizeCtl?.abort();
  }

  /** Create a DOM annotation (a "mark") against the active page's URL. Pushes
   *  the created row into `annotations` immediately for instant feedback —
   *  the later `browser_annotation_added` WS tick is a no-op dupe (see
   *  `applyEvent`'s `exists` guard) since this device already has it. */
  async createAnnotation(body: {
    url: string;
    selector: string;
    excerpt: string;
    text: string;
    comment?: string;
    color?: string;
  }): Promise<BrowserAnnotation> {
    const ws = this.wsId, tab = this.activeId, page = this.pageSeq;
    const ann = await browserApi.createAnnotation(ws, {
      ...body,
      tab_id: tab ?? undefined,
    });
    if (ws !== this.wsId || tab !== this.activeId || page !== this.pageSeq || this.activeTab?.url !== body.url) return ann;
    if (!this.annotations.some((a) => a.id === ann.id)) {
      this.annotations = [...this.annotations, ann];
    }
    this.markTick++;
    return ann;
  }

  async updateAnnotationComment(id: string, comment: string): Promise<void> {
    const ann = await browserApi.updateAnnotation(id, comment);
    this.annotations = this.annotations.map((a) => (a.id === ann.id ? ann : a));
  }

  async deleteAnnotation(id: string): Promise<void> {
    await browserApi.deleteAnnotation(id);
    this.annotations = this.annotations.filter((a) => a.id !== id);
  }

  async sendAnnotation(id: string, sessionId: string): Promise<void> {
    await browserApi.sendAnnotation(this.wsId, id, sessionId);
  }

  /** Bind (or unbind, with null) the Browser page's agent dock to a session. */
  setAgentSession(id: string | null): void {
    this.agentSessionId = id;
    if (this.wsId) lsSet(agentKey(this.wsId), id);
  }

  /** One "ask" turn: the page + marks + question, submitted into `sessionId`. */
  async ask(body: BrowserAskReq): Promise<void> {
    await browserApi.ask(this.wsId, body);
  }

  async vaultSave(url: string, vaultId: number, summary?: string) {
    return browserApi.vaultSave(this.wsId, { url, vault_id: vaultId, summary });
  }

  /** Live WS tick: a tab was created/navigated elsewhere — refresh the strip
   *  (and the open page, if it's the tab that changed) in place. */
  applyEvent(ev: Extract<OttoEvent, { type: 'browser_tab_updated' | 'browser_annotation_added' }>): void {
    if (this.wsId && ev.workspace_id !== this.wsId) return;
    if (ev.type === 'browser_tab_updated') {
      const tab = ev.tab as BrowserTab;
      const prev = this.tabs.find((t) => t.id === tab.id);
      this.tabs = prev ? this.tabs.map((t) => (t.id === tab.id ? tab : t)) : [...this.tabs, tab];
      // Re-fetch only when the URL actually changed: this client's own
      // navigate already loaded it, and the echo re-fetched the remote page.
      if (tab.id === this.activeId && !isNativeLive(tab) && prev?.url !== tab.url) {
        void this.loadPage(tab.url);
      }
    } else {
      const ann = ev.annotation as BrowserAnnotation;
      if (this.activeTab && ann.url === this.activeTab.url) {
        const exists = this.annotations.some((a) => a.id === ann.id);
        this.annotations = exists
          ? this.annotations.map((a) => (a.id === ann.id ? ann : a))
          : [...this.annotations, ann];
      }
    }
  }
}

export const browser = new BrowserStore();
// Routed by `peek()` in lib/events.svelte.ts (perf H1): let it see this store
// however it was first imported.
announceModule('browser', browser);
