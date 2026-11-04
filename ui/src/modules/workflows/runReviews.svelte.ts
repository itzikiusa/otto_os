// The reviews a run's review_run steps launched, shared by the Steps list
// (what a running step is waiting on) and the Agents tab (per-repo agent
// rows). One ref-counted poller per review-id set: both views mounting for the
// same run never double the `/reviews/{id}` traffic. Event-fed like the rest
// of the run view: `review_changed` re-reads a due review; running reviews are
// due every tick, finished ones (a retry can revive them) every 30 s.
import { untrack } from 'svelte';
import { api } from '../../lib/api/client';
import { mapLimit } from '../../lib/poll';
import { liveQuery, type LiveEvent } from '../../lib/live';
import type { Review } from '../../lib/api/types';

const TERMINAL_RECHECK_MS = 30_000;

class RunReviewsStore {
  reviews = $state<Record<string, Review>>({});
  private watchers = new Map<string, { count: number; stop: () => void }>();

  /** Keep `ids` fresh until the returned release is called. */
  watch(ids: string[]): () => void {
    const key = [...new Set(ids)].sort().join(',');
    if (!key) return () => {};
    const existing = this.watchers.get(key);
    if (existing) {
      existing.count += 1;
    } else {
      this.watchers.set(key, { count: 1, stop: this.poll(key.split(',')) });
    }
    return () => {
      const w = this.watchers.get(key);
      if (!w) return;
      w.count -= 1;
      if (w.count <= 0) {
        w.stop();
        this.watchers.delete(key);
      }
    };
  }

  private poll(ids: string[]): () => void {
    const fetchedAt = new Map<string, number>();
    const lastJson = new Map<string, string>();
    const onEvent = (ev: LiveEvent): boolean => {
      const id = ev.review_id as string;
      if (!ids.includes(id)) return false;
      fetchedAt.delete(id);
      return true;
    };
    const poller = liveQuery({ on: ['review_changed'], match: onEvent, fallbackMs: 2000, safetyMs: 30_000, run: async (signal) => {
      const now = Date.now();
      const due = ids.filter((id) => {
        const r = untrack(() => this.reviews[id]);
        if (!r || r.status === 'running') return true;
        return now - (fetchedAt.get(id) ?? 0) >= TERMINAL_RECHECK_MS;
      });
      if (!due.length) return;
      const results = await mapLimit(due, 3, async (id) => {
        try { return await api.bg.get<Review>(`/reviews/${id}`, signal); } catch { return null; }
      });
      if (signal.aborted) return;
      for (const review of results) {
        if (!review) continue;
        fetchedAt.set(review.id, Date.now());
        // Assign only a changed review: an unchanged tick re-renders nothing.
        const json = JSON.stringify(review);
        if (lastJson.get(review.id) === json) continue;
        lastJson.set(review.id, json);
        this.reviews[review.id] = review;
      }
    } });
    return () => poller.stop();
  }
}

export const runReviews = new RunReviewsStore();
