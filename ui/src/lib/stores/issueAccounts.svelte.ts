// Issue-tracker accounts (Jira / Linear / Confluence credentials), cached for
// every picker that offers "which account?" — Product's import / publish /
// search dialogs, the agents' Jira picker, git Focus and finding actions.
// Settings → Issue accounts is the one writer and calls `load(true)` (or the
// local `upsert` / `remove`) after a change so every consumer sees it.
// Accounts are PER USER, so the cache is keyed by the signed-in identity
// (`auth.me.id`): sign-out, sign-in as someone else and impersonation start /
// stop all change it without a page reload, and a cache warmed by the previous
// identity must never be offered to the next (ui-correctness-4-03).

import { listIssueAccounts } from '../api/issues';
import type { IssueAccount } from '../api/types';
import { loadErrorText } from '../loadError';
import { auth } from './auth.svelte';

class IssueAccountsStore {
  list = $state<IssueAccount[]>([]);
  loading = $state(false);
  /** Last load failure (human text) — render inline with Retry, never as "no accounts". */
  error = $state<string | null>(null);
  /** A load has succeeded at least once (the cache is warm). */
  loaded = $state(false);
  private inflight: Promise<IssueAccount[]> | null = null;
  /** The identity the cache (and any in-flight load) belongs to. */
  private owner: string | null = null;

  /** Drop the cache: nothing loaded, no error, no shared in-flight request. */
  reset(): void {
    this.list = [];
    this.loaded = false;
    this.loading = false;
    this.error = null;
    this.inflight = null;
  }

  /** The accounts, from cache unless `force` (or nothing loaded yet, or the
   *  signed-in identity changed). Shares one in-flight request between callers.
   *  Rejects on failure (and records `error`), so a caller can show its own
   *  message. */
  load(force = false): Promise<IssueAccount[]> {
    const who = auth.me?.id ?? null;
    if (who !== this.owner) {
      this.reset();
      this.owner = who;
    }
    if (this.loaded && !force) return Promise.resolve(this.list);
    if (this.inflight) return this.inflight;
    this.loading = true;
    this.error = null;
    const run = listIssueAccounts()
      .then((rows) => {
        // A load that started under the previous identity never fills the
        // cache of the next one.
        if (this.inflight === run) {
          this.list = rows;
          this.loaded = true;
        }
        return rows;
      })
      .catch((e: unknown) => {
        if (this.inflight === run) this.error = loadErrorText(e);
        throw e;
      })
      .finally(() => {
        if (this.inflight === run) {
          this.loading = false;
          this.inflight = null;
        }
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
