// Persisted "Viewed" marks for a PR's Code tab. Each mark records the file's
// stat signature at the time it was ticked, so a push that changes the file
// drops the mark (GitHub's "viewed until it changes" rule) while an untouched
// file stays viewed across reloads, tabs and app restarts. Storage is
// best-effort: a blocked/full localStorage just means marks don't persist.
import type { FileDiff } from '../../lib/api/types';

/** Change signature of one file — what a mark is pinned to. */
export function viewSig(f: Pick<FileDiff, 'status' | 'added' | 'deleted'>): string {
  return `${f.status ?? ''}:${f.added ?? ''}:${f.deleted ?? ''}`;
}

const PREFIX = 'otto_viewed:';
/** Oldest keys are evicted past this many stored PR diffs. */
const MAX_KEYS = 200;

function read(key: string): Record<string, string> {
  try {
    const raw = localStorage.getItem(PREFIX + key);
    if (!raw) return {};
    const v: unknown = JSON.parse(raw);
    return v && typeof v === 'object' && !Array.isArray(v) ? (v as Record<string, string>) : {};
  } catch {
    return {};
  }
}

/** The stored marks of `key` that still match the files in `files`. */
export function loadViewed(key: string, files: readonly FileDiff[]): Set<string> {
  const stored = read(key);
  const out = new Set<string>();
  for (const f of files) {
    const sig = stored[f.path];
    if (sig !== undefined && sig === viewSig(f)) out.add(f.path);
  }
  return out;
}

/** Persist `viewed` (paths) for `key`, pinned to each file's current signature. */
export function saveViewed(key: string, files: readonly FileDiff[], viewed: ReadonlySet<string>): void {
  try {
    const rec: Record<string, string> = {};
    for (const f of files) if (viewed.has(f.path)) rec[f.path] = viewSig(f);
    const k = PREFIX + key;
    if (Object.keys(rec).length === 0) localStorage.removeItem(k);
    else {
      // Re-insert so this key is the newest for eviction order.
      localStorage.removeItem(k);
      localStorage.setItem(k, JSON.stringify(rec));
    }
    evict();
  } catch {
    /* blocked storage: marks just don't persist */
  }
}

function evict(): void {
  const keys: string[] = [];
  for (let i = 0; i < localStorage.length; i++) {
    const k = localStorage.key(i);
    if (k?.startsWith(PREFIX)) keys.push(k);
  }
  // localStorage has no insertion order guarantee; drop arbitrary extras.
  for (let i = 0; i < keys.length - MAX_KEYS; i++) localStorage.removeItem(keys[i]);
}
