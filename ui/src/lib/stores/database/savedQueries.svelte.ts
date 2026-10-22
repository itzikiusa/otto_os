// Database Explorer store — the workspace's saved queries. The DatabaseStore
// façade (../database.svelte.ts) owns an instance and delegates its
// `savedQueries*` fields and methods here; the host supplies the workspace
// base, the access epoch and the query-tab hooks (a saved query is opened in,
// and saved from, a query tab).

import { api } from '../../api/client';
import type { DbSavedQuery, Id } from '../../api/types';
import { toasts } from '../../toast.svelte';
import { toastError } from '../../toastError';
import { loadErrorText } from '../../loadError';
import type { QueryTab } from './model';

/** What the saved-queries store reads from (and asks of) its DatabaseStore host. */
export interface SavedQueriesHost {
  /** `/workspaces/{id}/db`, or null with no workspace. */
  wsBase(): string | null;
  /** Bumped on every access change: a load started before one must not land. */
  accessEpoch(): number;
  selectedConnId(): Id | null;
  /** The visible connection's tabs and the active one. */
  tabs(): QueryTab[];
  activeTab(): QueryTab | null;
  /** Which connection (possibly a parked snapshot) owns a tab. */
  locateTab(tabId: number): { connId: Id; tab: QueryTab } | null;
  /** Persist the tabs of `connId` now / the visible ones (debounced). */
  persistTabsNow(connId: Id): void;
  persistTabs(): void;
  /** Focus tab `i` on the Query view. */
  focusQueryTab(i: number): void;
  /** Open a fresh tab with `statement` (it becomes the active tab). */
  newTab(statement: string): void;
}

export class DbSavedQueriesStore {
  private readonly host: SavedQueriesHost;
  constructor(host: SavedQueriesHost) {
    this.host = host;
  }

  savedQueries: DbSavedQuery[] = $state([]);
  savedQueriesLoading = $state(false);
  savedQueriesError: string | null = $state(null);
  private savedQueriesScope: string | null = null;
  private savedQueriesRequest = 0;

  async loadSavedQueries(): Promise<void> {
    const accessEpoch = this.host.accessEpoch();
    const base = this.host.wsBase();
    const request = ++this.savedQueriesRequest;
    if (this.savedQueriesScope !== base) {
      this.savedQueries = [];
      this.savedQueriesError = null;
      this.savedQueriesScope = base;
    }
    if (!base) { this.savedQueriesLoading = false; return; }
    const current = () =>
      request === this.savedQueriesRequest && base === this.host.wsBase() && accessEpoch === this.host.accessEpoch();
    this.savedQueriesLoading = true;
    try {
      const queries = await api.get<DbSavedQuery[]>(`${base}/saved-queries`);
      if (current()) { this.savedQueries = queries; this.savedQueriesError = null; }
    } catch (e) {
      if (current()) this.savedQueriesError = loadErrorText(e);
    } finally {
      if (current()) this.savedQueriesLoading = false;
    }
  }

  /** Create a NEW saved query, associating the active tab with it so a later
   *  "Save" updates it in place. Used by "Save as new" and first-time saves. */
  async saveQuery(name: string, statement: string): Promise<DbSavedQuery | null> {
    const tab = this.host.activeTab();
    const accessEpoch = this.host.accessEpoch();
    const base = this.host.wsBase();
    if (!base) return null;
    try {
      const saved = await api.post<DbSavedQuery>(`${base}/saved-queries`, {
        connection_id: this.host.selectedConnId(),
        name,
        statement,
      });
      if (accessEpoch !== this.host.accessEpoch() || base !== this.host.wsBase()) return saved;
      this.savedQueries = [saved, ...this.savedQueries.filter((q) => q.id !== saved.id)];
      // The initiating tab may be parked in another connection's snapshot.
      // Update and persist its owner without changing the visible connection.
      const owner = tab ? this.host.locateTab(tab.id) : null;
      if (tab && owner?.tab === tab) {
        tab.savedQueryId = saved.id;
        tab.name = saved.name;
        this.host.persistTabsNow(owner.connId);
      }
      toasts.success('Query saved', saved.name);
      return saved;
    } catch (e) {
      toastError('Couldn’t save query', e);
      return null;
    }
  }

  /** Update a saved query in place (name and/or statement) via PATCH. Callers
   *  own the success toast; this only surfaces errors. */
  async updateSavedQuery(
    id: Id,
    patch: { name?: string; statement?: string },
  ): Promise<DbSavedQuery | null> {
    try {
      const updated = await api.patch<DbSavedQuery>(`/db/saved-queries/${id}`, patch);
      this.savedQueries = this.savedQueries.map((q) => (q.id === updated.id ? updated : q));
      return updated;
    } catch (e) {
      toastError('Couldn’t update the query', e);
      return null;
    }
  }

  /** Inline rename of a saved query (Saved list), keeping any open tab's title
   *  in sync. */
  async renameSavedQuery(id: Id, name: string): Promise<void> {
    const trimmed = name.trim();
    if (!trimmed) return;
    const updated = await this.updateSavedQuery(id, { name: trimmed });
    if (!updated) return;
    for (const t of this.host.tabs()) if (t.savedQueryId === id) t.name = updated.name;
    this.host.persistTabs();
    toasts.success('Renamed', updated.name);
  }

  /**
   * Save the ACTIVE tab. When the tab already came from a saved query (and it
   * still exists) this updates that one in place (PATCH); otherwise it creates a
   * new saved query. `name` overrides the stored name when non-empty (the Save
   * dialog supplies it for the create case).
   */
  async saveActiveTab(name: string): Promise<DbSavedQuery | null> {
    const t = this.host.activeTab();
    if (!t) return null;
    const existing =
      t.savedQueryId && this.savedQueries.some((q) => q.id === t.savedQueryId)
        ? t.savedQueryId
        : null;
    if (existing) {
      const updated = await this.updateSavedQuery(existing, {
        name: name.trim() || undefined,
        statement: t.statement,
      });
      if (updated) {
        t.name = updated.name;
        this.host.persistTabs();
        toasts.success('Query updated', updated.name);
      }
      return updated;
    }
    return this.saveQuery(name.trim() || 'Query', t.statement);
  }

  async deleteSavedQuery(id: Id): Promise<void> {
    try {
      await api.del(`/db/saved-queries/${id}`);
      this.savedQueries = this.savedQueries.filter((q) => q.id !== id);
      // Detach any open tab that pointed at it (its "Save" reverts to create).
      for (const t of this.host.tabs()) if (t.savedQueryId === id) t.savedQueryId = undefined;
    } catch (e) {
      toastError('Couldn’t delete query', e);
    }
  }

  /** Open a saved query. If a tab already carries it, focus that tab (don't
   *  duplicate); otherwise open a fresh tab that remembers the saved-query id. */
  openSavedQuery(q: DbSavedQuery): void {
    const idx = this.host.tabs().findIndex((t) => t.savedQueryId === q.id);
    if (idx >= 0) {
      this.host.focusQueryTab(idx);
      return;
    }
    this.host.newTab(q.statement);
    const t = this.host.activeTab();
    if (t) {
      t.name = q.name;
      t.savedQueryId = q.id;
    }
    this.host.persistTabs();
  }
}
