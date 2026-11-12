// Merge-strategy labels per forge (pure — unit/prMergeGate.test.ts). The wire
// value is provider-neutral, but what `rebase` does is not: Bitbucket Cloud
// has no rebase-merge and runs a fast-forward instead (providers/bitbucket.rs
// `merge_body`), which fails outright when the branch is behind the target.
import type { MergeStrategy } from '../../lib/api/types';

export interface StrategyOption {
  value: MergeStrategy;
  label: string;
  hint: string;
}

export function mergeStrategies(provider: string | null | undefined): StrategyOption[] {
  if (provider === 'bitbucket') {
    return [
      { value: 'merge', label: 'Merge commit', hint: 'Adds a merge commit on the target branch' },
      { value: 'squash', label: 'Squash', hint: 'Combines every commit into one on the target branch' },
      { value: 'rebase', label: 'Fast-forward', hint: 'Moves the target branch to the source head; fails if the target has moved on' },
    ];
  }
  return [
    { value: 'merge', label: 'Merge commit', hint: 'Adds a merge commit on the target branch' },
    { value: 'squash', label: 'Squash', hint: 'Combines every commit into one on the target branch' },
    { value: 'rebase', label: 'Rebase', hint: 'Replays the commits onto the target branch' },
  ];
}

/** The ⋯ menu entry for a strategy ("Squash and merge", "Fast-forward merge"). */
export function strategyMenuLabel(provider: string | null | undefined, value: MergeStrategy): string {
  if (value === 'merge') return 'Merge';
  if (value === 'squash') return 'Squash and merge';
  return provider === 'bitbucket' ? 'Fast-forward merge' : 'Rebase and merge';
}
