// Stale-while-revalidate cache for the PR list and PR detail views (G1).
// PrList refetched on every mount / state-chip switch and PrDetail on every
// open — each one a forge round-trip (rate-limited on GitHub). Now a view
// renders the last answer instantly and revalidates in the background only
// once it is older than FRESH_MS; the daemon's ETag cache makes that
// revalidation a cheap 304. Writes the user makes (approve/decline/merge/
// edit/comment) invalidate the affected keys so the next read is real.
import type { PrCommit, PrDetail, PrListResp } from '../../lib/api/types';

/** Served without any request while younger than this. */
export const PR_FRESH_MS = 45_000;
/** Older entries are still painted (marked stale) up to this age, then dropped. */
export const PR_MAX_AGE_MS = 10 * 60_000;

export interface CacheHit<T> {
  value: T;
  /** True when younger than `fresh` — the caller may skip revalidating. */
  fresh: boolean;
}

export class SwrCache<T> {
  #map = new Map<string, { value: T; at: number }>();
  readonly max: number;
  readonly fresh: number;
  readonly maxAge: number;
  readonly now: () => number;
  constructor(max: number, fresh = PR_FRESH_MS, maxAge = PR_MAX_AGE_MS, now: () => number = () => Date.now()) {
    this.max = max;
    this.fresh = fresh;
    this.maxAge = maxAge;
    this.now = now;
  }

  get(key: string): CacheHit<T> | undefined {
    const e = this.#map.get(key);
    if (!e) return undefined;
    const age = this.now() - e.at;
    if (age > this.maxAge) {
      this.#map.delete(key);
      return undefined;
    }
    // LRU touch.
    this.#map.delete(key);
    this.#map.set(key, e);
    return { value: e.value, fresh: age < this.fresh };
  }

  set(key: string, value: T): void {
    this.#map.delete(key);
    this.#map.set(key, { value, at: this.now() });
    while (this.#map.size > this.max) {
      const oldest = this.#map.keys().next().value;
      if (oldest === undefined) break;
      this.#map.delete(oldest);
    }
  }

  delete(key: string): void {
    this.#map.delete(key);
  }

  /** Drop every key starting with `prefix`. */
  invalidate(prefix: string): void {
    for (const k of [...this.#map.keys()]) if (k.startsWith(prefix)) this.#map.delete(k);
  }

  get size(): number {
    return this.#map.size;
  }
}

export const prListCache = new SwrCache<PrListResp>(64);
export const prDetailCache = new SwrCache<PrDetail>(64);
export const prCommitsCache = new SwrCache<PrCommit[]>(64);

export const prListKey = (repoId: string, state: string, page: number): string => `${repoId}|${state}|${page}`;
export const prKey = (repoId: string, num: number): string => `${repoId}|${num}`;

/** After a write to PR `num`: its detail/commits and every list of the repo
 *  (state chips, CI, title) are stale. */
export function invalidatePr(repoId: string, num?: number): void {
  prListCache.invalidate(`${repoId}|`);
  if (num === undefined) {
    prDetailCache.invalidate(`${repoId}|`);
    prCommitsCache.invalidate(`${repoId}|`);
  } else {
    prDetailCache.delete(prKey(repoId, num));
    prCommitsCache.delete(prKey(repoId, num));
  }
}
