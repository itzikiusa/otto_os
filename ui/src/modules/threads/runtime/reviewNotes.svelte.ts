// Each thread's review notes on its diff (reviewNotes.ts), kept per device in
// localStorage: a note carries its state (draft / sent / resolved / outdated),
// which the daemon-saved runtime draft has no field for (its context items
// are what the next message sends). Bounded: the newest 50 threads, 200 notes
// each.
import { newId } from '../../../lib/id';
import type { ReviewNote } from './reviewNotes';

const KEY = 'otto_threads_review_v1';
const MAX_THREADS = 50;
const MAX_NOTES = 200;
const NONE: readonly ReviewNote[] = [];

function load(): Record<string, ReviewNote[]> {
  try {
    const v = JSON.parse(localStorage.getItem(KEY) ?? '{}') as unknown;
    return v && typeof v === 'object' ? (v as Record<string, ReviewNote[]>) : {};
  } catch {
    return {};
  }
}

class ReviewNotes {
  byThread = $state<Record<string, ReviewNote[]>>(load());

  for(threadId: string | null | undefined): readonly ReviewNote[] {
    return (threadId && this.byThread[threadId]) || NONE;
  }

  add(threadId: string, note: Omit<ReviewNote, 'id' | 'state'>): void {
    const list = [...this.for(threadId), { ...note, id: newId(), state: 'draft' as const }].slice(-MAX_NOTES);
    this.set(threadId, list);
  }

  update(threadId: string, id: string, patch: Partial<ReviewNote>): void {
    this.set(threadId, this.for(threadId).map((n) => (n.id === id ? { ...n, ...patch } : n)));
  }

  remove(threadId: string, id: string): void {
    this.set(threadId, this.for(threadId).filter((n) => n.id !== id));
  }

  clear(threadId: string): void {
    this.set(threadId, []);
  }

  private set(threadId: string, list: ReviewNote[]): void {
    const next = { ...this.byThread };
    delete next[threadId];
    if (list.length) next[threadId] = list;
    const ids = Object.keys(next);
    for (const old of ids.slice(0, Math.max(0, ids.length - MAX_THREADS))) delete next[old];
    this.byThread = next;
    try {
      localStorage.setItem(KEY, JSON.stringify(next));
    } catch {
      /* private mode: kept for this session */
    }
  }
}

export const reviewNotes = new ReviewNotes();
