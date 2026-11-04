// Resolve the git repo behind a thread's folder, even when nobody registered
// it: a folder no registered repo contains goes through the existing
// `POST /workspaces/{id}/repos/detect` (finds the enclosing repo and registers
// it, idempotently), cached per folder. Keyed by the folder as written, so a
// `/var/…` cwd still maps to the daemon's canonical `/private/var/…` repo path.
import { api } from '../../lib/api/client';
import type { Repo } from '../../lib/api/types';
import { git } from '../../lib/stores/git.svelte';
import { ws } from '../../lib/stores/workspace.svelte';
import { repoForCwd } from './threadsHelpers';

const norm = (p: string | null | undefined): string => (p ?? '').trim().replace(/\/+$/, '');

class ThreadRepos {
  /** folder → its repo (null = not inside a git repo). Absent = not asked yet. */
  private byCwd = $state<Record<string, Repo | null>>({});
  private inflight = new Set<string>();

  /** The repo for `cwd`, or null (unknown yet, or not a repo — see `checked`). */
  repoFor(cwd: string | null | undefined, workspaceId?: string | null): Repo | null {
    const c = norm(cwd);
    if (!c) return null;
    const known = repoForCwd(c, [...git.repos, ...git.allRepos]);
    if (known) return known;
    if (c in this.byCwd) return this.byCwd[c];
    const wid = workspaceId ?? ws.currentId ?? ws.scratch?.id ?? null;
    // Never write state from inside the read (a template / $derived).
    queueMicrotask(() => void this.detect(c, wid));
    return null;
  }

  /** Whether `cwd` has been resolved (so a null repo means "not a git repo"). */
  checked(cwd: string | null | undefined): boolean {
    const c = norm(cwd);
    return !c || c in this.byCwd || repoForCwd(c, [...git.repos, ...git.allRepos]) !== null;
  }

  /** Forget cached answers (the sidebar's Refresh), so folders are re-checked. */
  reset(): void {
    this.byCwd = {};
  }

  private async detect(cwd: string, workspaceId: string | null): Promise<void> {
    if (cwd in this.byCwd || this.inflight.has(cwd)) return;
    this.inflight.add(cwd);
    let repo: Repo | null = null;
    try {
      // No workspace to register into: say "not a git repository", never spin.
      if (workspaceId) repo = await api.post<Repo>(`/workspaces/${encodeURIComponent(workspaceId)}/repos/detect`, { path: cwd });
    } catch {
      repo = null; // not inside a git repo (or this workspace can't register one)
    } finally {
      this.inflight.delete(cwd);
    }
    this.byCwd = { ...this.byCwd, [cwd]: repo };
    if (repo) git.ensureStatus(repo.id);
  }
}

export const threadRepos = new ThreadRepos();
