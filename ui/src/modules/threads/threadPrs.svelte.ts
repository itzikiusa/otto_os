// The open pull request for a repo branch (the right panel's "Pull request"
// surface, and whether its launcher row applies), cached per repo:branch.
// One `GET /repos/{id}/prs?state=open` per key; `forget` re-reads.
import { api } from '../../lib/api/client';
import { gitApi } from '../../lib/api/git';
import type { GitAccount, GitProviderKind, PrSummary, Repo } from '../../lib/api/types';
import { forgeLabel } from './threadsHelpers';

const ACCOUNTS_TTL_MS = 60_000;

class ThreadPrs {
  private byKey = $state<Record<string, PrSummary | null>>({});
  private inflight = new Set<string>();
  private accountProviders: Set<GitProviderKind> | null = null;
  private accountsFetchedAt = 0;
  private accountsInflight: Promise<Set<GitProviderKind>> | null = null;

  /** The open PR from `branch`, null when none (or unknown yet — see `checked`). */
  prFor(repo: Repo | null, branch: string | null): PrSummary | null {
    if (!repo || !branch || !repo.provider || !forgeLabel(repo)) return null;
    const key = `${repo.id}:${branch}`;
    if (key in this.byKey) return this.byKey[key];
    queueMicrotask(() => void this.load(repo, branch, key));
    return null;
  }

  checked(repo: Repo | null, branch: string | null): boolean {
    return !repo || !branch || !repo.provider || !forgeLabel(repo) || `${repo.id}:${branch}` in this.byKey;
  }

  forget(repo: Repo | null, branch: string | null): void {
    this.accountProviders = null;
    this.accountsFetchedAt = 0;
    this.accountsInflight = null;
    if (!repo || !branch) return;
    const next = { ...this.byKey };
    delete next[`${repo.id}:${branch}`];
    this.byKey = next;
  }

  private async ensureAccountProviders(): Promise<Set<GitProviderKind>> {
    const now = Date.now();
    if (this.accountProviders && now - this.accountsFetchedAt < ACCOUNTS_TTL_MS) {
      return this.accountProviders;
    }
    if (this.accountsInflight) return this.accountsInflight;
    const task = api
      .get<GitAccount[]>('/git/accounts')
      .then((list) => {
        const set = new Set<GitProviderKind>();
        if (Array.isArray(list)) {
          for (let i = 0; i < list.length; i++) {
            if (list[i]?.provider) set.add(list[i].provider);
          }
        }
        this.accountProviders = set;
        this.accountsFetchedAt = Date.now();
        return set;
      })
      .catch(() => {
        const empty = new Set<GitProviderKind>();
        this.accountProviders = empty;
        this.accountsFetchedAt = Date.now();
        return empty;
      })
      .finally(() => {
        this.accountsInflight = null;
      });
    this.accountsInflight = task;
    return task;
  }

  private async load(repo: Repo, branch: string, key: string): Promise<void> {
    if (key in this.byKey || this.inflight.has(key)) return;
    this.inflight.add(key);
    let pr: PrSummary | null = null;
    try {
      if (!repo.provider) {
        pr = null;
      } else if (!repo.git_account_id) {
        const providers = await this.ensureAccountProviders();
        if (providers.has(repo.provider)) {
          const r = await gitApi.prs.list(repo.id, 'open', 1, 50);
          pr = r.items.find((p) => p.source_branch === branch) ?? null;
        }
      } else {
        const r = await gitApi.prs.list(repo.id, 'open', 1, 50);
        pr = r.items.find((p) => p.source_branch === branch) ?? null;
      }
    } catch {
      pr = null;
    } finally {
      this.inflight.delete(key);
    }
    this.byKey = { ...this.byKey, [key]: pr };
  }
}

export const threadPrs = new ThreadPrs();
