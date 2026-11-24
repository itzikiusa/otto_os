// +/− line counts for the WIP lists (split out of WipPanel.svelte): one
// `summary=true` numstat per side, no patches. Refreshed (debounced) on every
// status / live change. Past COUNTS_MAX files the counts are skipped: the
// list must never wait on them, and a failed read just leaves the rows
// without counts.
import { api } from '../../../lib/api/client';
import type { DiffResp } from '../../../lib/api/types';
import type { LineCounts } from './wipTree';

const COUNTS_MAX = 2000;

function countsOf(d: DiffResp): LineCounts {
  const m: LineCounts = new Map();
  for (const f of d.files) {
    m.set(f.path, f.is_binary ? { added: null, deleted: null } : { added: f.added ?? 0, deleted: f.deleted ?? 0 });
  }
  return m;
}

/** `src` is read inside the effect: the repo, how many rows each side has,
 *  and whatever should trigger a re-read (status changes, live edits). */
export function wipLineCounts(src: () => { repoId: string; unstaged: number; staged: number; rev: unknown }) {
  let unstagedCounts = $state.raw<LineCounts | null>(null);
  let stagedCounts = $state.raw<LineCounts | null>(null);
  $effect(() => {
    const { repoId: id, unstaged, staged } = src();
    const wantU = unstaged > 0 && unstaged <= COUNTS_MAX;
    const wantS = staged > 0 && staged <= COUNTS_MAX;
    const ctl = new AbortController();
    const load = (target: 'worktree' | 'staged', set: (m: LineCounts | null) => void): void => {
      void api
        .get<DiffResp>(`/repos/${id}/diff?target=${target}&summary=true`, ctl.signal)
        .then((d) => {
          if (!ctl.signal.aborted) set(countsOf(d));
        })
        .catch(() => {});
    };
    const t = setTimeout(() => {
      if (wantU) load('worktree', (m) => (unstagedCounts = m));
      else unstagedCounts = null;
      if (wantS) load('staged', (m) => (stagedCounts = m));
      else stagedCounts = null;
    }, 250);
    return () => {
      clearTimeout(t);
      ctl.abort();
    };
  });
  const totals = $derived.by(() => {
    let added = 0;
    let deleted = 0;
    for (const m of [unstagedCounts, stagedCounts]) {
      for (const v of m?.values() ?? []) {
        added += v.added ?? 0;
        deleted += v.deleted ?? 0;
      }
    }
    return added + deleted > 0 ? { added, deleted } : null;
  });
  return {
    get unstaged() {
      return unstagedCounts;
    },
    get staged() {
      return stagedCounts;
    },
    get totals() {
      return totals;
    },
  };
}
