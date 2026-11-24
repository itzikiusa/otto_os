// The open pull request for a repo branch — Git's commit card ("Open pull
// request"), Threads' details card and "Pull request" surface — cached per
// repo:branch.
// One `GET /repos/{id}/prs?state=open` per key; `forget` re-reads.
import { api } from '../api/client';
import { gitApi } from '../api/git';
import type { GitAccount, GitProviderKind, PrSummary, Repo } from '../api/types';
import { forgeLabel } from '../gitNextStep';

const ACCOUNTS_TTL_MS = 60_000;
const FAILED_RETRY_MS = 30_000;

class BranchPrs {
  private byKey = $state<Record<string, PrSummary | null>>({});
  /** Keys whose repo has no account and the person none for its forge:
   *  "no PR" there means "can't look", which the surface says. */
  private unlinked = $state<Record<string, boolean>>({});
  private inflight = new Set<string>();
  private accountProviders: Set<GitProviderKind> | null = null;
  private accountsFetchedAt = 0;
  private accountsInflight: Promise<Set<GitProviderKind>> | null = null;
  private failedAt = new Map<string, number>();

  /** The open PR from `branch`, null when none (or unknown yet — see `checked`). */
  prFor(repo: Repo | null, branch: string | null): PrSummary | null {
    if (!repo || !branch || !repo.provider || !forgeLabel(repo)) return null;
    const key = `${repo.id}:${branch}`;
    if (key in this.byKey) return this.byKey[key];
    // A failed lookup is retried after a short backoff instead of being memoized as "no PR".
    if (Date.now() - (this.failedAt.get(key) ?? -Infinity) < FAILED_RETRY_MS) return null;
    queueMicrotask(() => void this.load(repo, branch, key));
    return null;
  }

  checked(repo: Repo | null, branch: string | null): boolean {
    return !repo || !branch || !repo.provider || !forgeLabel(repo) || `${repo.id}:${branch}` in this.byKey;
  }

  /** Whether `repo`'s PRs can't be read because no git account covers its
   *  forge (known once `checked`). */
  needsAccount(repo: Repo | null, branch: string | null): boolean {
    return !!repo && !!branch && !!this.unlinked[`${repo.id}:${branch}`];
  }

  forget(repo: Repo | null, branch: string | null): void {
    this.accountProviders = null;
    this.accountsFetchedAt = 0;
    this.accountsInflight = null;
    if (!repo || !branch) return;
    const next = { ...this.byKey };
    delete next[`${repo.id}:${branch}`];
    this.byKey = next;
    if (this.unlinked[`${repo.id}:${branch}`]) {
      const { [`${repo.id}:${branch}`]: _gone, ...rest } = this.unlinked;
      this.unlinked = rest;
    }
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
    let unlinked = false;
    try {
      if (!repo.provider) {
        pr = null;
      } else if (!repo.git_account_id) {
        const providers = await this.ensureAccountProviders();
        if (providers.has(repo.provider)) {
          const r = await gitApi.prs.list(repo.id, 'open', 1, 50);
          pr = r.items.find((p) => p.source_branch === branch) ?? null;
        } else {
          unlinked = true;
        }
      } else {
        const r = await gitApi.prs.list(repo.id, 'open', 1, 50);
        pr = r.items.find((p) => p.source_branch === branch) ?? null;
      }
    } catch {
      this.failedAt.set(key, Date.now());
      return;
    } finally {
      this.inflight.delete(key);
    }
    this.failedAt.delete(key);
    if (unlinked) this.unlinked = { ...this.unlinked, [key]: true };
    this.byKey = { ...this.byKey, [key]: pr };
  }
}

export const branchPrs = new BranchPrs();
