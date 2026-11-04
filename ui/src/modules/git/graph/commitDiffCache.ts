// The commit-detail diff pane's module-level state (split out of
// graph-commit-diff.svelte): one summary / patch cache for every graph
// instance and tab, and the click-time summary prefetch.

import type { DiffResp, FileDiff, Hunk as H } from '../../../lib/api/types';
import { WeightedLru, diffKey } from '../graph-diff-cache';
import { api } from '../../../lib/api/client';

// The diff contract's summary-mode fields (`hunks_omitted`, `too_large`,
// `total_*`) are all optional, so an older daemon's full response fits too.
export type SumFile = FileDiff;
export type SumResp = DiffResp;

export type Body =
  | { kind: 'loading' }
  | { kind: 'ready'; hunks: H[]; rows: number }
  | { kind: 'too_large'; lines: number; hard: boolean }
  | { kind: 'error'; message: string };

// Module scope = one cache for every graph instance / tab (a commit's diff is
// immutable per sha). Summaries weigh their file count plus any inline lines
// (an old daemon that ignores `summary=true`); file patches weigh their rows.
export const summaryCache = new WeightedLru<SumResp>(20_000, (r) => {
  let w = r.files.length;
  for (const f of r.files) for (const h of f.hunks) w += h.lines.length;
  return w;
});
export const bodyCache = new WeightedLru<Body>(60_000, (b) => (b.kind === 'ready' ? b.rows : 0) + 1);

/** Summary requests started by `prefetchCommitSummary`, joined by the pane
 *  when it mounts for the same commit. */
export const summaryFlights = new Map<string, Promise<SumResp>>();

export function summaryUrl(repoId: string, sha: string): string {
  return `/repos/${repoId}/diff?target=${encodeURIComponent(`commit:${sha}`)}&summary=true`;
}

/** Warm the file list of `sha` while the pane is still held back (the
 *  graph's branch click waits out the double-click window before mounting
 *  the diff): the summary is one small `--numstat` read, so by the time the
 *  window closes it has usually landed and the list paints from cache in
 *  the same frame. Returns an abort for the double-click (= checkout) case. */
export function prefetchCommitSummary(repoId: string, sha: string): () => void {
  const key = diffKey(repoId, sha);
  if (summaryCache.get(key) || summaryFlights.has(key)) return () => {};
  const ctl = new AbortController();
  const p = api
    .get<SumResp>(summaryUrl(repoId, sha), ctl.signal)
    .then((r) => {
      summaryCache.set(key, r);
      return r;
    })
    .finally(() => {
      if (summaryFlights.get(key) === p) summaryFlights.delete(key);
    });
  summaryFlights.set(key, p);
  p.catch(() => {}); // a joiner handles failures; an unjoined abort is expected
  return () => ctl.abort();
}
