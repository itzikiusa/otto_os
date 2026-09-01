// Browser module store: the workspace's tabs, the active tab's fetched
// reader-mode page, and per-page annotations. Like the scheduled-tasks/loops
// stores it does NOT import events.svelte.ts — the event dispatcher calls
// `browser.applyEvent(...)` on this singleton when a `browser_tab_updated` /
// `browser_annotation_added` WS event arrives.

import * as browserApi from '../api/browser';
import type { BrowserAnnotation, BrowserPage, BrowserTab, OttoEvent } from '../api/types';

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
    this.tabs = [...this.tabs, tab];
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
    if (tab) void this.loadPage(tab.url);
  }

  async closeTab(id: string): Promise<void> {
    await browserApi.closeTab(id);
    this.tabs = this.tabs.filter((t) => t.id !== id);
    if (this.activeId === id) {
      this.activeId = this.tabs.length ? this.tabs[0].id : null;
      if (this.activeId) {
        const tab = this.activeTab;
        if (tab) await this.loadPage(tab.url);
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

  /** Live WS tick: a tab was created/navigated elsewhere — refresh the strip
   *  (and the open page, if it's the tab that changed) in place. */
  applyEvent(ev: Extract<OttoEvent, { type: 'browser_tab_updated' | 'browser_annotation_added' }>): void {
    if (this.wsId && ev.workspace_id !== this.wsId) return;
    if (ev.type === 'browser_tab_updated') {
      const tab = ev.tab as BrowserTab;
      const exists = this.tabs.some((t) => t.id === tab.id);
      this.tabs = exists ? this.tabs.map((t) => (t.id === tab.id ? tab : t)) : [...this.tabs, tab];
      if (tab.id === this.activeId) void this.loadPage(tab.url);
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
