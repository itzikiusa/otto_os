// Browser module store: the workspace's tabs, the active tab's fetched
// reader-mode page, and per-page annotations. Like the scheduled-tasks/loops
// stores it does NOT import events.svelte.ts — the event dispatcher calls
// `browser.applyEvent(...)` on this singleton when a `browser_tab_updated` /
// `browser_annotation_added` WS event arrives.

import * as browserApi from '../api/browser';
import { nativeBrowserAvailable } from '../nativeBrowser';
import type { BrowserAnnotation, BrowserPage, BrowserTab, OttoEvent } from '../api/types';

/** A `mode:"live"` tab only skips the reader fetch where something actually
 *  renders it live (a Tauri child webview) — off Tauri (remote/PWA), the view
 *  falls back to reader, so the store must still fetch the page for it. */
function isNativeLive(tab: BrowserTab): boolean {
  return tab.mode === 'live' && nativeBrowserAvailable;
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
  private wsId = '';

  get activeTab(): BrowserTab | null {
    return this.tabs.find((t) => t.id === this.activeId) ?? null;
  }

  async loadTabs(workspaceId: string): Promise<void> {
    this.wsId = workspaceId;
    this.loadingTabs = true;
    try {
      this.tabs = await browserApi.listTabs(workspaceId);
      if (!this.activeId && this.tabs.length) this.activeId = this.tabs[0].id;
    } catch {
      this.tabs = [];
    } finally {
      this.loadingTabs = false;
    }
  }

  async openTab(url: string): Promise<BrowserTab> {
    const tab = await browserApi.createTab(this.wsId, url);
    // The server's own `browser_tab_updated` broadcast for this exact create
    // can beat the HTTP response back to the client (WS push vs. awaited
    // fetch aren't ordered) — `applyEvent` may have already appended it. Dedupe
    // by id, same as `applyEvent` does for the reverse race.
    this.tabs = this.tabs.some((t) => t.id === tab.id)
      ? this.tabs.map((t) => (t.id === tab.id ? tab : t))
      : [...this.tabs, tab];
    this.activeId = tab.id;
    await this.loadPage(url);
    return tab;
  }

  /** Deselect the active tab (the URL bar clears for a fresh "new tab" entry;
   *  nothing is closed or navigated until the user submits a URL). */
  deselect(): void {
    this.activeId = null;
    this.page = null;
    this.annotations = [];
  }

  select(id: string): void {
    this.activeId = id;
    const tab = this.activeTab;
    // A live tab that's actually rendered natively skips the reader fetch —
    // off Tauri it still falls back to reader (isNativeLive is false there).
    if (tab && !isNativeLive(tab)) {
      void this.loadPage(tab.url);
    } else {
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
    const patched = await browserApi.navigateTab(id, { mode });
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
    const tab = await browserApi.createTab(this.wsId, url);
    const patched = await browserApi.navigateTab(tab.id, { mode: 'live' });
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
    await browserApi.closeTab(id);
    this.tabs = this.tabs.filter((t) => t.id !== id);
    if (this.activeId === id) {
      this.activeId = this.tabs.length ? this.tabs[0].id : null;
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
    if (isNativeLive(tab)) {
      // The native webview does the actual navigation (BrowserView's driver
      // effect picks up the URL change below); just persist it so the tab
      // strip and a future reload reflect it — no reader fetch.
      const patched = await browserApi.navigateTab(tab.id, { url, title: url });
      this.tabs = this.tabs.map((t) => (t.id === patched.id ? patched : t));
      return;
    }
    await this.loadPage(url);
    const patched = await browserApi.navigateTab(tab.id, {
      url,
      title: this.page?.title || url,
    });
    this.tabs = this.tabs.map((t) => (t.id === patched.id ? patched : t));
  }

  async loadPage(url: string): Promise<void> {
    this.loadingPage = true;
    this.pageError = '';
    try {
      this.page = await browserApi.getPage(this.wsId, url);
      await this.loadAnnotations(url);
    } catch (e) {
      this.page = null;
      this.pageError = e instanceof Error ? e.message : 'Failed to load page';
    } finally {
      this.loadingPage = false;
    }
  }

  async loadAnnotations(url: string): Promise<void> {
    try {
      this.annotations = await browserApi.listAnnotations(this.wsId, url);
    } catch {
      this.annotations = [];
    }
  }

  async summarize(url: string) {
    return browserApi.summarize(this.wsId, url);
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
    const ann = await browserApi.createAnnotation(this.wsId, {
      ...body,
      tab_id: this.activeId ?? undefined,
    });
    if (!this.annotations.some((a) => a.id === ann.id)) {
      this.annotations = [...this.annotations, ann];
    }
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

  async vaultSave(url: string, vaultId: number, summary?: string) {
    return browserApi.vaultSave(this.wsId, { url, vault_id: vaultId, summary });
  }

  /** Live WS tick: a tab was created/navigated elsewhere — refresh the strip
   *  (and the open page, if it's the tab that changed) in place. */
  applyEvent(ev: Extract<OttoEvent, { type: 'browser_tab_updated' | 'browser_annotation_added' }>): void {
    if (this.wsId && ev.workspace_id !== this.wsId) return;
    if (ev.type === 'browser_tab_updated') {
      const tab = ev.tab as BrowserTab;
      const exists = this.tabs.some((t) => t.id === tab.id);
      this.tabs = exists ? this.tabs.map((t) => (t.id === tab.id ? tab : t)) : [...this.tabs, tab];
      if (tab.id === this.activeId && !isNativeLive(tab)) void this.loadPage(tab.url);
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
