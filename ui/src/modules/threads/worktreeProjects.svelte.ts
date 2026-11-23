// A worktree belongs to its main repository. Each linked worktree folder maps
// to its main checkout (and branch), read with `GET /repos/{id}/worktrees`
// once per repo a thread works in — never per row — and remembered across
// reloads, so project names, badges, groups, the filter, ⌘K and project
// scripts all resolve a worktree thread through its main checkout.
import { api } from '../../lib/api/client';
import type { Repo, WorktreeInfo } from '../../lib/api/types';
import { lsGet, lsSet } from '../../lib/storage';

const KEY = 'otto.threads.worktrees';
const norm = (p: string | null | undefined): string => (p ?? '').trim().replace(/\/+$/, '');
/** The daemon writes `/private/var/…`, a thread's cwd may say `/var/…`. */
const canon = (p: string): string => p.replace(/^\/private(?=\/(?:var|tmp)\/)/, '');

type Linked = { main: string; branch: string | null };

function load(): Record<string, Linked> {
  try {
    return JSON.parse(lsGet(KEY) ?? '{}') as Record<string, Linked>;
  } catch {
    return {};
  }
}

class WorktreeProjects {
  /** canonical linked-worktree path → its main checkout and branch. */
  private linked = $state<Record<string, Linked>>(load());
  private asked = new Set<string>();

  /** The linked worktree `cwd` is in (its main checkout, branch), else null. */
  of(cwd: string | null | undefined): Linked | null {
    const c = canon(norm(cwd));
    if (!c) return null;
    for (const wt in this.linked) if (c === wt || c.startsWith(`${wt}/`)) return this.linked[wt];
    return null;
  }

  /** A thread folder's project: its main checkout when in a worktree (written
   *  the way the folder is), else the folder itself. */
  root(cwd: string | null | undefined): string {
    const c = norm(cwd);
    const main = this.of(c)?.main;
    if (!main) return c;
    return c.startsWith('/private/') ? main : canon(main);
  }

  /** Learn a repo's worktrees (once per repo, or again after `forget`). */
  ensure(repo: Repo): void {
    if (this.asked.has(repo.id)) return;
    this.asked.add(repo.id);
    void api
      .get<WorktreeInfo[]>(`/repos/${encodeURIComponent(repo.id)}/worktrees`)
      .then((list) => {
        const main = list.find((w) => w.is_main);
        if (!main) return;
        const next = Object.fromEntries(Object.entries(this.linked).filter(([, v]) => v.main !== main.path));
        for (const w of list) if (!w.is_main && !w.prunable) next[canon(norm(w.path))] = { main: main.path, branch: w.branch };
        if (JSON.stringify(next) === JSON.stringify(this.linked)) return;
        this.linked = next;
        lsSet(KEY, JSON.stringify(next));
      })
      .catch(() => this.asked.delete(repo.id));
  }

  /** A worktree Otto just made: known at once, confirmed by the next read. */
  note(repo: Repo, wt: WorktreeInfo): void {
    this.linked = { ...this.linked, [canon(norm(wt.path))]: { main: repo.path, branch: wt.branch } };
    lsSet(KEY, JSON.stringify(this.linked));
    this.asked.delete(repo.id);
  }
}

export const worktreeProjects = new WorktreeProjects();
