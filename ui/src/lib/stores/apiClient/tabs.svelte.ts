// The API client's open request tabs: the drafts, the active index, their
// per-workspace persistence and each tab's response slot. Split out of
// apiClient.svelte.ts (arch-6-03); the store keeps `tabs`, `draft`,
// `lastResponse`, … as forwarding members, so the UI still reads `apiClient.*`.
//
// Open-tab persistence: open request tabs survive app relaunches. The full
// drafts + active index are written per-workspace to localStorage (same
// pattern as the Git page's open repo tabs) and restored by loadAll(). Two
// windows on the same workspace are last-write-wins per debounce window —
// acceptable for a device-local editor.
//
// Per-tab response slots: every request tab keeps its own response / error /
// test results / script logs / in-flight send, so switching tabs never blanks
// a response, a send in one tab never cancels another tab's, and a response
// that lands while its tab is in the background is kept (not discarded). In
// memory only — bodies are large — and capped at MAX_TAB_RESPONSES idle slots
// (LRU).

import type { ApiAuth, ApiRequest, ApiResponse, Id } from '../../api/types';
import type { TestResult } from '../../api/scripts';
import { stripSecretsForStorage } from '../../api/apiSecretShapes';
import type { ApiClientStore } from '../apiClient.svelte';
import {
  blankDraft, emptySlot, MAX_TAB_RESPONSES, sanitizeDraft, tabsKey, TABS_FALLBACK_BODY_MAX, TABS_WRITE_DELAY_MS,
  type ApiDraft, type PersistedTabs, type TabResponse,
} from './model';

/** The store members this part reads and calls (its whole contract with the façade). */
export type TabsHost = Pick<ApiClientStore, 'fullRequests' | 'wsId' | 'upsertRequest'>;

export class ApiTabs {
  /** Open request tabs; the active one is edited via `draft`. */
  tabs: ApiDraft[] = $state([blankDraft()]);
  activeTab = $state(0);
  /** tabId → that tab's response state. Raw: slots are replaced wholesale. */
  responses: Map<string, TabResponse> = $state.raw(new Map());
  /** Workspace whose tabs are in memory; null until the first restore. Gates
   *  persistence so a not-yet-restored blank tab can't clobber a saved set. */
  wid: Id | null = null;
  private writeTimer: ReturnType<typeof setTimeout> | null = null;

  constructor(private readonly host: TabsHost) {
    // Flush a pending debounced write when the window/app goes away, so the
    // last keystrokes before quit are not lost.
    if (typeof window !== 'undefined') {
      window.addEventListener('pagehide', () => this.flush());
    }
  }

  /** The request currently in the builder (proxies the active tab). */
  get draft(): ApiDraft {
    return this.tabs[this.activeTab] ?? this.tabs[0];
  }
  set draft(d: ApiDraft) {
    this.tabs[this.activeTab] = {...d, tabId: d.tabId ?? this.tabs[this.activeTab]?.tabId ?? crypto.randomUUID()};
    this.persist();
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
    this.persist();
  }
  /** Switching keeps every tab's response slot — the target tab shows its own
   *  last response (or its in-flight send), never a blank pane. */
  switchTab(i: number): void {
    if (i >= 0 && i < this.tabs.length) {
      this.activeTab = i;
      this.persist();
    }
  }
  closeTab(i: number): void {
    this.dropSlot(this.tabs[i]?.tabId);
    if (this.tabs.length === 1) {
      this.tabs = [blankDraft()];
      this.activeTab = 0;
    } else {
      this.tabs = this.tabs.filter((_, idx) => idx !== i);
      if (this.activeTab >= this.tabs.length) this.activeTab = this.tabs.length - 1;
      else if (i < this.activeTab) this.activeTab -= 1;
    }
    this.persist();
  }

  // ── Persistence ───────────────────────────────────────────────────────────

  /** Queue a debounced write of the open tabs to this workspace's slot. */
  persist(): void {
    if (this.wid === null) return;
    if (this.writeTimer !== null) clearTimeout(this.writeTimer);
    this.writeTimer = setTimeout(() => this.flush(), TABS_WRITE_DELAY_MS);
  }

  /** Write immediately (debounce elapsed, workspace swap, or page hide). */
  flush(): void {
    if (this.writeTimer !== null) {
      clearTimeout(this.writeTimer);
      this.writeTimer = null;
    }
    if (this.wid === null || typeof localStorage === 'undefined') return;
    const key = tabsKey(this.wid);
    // Never at rest outside the Keychain: plaintext credentials typed into a
    // tab (or imported from curl / fetched via OAuth) are stripped from the
    // persisted copy (see stripSecretsForStorage).
    const blob: PersistedTabs = {
      tabs: ($state.snapshot(this.tabs) as ApiDraft[]).map((t) =>
        stripSecretsForStorage(t, t.requestId ? this.host.fullRequests.get(t.requestId) : undefined),
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

  /** Swap in `wid`'s persisted tabs: flush the previous workspace's pending
   *  write first so nothing is lost, and abort every slot's send. The store's
   *  `restoreTabs` resets the workspace's other state around this. */
  swapTo(wid: Id): void {
    this.flush();
    this.resetResponses();
    this.wid = wid;
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
  }

  /** OAuth may finish after navigation. Update the owning workspace's tabs,
   * keeping auth edits made after the flow started. Saved values are markers. */
  applySavedAuth(wid: Id, request: ApiRequest, previousAuth: ApiAuth): void {
    const before = JSON.stringify(previousAuth);
    const update = (tab: ApiDraft): ApiDraft => tab.requestId === request.id && JSON.stringify(tab.auth) === before
      ? {...tab,auth:{...request.auth}} : tab;
    if (this.wid === wid) {
      this.tabs = this.tabs.map(update); this.persist();
      if (this.host.wsId() === wid) this.host.upsertRequest(request);
    } else {
      try {
        const raw = localStorage.getItem(tabsKey(wid));
        if (raw) {const saved = JSON.parse(raw) as PersistedTabs; if (Array.isArray(saved.tabs)) {
          saved.tabs = saved.tabs.map(update);localStorage.setItem(tabsKey(wid),JSON.stringify(saved));
        }}
      } catch { /* The server retains the saved request if browser storage is unavailable. */ }
    }
  }

  // ── Per-tab response slots ────────────────────────────────────────────────

  slot(tabId: string | null | undefined): TabResponse | undefined {
    return tabId ? this.responses.get(tabId) : undefined;
  }
  /** Merge `patch` into `tabId`'s slot (moving it to most-recent), evicting
   *  the least-recently-touched idle slots beyond the cap. */
  patchSlot(tabId: string | null | undefined, patch: Partial<TabResponse>): void {
    if (!tabId) return;
    const next = new Map(this.responses);
    const prev = next.get(tabId) ?? emptySlot();
    next.delete(tabId);
    next.set(tabId, { ...prev, ...patch, at: Date.now() });
    for (const [k, v] of next) {
      if (next.size <= MAX_TAB_RESPONSES) break;
      if (k !== tabId && !v.sending) next.delete(k);
    }
    this.responses = next;
  }
  /** Abort `tabId`'s in-flight send (if any) and forget its slot. */
  dropSlot(tabId: string | null | undefined): void {
    const s = this.slot(tabId);
    if (!s || !tabId) return;
    s.ctrl?.abort();
    const next = new Map(this.responses);
    next.delete(tabId);
    this.responses = next;
  }
  /** Abort every tab's send and clear all slots (workspace swap). */
  resetResponses(): void {
    for (const s of this.responses.values()) s.ctrl?.abort();
    this.responses = new Map();
  }

  /** The active tab's last response, shown in the ResponseViewer. */
  get lastResponse(): ApiResponse | null {
    return this.slot(this.draft?.tabId)?.resp ?? null;
  }
  set lastResponse(resp: ApiResponse | null) {
    this.patchSlot(this.draft?.tabId, { resp, history: null });
  }
  /** Why the active tab's last send failed (shown inline in the response
   *  pane instead of the previous response); null after a success. */
  get lastError(): string | null {
    return this.slot(this.draft?.tabId)?.error ?? null;
  }
  set lastError(error: string | null) {
    this.patchSlot(this.draft?.tabId, { error });
  }
  /** The active tab has a send in flight. */
  get sending(): boolean {
    return this.slot(this.draft?.tabId)?.sending ?? false;
  }
  /** Combined pre/post script console output for the active tab's last run. */
  get scriptLogs(): string[] {
    return this.slot(this.draft?.tabId)?.logs ?? [];
  }
  /** Post-response test results for the active tab's last run. */
  get testResults(): TestResult[] {
    return this.slot(this.draft?.tabId)?.tests ?? [];
  }
  /** The active tab's response was restored from a history entry (not sent
   *  in this session) — the ResponseViewer's "From history" banner. */
  get responseFromHistory(): TabResponse['history'] {
    return this.slot(this.draft?.tabId)?.history ?? null;
  }
  /** Tab-chip indicator: `sending` while in flight, else the outcome of the
   *  tab's last send (`ok` 2xx/3xx, `fail` 4xx/5xx or an error), or null. */
  tabStatus(tabId: string | null | undefined): 'sending' | 'ok' | 'fail' | null {
    const s = this.slot(tabId);
    if (!s) return null;
    if (s.sending) return 'sending';
    if (s.error) return 'fail';
    if (s.resp) return s.resp.status >= 400 ? 'fail' : 'ok';
    return null;
  }
  /** Cancel `tabId`'s in-flight send (default: the active tab's). Other
   *  tabs' sends are untouched. No-op when idle. */
  cancelExecute(tabId: string | null | undefined = this.draft?.tabId): void {
    const s = this.slot(tabId);
    if (!s?.ctrl && !s?.sending) return;
    s.ctrl?.abort();
    this.patchSlot(tabId, { ctrl: null, sending: false });
  }
}
