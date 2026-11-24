// Pure PR-list grouping & candidate filtering helpers for FocusView.svelte.
import type { PrSummary, Repo } from '../../lib/api/types';

export interface FocusPrRow {
  repo: Repo;
  pr: PrSummary;
}

/** Repos that can answer a PR listing: a recognized forge AND a linked git
 *  account (an unlinked repo only fails with "no git account"). */
export function prCandidateRepos(allRepos: Repo[]): Repo[] {
  return allRepos.filter((r) => !!r.git_account_id && !!(r.provider ?? (r.forge && r.forge !== 'unrecognized' ? r.forge : null)));
}

/** Group PR rows by forge, preserving insertion order. */
export function groupPrRowsByForge(rows: FocusPrRow[]): [string, FocusPrRow[]][] {
  const groups = new Map<string, FocusPrRow[]>();
  for (const row of rows) {
    const forge = (row.repo.forge as string) ?? 'other';
    const list = groups.get(forge);
    if (list) list.push(row);
    else groups.set(forge, [row]);
  }
  return [...groups.entries()];
}

// Focus's PR rows outlive the view for a minute: reopening Focus re-reads
// nothing inside the window; its Refresh does.
const PR_CACHE_MS = 60_000;
const prCache = { at: 0, rows: [] as FocusPrRow[], errors: 0, handles: new Set<string>() };
/** The last PR read when it is under a minute old, else null. */
export const freshPrs = (): typeof prCache | null => (Date.now() - prCache.at < PR_CACHE_MS ? prCache : null);
export const savePrs = (rows: FocusPrRow[], errors: number, handles: Set<string>): void =>
  void Object.assign(prCache, { at: Date.now(), rows, errors, handles });
