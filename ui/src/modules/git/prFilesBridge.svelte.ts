// The PR workspace's "Files" panel (under the PR list, PrFilesPanel) shows
// the files of the PR in front and jumps into its Code tab. The files, their
// Viewed marks and the jump all belong to the DiffViewer inside PrDetail —
// this bridge lets that viewer publish them and the panel drive them, without
// threading props through PrWorkspace → PrDetail → DiffViewer and back.
//
// One feed at a time (the workspace mounts only the PR in front). A publish is
// tagged with its owner, so a viewer being torn down never clears the feed a
// newer viewer already published (a commit pick swaps the viewer).
import { tick } from 'svelte';
import type { FileDiff } from '../../lib/api/types';

export interface PrFilesFeed {
  /** The PR's Viewed-marks storage key (prFilesKey) — one per PR. */
  key: string;
  /** The files the viewer shows (test files hidden when it hides them). */
  files: readonly FileDiff[];
  viewed: ReadonlySet<string>;
  commentCounts: ReadonlyMap<string, number>;
  /** The file at the top of the diff's viewport (null = unknown). */
  current: string | null;
  jump: (path: string) => Promise<void>;
  toggleViewed: (path: string) => void;
}

/** The feed key of PR `n`, and the all-commits viewer's `viewedStorageKey`. */
export const prFilesKey = (repoId: string, n: number): string => `otto_pr_viewed:${repoId}#${n}`;
/** The Viewed-marks key of ONE commit's view of PR `n`: a record of its own,
 *  so ticking a file there never rewrites the all-commits marks. */
export const prCommitViewedKey = (repoId: string, n: number, sha: string): string => `${prFilesKey(repoId, n)}@${sha}`;
/** The feed key a viewer publishes under: its PR's, for a commit view too
 *  (the Files panel follows whichever view of the PR is in front). */
export const prFeedKey = (viewedKey: string): string => viewedKey.replace(/@[^@#]*$/, '');

class PrFilesBridge {
  feed = $state.raw<PrFilesFeed | null>(null);
  #owner: object | null = null;
  #reveal = new Map<string, () => void>();

  publish(owner: object, feed: PrFilesFeed): void {
    this.#owner = owner;
    this.feed = feed;
  }

  clear(owner: object): void {
    if (this.#owner !== owner) return;
    this.#owner = null;
    this.feed = null;
  }

  /** The PR detail's "bring the Code tab to the front" (returns the unregister). */
  onReveal(key: string, fn: () => void): () => void {
    this.#reveal.set(key, fn);
    return () => {
      if (this.#reveal.get(key) === fn) this.#reveal.delete(key);
    };
  }

  /** Show the Code tab, then jump to `path` once the diff is laid out. */
  async open(key: string, path: string): Promise<void> {
    this.#reveal.get(key)?.();
    await tick();
    await new Promise<void>((r) => requestAnimationFrame(() => r()));
    const f = this.feed;
    if (f?.key === key) await f.jump(path);
  }
}

export const prFilesBridge = new PrFilesBridge();

/** Called from a component's init: publish `get()` while it is non-null,
 *  and clear it when the component goes away. */
export function publishPrFiles(get: () => PrFilesFeed | null): void {
  const owner = {};
  $effect(() => {
    const f = get();
    if (f) prFilesBridge.publish(owner, f);
    else prFilesBridge.clear(owner);
  });
  $effect(() => () => prFilesBridge.clear(owner));
}
