// Pure PR-list grouping & candidate filtering helpers for FocusView.svelte.
import type { GitAccount, PrSummary, Repo } from '../../lib/api/types';

export interface FocusPrRow {
  repo: Repo;
  pr: PrSummary;
}

/** Filter registered repos that have a recognized forge and matching account. */
export function prCandidateRepos(allRepos: Repo[], accs: GitAccount[] | null): Repo[] {
  const configuredProviders = accs ? new Set(accs.map((a) => a.provider)) : null;
  return allRepos.filter((r) => {
    const forge = r.provider ?? (r.forge && r.forge !== 'unrecognized' ? r.forge : null);
    return Boolean(forge && (r.git_account_id || (configuredProviders ? configuredProviders.has(forge) : r.provider)));
  });
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
