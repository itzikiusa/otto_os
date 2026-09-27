// Per-repo snapshot of what the Git graph last loaded, so a remount (repo tab
// switch, sub-tab switch, leaving the conflict resolver) paints the previous
// graph IMMEDIATELY and then revalidates through the cheap `/refs` fingerprint
// (stale-while-revalidate) — instead of re-reading `log --all -n 10000`,
// stashes, worktrees and submodules from scratch every time.
//
// Plain module state, not runes: the graph copies a snapshot into its own
// `$state.raw` on mount and writes the latest values back as they change.
import type { CommitInfo, RefsResp, StashInfo, SubmoduleInfo, WorktreeInfo } from '../../lib/api/types';

export interface GraphSnapshot {
  refs: RefsResp;
  commits: CommitInfo[];
  stashes: StashInfo[];
  stashesKnown: boolean;
  worktrees: WorktreeInfo[];
  submodules: SubmoduleInfo[];
  hasMore: boolean;
  skipCursor: number;
}

/** Repos kept. Each snapshot holds at most what the graph had paged in
 *  (10k commits ≈ a few MB), so a handful of open tabs stays cheap. */
const MAX_REPOS = 4;
const snapshots = new Map<string, GraphSnapshot>();

export const graphCache = {
  get(repoId: string): GraphSnapshot | undefined {
    const s = snapshots.get(repoId);
    if (s) {
      snapshots.delete(repoId);
      snapshots.set(repoId, s);
    }
    return s;
  },
  set(repoId: string, snap: GraphSnapshot): void {
    snapshots.delete(repoId);
    snapshots.set(repoId, snap);
    while (snapshots.size > MAX_REPOS) {
      const oldest = snapshots.keys().next().value!;
      snapshots.delete(oldest);
    }
  },
  drop(repoId: string): void {
    snapshots.delete(repoId);
  },
};
