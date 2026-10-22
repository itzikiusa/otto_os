// Database Explorer store — the visible connection's query history. The
// DatabaseStore façade (../database.svelte.ts) owns an instance and delegates
// its `history*` fields and methods here; the host supplies the selected
// connection, the access epoch and the "open in a new tab" hook. The façade's
// connection switch (snapshot restore / fresh load) still resets the window
// through the delegated setters.

import { api } from '../../api/client';
import type { DbHistoryEntry, Id } from '../../api/types';
import { toastError } from '../../toastError';
import { loadErrorText } from '../../loadError';

/** Largest history window the API serves. */
const HISTORY_MAX = 1000;
const HISTORY_STEP = 100;

/** What the history store reads from (and asks of) its DatabaseStore host. */
export interface HistoryHost {
  selectedConnId(): Id | null;
  /** Bumped on every access change: a load started before one must not land. */
  accessEpoch(): number;
  /** Open a fresh query tab with `statement`. */
  newTab(statement: string): void;
}

export class DbHistoryStore {
  private readonly host: HistoryHost;
  constructor(host: HistoryHost) {
    this.host = host;
  }

  historyLoading = $state(false);
  historyError: string | null = $state(null);
  private historyRequest = 0;
  // Raw: only ever replaced wholesale (a refetch after each run). Deep, every
  // refetch proxied up to 1,000 FULL statements (an import batch is a
  // ~0.3 MB INSERT) and the sidebar's filter walked them through proxies.
  history: DbHistoryEntry[] = $state.raw([]);
  /** How many history rows the current window requested. Bumped by "Load more"
   *  (100 → up to the API's 1000 cap). Reset to 100 on a fresh connection load. */
  historyLimit = $state(HISTORY_STEP);
  /** True while a "Load more" fetch is in flight (disables the button). */
  historyLoadingMore = $state(false);

  private connBase(id: Id): string {
    return `/connections/${id}/db`;
  }

  async loadHistory(connId?: Id): Promise<void> {
    const accessEpoch = this.host.accessEpoch();
    const id = connId ?? this.host.selectedConnId();
    // A background query may finish after its connection is no longer visible.
    // Its history is refreshed when the connection is reopened.
    if (!id || id !== this.host.selectedConnId()) return;
    const request = ++this.historyRequest;
    const current = () =>
      request === this.historyRequest && this.host.selectedConnId() === id && accessEpoch === this.host.accessEpoch();
    this.historyLoading = true;
    try {
      const rows = await api.get<DbHistoryEntry[]>(`${this.connBase(id)}/history?limit=${this.historyLimit}`);
      if (current()) { this.history = rows; this.historyError = null; }
    } catch (e) {
      if (current()) this.historyError = loadErrorText(e);
    } finally {
      if (current()) this.historyLoading = false;
    }
  }

  /** Fetch a larger history window (100 → up to the API's 1000 cap). */
  async loadMoreHistory(): Promise<void> {
    if (this.historyLoadingMore || this.historyLimit >= HISTORY_MAX) return;
    this.historyLimit = Math.min(HISTORY_MAX, this.historyLimit + HISTORY_STEP);
    this.historyLoadingMore = true;
    try {
      await this.loadHistory();
    } finally {
      this.historyLoadingMore = false;
    }
  }

  /** Whether more history rows may exist (window is full and under the cap). */
  get canLoadMoreHistory(): boolean {
    return this.historyLimit < HISTORY_MAX && this.history.length >= this.historyLimit;
  }

  /** Open a history row in a new tab. The history LIST carries a 16 KiB
   *  preview of big statements (`statement_len` set); fetch the full text by
   *  id first so a re-run never runs the clipped preview. */
  async openHistory(h: DbHistoryEntry): Promise<void> {
    if (h.statement_len === undefined || h.statement_len <= h.statement.length) {
      this.host.newTab(h.statement);
      return;
    }
    const id = this.host.selectedConnId();
    if (!id) return;
    try {
      const full = await api.get<DbHistoryEntry>(
        `${this.connBase(id)}/history/${encodeURIComponent(h.id)}`,
      );
      if (this.host.selectedConnId() !== id) return; // switched connection meanwhile
      this.host.newTab(full.statement);
    } catch (e) {
      toastError('Couldn’t open this history entry', e);
    }
  }
}
