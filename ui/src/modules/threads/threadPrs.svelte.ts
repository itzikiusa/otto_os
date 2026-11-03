// The open pull request for a repo branch (the right panel's "Pull request"
// surface, and whether its launcher row applies), cached per repo:branch.
// One `GET /repos/{id}/prs?state=open` per key; `forget` re-reads.
import { gitApi } from '../../lib/api/git';
import type { PrSummary, Repo } from '../../lib/api/types';
import { forgeLabel } from './threadsHelpers';

class ThreadPrs {
  private byKey = $state<Record<string, PrSummary | null>>({});
  private inflight = new Set<string>();

  /** The open PR from `branch`, null when none (or unknown yet — see `checked`). */
  prFor(repo: Repo | null, branch: string | null): PrSummary | null {
    if (!repo || !branch || !forgeLabel(repo)) return null;
    const key = `${repo.id}:${branch}`;
    if (key in this.byKey) return this.byKey[key];
    queueMicrotask(() => void this.load(repo.id, branch, key));
    return null;
  }

  checked(repo: Repo | null, branch: string | null): boolean {
    return !repo || !branch || !forgeLabel(repo) || `${repo.id}:${branch}` in this.byKey;
  }

  forget(repo: Repo | null, branch: string | null): void {
    if (!repo || !branch) return;
    const next = { ...this.byKey };
    delete next[`${repo.id}:${branch}`];
    this.byKey = next;
  }

  private async load(repoId: string, branch: string, key: string): Promise<void> {
    if (key in this.byKey || this.inflight.has(key)) return;
    this.inflight.add(key);
    let pr: PrSummary | null = null;
    try {
      const r = await gitApi.prs.list(repoId, 'open', 1, 50);
      pr = r.items.find((p) => p.source_branch === branch) ?? null;
    } catch {
      pr = null;
    } finally {
      this.inflight.delete(key);
    }
    this.byKey = { ...this.byKey, [key]: pr };
  }
}

export const threadPrs = new ThreadPrs();
