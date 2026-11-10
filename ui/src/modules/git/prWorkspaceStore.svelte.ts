// Open pull-request tabs of the PR workspace, per repository.
//
// The Pull requests tab of a repo is a split view: the PR list on the start
// side, the open PRs as tabs on the other (T3-style). Which PRs are open, and
// which one is in front, outlives a reload and a repo-tab switch, so it lives
// here (persisted per repo) rather than in the component. A PR deep link
// (#/git/<repo>/pr/<n>) opens its PR here; GitPage then drops the route.
import { prTabsAfterClose, prTabsAfterOpen, type PrTabs } from './prTabs';

const KEY = 'otto_pr_tabs_v1';
/** Tabs kept per repo; the oldest background tab is dropped past this. */
export const MAX_PR_TABS = 12;

function load(): Record<string, PrTabs> {
  try {
    const raw = JSON.parse(localStorage.getItem(KEY) ?? '{}') as Record<string, PrTabs>;
    const out: Record<string, PrTabs> = {};
    for (const [repo, t] of Object.entries(raw ?? {})) {
      if (!t || !Array.isArray(t.open)) continue;
      const open = t.open.filter((n) => Number.isInteger(n) && n > 0);
      out[repo] = { open, active: open.includes(t.active as number) ? t.active : (open[0] ?? null) };
    }
    return out;
  } catch {
    return {};
  }
}

class PrWorkspaceStore {
  byRepo: Record<string, PrTabs> = $state(load());

  tabs(repoId: string): PrTabs {
    return this.byRepo[repoId] ?? { open: [], active: null };
  }

  #set(repoId: string, t: PrTabs): void {
    this.byRepo = { ...this.byRepo, [repoId]: t };
    try {
      localStorage.setItem(KEY, JSON.stringify(this.byRepo));
    } catch {
      /* blocked storage: tabs just don't persist */
    }
  }

  /** Open PR `n` (a background open keeps the current tab in front). */
  open(repoId: string, n: number, background = false): void {
    this.#set(repoId, prTabsAfterOpen(this.tabs(repoId), [n], background, MAX_PR_TABS));
  }

  /** Open several at once; the first one comes to the front. */
  openMany(repoId: string, ns: number[]): void {
    if (ns.length === 0) return;
    this.#set(repoId, prTabsAfterOpen(this.tabs(repoId), ns, false, MAX_PR_TABS));
  }

  activate(repoId: string, n: number): void {
    const t = this.tabs(repoId);
    if (t.open.includes(n) && t.active !== n) this.#set(repoId, { ...t, active: n });
  }

  close(repoId: string, n: number): void {
    this.#set(repoId, prTabsAfterClose(this.tabs(repoId), n));
  }

  closeOthers(repoId: string, keep: number): void {
    this.#set(repoId, { open: [keep], active: keep });
  }

  closeAll(repoId: string): void {
    this.#set(repoId, { open: [], active: null });
  }
}

export const prWorkspace = new PrWorkspaceStore();
