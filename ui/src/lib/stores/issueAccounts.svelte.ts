// Issue-tracker accounts (Jira / Linear / Confluence credentials), cached for
// every picker that offers "which account?" — Product's import / publish /
// search dialogs, the agents' Jira picker, git Focus and finding actions.
// Settings → Issue accounts is the one writer and calls `load(true)` (or the
// local `upsert` / `remove`) after a change so every consumer sees it.

import { listIssueAccounts } from '../api/issues';
import type { IssueAccount } from '../api/types';
import { loadErrorText } from '../loadError';

class IssueAccountsStore {
  list = $state<IssueAccount[]>([]);
  loading = $state(false);
  /** Last load failure (human text) — render inline with Retry, never as "no accounts". */
  error = $state<string | null>(null);
  /** A load has succeeded at least once (the cache is warm). */
  loaded = $state(false);
  private inflight: Promise<IssueAccount[]> | null = null;

  /** The accounts, from cache unless `force` (or nothing loaded yet). Shares
   *  one in-flight request between callers. Rejects on failure (and records
   *  `error`), so a caller can show its own message. */
  load(force = false): Promise<IssueAccount[]> {
    if (this.loaded && !force) return Promise.resolve(this.list);
    if (this.inflight) return this.inflight;
    this.loading = true;
    this.error = null;
    const run = listIssueAccounts()
      .then((rows) => {
        this.list = rows;
        this.loaded = true;
        return rows;
      })
      .catch((e: unknown) => {
        this.error = loadErrorText(e);
        throw e;
      })
      .finally(() => {
        this.loading = false;
        if (this.inflight === run) this.inflight = null;
      });
    this.inflight = run;
    return run;
  }

  /** Reflect a created / updated account without a refetch. */
  upsert(a: IssueAccount): void {
    this.list = this.list.some((x) => x.id === a.id)
      ? this.list.map((x) => (x.id === a.id ? a : x))
      : [...this.list, a];
  }

  /** Reflect a deleted account without a refetch. */
  remove(id: string): void {
    this.list = this.list.filter((x) => x.id !== id);
  }
}

export const issueAccounts = new IssueAccountsStore();
